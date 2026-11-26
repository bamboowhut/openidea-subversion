use crate::i18n::{tr, trf};
use openidea_plugin::{Decoration, FileDiff, FileKind, Position, Range, host};
use serde_json::{Value, json};

use crate::backend::{MAX_TEXT, Pending, Scope, Session, fingerprint, result, strings, text};
use crate::svn::{self, Info, Result, RevisionPath};
use crate::working::{confirm, pegs};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DifferenceEdit {
    left_start: usize,
    left_length: usize,
    right_start: usize,
    right_length: usize,
    append: bool,
}

fn apply_difference(left: &str, right: &str, hunk: &DifferenceEdit) -> Result<String> {
    let base = left.replace("\r\n", "\n");
    let bom = right.starts_with('\u{feff}');
    let local = right
        .strip_prefix('\u{feff}')
        .unwrap_or(right)
        .replace("\r\n", "\n");
    let eol = if right.contains("\r\n") { "\r\n" } else { "\n" };
    let base_lines: Vec<_> = base.split('\n').collect();
    let mut local_lines: Vec<_> = local.split('\n').collect();
    let left_end = hunk.left_start.saturating_add(hunk.left_length);
    let right_end = hunk.right_start.saturating_add(hunk.right_length);
    let replacement = base_lines
        .get(hunk.left_start..left_end)
        .ok_or(tr("The selected difference is out of date"))?;
    if right_end > local_lines.len()
        || (hunk.append && (hunk.left_length == 0 || hunk.right_length == 0))
    {
        return Err(tr("The selected difference is out of date").into());
    }
    let start = if hunk.append {
        right_end
    } else {
        hunk.right_start
    };
    local_lines.splice(start..right_end, replacement.iter().copied());
    let mut text = local_lines.join(eol);
    if bom {
        text.insert(0, '\u{feff}');
    }
    if text.len() > MAX_TEXT {
        return Err(tr("The edited file would exceed the text preview limit").into());
    }
    Ok(text)
}

#[cfg(test)]
mod preview_tests {
    use super::{DifferenceEdit, apply_difference};

    #[test]
    fn revert_and_append_change_only_the_selected_lines() {
        let base = "prefix\nbase\nsuffix\nolder change\n";
        let current = "\u{feff}prefix\r\nlocal\r\nsuffix\r\nother edit\r\n";
        let mut hunk = DifferenceEdit {
            left_start: 1,
            left_length: 1,
            right_start: 1,
            right_length: 1,
            append: false,
        };
        assert_eq!(
            apply_difference(base, current, &hunk).unwrap(),
            "\u{feff}prefix\r\nbase\r\nsuffix\r\nother edit\r\n"
        );
        hunk.append = true;
        assert_eq!(
            apply_difference(base, current, &hunk).unwrap(),
            "\u{feff}prefix\r\nlocal\r\nbase\r\nsuffix\r\nother edit\r\n"
        );
        hunk.right_length = 50;
        assert!(apply_difference(base, current, &hunk).is_err());
    }
}

pub fn show_diff(
    left: Option<String>,
    right: Option<String>,
    name: &str,
    left_label: &str,
    right_label: &str,
) -> Result<Value> {
    let title = format!("SVN · {}", name.rsplit('/').next().unwrap_or(name));
    result(host::open_file_diff(
        &FileDiff {
            left,
            right,
            left_label: Some(left_label.into()),
            right_label: Some(right_label.into()),
            title: Some(title),
        },
        Some(name.rsplit('/').next().unwrap_or(name)),
    ))?;
    Ok(json!({}))
}

fn dirty_text(session: &Session, path: &str, encoding: &str) -> Result<String> {
    let editors = result(host::open_editors())?;
    if editors
        .iter()
        .any(|e| e.dirty && svn::canonical(&e.path).is_ok_and(|p| svn::key(&p) == svn::key(path)))
    {
        let value = result(session.project.read_file(path))?;
        if value.len() > MAX_TEXT {
            return Err(tr("The file exceeds the 512 KiB text preview limit").into());
        }
        return Ok(value);
    }
    text(&result(session.project.read_bytes(path))?, encoding)
}

