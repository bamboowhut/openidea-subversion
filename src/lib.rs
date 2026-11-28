use crate::i18n::{tr, trf};
mod backend;
mod i18n;
mod repository;
mod review;
mod shelf;
pub mod svn;
#[cfg(test)]
mod tests;
mod working;

use std::collections::{HashMap, HashSet};

use openidea_plugin::views::{Body, View, ViewEvent, Viewport, Views};
use openidea_plugin::{
    Context, Error, Event, EventKind, FileDecoration, FileKind, Invocation, Plugin, host,
    register_plugin,
};
use serde::Deserialize;
use serde_json::{Value, json};

use backend::{Scope, Session, result, storage_key, strings};
use svn::{Result, RevisionPath};

const VIEW: &str = "subversion";
const PAGE: &str = include_str!("../web/panel.html");
const BOOKMARKS: &str = "repository-bookmarks";

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Operation {
    Refresh,
    ChooseScope,
    ResetScope,
    Authenticate,
    Bookmarks,
    Branches,
    SetBranches,
    Bookmark,
    Unbookmark,
    Browse,
    Checkout,
    Import,
    Share,
    Relocate,
    Compare,
    Copy,
    Switch,
    Merge,
    Recover,
    Diff,
    EditDifference,
    CompareClipboard,
    CreatePatch,
    OpenSource,
    CompareLocal,
    CurrentRevision,
    TargetUrl,
    History,
    Incoming,
    Shelves,
    Shelve,
    Unshelve,
    ShelfDiff,
    EditShelf,
    ImportShelf,
    HistoryDiff,
    Annotate,
    ClearAnnotations,
    Commit,
    Update,
    Add,
    Delete,
    Revert,
    Move,
    Cleanup,
    Lock,
    Unlock,
    Changelist,
    RemoveChangelist,
    Ignore,
    Unignore,
    Properties,
    PropertyDiff,
    PropertySet,
    PropertyDelete,
    Conflict,
    Resolve,
}

#[derive(Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Request {
    op: Operation,
    language_only: bool,
    seq: Option<u32>,
    panel: String,
    project: String,
    path: String,
    paths: Vec<String>,
    url: String,
    to: String,
    revision: String,
    to_revision: String,
    name: String,
    value: String,
    note: String,
    encoding: String,
    cursor: Option<u64>,
    peg: Option<u64>,
    change: Option<RevisionPath>,
    preview: bool,
    keep_locked: bool,
    auto_update: bool,
    force: bool,
    ignore_externals: bool,
    remove_applied: bool,
    merged: Option<String>,
    hash: String,
    hunk: Option<review::DifferenceEdit>,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            op: Operation::Refresh,
            language_only: false,
            seq: None,
            panel: String::new(),
            project: String::new(),
            path: String::new(),
            paths: Vec::new(),
            url: String::new(),
            to: String::new(),
            revision: String::new(),
            to_revision: String::new(),
            name: String::new(),
            value: String::new(),
            note: String::new(),
            encoding: "UTF-8".into(),
            cursor: None,
            peg: None,
            change: None,
            preview: false,
            keep_locked: false,
            auto_update: false,
            force: false,
            ignore_externals: true,
            remove_applied: false,
            merged: None,
            hash: String::new(),
            hunk: None,
        }
    }
}

#[derive(Default)]
struct Subversion {
    handoff: HashMap<String, Value>,
    shown: HashMap<String, String>,
    annotated: HashMap<String, String>,
}

fn session(ctx: &Invocation) -> Result<Session> {
    Session::new(
        ctx.project()
            .ok_or(tr("Open a trusted local project first"))?,
    )
}

fn post(data: &Value) -> std::result::Result<(), Error> {
    host::post_view_message(VIEW, &data.to_string())
}

fn bookmarks() -> Result<Vec<String>> {
    match result(host::storage::get(BOOKMARKS))? {
        Some(json) => serde_json::from_str(&json)
            .map_err(|_| tr("Cannot read the saved SVN repository addresses").into()),
        None => Ok(Vec::new()),
    }
}

