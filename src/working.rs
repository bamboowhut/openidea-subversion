use crate::i18n::{tr, trf};
use serde_json::{Value, json};

use crate::backend::{MAX_TEXT, Pending, Scope, Session, fingerprint, result, strings};
use crate::svn::{self, Change, Result};
use openidea_plugin::{FileKind, host};

pub const MAX_COMMIT_PATHS: usize = 200;

#[derive(Default)]
pub struct UpdateOptions<'a> {
    pub revision: &'a str,
    pub depth: &'a str,
    pub url: &'a str,
    pub force: bool,
    pub ignore_externals: bool,
}

pub fn confirm(title: &str, body: &str) -> Result<()> {
    if result(host::confirm(title, body))? {
        Ok(())
    } else {
        Err(tr("Operation canceled").into())
    }
}

pub fn display(paths: &[String]) -> String {
    let lines = paths
        .iter()
        .take(60)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    if paths.len() <= 60 {
        lines
    } else {
        trf!(
            "{0}\nSee the changes list for the remaining {1} items.",
            lines,
            paths.len() - 60
        )
    }
}

pub fn message(value: &str) -> Result<&str> {
    if value.trim().is_empty() || value.len() > 16_384 || value.contains('\0') {
        return Err(tr("Enter a commit message of at most 16 KiB").into());
    }
    Ok(value)
}

fn row<'a>(rows: &'a [Change], path: &str) -> Option<&'a Change> {
    rows.iter()
        .find(|row| svn::key(&row.path) == svn::key(path))
}

pub fn commit_targets(rows: &[Change], paths: &[String]) -> Result<()> {
    let selected: std::collections::HashSet<_> = paths.iter().map(|p| svn::key(p)).collect();
    for p in paths {
        let c = row(rows, p)
            .filter(|c| c.committable() || c.item == "unversioned" && !c.tree)
            .ok_or_else(|| {
                trf!(
                    "The target has no committable changes or still has conflicts: {0}",
                    p
                )
            })?;
        for pair in [&c.from, &c.to] {
            if !pair.is_empty() && !selected.contains(&svn::key(pair)) {
                return Err(trf!(
                    "Commit both sides of the move together: {0}\n{1}",
                    c.path,
                    pair
                ));
            }
        }
        if c.copied || matches!(c.item.as_str(), "deleted" | "replaced") {
            for other in rows.iter().filter(|other| {
                other.path != c.path && svn::within(&c.path, &other.path) && other.committable()
            }) {
                if !selected.contains(&svn::key(&other.path)) {
                    return Err(trf!(
                        "A directory copy, deletion or replacement must include its related changes: {0}\n{1}",
                        c.path,
                        other.path
                    ));
                }
            }
        }
    }
    Ok(())
}