impl Session {
    pub fn compare_local(
        &self,
        target: &str,
        info: &Info,
        branch: &str,
        revision: &str,
        encoding: &str,
    ) -> Result<Value> {
        let revision = svn::revision(revision)?;
        if !branch.is_empty() {
            let url = svn::url(branch)?;
            let source = svn::info(&self.read(
                "info",
                &strings(&["--xml", "-r", &revision]),
                &[format!("{url}@{revision}")],
            )?)?;
            if source.uuid != info.uuid || source.kind != "file" || info.kind != "file" {
                return Err(tr(
                    "Select the corresponding file on a branch in the same SVN repository",
                )
                .into());
            }
            self.check_special(target)?;
            let left = self.content(&url, &format!("@{revision}"), &revision, encoding)?;
            let right = dirty_text(self, target, encoding)?;
            return show_diff(Some(left), Some(right), target, &url, tr("Current version"));
        }
        if info.kind == "dir" {
            return self.patch(&[svn::peg(target)], &strings(&["-r", &revision]));
        }
        self.check_special(target)?;
        let left = self.content(
            &info.url,
            &format!("@{}", info.revision),
            &revision,
            encoding,
        )?;
        let right = dirty_text(self, target, encoding)?;
        show_diff(
            Some(left),
            Some(right),
            target,
            &revision,
            tr("Current version"),
        )
    }

    pub fn local_diff(
        &self,
        scope: &Scope,
        input: &str,
        encoding: &str,
        preview: bool,
    ) -> Result<Value> {
        let target = self.targets(scope, &[input.into()], true)?.remove(0);
        let rows = svn::status(
            &self.read(
                "status",
                &strings(&["--xml", "--depth", "empty", "--no-ignore"]),
                &[svn::peg(&target)],
            )?,
            &self.root,
        )?;
        let change = rows.iter().find(|c| svn::key(&c.path) == svn::key(&target));
        let info = self.info(&target);
        let revision = info.as_ref().ok().map(|info| info.revision);
        let disk = self.project.stat(&target);
        if info.as_ref().is_ok_and(|i| i.kind == "dir")
            || disk.as_ref().is_ok_and(|s| s.kind == FileKind::Directory)
        {
            if preview {
                return Ok(
                    json!({"localPreview": {"path": target, "message": tr("Select a file to show its differences")}}),
                );
            }
            return self.patch(&[svn::peg(&target)], &[]);
        }
        self.check_special(&target)?;
        let left = match (change, info) {
            (Some(c), Ok(i)) if c.copied && !i.copy_from.is_empty() => {
                let rev = i
                    .copy_rev
                    .ok_or(tr("SVN did not return the source revision of the copy"))?;
                Some(self.content(&i.copy_from, &format!("@{rev}"), &rev.to_string(), encoding)?)
            }
            (Some(c), _) if matches!(c.item.as_str(), "added" | "unversioned" | "ignored") => None,
            (_, Ok(_)) => Some(self.content(&target, "@BASE", "BASE", encoding)?),
            (_, Err(error)) => return Err(error),
        };
        let right = match disk {
            Ok(_) if !change.is_some_and(|c| c.item == "deleted") => {
                Some(dirty_text(self, &target, encoding)?)
            }
            _ => None,
        };
        if preview {
            let edit_hash = if let (Some(base), Some(current)) = (&left, &right) {
                if change.is_some_and(|change| {
                    change.item == "modified" && !change.tree && change.props != "conflicted"
                }) && self.dirty(std::slice::from_ref(&target)).is_ok()
                {
                    self.project.read_bytes(&target).ok().and_then(|bytes| {
                        let utf8 = std::str::from_utf8(&bytes).ok()?;
                        let normalized = utf8.replace("\r\n", "\n");
                        let other_endings = utf8.replace("\r\n", "");
                        (utf8.strip_prefix('\u{feff}').unwrap_or(utf8) == current
                            && !utf8.contains('\0')
                            && !base.contains('\0')
                            && !normalized.contains('\r')
                            && !(utf8.contains("\r\n") && other_endings.contains('\n')))
                        .then(|| format!("{:016x}", fingerprint((&target, base, &bytes))))
                    })
                } else {
                    None
                }
            } else {
                None
            };
            return Ok(
                json!({"localPreview": {"path": target, "left": left, "right": right, "revision": revision, "editHash": edit_hash, "leftLabel": tr("Base version"), "rightLabel": tr("Current version")}}),
            );
        }
        show_diff(
            left,
            right,
            &target,
            tr("Base version"),
            tr("Current version"),
        )
    }