fn bookmark(input: &str, remove: bool) -> Result<Value> {
    let url = svn::url(input)?;
    let mut urls = bookmarks()?;
    urls.retain(|u| u != &url);
    if !remove {
        if urls.len() >= 100 {
            return Err(tr("Remove a saved SVN repository address first").into());
        }
        urls.push(url);
    }
    urls.sort();
    result(host::storage::set(
        BOOKMARKS,
        &serde_json::to_string(&urls).map_err(|e| e.to_string())?,
    ))?;
    Ok(json!({"bookmarks": urls}))
}

fn branches(scope: &Scope, selection: Option<(&str, &str)>) -> Result<Value> {
    let key = storage_key("branches", &scope.path);
    if let Some((trunk, branches)) = selection {
        let normalize = |value: &str| {
            if value.trim().is_empty() {
                Ok(String::new())
            } else {
                svn::url(value)
            }
        };
        let config = json!({"trunk": normalize(trunk)?, "branches": normalize(branches)?});
        result(host::storage::set(&key, &config.to_string()))?;
        return Ok(config);
    }
    let config = result(host::storage::get(&key))?;
    if let Some(config) = config {
        return serde_json::from_str(&config).map_err(|e| e.to_string());
    }
    Ok(json!({"trunk": scope.info.url, "branches": ""}))
}

fn active_scope(s: &Session) -> Result<Option<Scope>> {
    let saved = result(host::storage::get(&storage_key("scope", &s.root)))?.unwrap_or_default();
    if !saved.is_empty()
        && let Ok(Some(scope)) = s.scope(&saved)
    {
        return Ok(Some(scope));
    }
    s.scope("")
}

fn select_scope(s: &Session, path: &str) -> Result<()> {
    let candidate = s
        .scope(path)?
        .ok_or(tr("The selected directory has no SVN working copy"))?;
    result(host::storage::set(
        &storage_key("scope", &s.root),
        &candidate.path,
    ))
}

fn need_scope(scope: Option<&Scope>) -> Result<&Scope> {
    scope.ok_or(
        tr("No SVN working copy in this scope. Select a subdirectory or check out a repository.")
            .into(),
    )
}

fn selected_path<'a>(req: &'a Request, scope: &'a Scope) -> &'a str {
    if req.path.is_empty() {
        &scope.path
    } else {
        &req.path
    }
}

fn write_output(data: &Value) {
    if let Some(out) = data
        .get("output")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        let _ = host::output_append("Subversion", &format!("{out}\n"));
    }
}

impl Subversion {
    fn snapshot(&mut self, s: &Session, scope: Option<&Scope>) -> Result<Value> {
        let pending = match scope {
            Some(scope) => s.pending(&scope.info.wc)?.or(s.pending(&s.root)?),
            None => s.pending(&s.root)?,
        }
        .map(|mut operation| {
            operation.label = i18n::pending_label(&operation.label).into();
            operation
        });
        let mut changes = scope
            .map(|scope| s.changes(scope))
            .transpose()?
            .unwrap_or_default();
        let total = changes.len();
        let set = storage_key("status", &s.root);
        let _ = host::clear_file_decorations(&set);
        for c in changes.iter().take(500) {
            let (badge, style) = if c.tree || c.item == "conflicted" || c.props == "conflicted" {
                ("C", "error")
            } else {
                match c.item.as_str() {
                    "added" => ("A", "added"),
                    "deleted" | "missing" => ("D", "removed"),
                    "unversioned" => ("?", "muted"),
                    "ignored" => continue,
                    "external" => ("X", "info"),
                    _ if c.locked && c.item == "normal" && c.props != "modified" => ("L", "info"),
                    _ => ("M", "modified"),
                }
            };
            let _ = host::set_file_decoration(
                &c.path,
                &set,
                &FileDecoration {
                    badge: Some(badge.into()),
                    style: style.into(),
                    tooltip: Some(format!("SVN {}", c.item)),
                },
            );
        }
        self.shown.insert(
            s.root.clone(),
            scope.map_or_else(|| s.root.clone(), |s| s.path.clone()),
        );
        changes.truncate(6_000);
        let changed_parents: HashSet<_> = changes
            .iter()
            .filter_map(|change| change.path.rsplit_once('/').map(|(dir, _)| dir.to_owned()))
            .collect();
        for change in &mut changes {
            change.directory = changed_parents.contains(&change.path)
                || s.project
                    .stat(&change.path)
                    .is_ok_and(|stat| stat.kind == FileKind::Directory);
        }
        Ok(json!({
            "project": s.root, "scope": scope.map(|s| &s.path), "info": scope.map(|s| &s.info),
            "count": total, "changes": changes, "pending": pending,
        }))
    }