impl Session {
    pub fn commit(
        &self,
        scope: &Scope,
        selected: &[String],
        value: &str,
        keep_locked: bool,
        auto_update: bool,
    ) -> Result<Value> {
        let note = message(value)?;
        let paths = self.targets(scope, selected, true)?;
        self.dirty(&paths)?;
        let (signature, _) = self.commit_signature(scope, &paths)?;
        self.recheck(scope, &paths)?;
        let (current, new) = self.commit_signature(scope, &paths)?;
        if current != signature {
            return Err(tr(
                "The selected changes have changed. Review them again before committing.",
            )
            .into());
        }
        let pending = Pending {
            label: "Commit".into(),
            path: scope.path.clone(),
            repository: Some(scope.info.repository.clone()),
        };
        let mut out = self.write(&scope.info.wc, &pending, || {
            self.with_text(note, |file| {
                let mut flags = strings(&[
                    "--depth",
                    "empty",
                    "--encoding",
                    "UTF-8",
                    "--file",
                    file,
                    "--force-log",
                    "--keep-changelists",
                ]);
                if keep_locked {
                    flags.push("--no-unlock".into());
                }
                self.verify_command("commit", &flags, &pegs(&paths))?;
                let mut paths = paths;
                let added = !new.is_empty();
                if added {
                    if let Err(error) = self.schedule_commit_additions(scope, &mut paths, new) {
                        return Err(trf!("{0}\nPaths already added to version control remain pending for commit.", error));
                    }
                    self.dirty(&paths)?;
                }
                let rows = self.changes(scope)?;
                commit_targets(&rows, &paths)?;
                if paths
                    .iter()
                    .any(|path| !row(&rows, path).is_some_and(Change::committable))
                {
                    return Err(tr("Some selected paths could not be added to version control. Refresh and try again.").into());
                }
                self.stream("commit", &flags, &pegs(&paths))
                    .map_err(|error| {
                        if added {
                            trf!("{0}\nNewly added files may still be pending for commit.", error)
                        } else {
                            error
                        }
                    })
            })
        })?;
        let update_error = if auto_update {
            match self.update(
                scope,
                &[],
                UpdateOptions {
                    ignore_externals: true,
                    ..UpdateOptions::default()
                },
            ) {
                Ok(data) => {
                    if let Some(text) = data["output"].as_str() {
                        out.push('\n');
                        out.push_str(text);
                    }
                    None
                }
                Err(error) => Some(trf!("Commit succeeded. Update failed: {0}", error)),
            }
        } else {
            None
        };
        let mut result = json!({"output": out, "committed": true});
        if let Some(error) = update_error {
            result["updateError"] = json!(error);
        }
        Ok(result)
    }

    pub(crate) fn schedule_commit_additions(
        &self,
        scope: &Scope,
        paths: &mut Vec<String>,
        mut level: Vec<String>,
    ) -> Result<()> {
        let mut included: std::collections::HashSet<_> =
            paths.iter().map(|p| svn::key(p)).collect();
        while !level.is_empty() {
            let mut directories = Vec::new();
            for path in &level {
                self.check_path(path)?;
                let stat = result(self.project.stat(path))?;
                if stat.kind == FileKind::Directory {
                    directories.push(path.clone());
                }
            }
            self.dirty(&level)?;
            self.read("add", &strings(&["--depth", "empty"]), &pegs(&level))?;
            if directories.is_empty() {
                break;
            }
            let status = self.read(
                "status",
                &strings(&[
                    "--xml",
                    "--depth",
                    "immediates",
                    "--no-ignore",
                    "--ignore-externals",
                ]),
                &pegs(&directories),
            )?;
            level = svn::status(&status, &self.root)?
                .into_iter()
                .filter(|change| change.item == "unversioned")
                .map(|change| change.path)
                .filter(|path| {
                    svn::project_path(&scope.path, path).is_ok() && included.insert(svn::key(path))
                })
                .collect();
            if paths.len() + level.len() > MAX_COMMIT_PATHS {
                return Err(trf!(
                    "The selected directory contains more than {0} paths to commit. Narrow the selection.",
                    MAX_COMMIT_PATHS
                ));
            }
            if !level.is_empty() {
                self.targets(scope, &level, false)?;
                paths.extend(level.iter().cloned());
            }
        }
        Ok(())
    }