    pub fn edit_difference(
        &self,
        scope: &Scope,
        path: &str,
        encoding: &str,
        hash: &str,
        hunk: &DifferenceEdit,
    ) -> Result<Value> {
        let target = self.targets(scope, &[path.into()], false)?.remove(0);
        let current = self.local_diff(scope, &target, encoding, true)?;
        let preview = &current["localPreview"];
        let stale = tr("The file or its base version changed. Refresh the diff and try again.");
        if preview["editHash"].as_str() != Some(hash) {
            return Err(stale.into());
        }
        let base = preview["left"].as_str().ok_or(stale)?;
        self.dirty(std::slice::from_ref(&target))?;
        let bytes = result(self.project.read_bytes(&target))?;
        if format!("{:016x}", fingerprint((&target, base, &bytes))) != hash {
            return Err(stale.into());
        }
        let local = std::str::from_utf8(&bytes).map_err(|_| stale)?;
        let result_text = apply_difference(base, local, hunk)?;
        result(self.project.write_file(&target, &result_text))?;
        Ok(
            json!({"previewEdited": target, "output": if hunk.append { tr("Appended difference") } else { tr("Reverted difference") }}),
        )
    }

    pub fn compare_clipboard(
        &self,
        scope: &Scope,
        path: &str,
        encoding: &str,
        side: &str,
        clipboard: &str,
    ) -> Result<Value> {
        if clipboard.len() > MAX_TEXT || !matches!(side, "left" | "right") {
            return Err(tr(
                "Clipboard text exceeds the diff limit or the selected side is unavailable",
            )
            .into());
        }
        let local = self.local_diff(scope, path, encoding, true)?;
        let original = local["localPreview"][side]
            .as_str()
            .ok_or(tr("The selected side is unavailable"))?;
        show_diff(
            Some(original.into()),
            Some(clipboard.into()),
            &format!("Clipboard · {}", path.rsplit('/').next().unwrap_or(path)),
            if side == "left" {
                tr("Base version")
            } else {
                tr("Current version")
            },
            tr("Clipboard"),
        )
    }

    fn check_special(&self, target: &str) -> Result<()> {
        if let Ok(xml) = self.read(
            "proplist",
            &strings(&["--xml", "--verbose"]),
            &[svn::peg(target)],
        ) {
            for p in svn::properties(&xml)? {
                if p.name == "svn:special"
                    || p.name == "svn:mime-type" && !p.value.starts_with("text/")
                {
                    return Err(tr("SVN reports a binary or special file. You can inspect its status and properties.").into());
                }
            }
        }
        Ok(())
    }

    pub fn patch(&self, targets: &[String], flags: &[String]) -> Result<Value> {
        let mut args = strings(&["--internal-diff", "--ignore-properties"]);
        if flags.iter().any(|f| f == "--properties-only") {
            args.pop();
        }
        args.extend_from_slice(flags);
        let p = self.read("diff", &args, targets)?;
        let truncated = p.len() > MAX_TEXT;
        let limit = p.floor_char_boundary(MAX_TEXT.min(p.len()));
        Ok(json!({"patch": &p[..limit], "truncated": truncated}))
    }