    fn execute(&mut self, ctx: &Invocation, req: &Request) -> Result<Value> {
        let s = session(ctx)?;
        if !req.project.is_empty() && svn::key(&req.project) != svn::key(&s.root) {
            return Err(tr("The project changed. Refresh Subversion before retrying.").into());
        }
        if req.paths.len() > 200
            || req.path.len() > 8192
            || req.url.len() > 8192
            || req.to.len() > 8192
        {
            return Err(
                tr("The request contains too many paths or a path that is too long").into(),
            );
        }
        if matches!(req.op, Operation::Refresh) {
            s.version()?;
        }
        if matches!(req.op, Operation::ChooseScope) {
            let picked = result(host::pick_directory(
                tr("Choose an SVN Working Directory in the Project"),
                Some(&s.root),
            ))?
            .ok_or(tr("Operation canceled"))?;
            select_scope(&s, &picked)?;
        }
        if matches!(req.op, Operation::ResetScope) {
            result(host::storage::delete(&storage_key("scope", &s.root)))?;
        }
        let scope = active_scope(&s)?;
        let scope = scope.as_ref();
        let selected = || -> Vec<String> {
            if !req.paths.is_empty() {
                return req.paths.clone();
            }
            if !req.path.is_empty() {
                return vec![req.path.clone()];
            }
            Vec::new()
        };
        let data = match req.op {
            Operation::Refresh | Operation::ChooseScope | Operation::ResetScope => {
                let state = self.snapshot(&s, scope)?;
                let handoff = self.handoff.remove(&s.root);
                let branches = scope.map(|scope| branches(scope, None)).transpose()?;
                return Ok(
                    json!({"snapshot": state, "bookmarks": bookmarks()?, "handoff": handoff, "branchConfig": branches}),
                );
            }
            Operation::Branches | Operation::SetBranches => json!({"branchConfig": branches(
                need_scope(scope)?, matches!(req.op, Operation::SetBranches).then_some((&req.url, &req.to)),
            )?}),
            Operation::Bookmarks => json!({"bookmarks": bookmarks()?}),
            Operation::Bookmark | Operation::Unbookmark => {
                bookmark(&req.url, matches!(req.op, Operation::Unbookmark))?
            }
            Operation::Authenticate => {
                let target = if !req.url.trim().is_empty() {
                    svn::url(&req.url)?
                } else {
                    need_scope(scope)?.info.url.clone()
                };
                s.authenticate(&target)?;
                json!({"output": tr("Complete SVN authentication in the IDE terminal, then retry the operation.")})
            }
            Operation::Browse => s.browse(&req.url, &req.revision)?,
            Operation::Checkout => s.checkout(&req.url, &req.revision, &req.name, &req.value)?,
            Operation::Import => s.import(&req.path, &req.url, &req.note)?,
            Operation::Share => s.share(&req.path, &req.url, &req.note, req.preview)?,
            Operation::Relocate => s.relocate(need_scope(scope)?, &req.url)?,
            Operation::Compare => s.compare(
                &req.url,
                &req.to,
                &req.revision,
                &req.to_revision,
                &req.encoding,
            )?,
            Operation::Copy => s.copy_branch(scope, &req.url, &req.to, &req.revision, &req.note)?,
            Operation::Switch => s.switch(need_scope(scope)?, &req.url, &req.revision)?,
            Operation::Merge => {
                let scope = need_scope(scope)?;
                let target;
                let destination = if req.path.is_empty() {
                    scope
                } else {
                    let path = s.targets(scope, &[req.path.clone()], false)?.remove(0);
                    target = Scope {
                        info: s.info(&path)?,
                        path,
                    };
                    &target
                };
                s.merge(
                    destination,
                    &req.url,
                    &req.revision,
                    &req.value,
                    req.preview,
                )?
            }
            Operation::Recover => s.recover(scope)?,
            Operation::Diff => {
                let scope = need_scope(scope)?;
                s.local_diff(scope, selected_path(req, scope), &req.encoding, req.preview)?
            }
            Operation::EditDifference => s.edit_difference(
                need_scope(scope)?,
                &req.path,
                &req.encoding,
                &req.hash,
                req.hunk.as_ref().ok_or(tr("Select a difference"))?,
            )?,
            Operation::CompareClipboard => s.compare_clipboard(
                need_scope(scope)?,
                &req.path,
                &req.encoding,
                &req.name,
                &req.value,
            )?,
            Operation::CreatePatch => {
                let paths = s.targets(need_scope(scope)?, &selected(), true)?;
                s.patch(&working::pegs(&paths), &[])?
            }
            Operation::OpenSource => {
                let scope = need_scope(scope)?;
                let path = s.targets(scope, &[req.path.clone()], false)?.remove(0);
                let line = req.cursor.and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
                result(host::open_location(&path, line, 0))?;
                json!({})
            }
            Operation::CompareLocal | Operation::CurrentRevision | Operation::TargetUrl => {
                let scope = need_scope(scope)?;
                let path = s
                    .targets(scope, &[selected_path(req, scope).into()], true)?
                    .remove(0);
                let info = s.info(&path)?;
                match req.op {
                    Operation::CurrentRevision => {
                        json!({"currentRevision": {"path": path, "revision": info.revision}})
                    }
                    Operation::TargetUrl => json!({"targetUrl": info.url}),
                    _ => s.compare_local(&path, &info, &req.url, &req.revision, &req.encoding)?,
                }
            }
            Operation::History => {
                if !req.url.trim().is_empty() {
                    let target = svn::url(&req.url)?;
                    let peg = req
                        .peg
                        .map_or_else(|| svn::peg(&target), |r| svn::pegged(&target, r));
                    let info = svn::info(&s.read("info", &strings(&["--xml"]), &[peg])?)?;
                    let mut data = s.log(&target, req.cursor.or(req.peg), req.peg)?;
                    data["historyRoot"] = json!(info.repository);
                    data
                } else {
                    let scope = need_scope(scope)?;
                    let path = s
                        .targets(scope, &[selected_path(req, scope).into()], true)?
                        .remove(0);
                    let mut data = s.log(&path, req.cursor, None)?;
                    data["historyRoot"] = json!(scope.info.repository);
                    data
                }
            }
            Operation::HistoryDiff => s.history_diff(
                &req.url,
                req.change
                    .as_ref()
                    .ok_or(tr("Select a change from history"))?,
                req.peg.ok_or(tr("A revision is required"))?,
                &req.encoding,
            )?,
            Operation::Incoming => s.incoming(need_scope(scope)?, req.cursor)?,
            Operation::Shelves => s.shelves(need_scope(scope)?)?,
            Operation::Shelve => s.shelve(need_scope(scope)?, &selected(), &req.name)?,
            Operation::Unshelve => s.unshelve(
                need_scope(scope)?,
                req.peg.ok_or(tr("Select a shelf"))?,
                &selected(),
                &req.name,
                req.remove_applied,
            )?,
            Operation::ShelfDiff => s.shelf_diff(
                need_scope(scope)?,
                req.peg.ok_or(tr("Select a shelf"))?,
                &req.path,
                &req.value,
                &req.encoding,
            )?,
            Operation::EditShelf => s.edit_shelf(
                need_scope(scope)?,
                req.peg.ok_or(tr("Select a shelf"))?,
                &selected(),
                &req.value,
                &req.name,
            )?,
            Operation::ImportShelf => s.import_shelf(need_scope(scope)?, &req.name, &req.value)?,
            Operation::Annotate => {
                let data = s.annotate(need_scope(scope)?, &req.path, &req.encoding)?;
                if data["matchesEditor"] == true
                    && let Some(path) = data["annotatedPath"].as_str()
                {
                    let relative = svn::local_argument(&s.root, path);
                    self.annotated
                        .insert(s.root.clone(), relative.trim_start_matches("./").into());
                    if let Err(error) = self.subscribe_annotated() {
                        let _ = self.clear_annotations(&s.root);
                        return Err(error);
                    }
                }
                data
            }
            Operation::ClearAnnotations => {
                self.clear_annotations(&s.root)?;
                json!({"clearAnnotations": true})
            }
            Operation::Commit => s.commit(
                need_scope(scope)?,
                &selected(),
                &req.note,
                req.keep_locked,
                req.auto_update,
            )?,
            Operation::Update => s.update(
                need_scope(scope)?,
                &selected(),
                working::UpdateOptions {
                    revision: &req.revision,
                    depth: &req.value,
                    url: &req.url,
                    force: req.force,
                    ignore_externals: req.ignore_externals,
                },
            )?,
            Operation::Add => s.add(need_scope(scope)?, &selected())?,
            Operation::Delete => s.delete_or_revert(need_scope(scope)?, &selected(), false)?,
            Operation::Revert => s.delete_or_revert(need_scope(scope)?, &selected(), true)?,
            Operation::Move => s.rename(need_scope(scope)?, &selected(), &req.name)?,
            Operation::Cleanup => s.cleanup(need_scope(scope)?, &selected())?,
            Operation::Lock | Operation::Unlock => s.lock(
                need_scope(scope)?,
                &selected(),
                matches!(req.op, Operation::Lock),
                &req.note,
            )?,
            Operation::Changelist | Operation::RemoveChangelist => s.changelist(
                need_scope(scope)?,
                &selected(),
                &req.name,
                matches!(req.op, Operation::RemoveChangelist),
            )?,
            Operation::Ignore | Operation::Unignore => s.ignore(
                need_scope(scope)?,
                &selected(),
                matches!(req.op, Operation::Unignore),
            )?,
            Operation::Properties | Operation::PropertyDiff => {
                let scope = need_scope(scope)?;
                s.props(
                    scope,
                    selected_path(req, scope),
                    matches!(req.op, Operation::PropertyDiff),
                )?
            }
            Operation::PropertySet | Operation::PropertyDelete => {
                let scope = need_scope(scope)?;
                let paths = if selected().is_empty() {
                    vec![scope.path.clone()]
                } else {
                    selected()
                };
                s.property_edit(
                    scope,
                    &paths,
                    &req.name,
                    &req.value,
                    matches!(req.op, Operation::PropertyDelete),
                )?
            }
            Operation::Conflict => {
                s.conflict_panel(need_scope(scope)?, &req.path, &req.encoding)?
            }
            Operation::Resolve => s.resolve(
                need_scope(scope)?,
                &req.path,
                &req.value,
                req.merged.as_deref(),
                &req.hash,
            )?,
        };
        write_output(&data);
        if data.get("output").is_some()
            && !matches!(req.op, Operation::Authenticate)
            && !(matches!(req.op, Operation::Merge) && req.preview)
        {
            let mut data = data;
            match active_scope(&s).and_then(|scope| self.snapshot(&s, scope.as_ref())) {
                Ok(snapshot) => data["snapshot"] = snapshot,
                Err(error) => data["refreshError"] = json!(error),
            }
            return Ok(data);
        }
        Ok(data)
    }

