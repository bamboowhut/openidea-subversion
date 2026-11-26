use crate::i18n::{tr, trf};
use openidea_plugin::host;
use serde_json::{Value, json};

use crate::backend::{Pending, Scope, Session, result, strings};
use crate::svn::{self, Result};
use crate::working::{confirm, message};

impl Session {
    pub fn browse(&self, input: &str, rev: &str) -> Result<Value> {
        let url = svn::url(input)?;
        let revision = svn::revision(rev)?;
        let target = format!("{url}@{revision}");
        let info = svn::info(&self.read(
            "info",
            &strings(&["--xml", "-r", &revision]),
            &[target.clone()],
        )?)?;
        if info.kind != "dir" {
            return Err(tr("Repository browsing requires a directory URL").into());
        }
        let raw = self.read("list", &strings(&["--xml", "-r", &revision]), &[target])?;
        let entries = svn::list(&raw)?
            .into_iter()
            .map(|e| {
                Ok(json!({
                    "url": svn::url_path(&url, &e.name)?, "name": e.name,
                    "kind": e.kind, "author": e.author, "revision": e.revision,
                }))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(
            json!({"repository": info.repository, "location": url, "entries": entries, "revision": info.revision}),
        )
    }

    pub fn checkout(&self, input: &str, rev: &str, name: &str, depth: &str) -> Result<Value> {
        let url = svn::url(input)?;
        let revision = svn::revision(rev)?;
        let depth = if depth.is_empty() { "infinity" } else { depth };
        if !matches!(depth, "empty" | "files" | "immediates" | "infinity") {
            return Err(tr("Invalid checkout depth").into());
        }
        let name = svn::leaf(name.trim())?;
        let parent = result(host::pick_directory(
            tr("Choose a Checkout Parent Inside the Open Project"),
            Some(&self.root),
        ))?
        .ok_or(tr("Operation canceled"))?;
        let parent = self.path(&parent)?;
        self.dir(&parent)?;
        let target = self.path(&svn::join(&parent, name)?)?;
        let usable = || -> Result<()> {
            self.dir(&parent)?;
            if result(self.project.exists(&target))? {
                return Err(tr("The checkout directory already exists. Enter a new name.").into());
            }
            if self.scope(&parent)?.is_some() {
                return Err(
                    tr("Choose a checkout location outside existing SVN working copies").into(),
                );
            }
            Ok(())
        };
        usable()?;
        confirm(
            tr("Check Out SVN Project"),
            &trf!(
                "{0}\nRevision {1} · {2}\n→ {3}",
                url,
                revision,
                depth,
                target
            ),
        )?;
        usable()?;
        let pending = Pending {
            label: "Checkout".into(),
            path: target.clone(),
            repository: None,
        };
        let flags = strings(&["--ignore-externals", "--depth", depth, "-r", &revision]);
        let args = [format!("{url}@{revision}"), target.clone()];
        self.verify_command("checkout", &flags, &args)?;
        let out = self.write(&self.root, &pending, || {
            let out = self.stream("checkout", &flags, &args)?;
            self.dir(&target)?;
            if self.scope(&target)?.is_none() {
                return Err(tr("Could not verify the new working copy").into());
            }
            Ok(out)
        })?;
        if result(host::confirm(
            tr("Checkout Complete"),
            tr("Open the checked-out project in a new window?"),
        ))? {
            result(host::open_project(&target, true))?;
        }
        Ok(json!({"output": out, "checkoutPath": target}))
    }

    pub fn import(&self, local: &str, input: &str, note: &str) -> Result<Value> {
        let source = self.cwd(local)?;
        let url = svn::url(input)?;
        let note = message(note)?;
        self.dirty(std::slice::from_ref(&source))?;
        if self.scope(&source)?.is_some() {
            return Err(tr(
                "Import requires an unversioned directory. Use Commit for a working copy.",
            )
            .into());
        }
        confirm(
            tr("Import Directory into SVN Repository"),
            &trf!(
                "SVN will upload the directory recursively using its ignore rules.\nMove any files that should stay local out of the directory before continuing.\n\n{0}\n→ {1}",
                source,
                url
            ),
        )?;
        self.dir(&source)?;
        self.dirty(std::slice::from_ref(&source))?;
        let pending = Pending {
            label: "Import".into(),
            path: source.clone(),
            repository: Some(url.clone()),
        };
        let out = self.write(&self.root, &pending, || {
            self.with_text(note, |file| {
                self.stream(
                    "import",
                    &strings(&[
                        "--depth",
                        "infinity",
                        "--encoding",
                        "UTF-8",
                        "--file",
                        file,
                        "--force-log",
                    ]),
                    &[source.clone(), svn::peg(&url)],
                )
            })
        })?;
        Ok(json!({"output": out}))
    }

    pub fn share(&self, local: &str, input: &str, note: &str, resume: bool) -> Result<Value> {
        let source = self.cwd(local)?;
        let url = svn::url(input)?;
        self.dirty(std::slice::from_ref(&source))?;
        if self.scope(&source)?.is_some() {
            return Err(tr("This path is already an SVN working copy").into());
        }
        confirm(
            tr("Share Local Project with SVN"),
            &trf!(
                "{0}\n→ {1}\n{2}\nAfter checkout, use Local Changes to review, add and commit files.",
                source,
                url,
                if resume {
                    tr("Connect to an existing empty repository directory.")
                } else {
                    tr("Create a repository directory and link the local directory to it.")
                }
            ),
        )?;
        self.dir(&source)?;
        self.dirty(std::slice::from_ref(&source))?;
        if self.scope(&source)?.is_some() {
            return Err(tr("This path is already an SVN working copy").into());
        }
        let pending = Pending {
            label: "Share project".into(),
            path: source.clone(),
            repository: Some(url.clone()),
        };
        let out = self.write(&self.root, &pending, || {
            if !resume {
                self.with_text(message(note)?, |file| {
                    self.stream(
                        "mkdir",
                        &strings(&["--encoding", "UTF-8", "--file", file, "--force-log"]),
                        &[svn::peg(&url)],
                    )
                })?;
            }
            let remote = self.browse(&url, "HEAD")?;
            if remote["entries"].as_array().is_none_or(|e| !e.is_empty()) {
                return Err(tr("Choose an empty repository directory to resume sharing").into());
            }
            let rev = remote["revision"]
                .as_u64()
                .ok_or(tr("Cannot determine the shared revision"))?;
            self.stream(
                "checkout",
                &strings(&["--ignore-externals", "-r", &rev.to_string()]),
                &[svn::pegged(&url, rev), source.clone()],
            )
        })?;
        Ok(json!({"output": out, "checkoutPath": source}))
    }

    pub fn relocate(&self, scope: &Scope, input: &str) -> Result<Value> {
        let to = svn::url(input)?;
        if svn::key(&svn::canonical(&scope.info.wc)?) != svn::key(&scope.path) {
            return Err(
                tr("Open the working copy root as its own project before relocating it").into(),
            );
        }
        let remote = self.info(&to)?;
        if remote.uuid != scope.info.uuid || remote.repository != remote.url {
            return Err(tr("Enter the new root URL for the same SVN repository UUID").into());
        }
        let from = &scope.info.repository;
        confirm(
            tr("Relocate SVN Repository"),
            &trf!("Working copy {0}\n{1}\n→ {2}", scope.path, from, remote.url),
        )?;
        let paths = [scope.path.clone()];
        let out = self.scoped_write(
            scope,
            "Relocate",
            "relocate",
            &strings(&["--ignore-externals"]),
            &[from.clone(), remote.url, scope.path.clone()],
            &paths,
            None,
        )?;
        Ok(json!({"output": out}))
    }

    pub fn copy_branch(
        &self,
        scope: Option<&Scope>,
        from: &str,
        to: &str,
        rev: &str,
        note: &str,
    ) -> Result<Value> {
        let from = svn::url(from)?;
        let to = svn::url(to)?;
        let revision = svn::revision(rev)?;
        let note = message(note)?;
        let source = svn::info(&self.read(
            "info",
            &strings(&["--xml", "-r", &revision]),
            &[format!("{from}@{revision}")],
        )?)?;
        if !to.starts_with(&format!("{}/", source.repository.trim_end_matches('/'))) {
            return Err(
                tr("The branch or tag destination must be in the same SVN repository").into(),
            );
        }
        let available = || -> Result<()> {
            let parent = self.info(&url_parent(&to)?)?;
            if parent.kind != "dir" || parent.uuid != source.uuid {
                return Err(
                    tr("Specify an existing parent directory in the same repository").into(),
                );
            }
            match self.info(&to) {
                Ok(_) => Err(tr("The branch or tag destination already exists").into()),
                Err(error)
                    if ["E160013", "W170000", "E200009"]
                        .iter()
                        .any(|code| error.contains(code)) =>
                {
                    Ok(())
                }
                Err(error) => Err(error),
            }
        };
        available()?;
        confirm(
            tr("Create SVN Branch or Tag"),
            &format!("{from} @ r{}\n→ {to}\n\n{note}", source.revision),
        )?;
        available()?;
        let key = scope.map_or(&self.root, |s| &s.info.wc);
        let pending = Pending {
            label: "Create branch or tag".into(),
            path: scope.map_or(self.root.clone(), |s| s.path.clone()),
            repository: Some(source.repository),
        };
        let out = self.write(key, &pending, || {
            self.with_text(note, |file| {
                self.stream(
                    "copy",
                    &strings(&[
                        "--encoding",
                        "UTF-8",
                        "--file",
                        file,
                        "--force-log",
                        "-r",
                        &source.revision.to_string(),
                    ]),
                    &[svn::pegged(&from, source.revision), to.clone()],
                )
            })
        })?;
        Ok(json!({"output": out}))
    }

    pub fn switch(&self, scope: &Scope, input: &str, rev: &str) -> Result<Value> {
        let url = svn::url(input)?;
        let revision = svn::revision(rev)?;
        let uri = format!("{url}@{revision}");
        let remote = svn::info(&self.read(
            "info",
            &strings(&["--xml", "-r", &revision]),
            &[uri.clone()],
        )?)?;
        if remote.uuid != scope.info.uuid || remote.kind != "dir" {
            return Err(tr("Switch requires a directory in the same repository").into());
        }
        confirm(
            tr("Switch SVN Branch"),
            &format!(
                "{}\n{}\n→ {url} @ r{}",
                scope.path, scope.info.url, remote.revision
            ),
        )?;
        let paths = [scope.path.clone()];
        let out = self.scoped_write(
            scope,
            "Switch branch",
            "switch",
            &strings(&[
                "--ignore-externals",
                "--accept",
                "postpone",
                "-r",
                &remote.revision.to_string(),
            ]),
            &[svn::pegged(&url, remote.revision), svn::peg(&scope.path)],
            &paths,
            None,
        )?;
        Ok(json!({"output": out}))
    }

    pub fn merge(
        &self,
        scope: &Scope,
        input: &str,
        rev: &str,
        changes: &str,
        preview: bool,
    ) -> Result<Value> {
        let url = svn::url(input)?;
        let rev = svn::revision(rev)?;
        let source = svn::info(&self.read(
            "info",
            &strings(&["--xml", "-r", &rev]),
            &[format!("{url}@{rev}")],
        )?)?;
        if source.uuid != scope.info.uuid || source.kind != scope.info.kind {
            return Err(
                tr("Merge requires a source of the same type in the same repository").into(),
            );
        }
        if source.url == scope.info.url {
            return Err(tr("Select a different branch as the merge source").into());
        }
        let mut flags = strings(&["--accept", "postpone"]);
        if !changes.trim().is_empty() {
            flags.extend(strings(&["-c", &svn::revisions(changes)?]));
        }
        let targets = [svn::pegged(&url, source.revision), svn::peg(&scope.path)];
        self.dirty(std::slice::from_ref(&scope.path))?;
        if preview {
            flags.push("--dry-run".into());
            return Ok(json!({"output": self.stream("merge", &flags, &targets)?, "preview": true}));
        }
        confirm(
            tr("Merge SVN Branch"),
            &trf!(
                "{0} @ r{1}\n→ {2}\n{3}\nThe merge result stays local for review and commit.",
                url,
                source.revision,
                scope.path,
                if changes.trim().is_empty() {
                    tr("Merge using SVN mergeinfo").into()
                } else {
                    trf!("Selected revisions: {0}", changes)
                }
            ),
        )?;
        let paths = [scope.path.clone()];
        let out = self.scoped_write(
            scope,
            "Merge branch",
            "merge",
            &flags,
            &targets,
            &paths,
            None,
        )?;
        Ok(json!({"output": out}))
    }

    pub fn recover(&self, scope: Option<&Scope>) -> Result<Value> {
        let scope_key = scope.map_or(&self.root, |s| &s.info.wc);
        let (key, record) = match self.pending(scope_key)? {
            Some(p) => (scope_key, p),
            None => match self.pending(&self.root)? {
                Some(p) => (&self.root, p),
                None => return Err(tr("There is no pending SVN operation to review").into()),
            },
        };
        let state = if let Some(local) = self.scope(&record.path).ok().flatten() {
            let rows = self.changes(&local)?;
            let lines = rows
                .iter()
                .take(15)
                .map(|r| format!("{}  {}", r.item, r.path))
                .collect::<Vec<_>>()
                .join("\n");
            trf!("Current status entries: {0}\n{1}", rows.len(), lines)
        } else if result(self.project.exists(&record.path))? {
            tr("The directory exists, but SVN did not report a usable working copy").into()
        } else {
            tr("The operation target does not exist on disk").into()
        };
        let remote = if let Some(address) = &record.repository {
            let info = match self.info(address) {
                Ok(i) => i,
                Err(e)
                    if (e.contains("E160013") || e.contains("W170000"))
                        && crate::i18n::pending_source(&record.label) == "Import" =>
                {
                    let parent = url_parent(address)?;
                    self.info(&parent)?
                }
                Err(e) => return Err(e),
            };
            let log = self.log(&info.repository, None, None)?;
            let summary = log["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .take(8)
                .map(|r| {
                    let note = r["message"]
                        .as_str()
                        .unwrap_or_default()
                        .lines()
                        .next()
                        .unwrap_or_default();
                    format!(
                        "r{} · {} · {}",
                        r["revision"],
                        r["author"].as_str().unwrap_or_default(),
                        note.chars().take(100).collect::<String>()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            trf!("\n\nRecent repository commits\n{0}", summary)
        } else {
            String::new()
        };
        let label = crate::i18n::pending_label(&record.label);
        confirm(
            tr("Review Previous SVN Operation"),
            &trf!(
                "Previous operation: {0}\n{1}\n\n{2}{3}\n\nConfirm the result before retrying.",
                label,
                record.path,
                state,
                remote
            ),
        )?;
        self.clear_pending(key)?;
        Ok(json!({"output": trf!("Reviewed {0}\n{1}{2}", label, state, remote), "recovered": true}))
    }
}

fn url_parent(address: &str) -> Result<String> {
    let mut address = url::Url::parse(address).map_err(|_| tr("Invalid repository URL"))?;
    address
        .path_segments_mut()
        .map_err(|_| tr("Invalid repository URL"))?
        .pop_if_empty()
        .pop();
    Ok(address.into())
}