    fn commit_signature(&self, scope: &Scope, paths: &[String]) -> Result<(u64, Vec<String>)> {
        let rows = self.changes(scope)?;
        commit_targets(&rows, paths)?;
        let mut new = Vec::new();
        let selected = paths
            .iter()
            .map(|p| {
                let row = row(&rows, p).ok_or(tr("The commit status has changed"))?;
                if row.item == "unversioned" {
                    new.push(p.clone());
                }
                let status = serde_json::to_string(row).map_err(|e| e.to_string())?;
                let stat = match self.project.stat(p) {
                    Ok(stat) => Some(stat),
                    Err(openidea_plugin::Error::NotFound(_)) if row.item == "deleted" => None,
                    Err(error) => return result(Err(error)),
                };
                let bytes = if stat.as_ref().is_some_and(|s| {
                    s.kind == openidea_plugin::FileKind::File && s.size <= MAX_TEXT as u64
                }) {
                    Some(fingerprint(result(self.project.read_bytes(p))?))
                } else {
                    None
                };
                Ok((status, stat.map(|s| (s.size, s.modified)), bytes))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((fingerprint(selected), new))
    }

    pub fn recheck(&self, scope: &Scope, paths: &[String]) -> Result<()> {
        let current = self
            .scope(&scope.path)?
            .ok_or(tr("The working copy changed. Select it again."))?;
        if current.info.uuid != scope.info.uuid
            || current.info.url != scope.info.url
            || svn::key(&current.info.wc) != svn::key(&scope.info.wc)
        {
            return Err(tr("The working copy URL or root changed. Refresh and try again.").into());
        }
        self.targets(scope, paths, true).map(drop)
    }

    pub fn scoped_write(
        &self,
        scope: &Scope,
        label: &str,
        verb: &str,
        flags: &[String],
        targets: &[String],
        checked: &[String],
        repo: Option<String>,
    ) -> Result<String> {
        self.dirty(checked)?;
        self.recheck(scope, checked)?;
        self.verify_command(verb, flags, targets)?;
        let pending = Pending {
            label: label.into(),
            path: scope.path.clone(),
            repository: repo,
        };
        self.write(&scope.info.wc, &pending, || {
            self.stream(verb, flags, targets)
        })
    }

    pub fn update(
        &self,
        scope: &Scope,
        selected: &[String],
        options: UpdateOptions,
    ) -> Result<Value> {
        let revision = svn::revision(options.revision)?;
        let paths = if selected.is_empty() {
            vec![scope.path.clone()]
        } else {
            self.targets(scope, selected, true)?
        };
        let mut flags = strings(&["--accept", "postpone", "-r", &revision]);
        if options.ignore_externals {
            flags.push("--ignore-externals".into());
        }
        if options.force {
            flags.push("--force".into());
        }
        if !options.depth.is_empty() {
            if !matches!(options.depth, "empty" | "files" | "immediates" | "infinity") {
                return Err(tr("Invalid update depth").into());
            }
            flags.extend(strings(&["--depth", options.depth]));
        }
        let mut targets = pegs(&paths);
        let verb = if options.url.trim().is_empty() {
            "update"
        } else {
            if paths.len() != 1 {
                return Err(tr("Select one working copy target to switch").into());
            }
            let url = svn::url(options.url)?;
            let uri = format!("{url}@{revision}");
            let remote = svn::info(&self.read(
                "info",
                &strings(&["--xml", "-r", &revision]),
                std::slice::from_ref(&uri),
            )?)?;
            if remote.uuid != scope.info.uuid || remote.kind != self.info(&paths[0])?.kind {
                return Err(
                    tr("Select a destination of the same type in the same repository").into(),
                );
            }
            targets.insert(0, uri);
            "switch"
        };
        let out = self.scoped_write(scope, "Update", verb, &flags, &targets, &paths, None)?;
        Ok(json!({"output": out, "updated": true}))
    }

    pub fn add(&self, scope: &Scope, paths: &[String]) -> Result<Value> {
        let paths = self.targets(scope, paths, false)?;
        for p in &paths {
            result(self.project.stat(p))?;
        }
        let out = self.scoped_write(
            scope,
            "Add",
            "add",
            &strings(&["--depth", "empty"]),
            &pegs(&paths),
            &paths,
            None,
        )?;
        Ok(json!({"output": out}))
    }

    pub fn delete_or_revert(&self, scope: &Scope, paths: &[String], revert: bool) -> Result<Value> {
        let paths = self.targets(scope, paths, false)?;
        let verb = if revert { "revert" } else { "delete" };
        let title = if revert {
            "Revert SVN Changes"
        } else {
            "Delete Versioned Targets"
        };
        self.dirty(&paths)?;
        confirm(
            tr(title),
            &trf!(
                "Review the selected paths. If you selected a directory, its contents are included.\n\n{0}",
                display(&paths)
            ),
        )?;
        let flags = if revert {
            strings(&["--depth", "infinity"])
        } else {
            Vec::new()
        };
        let out = self.scoped_write(scope, title, verb, &flags, &pegs(&paths), &paths, None)?;
        Ok(json!({"output": out}))
    }

    pub fn rename(&self, scope: &Scope, paths: &[String], name: &str) -> Result<Value> {
        let paths = self.targets(scope, paths, false)?;
        if paths.len() != 1 {
            return Err(tr("Select one target to move or rename").into());
        }
        let input = name.trim();
        let dest = if input.contains(['/', '\\']) {
            svn::project_path(&scope.path, input)?
        } else {
            let parent =
                svn::parent(&paths[0]).ok_or(tr("Cannot determine the destination directory"))?;
            svn::join(&parent, svn::leaf(input)?)?
        };
        let source = &paths[0];
        let parent = svn::parent(&dest).ok_or(tr("The destination directory is missing"))?;
        self.dir(&parent)?;
        self.targets(scope, std::slice::from_ref(&parent), true)?;
        if svn::within(source, &dest) || result(self.project.exists(&dest))? {
            return Err(
                tr("The destination already exists or is inside the source directory").into(),
            );
        }
        confirm(tr("Move or Rename in SVN"), &format!("{source}\n→ {dest}"))?;
        if result(self.project.exists(&dest))? {
            return Err(tr("The destination already exists").into());
        }
        let checked = [source.clone(), parent];
        let out = self.scoped_write(
            scope,
            "Move or rename",
            "move",
            &[],
            &[svn::peg(source), dest],
            &checked,
            None,
        )?;
        Ok(json!({"output": out}))
    }

    pub fn cleanup(&self, scope: &Scope, selected: &[String]) -> Result<Value> {
        let paths = if selected.is_empty() {
            vec![scope.path.clone()]
        } else {
            self.targets(scope, selected, true)?
        };
        for path in &paths {
            self.dir(path)?;
        }
        let out = self.scoped_write(
            scope,
            "Clean Up SVN Administrative Locks",
            "cleanup",
            &[],
            &pegs(&paths),
            &paths,
            None,
        )?;
        Ok(json!({"output": out}))
    }

    pub fn lock(&self, scope: &Scope, inputs: &[String], lock: bool, note: &str) -> Result<Value> {
        let paths = self.targets(scope, inputs, false)?;
        let label = if lock { "Lock" } else { "Unlock" };
        let verb = if lock { "lock" } else { "unlock" };
        let flags = if !lock || note.is_empty() {
            Vec::new()
        } else {
            strings(&["--encoding", "UTF-8", "--message", message(note)?])
        };
        confirm(&format!("SVN · {}", tr(label)), &display(&paths))?;
        let out = self.scoped_write(
            scope,
            label,
            verb,
            &flags,
            &pegs(&paths),
            &paths,
            Some(scope.info.repository.clone()),
        )?;
        Ok(json!({"output": out}))
    }

    pub fn changelist(
        &self,
        scope: &Scope,
        inputs: &[String],
        name: &str,
        remove: bool,
    ) -> Result<Value> {
        let paths = self.targets(scope, inputs, false)?;
        let flags = if remove {
            strings(&["--remove"])
        } else {
            if name.trim().is_empty()
                || name.len() > 128
                || name.starts_with('-')
                || name.contains(['\0', '\r', '\n'])
            {
                return Err(tr("Enter a valid changelist name").into());
            }
            strings(&[name.trim()])
        };
        let out = self.scoped_write(
            scope,
            "Changelist",
            "changelist",
            &flags,
            &pegs(&paths),
            &paths,
            None,
        )?;
        Ok(json!({"output": out}))
    }

    pub fn property_edit(
        &self,
        scope: &Scope,
        targets: &[String],
        name: &str,
        value: &str,
        delete: bool,
    ) -> Result<Value> {
        if name.is_empty()
            || name.starts_with('-')
            || name.len() > 128
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c))
        {
            return Err(tr("Enter a valid SVN property name").into());
        }
        if value.len() > 512 << 10 || value.contains('\0') {
            return Err(tr("The property value is too long or contains a NUL character").into());
        }
        let paths = self.targets(scope, targets, true)?;
        let body = trf!("Property {0}\n{1}", name, display(&paths));
        confirm(
            if delete {
                tr("Delete SVN Property")
            } else {
                tr("Save SVN Property")
            },
            &body,
        )?;
        let out = if delete {
            self.scoped_write(
                scope,
                "Delete property",
                "propdel",
                &strings(&[name]),
                &pegs(&paths),
                &paths,
                None,
            )?
        } else {
            self.with_text(value, |file| {
                let mut flags = strings(&[name, "--file", file]);
                if name.starts_with("svn:") {
                    flags.extend(strings(&["--encoding", "UTF-8"]));
                }
                self.scoped_write(
                    scope,
                    "Set property",
                    "propset",
                    &flags,
                    &pegs(&paths),
                    &paths,
                    None,
                )
            })?
        };
        Ok(json!({"output": out}))
    }