    pub fn compare(
        &self,
        from: &str,
        to: &str,
        from_rev: &str,
        to_rev: &str,
        encoding: &str,
    ) -> Result<Value> {
        let a = svn::url(from)?;
        let b = svn::url(to)?;
        let ar = svn::revision(from_rev)?;
        let br = svn::revision(to_rev)?;
        let ap = format!("{a}@{ar}");
        let bp = format!("{b}@{br}");
        let left =
            svn::info(&self.read("info", &strings(&["--xml", "-r", &ar]), &[ap.clone()])?)?;
        let right =
            svn::info(&self.read("info", &strings(&["--xml", "-r", &br]), &[bp.clone()])?)?;
        if left.kind != right.kind || left.uuid != right.uuid {
            return Err(tr("Compare targets of the same type in the same repository").into());
        }
        if left.kind == "dir" {
            return self.patch(&[ap, bp], &[]);
        }
        let base = self.content(&a, &format!("@{ar}"), &ar, encoding)?;
        let next = self.content(&b, &format!("@{br}"), &br, encoding)?;
        show_diff(
            Some(base),
            Some(next),
            &a,
            &trf!("Source r{0}", left.revision),
            &trf!("Destination r{0}", right.revision),
        )
    }

    pub fn log(&self, target: &str, cursor: Option<u64>, peg: Option<u64>) -> Result<Value> {
        let target = peg.map_or_else(|| svn::peg(target), |r| svn::pegged(target, r));
        let range = cursor.map_or_else(|| "HEAD:0".into(), |r| format!("{r}:0"));
        let raw = self.read(
            "log",
            &strings(&["--xml", "--verbose", "--limit", "30", "-r", &range]),
            &[target],
        )?;
        let entries = svn::history(&raw)?;
        let next = (entries.len() == 30)
            .then(|| entries.last().and_then(|e| e.revision.checked_sub(1)))
            .flatten();
        Ok(json!({"entries": entries, "next": next}))
    }

    pub fn incoming(&self, scope: &Scope, cursor: Option<u64>) -> Result<Value> {
        let source = svn::peg(&scope.path);
        let status = self.read(
            "status",
            &strings(&["--xml", "--show-updates", "--quiet", "--ignore-externals"]),
            std::slice::from_ref(&source),
        )?;
        let (head, updates) = svn::remote_status(&status, &self.root)?;
        let mut candidates = Vec::new();
        for update in updates {
            if !svn::within(&scope.path, &update.path) {
                continue;
            }
            let relative = update
                .path
                .get(scope.path.trim_end_matches('/').len()..)
                .ok_or(tr("Invalid working copy path"))?;
            let url = svn::url_path(&scope.info.url, relative)?;
            let revision = if let Some(revision) = update.revision {
                revision
            } else if let Some(parent) = svn::parent(&update.path) {
                self.info(&parent)
                    .map_or(scope.info.revision, |info| info.revision)
            } else {
                scope.info.revision
            };
            candidates.push((url, revision, update.item));
        }
        let oldest = candidates.iter().map(|(_, revision, _)| *revision).min();
        let (Some(upper), Some(low)) = (cursor.or(head), oldest) else {
            return Ok(json!({"entries": [], "next": null}));
        };
        if upper <= low {
            return Ok(json!({"entries": [], "next": null}));
        }
        let range = format!("{upper}:{}", low + 1);
        let log = self.read(
            "log",
            &strings(&[
                "--xml",
                "--verbose",
                "--stop-on-copy",
                "--limit",
                "100",
                "-r",
                &range,
            ]),
            &[source],
        )?;
        let mut entries = svn::history(&log)?;
        let next = if entries.len() == 100 {
            entries
                .last()
                .map(|entry| entry.revision - 1)
                .filter(|rev| *rev > low)
        } else {
            None
        };
        for entry in &mut entries {
            entry.changes.retain(|change| {
                let Ok(changed) = svn::url_path(&scope.info.repository, &change.path) else {
                    return false;
                };
                candidates.iter().any(|(path, revision, item)| {
                    entry.revision > *revision
                        && (*path == changed
                            || matches!(item.as_str(), "added" | "deleted" | "replaced")
                                && svn::within(path, &changed)
                            || change.kind == "dir"
                                && matches!(change.action.as_str(), "A" | "D" | "R")
                                && svn::within(&changed, path))
                })
            });
        }
        entries.retain(|entry| !entry.changes.is_empty());
        Ok(json!({"entries": entries, "next": next, "historyRoot": scope.info.repository}))
    }

