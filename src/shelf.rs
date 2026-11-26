use crate::i18n::{tr, trf};
use openidea_plugin::{Error, FileKind, host};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::backend::{MAX_TEXT, Pending, Scope, Session, result, storage_key, strings};
use crate::svn::{self, Result};
use crate::working::pegs;

const MAX_BINARY_PATCH: usize = 12 << 20;

#[derive(Clone, Deserialize, Serialize)]
struct Shelf {
    id: u64,
    name: String,
    paths: Vec<String>,
    patch: String,
    restored: bool,
    #[serde(default)]
    created: u64,
    #[serde(default)]
    removed: u64,
    #[serde(default)]
    added: Vec<String>,
    #[serde(default)]
    deleted: Vec<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn next_id(shelves: &[Shelf]) -> u64 {
    shelves.iter().map(|shelf| shelf.id).max().unwrap_or(0) + 1
}

fn load(key: &str) -> Result<Vec<Shelf>> {
    result(host::storage::get(key))?
        .map(|data| serde_json::from_str(&data).map_err(|error| error.to_string()))
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn save(key: &str, shelves: &[Shelf]) -> Result<()> {
    let data = serde_json::to_string(shelves).map_err(|error| error.to_string())?;
    result(host::storage::set(key, &data))
}

fn summary(shelves: &[Shelf]) -> Value {
    json!({"shelves": shelves.iter().rev().map(|shelf| json!({
        "id": shelf.id, "name": shelf.name, "paths": shelf.paths, "restored": shelf.restored,
        "created": shelf.created, "removed": shelf.removed, "added": shelf.added, "deleted": shelf.deleted
    })).collect::<Vec<_>>()})
}

fn sections(patch: &str) -> impl Iterator<Item = (&str, &str)> {
    let breaks = patch
        .match_indices("\nIndex: ")
        .map(|(offset, _)| offset + 1);
    std::iter::once(0)
        .chain(breaks.clone())
        .zip(breaks.chain(std::iter::once(patch.len())))
        .filter_map(move |(start, end)| {
            let section = patch[start..end].strip_prefix("Index: ")?;
            let (path, body) = section.split_once('\n')?;
            Some((path.trim_end_matches('\r'), body))
        })
}

impl Shelf {
    fn select(&self, root: &str, paths: &[String]) -> Result<Self> {
        let paths = if paths.is_empty() { &self.paths } else { paths };
        if paths.is_empty() || paths.iter().any(|path| !self.paths.contains(path)) {
            return Err(tr("Select files from this shelf").into());
        }
        let mut selected = self.clone();
        selected.paths.retain(|path| paths.contains(path));
        selected.added.retain(|path| paths.contains(path));
        selected.deleted.retain(|path| paths.contains(path));
        selected.patch.clear();
        for (index, body) in sections(&self.patch) {
            if paths.contains(&svn::project_path(root, index)?) {
                selected.patch.push_str(&format!("Index: {index}\n{body}"));
                if !selected.patch.ends_with('\n') {
                    selected.patch.push('\n');
                }
            }
        }
        Ok(selected)
    }
}

fn split_selected(
    shelves: &mut Vec<Shelf>,
    at: usize,
    root: &str,
    mut selected: Shelf,
) -> Result<()> {
    let remaining: Vec<_> = shelves[at]
        .paths
        .iter()
        .filter(|p| !selected.paths.contains(p))
        .cloned()
        .collect();
    if remaining.is_empty() {
        shelves[at] = selected;
    } else {
        shelves[at] = shelves[at].select(root, &remaining)?;
        selected.id = next_id(shelves);
        shelves.push(selected);
    }
    Ok(())
}

fn shelf_index(shelves: &[Shelf], id: u64) -> Result<usize> {
    shelves
        .iter()
        .position(|shelf| shelf.id == id)
        .ok_or(tr("Select a shelf").into())
}

fn patch_sides(body: &str) -> (String, String) {
    let mut left = String::new();
    let mut right = String::new();
    let mut in_hunk = false;
    for line in body.lines() {
        if line.starts_with("@@ ") {
            in_hunk = true;
        } else if line.starts_with("Property changes on: ") || line.starts_with("## ") {
            break;
        } else if in_hunk {
            if let Some(text) = line.strip_prefix(' ').or_else(|| line.strip_prefix('-')) {
                left.push_str(text);
                left.push('\n');
            }
            if let Some(text) = line.strip_prefix(' ').or_else(|| line.strip_prefix('+')) {
                right.push_str(text);
                right.push('\n');
            }
        }
    }
    (left, right)
}

fn patch_base_revision(body: &str) -> Option<u64> {
    let before = body.lines().find(|line| line.starts_with("--- "))?;
    before
        .split_once("\t(revision ")?
        .1
        .strip_suffix(')')?
        .parse()
        .ok()
}

fn patched_text(base: &str, body: &str) -> Option<String> {
    let base = base.replace("\r\n", "\n");
    let lines: Vec<_> = base.split_inclusive('\n').collect();
    let mut output = String::new();
    let mut cursor = 0;
    let mut in_hunk = false;
    let mut previous = ' ';
    for line in body.lines() {
        if let Some(header) = line.strip_prefix("@@ -") {
            let range = header.split_whitespace().next()?;
            let (start, count) = range.split_once(',').unwrap_or((range, "1"));
            let start = start.parse::<usize>().ok()?;
            let start = if count == "0" {
                start
            } else {
                start.checked_sub(1)?
            };
            for line in lines.get(cursor..start)? {
                output.push_str(line);
            }
            cursor = start;
            in_hunk = true;
        } else if line.starts_with("Property changes on: ") || line.starts_with("## ") {
            break;
        } else if in_hunk && let Some(prefix) = line.chars().next() {
            match prefix {
                '+' => {
                    output.push_str(&line[1..]);
                    output.push('\n');
                }
                ' ' | '-' => {
                    let original = *lines.get(cursor)?;
                    if original.trim_end_matches('\n') != &line[1..] {
                        return None;
                    }
                    if prefix == ' ' {
                        output.push_str(original);
                    }
                    cursor += 1;
                }
                '\\' if previous == '+' && output.ends_with('\n') => {
                    output.pop();
                }
                _ => (),
            }
            previous = prefix;
        }
    }
    for line in lines.get(cursor..)? {
        output.push_str(line);
    }
    Some(output)
}

pub(crate) fn normalize_shelf_patch(raw: &str, root: &str, paths: &[String]) -> Result<String> {
    use std::collections::HashSet;

    let expected: HashSet<_> = paths.iter().map(|path| svn::key(path)).collect();
    let mut seen = HashSet::new();
    let mut patch = String::new();
    for (index, body) in sections(raw) {
        let Ok(path) = svn::project_path(root, index) else {
            continue;
        };
        let key = svn::key(&path);
        if !expected.contains(&key) {
            continue;
        }
        if !seen.insert(key) || path.contains(['\t', '\r', '\n']) {
            return Err(trf!("Cannot shelve this patch path: {0}", index));
        }

        let relative = svn::local_argument(root, &path);
        let relative = relative.strip_prefix("./").unwrap_or(&relative);
        patch.push_str(&format!("Index: {relative}\n"));
        let mut headers = true;
        for physical_line in body.split_inclusive('\n') {
            let line = physical_line.trim_end_matches(['\r', '\n']);
            let ending = &physical_line[line.len()..];
            let line = if line.starts_with("diff --git ") {
                format!("diff --git a/{relative} b/{relative}")
            } else if line.starts_with("Property changes on: ") {
                format!("Property changes on: {relative}")
            } else if headers && (line.starts_with("--- ") || line.starts_with("+++ ")) {
                let (header, rest) = line.split_at(4);
                let (name, suffix) = rest.split_once('\t').unwrap_or((rest, ""));
                if name == "/dev/null" {
                    line.into()
                } else {
                    let side = if header == "--- " { 'a' } else { 'b' };
                    format!("{header}{side}/{relative}\t{suffix}")
                }
            } else {
                if line.starts_with("@@") || line == "GIT binary patch" {
                    headers = false;
                }
                if headers && line.ends_with(" 120000") {
                    return Err(trf!("Cannot shelve symbolic links: {0}", relative));
                }
                line.into()
            };
            patch.push_str(&line);
            patch.push_str(ending);
        }
    }
    if seen != expected {
        return Err(tr("SVN did not return a patch for every selected file").into());
    }
    Ok(patch)
}

pub(crate) fn check_shelf_result(
    output: &str,
    root: &str,
    expected: &[(String, char)],
) -> Result<()> {
    use std::collections::HashMap;

    let mut applied = HashMap::new();
    for line in output.lines() {
        // svn patch can report a rejected or skipped hunk and still exit with 0.
        let (columns, path) = line
            .split_at_checked(10)
            .ok_or(tr("Patch would skip files or introduce conflicts"))?;
        let code = match columns.trim_end() {
            "U" | " U" | "UU" => 'U',
            "A" => 'A',
            "D" => 'D',
            _ => return Err(tr("Patch would skip files or introduce conflicts").into()),
        };
        applied.insert(svn::key(&svn::project_path(root, path)?), code);
    }
    if applied.len() != expected.len()
        || !expected
            .iter()
            .all(|(path, action)| applied.get(&svn::key(path)) == Some(action))
    {
        return Err(tr("Patch would not apply to all selected files as expected").into());
    }
    Ok(())
}

impl Session {
    fn shelf_key(&self, scope: &Scope) -> String {
        storage_key("shelf", &format!("{}/{}", scope.path, scope.info.url))
    }

    pub fn shelves(&self, scope: &Scope) -> Result<Value> {
        Ok(summary(&load(&self.shelf_key(scope))?))
    }

    pub fn import_shelf(&self, scope: &Scope, name: &str, patch: &str) -> Result<Value> {
        let name = name.trim();
        if name.is_empty()
            || name.len() > 256
            || patch.len() > MAX_BINARY_PATCH
            || patch.contains('\0')
        {
            return Err(
                tr("Select an intact patch and enter a shelf name of at most 256 bytes").into(),
            );
        }
        let mut paths = Vec::new();
        let mut added = Vec::new();
        let mut deleted = Vec::new();
        for (index, body) in sections(patch) {
            let path = svn::project_path(&self.root, index)?;
            if paths.contains(&path) || paths.len() >= 200 || !svn::within(&scope.path, &path) {
                return Err(tr(
                    "Patch contains duplicate targets or paths outside the working directory",
                )
                .into());
            }
            let header: Vec<_> = body
                .lines()
                .take_while(|line| !line.starts_with("@@ ") && *line != "GIT binary patch")
                .collect();
            if header.iter().any(|line| {
                ["rename from ", "rename to ", "copy from ", "copy to "]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
            }) {
                return Err(
                    tr("Import only patches with file edits, additions or deletions").into(),
                );
            }
            if header
                .iter()
                .any(|line| line.starts_with("new file mode ") || line.starts_with("--- /dev/null"))
            {
                added.push(path.clone());
            } else if header.iter().any(|line| {
                line.starts_with("deleted file mode ") || line.starts_with("+++ /dev/null")
            }) {
                deleted.push(path.clone());
            }
            paths.push(path);
        }
        if paths.is_empty() {
            return Err(tr("The patch has no indexed file entries").into());
        }
        self.targets(scope, &paths, false)?;
        let patch = normalize_shelf_patch(patch, &self.root, &paths)?;
        let key = self.shelf_key(scope);
        let mut shelves = load(&key)?;
        let id = next_id(&shelves);
        shelves.push(Shelf {
            id,
            name: name.into(),
            paths,
            patch,
            restored: false,
            created: now(),
            removed: 0,
            added,
            deleted,
        });
        save(&key, &shelves)?;
        let mut data = summary(&shelves);
        data["shelved"] = json!(id);
        Ok(data)
    }

    pub fn shelve(&self, scope: &Scope, inputs: &[String], name: &str) -> Result<Value> {
        let name = name.trim();
        if name.is_empty() || name.len() > 256 {
            return Err(tr("Enter a shelf name of at most 256 bytes").into());
        }
        let paths = self.targets(scope, inputs, false)?;
        if let Some(pending) = self.pending(&scope.info.wc)? {
            return Err(trf!(
                "Review the previous pending operation first: {0}",
                crate::i18n::pending_label(&pending.label)
            ));
        }
        let rows = self.changes(scope)?;
        self.dirty(&paths)?;
        let mut added = Vec::new();
        let mut deleted = Vec::new();
        for path in &paths {
            let change = rows
                .iter()
                .find(|row| svn::key(&row.path) == svn::key(path))
                .filter(|row| {
                    !row.tree
                        && !row.copied
                        && row.from.is_empty()
                        && row.to.is_empty()
                        && row.props != "conflicted"
                        && matches!(row.item.as_str(), "modified" | "added" | "deleted")
                })
                .ok_or(tr(
                    "Select added, deleted, or modified files without conflicts or moves",
                ))?;
            match change.item.as_str() {
                "deleted" => {
                    if self.info(path)?.kind != "file" || result(self.project.exists(path))? {
                        return Err(trf!("Expected a deleted file: {0}", path));
                    }
                    deleted.push(path.clone());
                }
                item => {
                    if result(self.project.stat(path))?.kind != FileKind::File {
                        return Err(trf!("Shelve requires a regular file: {0}", path));
                    }
                    if item == "added" {
                        added.push(path.clone());
                    }
                }
            }
        }

        // The SVN CLI emits byte-preserving Git binary patches for its binary files.
        // Index: uses local paths, while the actual Git paths start at the repository root.
        let mut targets = Vec::new();
        for path in &paths {
            let mut target = path.clone();
            while svn::local_argument(&self.root, &target).contains('@') {
                target = svn::parent(&target).ok_or(tr("Cannot locate patch parent"))?;
            }
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        let shallow: Vec<String> = targets
            .iter()
            .filter(|path| {
                !targets
                    .iter()
                    .any(|other| other != *path && svn::within(other, path))
            })
            .cloned()
            .collect();
        let flags = strings(&["--internal-diff", "--git"]);
        let targets = pegs(&shallow);
        let make_patch = || {
            let raw = self.read("diff", &flags, &targets)?;
            normalize_shelf_patch(&raw, &self.root, &paths)
        };
        let patch = make_patch()?;
        let max = if patch.lines().any(|line| line == "GIT binary patch") {
            MAX_BINARY_PATCH
        } else {
            MAX_TEXT
        };
        if patch.is_empty() || patch.contains(['\0', '\u{fffd}']) || patch.len() > max {
            return Err(trf!(
                "SVN did not produce an intact patch within {0} KiB",
                max >> 10
            ));
        }
        self.dirty(&paths)?;
        if make_patch()? != patch {
            return Err(tr("Working copy contents changed while preparing the shelf").into());
        }
        let key = self.shelf_key(scope);
        let mut shelves = load(&key)?;
        let id = next_id(&shelves);
        let shelf = Shelf {
            id,
            name: name.into(),
            paths,
            patch,
            restored: false,
            created: now(),
            removed: 0,
            added,
            deleted,
        };
        self.apply_shelf(scope, &shelf, true, true, "")?;
        shelves.push(shelf);
        save(&key, &shelves)?;
        let output = self.apply_shelf(scope, shelves.last().unwrap(), true, false, "")?;
        let mut data = summary(&shelves);
        data["output"] = json!(output);
        data["shelved"] = json!(id);
        Ok(data)
    }

    pub fn unshelve(
        &self,
        scope: &Scope,
        id: u64,
        paths: &[String],
        group: &str,
        remove: bool,
    ) -> Result<Value> {
        let key = self.shelf_key(scope);
        let mut shelves = load(&key)?;
        let at = shelf_index(&shelves, id)?;
        let shelf = shelves[at].select(&self.root, paths)?;
        let group = group.trim();
        if group.len() > 128 || group.starts_with('-') || group.contains(['\0', '\r', '\n']) {
            return Err(tr("Enter a valid changelist name").into());
        }
        self.apply_shelf(scope, &shelf, false, true, group)?;
        let output = self.apply_shelf(scope, &shelf, false, false, group)?;
        if remove && shelf.removed == 0 && !shelf.restored {
            let mut shelf = shelf;
            shelf.restored = true;
            split_selected(&mut shelves, at, &self.root, shelf)?;
            save(&key, &shelves)?;
        }
        let mut data = summary(&shelves);
        data["output"] = json!(output);
        data["unshelved"] = json!(id);
        Ok(data)
    }

    pub fn edit_shelf(
        &self,
        scope: &Scope,
        id: u64,
        paths: &[String],
        action: &str,
        name: &str,
    ) -> Result<Value> {
        let key = self.shelf_key(scope);
        let mut shelves = load(&key)?;
        let at = shelf_index(&shelves, id)?;
        let mut shelf = shelves[at].select(&self.root, paths)?;
        match action {
            "rename" => {
                let name = name.trim();
                if name.is_empty() || name.len() > 256 {
                    return Err(tr("Enter a shelf name of at most 256 bytes").into());
                }
                shelves[at].name = name.into();
            }
            "delete" => {
                if shelf.removed != 0 {
                    return Err(tr("This shelf is already in Recently Deleted").into());
                }
                shelf.removed = now();
                split_selected(&mut shelves, at, &self.root, shelf)?;
            }
            "restore" => {
                if shelf.removed == 0 {
                    return Err(tr("Select a recently deleted shelf").into());
                }
                shelves[at].removed = 0;
            }
            _ => return Err(tr("Unknown shelf action").into()),
        }
        save(&key, &shelves)?;
        Ok(summary(&shelves))
    }

    pub fn shelf_diff(
        &self,
        scope: &Scope,
        id: u64,
        path: &str,
        action: &str,
        encoding: &str,
    ) -> Result<Value> {
        let entry = load(&self.shelf_key(scope))?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or(tr("Select a shelf"))?;
        let paths: Vec<_> = (!path.is_empty())
            .then(|| path.to_owned())
            .into_iter()
            .collect();
        let entry = entry.select(&self.root, &paths)?;
        if action == "patch" {
            return Ok(json!({"patch": entry.patch}));
        }
        let path = entry.paths.first().ok_or(tr("Select a shelved file"))?;
        let body = sections(&entry.patch)
            .find_map(|(index, body)| {
                (svn::project_path(&self.root, index).as_ref() == Ok(path)).then_some(body)
            })
            .ok_or(tr("The shelf does not contain this file"))?;
        if body.lines().any(|line| line == "GIT binary patch") {
            return Ok(json!({"shelfPreview": {"message": tr("Binary files differ.")}}));
        }
        let added = entry.added.contains(path);
        let deleted = entry.deleted.contains(path);
        let base = if added {
            Some(String::new())
        } else {
            patch_base_revision(body).and_then(|revision| {
                let info = self.info(path).ok()?;
                if info.revision != revision {
                    return None;
                }
                self.content(path, "@BASE", "BASE", encoding).ok()
            })
        };
        let (before, after) = base
            .and_then(|base| patched_text(&base, body).map(|after| (base, after)))
            .unwrap_or_else(|| patch_sides(body));
        let mut left = (!added).then_some(before);
        let mut right = (!deleted).then_some(after);
        let mut left_label = tr("Base Version");
        let mut right_label = tr("Shelved Version");
        if action == "local" {
            left = right;
            right = match self.project.read_bytes(path) {
                Ok(bytes) => Some(crate::backend::text(&bytes, encoding)?),
                Err(Error::NotFound(_)) => None,
                Err(error) => return result(Err(error)),
            };
            left_label = tr("Shelved Version");
            right_label = tr("Current version");
        }
        if action == "native" || action == "local" {
            return crate::review::show_diff(left, right, path, left_label, right_label);
        }
        Ok(
            json!({"shelfPreview": {"left": left, "right": right, "path": path, "revision": left_label, "rightLabel": right_label}}),
        )
    }

    fn apply_shelf(
        &self,
        scope: &Scope,
        shelf: &Shelf,
        reverse: bool,
        preview: bool,
        group: &str,
    ) -> Result<String> {
        self.dirty(&shelf.paths)?;
        self.recheck(scope, &shelf.paths)?;
        let deleted: Vec<_> = if reverse {
            shelf.deleted.clone()
        } else {
            Vec::new()
        };
        let expected: Vec<_> = shelf
            .paths
            .iter()
            .filter(|path| !deleted.contains(path))
            .map(|path| {
                let action = if shelf.added.contains(path) {
                    if reverse { 'D' } else { 'A' }
                } else if shelf.deleted.contains(path) {
                    'D'
                } else {
                    'U'
                };
                (path.clone(), action)
            })
            .collect();
        let mut patch = String::new();
        for (path, body) in sections(&shelf.patch) {
            let absolute = svn::project_path(&self.root, path)?;
            if deleted.iter().any(|d| svn::key(d) == svn::key(&absolute)) {
                continue;
            }
            patch.push_str(&format!("Index: {path}\n{body}"));
            if !patch.ends_with('\n') {
                patch.push('\n');
            }
        }
        for path in &deleted {
            match self.project.stat(path) {
                Err(Error::NotFound(_)) => (),
                Err(error) => return result(Err(error)),
                Ok(_) => {
                    return Err(trf!(
                        "An existing file blocks undoing the scheduled deletion: {0}",
                        path
                    ));
                }
            }
        }
        let apply_patch = || -> Result<String> {
            if expected.is_empty() {
                return Ok(String::new());
            }
            self.with_text(&patch, |file| {
                let targets = vec![file.into(), self.root.clone()];
                let mut flags = Vec::new();
                if reverse {
                    flags.push("--reverse-diff".into());
                }
                if preview {
                    flags.push("--dry-run".into());
                }
                let output = self.read("patch", &flags, &targets)?;
                check_shelf_result(&output, &self.root, &expected)?;
                Ok(output)
            })
        };
        if preview {
            return apply_patch();
        }
        let label = if reverse { "Shelve" } else { "Unshelve" };
        let pending = Pending {
            label: label.into(),
            path: scope.path.clone(),
            repository: None,
        };
        self.write(&scope.info.wc, &pending, || {
            let mut output = apply_patch()?;
            if !deleted.is_empty() {
                // Reversing a deleted-file patch creates a replacement. Revert
                // restores both the original bytes and the original SVN schedule.
                self.dirty(&deleted)?;
                let reverted =
                    self.read("revert", &strings(&["--depth", "empty"]), &pegs(&deleted))?;
                output.push_str(&reverted);
            }
            if !reverse && !group.is_empty() {
                let flags = if group == "Changes" {
                    strings(&["--remove"])
                } else {
                    strings(&[group])
                };
                output.push_str(&self.read("changelist", &flags, &pegs(&shelf.paths))?);
            }
            Ok(output)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partially_recycled_shelf_keeps_the_other_file_active() {
        let mut shelves: Vec<Shelf> = serde_json::from_value(json!([{
            "id": 7, "name": "legacy", "paths": ["/wc/a", "/wc/b"], "restored": false,
            "patch": "Index: a\n@@ -1 +1 @@\n-a\n+A\nIndex: b\n@@ -1 +1 @@\n-b\n+B\n"
        }]))
        .unwrap();
        let mut applied = shelves[0].select("/wc", &["/wc/b".into()]).unwrap();
        applied.restored = true;
        split_selected(&mut shelves, 0, "/wc", applied).unwrap();
        assert_eq!(shelves[0].id, 7);
        assert_eq!(shelves[0].paths, ["/wc/a"]);
        assert!(!shelves[0].restored);
        assert_eq!(shelves[1].id, 8);
        assert_eq!(shelves[1].paths, ["/wc/b"]);
        assert!(shelves[1].restored);
        assert_eq!(
            sections(&shelves[0].patch)
                .map(|(p, _)| p)
                .collect::<Vec<_>>(),
            ["a"]
        );
        assert_eq!(
            sections(&shelves[1].patch)
                .map(|(p, _)| p)
                .collect::<Vec<_>>(),
            ["b"]
        );
    }

    #[test]
    fn preview_retains_unmodified_lines_and_final_newline_state() {
        let patch = "--- a/file\t(revision 3)\n+++ b/file\t(working copy)\n@@ -2 +2 @@\n-before\n+after\n@@ -4 +4 @@\n-end\n+last\n\\ No newline at end of file\n";
        assert_eq!(patch_base_revision(patch), Some(3));
        assert_eq!(
            patched_text("first\nbefore\nkeep\nend\n", patch).as_deref(),
            Some("first\nafter\nkeep\nlast")
        );
        assert!(patched_text("first\nother\nkeep\nend\n", patch).is_none());
    }
}