    fn route(&mut self, ctx: &Invocation, id: &str) -> Result<()> {
        if id == "open" {
            return result(host::reveal_view(VIEW));
        }
        let s = session(ctx)?;
        let file_paths = || -> Result<Vec<String>> {
            let mut paths = ctx.selected_paths();
            if paths.is_empty()
                && let Some(editor) = ctx.editor()
            {
                paths.push(editor.path());
            }
            if id == "cleanup" && paths.is_empty() {
                return Ok(paths);
            }
            if paths.is_empty() || paths.iter().any(|path| !svn::absolute(path)) {
                return Err(tr("Select an SVN file or directory first").into());
            }
            paths.iter().map(|path| s.path(path)).collect()
        };
        let out = match id {
            "refresh" => self.execute(ctx, &Request::default())?,
            "commit-project" => json!({"openAction": "commit"}),
            "repositories" | "update" | "relocate" => json!({"openAction": id}),
            "commit" | "update-file" | "set-property" | "lock" | "branch-tag"
            | "compare-latest" | "compare-revision" | "ignore" => {
                let paths = file_paths()?;
                let scope = active_scope(&s)?;
                let paths = s.targets(need_scope(scope.as_ref())?, &paths, true)?;
                json!({"openAction": id, "paths": paths})
            }
            "add" | "revert" | "cleanup" | "unlock" => {
                let op = match id {
                    "add" => Operation::Add,
                    "revert" => Operation::Revert,
                    "cleanup" => Operation::Cleanup,
                    _ => Operation::Unlock,
                };
                self.execute(
                    ctx,
                    &Request {
                        op,
                        paths: file_paths()?,
                        ..Request::default()
                    },
                )?
            }
            "diff" | "history" | "annotate" | "resolve" | "properties" | "current-revision" => {
                let path = file_paths()?.remove(0);
                let op = match id {
                    "diff" => Operation::Diff,
                    "history" => Operation::History,
                    "annotate" => Operation::Annotate,
                    "properties" => Operation::Properties,
                    "current-revision" => Operation::CurrentRevision,
                    _ => Operation::Conflict,
                };
                let mut output = self.execute(
                    ctx,
                    &Request {
                        op,
                        path: path.clone(),
                        ..Request::default()
                    },
                )?;
                if output.as_object().is_some_and(|m| m.is_empty()) {
                    return Ok(());
                }
                if matches!(op, Operation::History) {
                    output["historyTarget"] = json!({"path": path});
                }
                if matches!(op, Operation::Properties) {
                    output["propertyTarget"] = json!(path);
                }
                output
            }
            _ => return Err(trf!("Unknown SVN action: {0}", id)),
        };
        self.handoff.insert(s.root.clone(), out);
        result(host::reveal_view(VIEW))?;
        // A newly opened frame can pick up the command on its initial refresh.
        let _ = post(&json!({"invalidated": true, "project": s.root}));
        Ok(())
    }