    pub fn history_diff(
        &self,
        root: &str,
        change: &RevisionPath,
        rev: u64,
        encoding: &str,
    ) -> Result<Value> {
        let root = svn::url(root)?;
        if rev == 0 || !matches!(change.action.as_str(), "A" | "D" | "M" | "R") {
            return Err(tr("Invalid change from history").into());
        }
        let url = svn::url_path(&root, &change.path)?;
        let prev = if let Some(rev) = change.from_rev {
            Some((svn::url_path(&root, &change.from)?, rev))
        } else if change.action == "A" {
            None
        } else {
            Some((url.clone(), rev - 1))
        };
        let after = (change.action != "D").then_some((url.clone(), rev));
        if change.kind == "dir" {
            if let (Some((a, ar)), Some((b, br))) = (&prev, &after) {
                return self.patch(&[svn::pegged(a, *ar), svn::pegged(b, *br)], &[]);
            }
            let (url, rev) = prev
                .as_ref()
                .or(after.as_ref())
                .ok_or(tr("No directory revision available"))?;
            let data = self.read(
                "list",
                &strings(&["--xml", "-r", &rev.to_string()]),
                &[svn::pegged(url, *rev)],
            )?;
            return Ok(
                json!({"patch": svn::list(&data)?.into_iter().map(|r| r.name).collect::<Vec<_>>().join("\n")}),
            );
        }
        let content = |address: &Option<(String, u64)>| -> Result<Option<String>> {
            address
                .as_ref()
                .map(|(uri, rev)| self.content(uri, &format!("@{rev}"), &rev.to_string(), encoding))
                .transpose()
        };
        let left = content(&prev)?;
        let right = content(&after)?;
        let base = prev.as_ref().map_or_else(
            || tr("Does not exist").into(),
            |(u, r)| format!("{u} @ r{r}"),
        );
        show_diff(left, right, &change.path, &base, &format!("r{rev}"))
    }

    pub fn annotate(&self, scope: &Scope, path: &str, encoding: &str) -> Result<Value> {
        let path = self.targets(scope, &[path.into()], false)?.remove(0);
        let info = self.info(&path)?;
        if info.kind != "file" {
            return Err(tr("Select a versioned file").into());
        }
        self.dirty(std::slice::from_ref(&path))?;
        self.check_special(&path)?;
        let current = dirty_text(self, &path, encoding)?;
        let base = self.content(&path, "@BASE", "BASE", encoding)?;
        let raw = self.read(
            "blame",
            &strings(&["--xml", "-r", "1:BASE"]),
            &[svn::peg(&path)],
        )?;
        let annotations = svn::blame(&raw)?;
        let unchanged = current.lines().eq(base.lines());
        if unchanged {
            self.show_annotations(&path, &annotations)?;
        }
        Ok(
            json!({"annotations": annotations.into_iter().take(10_000).collect::<Vec<_>>(), "annotatedPath": path, "matchesEditor": unchanged}),
        )
    }

    fn show_annotations(&self, path: &str, lines: &[svn::Attribution]) -> Result<()> {
        let set = crate::backend::storage_key("annotation", &self.root);
        let _ = host::clear_decorations(&set);
        let items = lines
            .iter()
            .take(10_000)
            .filter(|a| a.line > 0)
            .map(|a| {
                let pos = Position::new(a.line - 1, 0);
                Decoration {
                    range: Range {
                        start: pos,
                        end: pos,
                    },
                    style: "muted".into(),
                    hover: Some(format!("r{} · {}\n{}", a.revision, a.author, a.date)),
                    after: Some(format!(
                        "  r{} {}",
                        a.revision,
                        a.author.chars().take(60).collect::<String>()
                    )),
                }
            })
            .collect::<Vec<_>>();
        result(host::set_decorations(path, &set, &items))
    }

    pub fn props(&self, scope: &Scope, path: &str, against_base: bool) -> Result<Value> {
        let target = self.targets(scope, &[path.into()], true)?.remove(0);
        if against_base {
            return self.patch(&[svn::peg(&target)], &strings(&["--properties-only"]));
        }
        let xml = self.read(
            "proplist",
            &strings(&["--xml", "--verbose"]),
            &[svn::peg(&target)],
        )?;
        Ok(json!({"properties": svn::properties(&xml)?}))
    }