    pub fn ignore(&self, scope: &Scope, inputs: &[String], remove: bool) -> Result<Value> {
        let paths = self.targets(scope, inputs, false)?;
        if paths.len() != 1 {
            return Err(tr("Change ignore rules one target at a time").into());
        }
        let original = paths[0].clone();
        let rows = self.changes(scope)?;
        let expected = if remove { "ignored" } else { "unversioned" };
        row(&rows, &original)
            .filter(|r| r.item == expected)
            .ok_or_else(|| trf!("The target must have {0} status", expected))?;
        let parent = svn::parent(&original).ok_or(tr("Cannot determine the parent directory"))?;
        let name = original
            .rsplit('/')
            .next()
            .ok_or(tr("A file name is required"))?;
        let raw = self.read(
            "proplist",
            &strings(&["--xml", "--verbose"]),
            &[svn::peg(&parent)],
        )?;
        let properties = svn::properties(&raw)?;
        let current = properties.iter().find(|p| p.name == "svn:ignore");
        if current.is_some_and(|p| p.binary) {
            return Err(tr("Use the Properties tab to edit binary ignore properties").into());
        }
        let old = current.map_or("", |p| p.value.as_str());
        let mut lines: Vec<_> = old.lines().collect();
        let literal = literal_glob(name);
        if remove {
            if !lines.contains(&literal.as_str())
                && (!lines.contains(&name) || name.contains(['*', '?', '[', ']']))
            {
                return Err(
                    tr("No matching rule on the immediate parent. Check wildcard or inherited ignore rules in Properties.").into(),
                );
            }
            lines.retain(|line| *line != literal && *line != name);
        } else if !lines.contains(&literal.as_str()) {
            lines.push(&literal);
        }
        let value = lines.join("\n");
        self.property_edit(
            scope,
            &[parent],
            "svn:ignore",
            &value,
            remove && lines.is_empty(),
        )
    }
}

pub fn pegs(paths: &[String]) -> Vec<String> {
    paths.iter().map(|p| svn::peg(p)).collect()
}

fn literal_glob(name: &str) -> String {
    let mut result = String::new();
    for c in name.chars() {
        match c {
            '[' => result.push_str("[[]"),
            ']' => result.push_str("[]]"),
            '?' => result.push_str("[?]"),
            '*' => result.push_str("[*]"),
            other => result.push(other),
        }
    }
    result
}