    fn subscribe_annotated(&self) -> Result<()> {
        let mut paths: Vec<String> = self
            .annotated
            .values()
            .map(|path| {
                let mut glob = String::new();
                for ch in path.chars() {
                    if matches!(ch, '?' | '*' | '[' | ']' | '{' | '}') {
                        glob.push('[');
                        glob.push(ch);
                        glob.push(']');
                    } else {
                        glob.push(ch);
                    }
                }
                glob
            })
            .collect();
        paths.sort_unstable();
        paths.dedup();
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        result(host::subscribe_paths(
            EventKind::DocumentChanged.name(),
            &paths,
        ))
    }

    fn clear_annotations(&mut self, root: &str) -> Result<()> {
        result(host::clear_decorations(&storage_key("annotation", root)))?;
        if self.annotated.remove(root).is_some() {
            self.subscribe_annotated()?;
        }
        Ok(())
    }
}

impl Plugin for Subversion {
    fn activate(&mut self, _ctx: &Context) -> std::result::Result<(), Error> {
        host::subscribe(EventKind::FilesChanged)?;
        host::subscribe(EventKind::FileSaved)?;
        host::subscribe(EventKind::SettingsChanged)?;
        Ok(())
    }

    fn run_command(
        &mut self,
        id: &str,
        ctx: &Invocation,
        _arg: Option<&str>,
    ) -> std::result::Result<Option<String>, Error> {
        i18n::with_language(|| self.route(ctx, id)).map_err(Error::from)?;
        Ok(None)
    }

