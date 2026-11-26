use crate::i18n::{tr, trf};
use std::cell::Cell;
use std::hash::{DefaultHasher, Hash, Hasher};

use openidea_plugin::{Error, FileKind, Project, host};
use serde::{Deserialize, Serialize};

use crate::svn::{self, Change, Info, Result};

const MAX_CAPTURE: u32 = 16 << 20;
pub const MAX_TEXT: usize = 512 << 10;

pub fn result<T>(value: std::result::Result<T, Error>) -> Result<T> {
    value.map_err(|error| match error {
        Error::Denied(value) => trf!("OpenIDEA denied the operation ({0}). Trust the project and allow and select an SVN 1.14 client in plugin permissions.", value),
        Error::Cancelled => tr("Operation canceled").into(),
        other => other.to_string(),
    })
}

pub fn fingerprint(value: impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

pub fn storage_key(prefix: &str, path: &str) -> String {
    format!("{prefix}-{:016x}", fingerprint(svn::key(path)))
}

#[derive(Clone)]
pub struct Scope {
    pub path: String,
    pub info: Info,
}

#[derive(Deserialize, Serialize)]
pub struct Pending {
    pub label: String,
    pub path: String,
    pub repository: Option<String>,
}

pub struct Session {
    pub project: Project,
    pub root: String,
    unicode_args: Cell<Option<bool>>,
}

impl Session {
    pub fn new(project: Project) -> Result<Self> {
        let root = svn::canonical(&result(project.root())?)?;
        Ok(Self {
            project,
            root,
            unicode_args: Cell::new(None),
        })
    }

    pub fn path(&self, input: &str) -> Result<String> {
        svn::project_path(&self.root, input)
    }

    /// The host resolves each existing component under the project. Missing
    /// endpoints (svn delete/revert/move) still require a verified parent.
    pub fn check_path(&self, path: &str) -> Result<()> {
        let mut candidate = self.path(path)?;
        loop {
            match self.project.stat(&candidate) {
                Ok(stat) if matches!(stat.kind, FileKind::Symlink | FileKind::Other) => {
                    return Err(trf!(
                        "This operation does not support symbolic links or special files: {0}",
                        candidate
                    ));
                }
                Ok(_) => return Ok(()),
                Err(Error::NotFound(_)) => {
                    candidate = svn::parent(&candidate)
                        .filter(|p| svn::within(&self.root, p))
                        .ok_or(tr(
                            "Cannot verify the target's parent directory within the project",
                        ))?;
                }
                Err(error) => return result(Err(error)),
            }
        }
    }

    pub fn cwd(&self, requested: &str) -> Result<String> {
        if requested.is_empty() {
            return Ok(self.root.clone());
        }
        let path = self.path(requested)?;
        self.dir(&path)?;
        Ok(path)
    }

    pub fn dir(&self, path: &str) -> Result<()> {
        let stat = result(self.project.stat(path))?;
        if stat.kind != FileKind::Directory {
            return Err(trf!("A project directory is required: {0}", path));
        }
        self.check_path(path)
    }

    pub fn version(&self) -> Result<String> {
        let text = self.read("--version", &strings(&["--quiet"]), &[])?;
        let version = text.trim();
        let mut pieces = version.split('.');
        if pieces.next() != Some("1")
            || pieces
                .next()
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(0)
                < 14
        {
            return Err(trf!(
                "SVN 1.14 or a later 1.x client is required. Detected version: {0}",
                version
            ));
        }
        Ok(version.into())
    }

    fn command(
        &self,
        verb: &str,
        flags: &[String],
        targets: &[String],
        interactive: bool,
    ) -> host::Command {
        let mut args = vec![verb.into()];
        if !interactive && verb != "--version" {
            args.push("--non-interactive".into());
        }
        args.extend_from_slice(flags);
        if !targets.is_empty() {
            args.push("--".into());
            args.extend(targets.iter().map(|p| svn::local_argument(&self.root, p)));
        }
        host::Command {
            program: "svn".into(),
            args,
            env: Vec::new(),
        }
    }

    fn check_argv(&self, cmd: &host::Command) -> Result<()> {
        if svn::windows(&self.root)
            && cmd
                .args
                .iter()
                .map(|s| 2 * s.encode_utf16().count() + 3)
                .sum::<usize>()
                > 30_000
        {
            return Err(tr(
                "The selected paths exceed the Windows command length limit. Select fewer paths.",
            )
            .into());
        }
        if !svn::windows(&self.root) || cmd.args.iter().all(|s| s.is_ascii()) {
            return Ok(());
        }
        let usable = if let Some(ok) = self.unicode_args.get() {
            ok
        } else {
            const PROBE: &str = ".openidea-svn-中文路径";
            let probe = self.command(
                "status",
                &strings(&["--xml", "--depth", "empty"]),
                &strings(&[PROBE]),
                false,
            );
            let output = result(host::run_captured(&probe, Some(&self.project), None, None))?;
            let ok = output.status == Some(0)
                && svn::status_target(&output.stdout).is_ok_and(|p| p == PROBE);
            self.unicode_args.set(Some(ok));
            ok
        };
        if usable {
            Ok(())
        } else {
            Err(tr("This svn.exe cannot reliably receive Unicode paths without a console. Choose an SVN 1.14 client with Unicode argument support in the plugin permissions.").into())
        }
    }

    pub fn read(&self, verb: &str, flags: &[String], targets: &[String]) -> Result<String> {
        let task = host::begin_task(&format!("SVN {verb}"), true);
        let command = self.command(verb, flags, targets, false);
        self.check_argv(&command)?;
        let output = result(host::run_captured(
            &command,
            Some(&self.project),
            None,
            Some(host::ProcessOptions {
                max_output_bytes: Some(MAX_CAPTURE),
                buffer_bytes: None,
            }),
        ))?;
        task.report(1, Some(1), None);
        if output.status != Some(0) {
            return Err(svn_error(verb, &output.stderr));
        }
        Ok(output.stdout)
    }

    pub fn verify_command(&self, verb: &str, flags: &[String], targets: &[String]) -> Result<()> {
        self.check_argv(&self.command(verb, flags, targets, false))
    }

    pub fn stream(&self, verb: &str, flags: &[String], targets: &[String]) -> Result<String> {
        let task = host::begin_task(&format!("SVN {verb}"), true);
        let command = self.command(verb, flags, targets, false);
        self.check_argv(&command)?;
        let child = result(host::start_stream(
            &command,
            Some(&self.project),
            Some(host::ProcessOptions {
                max_output_bytes: Some(MAX_CAPTURE),
                buffer_bytes: None,
            }),
        ))?;
        let mut out = String::new();
        let mut err = String::new();
        loop {
            if task.is_cancelled() {
                let _ = child.kill();
                return Err(
                    tr("Canceled. Changes already made by SVN may still be in effect.").into(),
                );
            }
            let chunk = result(child.wait(250))?;
            tail(&mut out, &chunk.stdout);
            tail(&mut err, &chunk.stderr);
            if let Some(code) = chunk.status {
                if code != 0 {
                    return Err(svn_error(verb, &format!("{err}\n{out}")));
                }
                if !err.trim().is_empty() {
                    tail(&mut out, &err);
                }
                return Ok(out);
            }
            task.report(0, None, out.lines().last());
        }
    }

    pub fn authenticate(&self, target: &str) -> Result<()> {
        let cmd = self.command("info", &strings(&["-r", "HEAD"]), &[svn::peg(target)], true);
        result(host::open_terminal_session(
            "svn-auth",
            tr("Subversion Authentication"),
            &cmd,
        ))
        .map(drop)
    }

    pub fn info(&self, target: &str) -> Result<Info> {
        svn::info(&self.read("info", &strings(&["--xml"]), &[svn::peg(target)])?)
    }

    pub fn scope(&self, path: &str) -> Result<Option<Scope>> {
        let path = self.cwd(path)?;
        match self.info(&path) {
            Ok(info) if !info.wc.is_empty() => Ok(Some(Scope { path, info })),
            Ok(_) => Ok(None),
            Err(error) if missing_wc(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn changes(&self, scope: &Scope) -> Result<Vec<Change>> {
        let content = self.read(
            "status",
            &strings(&["--xml", "--no-ignore", "--ignore-externals"]),
            &[svn::peg(&scope.path)],
        )?;
        let mut changes = svn::status(&content, &self.root)?;
        changes.retain(|change| svn::project_path(&scope.path, &change.path).is_ok());
        Ok(changes)
    }

    pub fn targets(
        &self,
        scope: &Scope,
        inputs: &[String],
        allow_scope: bool,
    ) -> Result<Vec<String>> {
        if inputs.is_empty() || inputs.len() > 200 {
            return Err(tr("Select 1 to 200 paths").into());
        }
        let mut paths = Vec::new();
        for input in inputs {
            let path = svn::project_path(&scope.path, input)?;
            if !allow_scope && svn::key(&scope.path) == svn::key(&path) {
                return Err(tr("Select a file or subdirectory in the current scope").into());
            }
            if paths
                .iter()
                .any(|p: &String| svn::key(p) == svn::key(&path))
            {
                continue;
            }
            self.check_path(&path)?;
            paths.push(path);
        }
        let wc = svn::key(&svn::canonical(&scope.info.wc)?);
        let same_wc =
            |info: &Info| svn::canonical(&info.wc).is_ok_and(|root| svn::key(&root) == wc);
        if paths.len() > 1 {
            let flags = strings(&["--xml"]);
            let targets = crate::working::pegs(&paths);
            let infos = match self.read("info", &flags, &targets) {
                Ok(raw) => svn::infos(&raw)?,
                Err(error) if missing_wc(&error) => Vec::new(),
                Err(error) => return Err(error),
            };
            if infos.len() == paths.len() {
                if infos.iter().all(same_wc) {
                    return Ok(paths);
                }
                return Err(tr("The selection includes another working copy or external. Select it separately.").into());
            }
        }
        for path in &paths {
            let mut candidate = path.clone();
            let info = loop {
                match self.info(&candidate) {
                    Ok(info) => break info,
                    Err(e) if missing_wc(&e) && svn::key(&candidate) != svn::key(&scope.path) => {
                        candidate = svn::parent(&candidate)
                            .filter(|p| svn::within(&scope.path, p))
                            .ok_or(tr("The target is outside the current SVN working copy"))?;
                    }
                    Err(e) => return Err(e),
                }
            };
            if !same_wc(&info) {
                return Err(trf!(
                    "Select this working copy or external separately: {0}",
                    path
                ));
            }
        }
        Ok(paths)
    }

    pub fn dirty(&self, paths: &[String]) -> Result<()> {
        if let Some(editor) = result(host::open_editors())?.into_iter().find(|e| {
            e.dirty
                && svn::canonical(&e.path)
                    .is_ok_and(|p| paths.iter().any(|path| svn::within(path, &p)))
        }) {
            return Err(trf!(
                "Save or close this unsaved file first: {0}",
                editor.path
            ));
        }
        Ok(())
    }

    pub fn pending(&self, wc: &str) -> Result<Option<Pending>> {
        let key = storage_key("pending", wc);
        match result(host::storage::get(&key))? {
            None => Ok(None),
            Some(s) => serde_json::from_str(&s)
                .map(Some)
                .map_err(|_| tr("Cannot read the SVN recovery record").into()),
        }
    }

    pub fn clear_pending(&self, wc: &str) -> Result<()> {
        result(host::storage::delete(&storage_key("pending", wc)))
    }

    pub fn write<T>(
        &self,
        wc: &str,
        pending: &Pending,
        write: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        if let Some(earlier) = self.pending(wc)? {
            return Err(trf!(
                "Review the previous pending operation first: {0}",
                crate::i18n::pending_label(&earlier.label)
            ));
        }
        let payload = serde_json::to_string(pending).map_err(|e| e.to_string())?;
        result(host::storage::set(&storage_key("pending", wc), &payload))?;
        match write() {
            Ok(out) => {
                self.clear_pending(wc)?;
                Ok(out)
            }
            Err(error) => Err(trf!(
                "{0}\nRefresh and review the previous operation before retrying.",
                error
            )),
        }
    }

    /// 'svn export' preserves bytes across the host's string-only process
    /// interface. A single private scratch slot is cleared after each read.
    pub fn content(
        &self,
        target: &str,
        peg: &str,
        revision: &str,
        encoding: &str,
    ) -> Result<String> {
        let folder = result(host::fs::plugin_dir())?;
        let scratch = svn::join(&folder, "revision-content.svn")?;
        result(host::fs::write_bytes(&scratch, &[]))?;
        let args = [format!("{target}{peg}"), scratch.clone()];
        let flags = strings(&[
            "--quiet",
            "--force",
            "--ignore-externals",
            "--ignore-keywords",
            "-r",
            revision,
        ]);
        let output = self
            .read("export", &flags, &args)
            .and_then(|_| result(host::fs::read_bytes(&scratch)))
            .and_then(|bytes| text(&bytes, encoding));
        result(host::fs::write_bytes(&scratch, &[]))?;
        output
    }

    pub fn with_text<T>(&self, value: &str, run: impl FnOnce(&str) -> Result<T>) -> Result<T> {
        let dir = result(host::fs::plugin_dir())?;
        let path = svn::join(&dir, "svn-input.txt")?;
        result(host::fs::write_bytes(&path, value.as_bytes()))?;
        let output = run(&path);
        result(host::fs::write_bytes(&path, &[]))?;
        output
    }
}

pub fn missing_wc(error: &str) -> bool {
    ["E155007", "W155010", "E155010", "W155007"]
        .iter()
        .any(|code| error.contains(code))
}

pub fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

fn tail(out: &mut String, more: &str) {
    out.push_str(more);
    if out.len() > 16_000 {
        let mut n = out.len() - 16_000;
        while !out.is_char_boundary(n) {
            n += 1;
        }
        out.drain(..n);
    }
}

fn svn_error(verb: &str, stderr: &str) -> String {
    let mut error = trf!("SVN {0} failed", verb);
    if ["E170001", "E215004", "E230001", "E175013"]
        .iter()
        .any(|code| stderr.contains(code))
    {
        error.push_str(tr(
            "\nUse Authenticate in the Subversion tool window to open an SVN terminal.",
        ));
    }
    if !stderr.trim().is_empty() {
        error.push('\n');
        error.extend(stderr.trim().chars().take(5500));
    }
    error
}

pub fn text(bytes: &[u8], encoding: &str) -> Result<String> {
    if bytes.len() > MAX_TEXT {
        return Err(tr("The file exceeds the 512 KiB text preview limit").into());
    }
    svn::decode(bytes, encoding)
}