    pub fn conflict_panel(&self, scope: &Scope, path: &str, encoding: &str) -> Result<Value> {
        let target = self.targets(scope, &[path.into()], false)?.remove(0);
        let xml = self.read("info", &strings(&["--xml"]), &[svn::peg(&target)])?;
        let c = svn::conflict(&xml)?;
        if !c.text && !c.property && !c.tree {
            return Err(tr("This path has no SVN conflicts").into());
        }
        let artifact = |file: &str| -> Option<String> {
            if file.is_empty() {
                return None;
            }
            let here = svn::parent(&target)?;
            let path = svn::project_path(&scope.path, &svn::join(&here, file).ok()?).ok()?;
            self.check_path(&path).ok()?;
            text(&self.project.read_bytes(&path).ok()?, encoding).ok()
        };
        let working = if c.text {
            dirty_text(self, &target, encoding).ok()
        } else {
            None
        };
        let hash = self
            .project
            .read_bytes(&target)
            .ok()
            .map(|b| format!("{:016x}", fingerprint(b)));
        Ok(json!({
            "conflict": c, "path": target, "base": artifact(&c.base), "mine": artifact(&c.mine),
            "incoming": artifact(&c.incoming), "working": working, "hash": hash,
        }))
    }

    pub fn resolve(
        &self,
        scope: &Scope,
        path: &str,
        accept: &str,
        merged: Option<&str>,
        hash: &str,
    ) -> Result<Value> {
        let paths = self.targets(scope, &[path.into()], false)?;
        let path = &paths[0];
        let refresh = || -> Result<_> {
            let xml = self.read("info", &strings(&["--xml"]), &[svn::peg(path)])?;
            let c = svn::conflict(&xml)?;
            if !c.text && !c.property && !c.tree {
                return Err(tr("The conflict changed. Refresh and try again.").into());
            }
            Ok(c)
        };
        let c = refresh()?;
        if !matches!(accept, "working" | "mine-full" | "theirs-full" | "base")
            || (!c.text || c.tree) && accept != "working"
            || merged.is_some() && (!c.text || c.tree || c.property || accept != "working")
        {
            return Err(tr("The selected resolution does not apply to this conflict").into());
        }
        let verify_hash = |merged: &str| -> Result<()> {
            if merged.len() > MAX_TEXT || merged.contains('\0') {
                return Err(tr("The merge result exceeds the text write limit").into());
            }
            let current = result(self.project.read_bytes(path))?;
            if format!("{:016x}", fingerprint(current)) != hash {
                return Err(
                    tr("The file changed outside the editor. Reopen the conflict editor.").into(),
                );
            }
            Ok(())
        };
        if let Some(value) = merged {
            verify_hash(value)?;
        }
        self.dirty(&paths)?;
        confirm(
            tr("Resolve SVN Conflict"),
            &trf!(
                "{0}\nResolution: {1}. Confirm that you reviewed the conflicting text, properties and tree status.",
                path,
                accept
            ),
        )?;
        self.dirty(&paths)?;
        self.recheck(scope, &paths)?;
        let fresh = refresh()?;
        if fresh.text != c.text
            || fresh.property != c.property
            || fresh.tree != c.tree
            || fresh.base != c.base
            || fresh.mine != c.mine
            || fresh.incoming != c.incoming
        {
            return Err(tr("The conflict changed. Refresh and try again.").into());
        }
        if let Some(value) = merged {
            verify_hash(value)?;
        }
        let pending = Pending {
            label: "Resolve conflict".into(),
            path: scope.path.clone(),
            repository: None,
        };
        let out = self.write(&scope.info.wc, &pending, || {
            if let Some(value) = merged {
                result(self.project.write_file(path, value))?;
            }
            self.stream(
                "resolve",
                &strings(&["--depth", "empty", "--accept", accept]),
                &pegs(&paths),
            )
        })?;
        Ok(json!({"output": out, "resolved": true}))
    }
}