    fn handle_events(&mut self, events: &[Event], ctx: &Invocation) {
        if events.iter().any(|event| {
            matches!(event, Event::Other { kind, data }
                if kind == EventKind::SettingsChanged.name()
                    && serde_json::from_str::<Vec<&str>>(data).is_ok_and(|keys| {
                        keys.iter().any(|key| matches!(*key, "language" | "panelFontSize" | "diffFontSize"))
                    }))
        }) {
            // View messages are broadcast. Let each panel read its own project settings.
            let _ = post(&json!({"languageChanged": true}));
        }
        let Ok(s) = session(ctx) else {
            return;
        };
        if events.iter().any(|event| match event {
            Event::DocumentChanged(change) => {
                self.annotated.get(&s.root) == Some(&change.path)
                    && change.project.as_deref().is_none_or(|root| {
                        svn::canonical(root).is_ok_and(|root| svn::key(&root) == svn::key(&s.root))
                    })
            }
            Event::FilesChanged(_) => self.shown.contains_key(&s.root),
            _ => false,
        }) {
            let _ = self.clear_annotations(&s.root);
        }
        let Some(path) = self.shown.get(&s.root) else {
            return;
        };
        let relevant =
            |p: &str| svn::canonical(p).is_ok_and(|abs| svn::project_path(path, &abs).is_ok());
        if events.iter().any(|e| match e {
            Event::FileSaved(p) => relevant(p),
            Event::FilesChanged(e) => e
                .created
                .iter()
                .chain(&e.modified)
                .chain(&e.removed)
                .any(|p| relevant(p)),
            _ => false,
        }) {
            let _ = post(&json!({"invalidated": true, "project": s.root}));
        }
    }
}

impl Views for Subversion {
    fn render(
        &mut self,
        view: &str,
        _area: Option<Viewport>,
        _ctx: &Invocation,
    ) -> std::result::Result<View, Error> {
        if view != VIEW {
            return Err(Error::NotFound(view.into()));
        }
        let scripts = [
            ("zh-CN.js", include_str!("../web/zh-CN.js")),
            ("i18n.js", include_str!("../web/i18n.js")),
            ("icons.js", include_str!("../web/icons.js")),
            ("diff-preview.js", include_str!("../web/diff-preview.js")),
            ("shelf.js", include_str!("../web/shelf.js")),
            ("panel.js", include_str!("../web/panel.js")),
        ];
        let mut page = PAGE.to_owned();
        for (name, script) in scripts {
            page = page.replace(
                &format!(r#"<script src="{name}"></script>"#),
                &format!("<script>{script}</script>"),
            );
        }
        Ok(View::new(Body::Html(page)))
    }

    fn handle(
        &mut self,
        view: &str,
        event: ViewEvent,
        ctx: &Invocation,
    ) -> std::result::Result<(), Error> {
        if view != VIEW {
            return Err(Error::NotFound(view.into()));
        }
        let ViewEvent::Message(message) = event else {
            return Ok(());
        };
        i18n::with_language(|| {
            let request: Request = serde_json::from_str(&message)
                .map_err(|_| Error::Invalid(tr("Invalid SVN panel message").into()))?;
            if request.seq.is_none()
                || request.panel.len() != 32
                || !request.panel.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(Error::Invalid(
                    tr("Invalid SVN panel request identifier").into(),
                ));
            }
            let language = i18n::active_language();
            if request.language_only {
                return post(&json!({
                    "panel": request.panel,
                    "seq": request.seq,
                    "language": language,
                    "languageOnly": true,
                    "panelFontSize": host::setting_number("panelFontSize"),
                    "diffFontSize": host::setting_number("diffFontSize"),
                }));
            }
            match self.execute(ctx, &request) {
                Ok(data) => post(
                    &json!({"panel": request.panel, "seq": request.seq, "language": language, "data": data}),
                )?,
                Err(error) if error == tr("Operation canceled") => post(
                    &json!({"panel": request.panel, "seq": request.seq, "language": language, "data": {"canceled": true}}),
                )?,
                Err(error) => {
                    let _ = host::output_append("Subversion", &format!("{error}\n"));
                    let snapshot = session(ctx).ok().and_then(|s| {
                        let scope = active_scope(&s).ok().flatten();
                        let pending = scope.as_ref().map(|p| &p.info.wc).unwrap_or(&s.root);
                        if s.pending(pending).ok().flatten().is_some()
                            || s.pending(&s.root).ok().flatten().is_some()
                        {
                            self.snapshot(&s, scope.as_ref()).ok()
                        } else {
                            None
                        }
                    });
                    post(
                        &json!({"panel": request.panel, "seq": request.seq, "language": language, "error": error, "snapshot": snapshot}),
                    )?;
                }
            }
            Ok(())
        })
    }
}

register_plugin!(Subversion, views);
