use std::path::Path;
use std::process::{Command, Output};

use openidea_plugin::mock::MockHost;
use openidea_plugin::{DocumentChanged, Error, Event, EventKind, Plugin, ProcessOutput, host};
use serde_json::json;
use url::Url;

use crate::{Session, Subversion, svn, working};

#[test]
fn plugin_activates_and_only_watches_annotated_document_paths() {
    let mock = MockHost::install().with_capabilities(&["events", "project.read", "editor.read"]);
    let root = "/project";
    mock.set_project_root(Some(root));
    let mut plugin = Subversion::default();
    plugin
        .activate(&mock.context("bamboowhut.subversion"))
        .unwrap();
    assert_eq!(
        mock.subscriptions(),
        [
            EventKind::FileSaved,
            EventKind::FilesChanged,
            EventKind::SettingsChanged
        ]
        .into()
    );
    assert!(mock.path_subscriptions().is_empty());

    plugin.annotated.insert(root.into(), "remote[1].txt".into());
    plugin.subscribe_annotated().unwrap();
    assert_eq!(
        mock.path_subscriptions()["document-changed"],
        ["remote[[]1[]].txt"]
    );
    host::set_decorations(
        "/project/remote[1].txt",
        &crate::storage_key("annotation", root),
        &[],
    )
    .unwrap();
    let changed = |path: &str| {
        Event::DocumentChanged(DocumentChanged {
            path: path.into(),
            project: Some(root.into()),
            base_version: 1,
            version: 2,
            changes: vec![],
        })
    };
    let ctx = mock.invocation();
    plugin.handle_events(&[changed("remote1.txt")], &ctx);
    assert!(!mock.decorations().is_empty());
    plugin.handle_events(&[changed("remote[1].txt")], &ctx);
    assert!(mock.decorations().is_empty());
    assert!(mock.path_subscriptions()["document-changed"].is_empty());
}

#[test]
fn file_commands_require_a_target_in_the_active_working_copy() {
    let mock = MockHost::install().with_capabilities(&["process:svn", "project.read", "storage"]);
    let root = if cfg!(windows) {
        "C:/project"
    } else {
        "/project"
    };
    let file = format!("{root}/migration.sql");
    let console = format!("{root}/../consoles/console_1.sql");
    mock.set_project_root(Some(root));
    mock.add_file(&file, "select 1;");
    let ctx = mock.invocation();
    let mut plugin = Subversion::default();

    assert!(plugin.run_command("commit", &ctx, None).is_err());
    mock.open_editor(&console, "sql", "select 1;", &[]);
    assert!(plugin.run_command("commit", &ctx, None).is_err());
    mock.open_editor("untitled:Untitled-1", "sql", "select 1;", &[]);
    assert!(plugin.run_command("commit", &ctx, None).is_err());
    assert!(mock.revealed_views().is_empty());

    mock.on_process("svn", |_, _| {
        Ok(ProcessOutput {
            status: Some(1),
            stdout: String::new(),
            stderr: "svn: E155007: not a working copy".into(),
        })
    });
    mock.open_editor(&file, "sql", "select 1;", &[]);
    assert!(plugin.run_command("commit", &ctx, None).is_err());
    assert!(mock.revealed_views().is_empty());

    mock.on_process("svn", move |args, _| {
        assert_eq!(args[0], "info");
        Ok(ProcessOutput {
            status: Some(0),
            stdout: format!(
                "<info><entry><repository/><wc-info><wcroot-abspath>{root}</wcroot-abspath></wc-info></entry></info>"
            ),
            stderr: String::new(),
        })
    });
    plugin.run_command("commit", &ctx, None).unwrap();
    assert_eq!(plugin.handoff[root]["paths"], json!([file]));
    assert_eq!(mock.revealed_views(), ["subversion"]);
}

#[test]
fn windows_unicode_checkout_and_selected_commit_with_unicode_client() {
    if !cfg!(windows) {
        return;
    }
    let (dir, base) = fixture();
    let cwd = dir.path();
    let probe = svn(
        cwd,
        &[
            "status",
            "--xml",
            "--depth",
            "empty",
            "--",
            ".openidea-svn-中文路径",
        ],
    );
    assert_eq!(
        svn::status_target(&probe).unwrap(),
        ".openidea-svn-中文路径",
        "Windows 测试需要支持 Unicode 的 svn.exe，例如 SlikSVN 1.14.5"
    );
    let local = "验收 工作副本";
    svn(
        cwd,
        &[
            "checkout",
            "--non-interactive",
            "--ignore-externals",
            "-r",
            "HEAD",
            "--",
            &format!("{base}/trunk@HEAD"),
            local,
        ],
    );
    let wc = cwd.join(local);
    let file = "module/验收@文件.txt";
    write(&wc.join(file), "第一行\n");
    let target = format!("./{file}@");
    svn(&wc, &["add", "--depth", "empty", "--", &target]);
    write(&cwd.join("svn-note.txt"), "验收提交\n中文路径");
    svn(
        &wc,
        &[
            "commit",
            "--depth",
            "empty",
            "--encoding",
            "UTF-8",
            "--file",
            "../svn-note.txt",
            "--force-log",
            "--",
            &target,
        ],
    );
    let info = svn::info(&svn(&wc, &["info", "--xml", "--", &target])).unwrap();
    assert_eq!(info.wc, abs(&wc));
    let log = svn::history(&svn(
        &wc,
        &["log", "--xml", "--verbose", "--limit", "1", "--", &target],
    ))
    .unwrap();
    assert_eq!(log[0].message, "验收提交\n中文路径");
    assert!(log[0].changes[0].path.ends_with("/验收@文件.txt"));
}

#[test]
fn properties_locks_and_directory_deletion_use_explicit_targets() {
    let (dir, _) = fixture();
    let a = dir.path().join("a");
    write(&a.join("note.txt"), "属性值\nnext");
    svn(
        &a,
        &[
            "propset",
            "test:notes",
            "--file",
            "note.txt",
            "--",
            "module/remote.txt",
        ],
    );
    let p = svn::properties(&svn(
        &a,
        &["proplist", "--xml", "--verbose", "--", "module/remote.txt"],
    ))
    .unwrap();
    assert!(
        p.iter()
            .any(|p| p.name == "test:notes" && p.value == "属性值\nnext")
    );
    let diff = svn(
        &a,
        &[
            "diff",
            "--internal-diff",
            "--properties-only",
            "--",
            "module/remote.txt",
        ],
    );
    assert!(diff.contains("test:notes"));
    svn(
        &a,
        &["lock", "--non-interactive", "--", "module/remote.txt"],
    );
    let status = |cwd: &Path| {
        svn::status(
            &svn(cwd, &["status", "--xml", "--ignore-externals"]),
            &abs(cwd),
        )
        .unwrap()
    };
    assert!(
        status(&a)
            .iter()
            .any(|row| row.path.ends_with("/remote.txt") && row.locked)
    );
    svn(
        &a,
        &["unlock", "--non-interactive", "--", "module/remote.txt"],
    );
    svn(&a, &["delete", "--", "sibling"]);
    let rows = status(&a);
    let target = abs(&a.join("sibling"));
    let required = rows
        .iter()
        .filter(|row| svn::within(&target, &row.path))
        .map(|row| row.path.clone())
        .collect::<Vec<_>>();
    assert!(required.iter().any(|p| p == &target));
    working::commit_targets(&rows, &required).unwrap();
    svn(
        &a,
        &[
            "commit",
            "--depth",
            "empty",
            "-m",
            "directory removal",
            "--",
            "sibling",
        ],
    );
    let b = dir.path().join("b");
    svn(&b, &["update", "--ignore-externals", "--", "."]);
    assert!(!b.join("sibling").exists());
    let p = svn::properties(&svn(
        &b,
        &["proplist", "--xml", "--verbose", "--", "module/remote.txt"],
    ))
    .unwrap();
    assert!(!p.iter().any(|p| p.name == "test:notes"));
}

#[test]
fn missing_authentication_is_reported_without_mutating_or_recording_input() {
    let mock = MockHost::install().with_capabilities(&["process:svn", "project.read"]);
    mock.on_process("svn", |args, stdin| {
        assert!(args.contains(&"--non-interactive".into()));
        assert!(stdin.is_none());
        assert!(!args.contains(&"--password".into()));
        Ok(ProcessOutput {
            status: Some(1),
            stdout: String::new(),
            stderr: "svn: E170001: Authentication required".into(),
        })
    });
    let ctx = mock.invocation();
    let session = Session::new(ctx.project().unwrap()).unwrap();
    let error = session.info("https://example.invalid/svn").unwrap_err();
    assert!(error.contains("Use Authenticate"));
    assert!(error.contains("E170001"));
}

fn cli(bin: &str, cwd: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("The tests require the SVN 1.14 command-line client")
}

fn svn(cwd: &Path, args: &[&str]) -> String {
    let out = cli("svn", cwd, args);
    assert!(
        out.status.success(),
        "{args:?}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write(path: &Path, data: &str) {
    std::fs::write(path, data.as_bytes()).unwrap();
}

fn abs(path: &Path) -> String {
    svn::canonical(&path.to_string_lossy()).unwrap()
}

fn fixture() -> (tempfile::TempDir, String) {
    let root = tempfile::Builder::new()
        .prefix("openidea-svn-")
        .tempdir()
        .unwrap();
    let dir = root.path();
    let repo = dir.join("repo");
    assert!(cli("svnadmin", dir, &["create", "repo"]).status.success());
    let base = Url::from_file_path(&repo).unwrap().to_string();
    let trunk = format!("{base}/trunk");
    let branches = format!("{base}/branches");
    svn(
        dir,
        &[
            "mkdir",
            "--non-interactive",
            "-m",
            "layout",
            "--",
            &trunk,
            &branches,
        ],
    );
    svn(
        dir,
        &[
            "checkout",
            "--non-interactive",
            "--",
            &format!("{trunk}@HEAD"),
            "a",
        ],
    );
    let a = dir.join("a");
    std::fs::create_dir(a.join("module")).unwrap();
    std::fs::create_dir(a.join("sibling")).unwrap();
    for p in [
        "module/main@.txt",
        "module/remote.txt",
        "module/tree.txt",
        "sibling/other.txt",
    ] {
        write(&a.join(p), "initial\n");
    }
    svn(&a, &["add", "--", "module", "sibling"]);
    svn(
        &a,
        &["propset", "test:value", "base", "--", "module/remote.txt"],
    );
    svn(
        &a,
        &["commit", "--non-interactive", "-m", "initial", "--", "."],
    );
    svn(
        dir,
        &[
            "checkout",
            "--non-interactive",
            "--",
            &format!("{trunk}@HEAD"),
            "b",
        ],
    );
    (root, base)
}

#[test]
fn validates_windows_path_boundaries_and_svn_target_syntax() {
    assert_eq!(
        svn::canonical(r"C:\抚州\代码\..\项目\").unwrap(),
        "C:/抚州/项目"
    );
    assert_eq!(
        svn::local_argument("C:/抚州/项目", "c:/抚州/项目/中文@测试.txt@"),
        "./中文@测试.txt@"
    );
    assert_eq!(
        svn::local_argument("C:/抚州/项目", "C:/抚州/项目@BASE"),
        ".@BASE"
    );
    assert!(svn::project_path("C:/抚州/项目", "../项目2/a").is_err());
    assert!(svn::project_path("C:/抚州/项目", "nested/.SVN /wc.db").is_err());
    assert!(svn::project_path("C:/抚州/项目", "a.txt:stream").is_err());
    assert!(svn::url("https://user:secret@svn.example.com").is_err());
    assert!(svn::url_path("https://svn.example.com/project", "../outside").is_err());
    assert_eq!(
        svn::url_path("https://svn.example.com/project", "/空 格/a#b").unwrap(),
        "https://svn.example.com/project/%E7%A9%BA%20%E6%A0%BC/a%23b"
    );
    assert_eq!(svn::peg("./x@y"), "./x@y@");
    assert_eq!(svn::revisions("13, -14").unwrap(), "13,-14");
    assert!(svn::revisions("13, 0").is_err());
    let (gbk, _, errors) = encoding_rs::GB18030.encode("中文差异");
    assert!(!errors);
    assert_eq!(svn::decode(&gbk, "GB18030").unwrap(), "中文差异");
}

#[test]
fn scoped_selected_commit_preserves_unselected_and_reports_moves() {
    let (dir, base) = fixture();
    let a = dir.path().join("a");
    let root = a.join("module");
    let b = dir.path().join("b");
    write(&root.join("main@.txt"), "selected\n");
    write(&root.join("remote.txt"), "unselected\n");
    write(&a.join("sibling/other.txt"), "sibling\n");
    let info = svn::info(&svn(&root, &["info", "--xml", "--", "."])).unwrap();
    assert_eq!(info.wc, abs(&a));
    assert_eq!(info.url, format!("{base}/trunk/module"));
    let status = || {
        svn::status(
            &svn(
                &root,
                &[
                    "status",
                    "--xml",
                    "--no-ignore",
                    "--ignore-externals",
                    "--",
                    ".",
                ],
            ),
            &abs(&root),
        )
        .unwrap()
    };
    let before = status();
    assert_eq!(before.len(), 2);
    assert!(before.iter().all(|row| svn::within(&abs(&root), &row.path)));
    let path = abs(&root.join("main@.txt"));
    let selected = vec![path.clone()];
    working::commit_targets(&before, &selected).unwrap();
    let note = dir.path().join("description.txt");
    write(&note, "中文提交说明 🧪\n第二行");
    svn(
        &root,
        &[
            "commit",
            "--non-interactive",
            "--depth",
            "empty",
            "--encoding",
            "UTF-8",
            "--file",
            note.to_str().unwrap(),
            "--force-log",
            "--keep-changelists",
            "--",
            "./main@.txt@",
        ],
    );
    let log = svn::history(&svn(
        &root,
        &[
            "log",
            "--xml",
            "--verbose",
            "--limit",
            "1",
            "-r",
            "HEAD:0",
            "--",
            ".",
        ],
    ))
    .unwrap();
    assert_eq!(log[0].message, "中文提交说明 🧪\n第二行");
    assert_eq!(log[0].changes.len(), 1);
    assert_eq!(log[0].changes[0].path, "/trunk/module/main@.txt");
    assert_eq!(status()[0].path, abs(&root.join("remote.txt")));
    svn(&root, &["move", "--", "./main@.txt@", "./rename@.txt"]);
    let moved = status();
    let to = abs(&root.join("rename@.txt"));
    assert!(working::commit_targets(&moved, std::slice::from_ref(&to)).is_err());
    working::commit_targets(&moved, &[path, to]).unwrap();
    svn(
        &root,
        &[
            "commit",
            "--depth",
            "empty",
            "-m",
            "move",
            "--",
            "./main@.txt@",
            "./rename@.txt@",
        ],
    );
    let history =
        svn::history(&svn(&root, &["log", "--xml", "-v", "--", "./rename@.txt@"])).unwrap();
    assert_eq!(
        history[0]
            .changes
            .iter()
            .find(|c| c.action == "A")
            .unwrap()
            .from,
        "/trunk/module/main@.txt"
    );
    svn(&b, &["update", "--ignore-externals", "--", "."]);
    assert!(b.join("module/rename@.txt").is_file());
    assert_eq!(
        std::fs::read_to_string(b.join("module/remote.txt")).unwrap(),
        "initial\n"
    );
}

#[test]
fn two_working_copies_report_text_property_and_tree_conflicts() {
    let (dir, _) = fixture();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    write(&a.join("module/main@.txt"), "local\n");
    write(&b.join("module/main@.txt"), "incoming\n");
    write(&a.join("module/tree.txt"), "kept locally\n");
    svn(
        &a,
        &["propset", "test:value", "local", "--", "module/remote.txt"],
    );
    svn(
        &b,
        &[
            "propset",
            "test:value",
            "incoming",
            "--",
            "module/remote.txt",
        ],
    );
    svn(&b, &["delete", "--", "module/tree.txt"]);
    svn(&b, &["commit", "-m", "incoming", "--", "."]);
    let failed = cli(
        "svn",
        &a,
        &[
            "commit",
            "--non-interactive",
            "-m",
            "stale",
            "--",
            "module/main@.txt@",
        ],
    );
    assert!(!failed.status.success(), "stale commit must fail");
    svn(
        &a,
        &[
            "update",
            "--ignore-externals",
            "--accept",
            "postpone",
            "--",
            ".",
        ],
    );
    let changes = svn::status(
        &svn(
            &a,
            &["status", "--xml", "--no-ignore", "--ignore-externals"],
        ),
        &abs(&a),
    )
    .unwrap();
    assert!(changes.iter().any(|c| c.item == "conflicted"));
    assert!(changes.iter().any(|c| c.props == "conflicted"));
    assert!(changes.iter().any(|c| c.tree));
    let read = |file| svn::conflict(&svn(&a, &["info", "--xml", "--", file])).unwrap();
    let c = read("module/main@.txt@");
    assert!(c.text && !c.tree);
    assert!(!c.base.is_empty() && !c.mine.is_empty() && !c.incoming.is_empty());
    assert!(read("module/remote.txt").property);
    assert!(read("module/tree.txt").tree);
    let scratch = dir.path().join("version.bin");
    write(&scratch, "");
    svn(
        &a,
        &[
            "export",
            "--quiet",
            "--force",
            "--ignore-externals",
            "--ignore-keywords",
            "-r",
            "BASE",
            "--",
            "module/main@.txt@BASE",
            scratch.to_str().unwrap(),
        ],
    );
    assert_eq!(std::fs::read_to_string(scratch).unwrap(), "incoming\n");
    write(&a.join("module/main@.txt"), "merged\n");
    svn(
        &a,
        &[
            "resolve",
            "--depth",
            "empty",
            "--accept",
            "working",
            "--",
            "module/main@.txt@",
        ],
    );
    assert!(!read("module/main@.txt@").text);
    assert_eq!(
        std::fs::read_to_string(a.join("module/main@.txt")).unwrap(),
        "merged\n"
    );
}

#[test]
fn branch_preview_and_reverse_merge_leave_expected_content() {
    let (dir, base) = fixture();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    let source = format!("{base}/trunk");
    let branch = format!("{base}/branches/feature");
    svn(
        &a,
        &[
            "copy",
            "--non-interactive",
            "-r",
            "HEAD",
            "-m",
            "branch",
            "--",
            &format!("{source}@HEAD"),
            &branch,
        ],
    );
    svn(
        &b,
        &[
            "switch",
            "--ignore-externals",
            "--accept",
            "postpone",
            "-r",
            "HEAD",
            "--",
            &format!("{branch}@HEAD"),
            ".",
        ],
    );
    write(&b.join("module/remote.txt"), "feature\n");
    svn(
        &b,
        &["commit", "-m", "feature change", "--", "module/remote.txt"],
    );
    let revision = svn::info(&svn(&b, &["info", "--xml", "--", "module/remote.txt"]))
        .unwrap()
        .revision;
    let r = revision.to_string();
    let peg = format!("{branch}@{revision}");
    svn(&a, &["update", "--ignore-externals", "--", "."]);
    let preview = svn(
        &a,
        &[
            "merge",
            "--accept",
            "postpone",
            "-c",
            &r,
            "--dry-run",
            "--",
            &peg,
            ".",
        ],
    );
    assert!(preview.contains("remote.txt"));
    assert_eq!(
        std::fs::read_to_string(a.join("module/remote.txt")).unwrap(),
        "initial\n"
    );
    svn(
        &a,
        &["merge", "--accept", "postpone", "-c", &r, "--", &peg, "."],
    );
    assert_eq!(
        std::fs::read_to_string(a.join("module/remote.txt")).unwrap(),
        "feature\n"
    );
    svn(&a, &["commit", "-m", "merge feature", "--", "."]);
    svn(&a, &["update", "--ignore-externals", "--", "."]);
    svn(
        &a,
        &[
            "merge",
            "--accept",
            "postpone",
            "-c",
            &format!("-{r}"),
            "--",
            &peg,
            ".",
        ],
    );
    assert_eq!(
        std::fs::read_to_string(a.join("module/remote.txt")).unwrap(),
        "initial\n"
    );
    let blame = svn::blame(&svn(
        &a,
        &["blame", "--xml", "-r", "1:BASE", "--", "module/remote.txt"],
    ))
    .unwrap();
    assert_eq!(blame[0].line, 1);
}

#[test]
fn plugin_refreshes_with_svn_cli_and_contains_scope() {
    let (dir, _) = fixture();
    let project = dir.path().join("a/module");
    write(&project.join("main@.txt"), "changed\n");
    std::fs::create_dir(project.join("new.folder")).unwrap();
    write(&dir.path().join("a/sibling/other.txt"), "outside\n");
    let mock = MockHost::install().with_capabilities(&[
        "process:svn",
        "project.read",
        "project.write",
        "editor.read",
        "events",
        "storage",
    ]);
    mock.set_project_root(Some(&abs(&project)));
    mock.add_file("main@.txt", "changed\n");
    mock.add_dir("new.folder");
    let cwd = project.clone();
    mock.on_process("svn", move |args, _| {
        // The mock SDK keeps its scratch files in memory. A successful export
        // of an empty base leaves the mock scratch file empty.
        if args[0] == "export" {
            return Ok(ProcessOutput {
                stdout: String::new(),
                stderr: String::new(),
                status: Some(0),
            });
        }
        let out = Command::new("svn")
            .current_dir(&cwd)
            .args(args)
            .output()
            .map_err(|e| Error::Failed(e.to_string()))?;
        Ok(ProcessOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            status: out.status.code(),
        })
    });
    let ctx = mock.invocation();
    let mut plugin = Subversion::default();
    let snapshot = plugin.execute(&ctx, &crate::Request::default()).unwrap();
    assert_eq!(snapshot["snapshot"]["count"], 2);
    assert_eq!(
        snapshot["snapshot"]["changes"][0]["path"],
        abs(&project.join("main@.txt"))
    );
    assert_eq!(snapshot["snapshot"]["changes"][0]["directory"], false);
    assert_eq!(
        snapshot["snapshot"]["changes"][1]["path"],
        abs(&project.join("new.folder"))
    );
    assert_eq!(snapshot["snapshot"]["changes"][1]["directory"], true);
    assert_eq!(
        snapshot["snapshot"]["info"]["wc"],
        abs(&dir.path().join("a"))
    );
    let session = Session::new(ctx.project().unwrap()).unwrap();
    let scope = session.scope("").unwrap().unwrap();
    let preview = session
        .local_diff(&scope, "main@.txt", "UTF-8", true)
        .unwrap();
    let token = preview["localPreview"]["editHash"].as_str().unwrap();
    let base = preview["localPreview"]["left"].as_str().unwrap();
    let req: crate::review::DifferenceEdit = serde_json::from_value(json!({
        "leftStart": 0, "leftLength": 1, "rightStart": 0, "rightLength": 2, "append": false
    }))
    .unwrap();
    mock.add_file("main@.txt", "edit that happened after preview\n");
    assert!(
        session
            .edit_difference(&scope, "main@.txt", "UTF-8", token, &req)
            .unwrap_err()
            .contains("changed")
    );
    mock.add_file("main@.txt", "changed\n");
    session
        .edit_difference(&scope, "main@.txt", "UTF-8", token, &req)
        .unwrap();
    assert_eq!(session.project.read_file("main@.txt").unwrap(), base);
    assert!(
        session
            .targets(&scope, &["../sibling/other.txt".into()], false)
            .is_err()
    );
    let record = crate::backend::Pending {
        label: "提交".into(),
        path: scope.path,
        repository: Some(scope.info.repository),
    };
    assert!(
        session
            .write::<()>(&scope.info.wc, &record, || Err("网络断开".into()))
            .is_err()
    );
    assert_eq!(
        session.pending(&scope.info.wc).unwrap().unwrap().label,
        "提交"
    );
    assert!(
        session
            .write(&scope.info.wc, &record, || Ok(json!({})))
            .is_err()
    );
    session.clear_pending(&scope.info.wc).unwrap();
    assert!(session.pending(&scope.info.wc).unwrap().is_none());
}

#[test]
fn direct_commit_adds_only_selected_unversioned_paths() {
    let (dir, base) = fixture();
    let wc = dir.path().join("a");
    let project = wc.join("module");
    let contents = [
        ("new @ file.txt", "selected"),
        ("new folder/nested/child.txt", "selected descendant"),
        ("new folder/nested/cache.tmp", "ignored by SVN"),
        ("new folder/.git/config", "outside commit scope"),
        ("untouched.txt", "not selected"),
        ("remote.txt", "not selected tracked change"),
    ];
    for (path, content) in contents {
        let path = project.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write(&path, content);
    }
    svn(&project, &["propset", "svn:global-ignores", "*.tmp", "."]);
    svn(
        &project,
        &["commit", "-m", "ignore rule", "--depth", "empty", "."],
    );

    let mock = MockHost::install().with_capabilities(&[
        "process:svn",
        "project.read",
        "editor.read",
        "storage",
    ]);
    mock.set_project_root(Some(&abs(&project)));
    for (path, content) in contents {
        mock.add_file(path, content);
    }
    mock.add_dir("new folder");
    mock.add_dir("new folder/nested");
    let cwd = project.clone();
    mock.on_process("svn", move |args, _| {
        let argv: Vec<_> = args.iter().map(String::as_str).collect();
        let out = cli("svn", &cwd, &argv);
        Ok(ProcessOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            status: out.status.code(),
        })
    });
    let ctx = mock.invocation();
    let session = Session::new(ctx.project().unwrap()).unwrap();
    let scope = session.scope("").unwrap().unwrap();
    let mut paths = vec![
        abs(&project.join("new @ file.txt")),
        abs(&project.join("new folder")),
    ];
    let rows = session.changes(&scope).unwrap();
    working::commit_targets(&rows, &paths).unwrap();
    let additions = paths.clone();
    session
        .schedule_commit_additions(&scope, &mut paths, additions)
        .unwrap();
    assert_eq!(paths.len(), 4);
    let rows = session.changes(&scope).unwrap();
    working::commit_targets(&rows, &paths).unwrap();
    assert!(
        rows.iter()
            .any(|row| row.path.ends_with("/child.txt") && row.item == "added")
    );
    assert!(
        rows.iter()
            .any(|row| row.path.ends_with("/untouched.txt") && row.item == "unversioned")
    );
    assert!(
        rows.iter()
            .all(|row| !row.path.ends_with("/cache.tmp") || row.item == "ignored")
    );
    let targets = working::pegs(&paths);
    let mut args = vec!["commit", "-m", "direct commit", "--depth", "empty", "--"];
    args.extend(targets.iter().map(String::as_str));
    svn(&project, &args);
    let listing = svn(&project, &["list", "-R", &format!("{base}/trunk/module")]);
    assert!(listing.contains("new @ file.txt") && listing.contains("new folder/nested/child.txt"));
    assert!(
        !listing.contains("untouched.txt")
            && !listing.contains("cache.tmp")
            && !listing.contains(".git")
    );
    assert_eq!(svn(&project, &["cat", "remote.txt"]).trim(), "initial");
}

#[test]
fn shelf_patch_round_trips_crlf_and_mixed_line_endings() {
    use crate::shelf::{check_shelf_result, normalize_shelf_patch};

    let (dir, _) = fixture();
    let project = dir.path().join("a/module");
    let files = ["remote.txt", "tree.txt"];
    let base = [
        b"CRLF baseline\r\n".as_slice(),
        b"Another baseline\r\n".as_slice(),
    ];
    let modified = [
        b"CRLF baseline\r\n\nmixed line\n".as_slice(),
        b"Another baseline\n".as_slice(),
    ];
    for (file, value) in files.into_iter().zip(base) {
        std::fs::write(project.join(file), value).unwrap();
    }
    svn(
        &project,
        &["commit", "-m", "CRLF baselines", "--", files[0], files[1]],
    );
    for (file, value) in files.into_iter().zip(modified) {
        std::fs::write(project.join(file), value).unwrap();
    }

    let root = abs(&project);
    let paths: Vec<_> = files.iter().map(|name| abs(&project.join(name))).collect();
    let raw = svn(
        &project,
        &["diff", "--internal-diff", "--git", "--", files[0], files[1]],
    );
    let patch = normalize_shelf_patch(&raw, &root, &paths).unwrap();
    let patch_file = dir.path().join("windows-lines.patch");
    std::fs::write(&patch_file, patch).unwrap();
    let patch_file = patch_file.to_str().unwrap();
    let expected: Vec<_> = paths.into_iter().map(|path| (path, 'U')).collect();
    let output = svn(&project, &["patch", "--reverse-diff", patch_file]);
    check_shelf_result(&output, &root, &expected).unwrap();
    for (file, value) in files.into_iter().zip(base) {
        assert_eq!(std::fs::read(project.join(file)).unwrap(), value);
    }
    assert!(svn(&project, &["status", "--", files[0], files[1]]).is_empty());
    let output = svn(&project, &["patch", patch_file]);
    check_shelf_result(&output, &root, &expected).unwrap();
    for (file, value) in files.into_iter().zip(modified) {
        assert_eq!(std::fs::read(project.join(file)).unwrap(), value);
    }
}

#[test]
fn binary_shelf_patch_round_trips_in_a_nested_working_scope() {
    use crate::shelf::{check_shelf_result, normalize_shelf_patch};

    let (dir, _) = fixture();
    let wc = dir.path().join("a");
    let project = wc.join("module");
    let bytes = |name: &str| std::fs::read(project.join(name)).unwrap();
    let base = [0, 255, 194, 10, 0];
    let changed = [0, 254, 33, 0, 13, 10];
    for path in ["modified @ file.bin", "deleted.bin"] {
        std::fs::write(project.join(path), base).unwrap();
    }
    svn(
        &project,
        &["add", "--", "modified @ file.bin@", "deleted.bin"],
    );
    svn(&project, &["commit", "-m", "binary base", "."]);
    std::fs::write(project.join("modified @ file.bin"), changed).unwrap();
    std::fs::write(project.join("added.bin"), changed).unwrap();
    svn(&project, &["add", "added.bin"]);
    svn(&project, &["delete", "deleted.bin"]);
    write(&project.join("remote.txt"), "text shelf\n");
    write(&project.join("tree.txt"), "outside selection\n");
    let path = |name: &str| abs(&project.join(name));
    let expected = [
        (path("modified @ file.bin"), 'U'),
        (path("added.bin"), 'A'),
        (path("deleted.bin"), 'D'),
        (path("remote.txt"), 'U'),
    ];
    let chosen: Vec<_> = expected.iter().map(|(path, _)| path.clone()).collect();
    let raw = svn(&project, &["diff", "--internal-diff", "--git", "--", "."]);
    let complete = normalize_shelf_patch(&raw, &abs(&project), &chosen).unwrap();
    assert_eq!(complete.matches("GIT binary patch").count(), 3);
    assert!(!complete.contains("tree.txt"));
    let reverse_paths = [chosen[0].clone(), chosen[1].clone(), chosen[3].clone()];
    let reverse = normalize_shelf_patch(&raw, &abs(&project), &reverse_paths).unwrap();
    let patch = dir.path().join("shelf.patch");
    let patch_path = abs(&patch);
    write(&patch, &reverse);
    let inverted = [
        (chosen[0].clone(), 'U'),
        (chosen[1].clone(), 'D'),
        (chosen[3].clone(), 'U'),
    ];
    for args in [
        vec!["patch", "--dry-run", "--reverse-diff", &patch_path, "."],
        vec!["patch", "--reverse-diff", &patch_path, "."],
    ] {
        check_shelf_result(&svn(&project, &args), &abs(&project), &inverted).unwrap();
    }
    svn(&project, &["revert", "deleted.bin"]);
    assert_eq!(bytes("modified @ file.bin"), base);
    assert_eq!(bytes("deleted.bin"), base);
    assert!(!project.join("added.bin").exists());
    assert_eq!(bytes("remote.txt"), b"initial\n");

    write(&patch, &complete);
    let competing = [0, 123, 252, 0, 22, 17];
    std::fs::write(project.join("modified @ file.bin"), competing).unwrap();
    let blocked = svn(&project, &["patch", "--dry-run", &patch_path, "."]);
    assert!(check_shelf_result(&blocked, &abs(&project), &expected).is_err());
    assert_eq!(bytes("modified @ file.bin"), competing);
    assert!(!project.join("added.bin").exists());
    std::fs::write(project.join("modified @ file.bin"), base).unwrap();
    for args in [
        vec!["patch", "--dry-run", &patch_path, "."],
        vec!["patch", &patch_path, "."],
    ] {
        check_shelf_result(&svn(&project, &args), &abs(&project), &expected).unwrap();
    }
    assert_eq!(bytes("modified @ file.bin"), changed);
    assert_eq!(bytes("added.bin"), changed);
    assert!(!project.join("deleted.bin").exists());
    assert_eq!(bytes("remote.txt"), b"text shelf\n");
    assert_eq!(bytes("tree.txt"), b"outside selection\n");
    assert_eq!(
        svn(&project, &["propget", "svn:mime-type", "added.bin"]).trim(),
        "application/octet-stream"
    );
}

#[test]
fn responses_keep_the_calling_panel_id_and_reject_another_project() {
    use openidea_plugin::views::{ViewEvent, Views};

    let mock = MockHost::install();
    let root = if cfg!(windows) { "C:/svn-a" } else { "/svn-a" };
    mock.set_project_root(Some(root));
    let panel = "d63979091bf2292fbeb6d70df0cc3bc5";
    let ctx = mock.invocation();
    let mut plugin = Subversion::default();
    plugin
        .handle(
            "subversion",
            ViewEvent::Message(
                json!({"op":"update","seq":1,"panel":panel,"project":"/svn-b"}).to_string(),
            ),
            &ctx,
        )
        .unwrap();
    let messages = mock.view_messages();
    let message: serde_json::Value = serde_json::from_str(&messages[0].1).unwrap();
    assert_eq!(message["panel"], panel);
    assert_eq!(message["seq"], 1);
    assert!(
        message["error"]
            .as_str()
            .unwrap()
            .contains("project changed")
    );
}

#[test]
fn panel_language_follows_ide_and_accepts_explicit_overrides() {
    use openidea_plugin::views::{ViewEvent, Views};

    let mock = MockHost::install();
    mock.set_project_root(None);
    mock.set_locale("zh-CN");
    let mut plugin = Subversion::default();
    let ctx = mock.invocation();
    let panel = "d63979091bf2292fbeb6d70df0cc3bc5";
    let mut request = |seq| {
        plugin
            .handle(
                "subversion",
                ViewEvent::Message(json!({"op":"refresh","seq":seq,"panel":panel}).to_string()),
                &ctx,
            )
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&mock.view_messages().last().unwrap().1).unwrap()
    };
    let chinese = request(1);
    assert_eq!(chinese["language"], "zh-CN");
    assert_eq!(chinese["error"], "请先打开受信任的本地项目目录");

    mock.set_setting("language", r#""English""#);
    let english = request(2);
    assert_eq!(english["language"], "en");
    assert_eq!(english["error"], "Open a trusted local project first");

    mock.set_locale("fr");
    mock.set_setting("language", r#""Follow IDE""#);
    assert_eq!(request(3)["language"], "en");
    mock.set_setting("language", r#""Simplified Chinese""#);
    assert_eq!(request(4)["language"], "zh-CN");

    assert_eq!(crate::i18n::pending_label("导入"), "Import");
    assert_eq!(
        crate::i18n::with_language(|| crate::i18n::pending_label("Import")),
        "导入"
    );
    assert_eq!(
        crate::i18n::with_language(|| crate::i18n::trf!("Property {0}\n{1}", "svn:ignore", "dir")),
        "属性 svn:ignore\ndir"
    );
}

#[test]
fn panel_can_sync_language_without_an_svn_operation() {
    use openidea_plugin::views::{ViewEvent, Views};
    use serde_json::Value;

    let mock = MockHost::install();
    mock.set_project_root(None);
    mock.set_locale("zh-CN");
    let ctx = mock.invocation();
    let mut plugin = Subversion::default();
    let settings_changed = |data: &str| Event::Other {
        kind: EventKind::SettingsChanged.name().into(),
        data: data.into(),
    };

    plugin.handle_events(&[settings_changed(r#"["unrelated"]"#)], &ctx);
    assert!(mock.view_messages().is_empty());
    plugin.handle_events(&[settings_changed(r#"["language"]"#)], &ctx);
    let messages = mock.view_messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&messages[0].1).unwrap(),
        json!({"languageChanged": true})
    );

    let panel = "d63979091bf2292fbeb6d70df0cc3bc5";
    mock.set_setting("language", r#""English""#);
    plugin
        .handle(
            "subversion",
            ViewEvent::Message(json!({"languageOnly": true, "panel": panel, "seq": 5}).to_string()),
            &ctx,
        )
        .unwrap();
    let messages = mock.view_messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        serde_json::from_str::<Value>(&messages[1].1).unwrap(),
        json!({"languageOnly": true, "panel": panel, "seq": 5, "language": "en"})
    );
}

#[test]
fn incoming_uses_each_files_base_revision_in_a_mixed_working_copy() {
    let (dir, _) = fixture();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    write(&b.join("module/remote.txt"), "first repository change\n");
    svn(
        &b,
        &[
            "commit",
            "-m",
            "first repository change",
            "module/remote.txt",
        ],
    );
    write(&b.join("module/main@.txt"), "second repository change\n");
    svn(
        &b,
        &[
            "commit",
            "-m",
            "second repository change",
            "module/main@.txt@",
        ],
    );
    svn(&a, &["update", "module/remote.txt"]);
    let project = a.join("module");
    let mock = MockHost::install().with_capabilities(&["process:svn", "project.read", "storage"]);
    mock.set_project_root(Some(&abs(&project)));
    let cwd = project.clone();
    mock.on_process("svn", move |args, _| {
        let out = Command::new("svn")
            .current_dir(&cwd)
            .args(args)
            .output()
            .map_err(|e| Error::Failed(e.to_string()))?;
        Ok(ProcessOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            status: out.status.code(),
        })
    });
    let ctx = mock.invocation();
    let session = Session::new(ctx.project().unwrap()).unwrap();
    let scope = session.scope("").unwrap().unwrap();
    let incoming = session.incoming(&scope, None).unwrap();
    let entries = incoming["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["message"], "second repository change");
    assert_eq!(entries[0]["changes"][0]["path"], "/trunk/module/main@.txt");
}

#[test]
fn guarded_unicode_paths_never_write_when_client_mangles_arguments() {
    let mock = MockHost::install().with_capabilities(&["process:svn", "project.read"]);
    mock.set_project_root(Some("C:/fixture"));
    mock.on_process("svn", |args, _| {
        assert_eq!(args[0], "status");
        Ok(ProcessOutput {
            status: Some(0),
            stderr: String::new(),
            stdout: "<status><target path=\".openidea-svn-garbled\"/></status>".into(),
        })
    });
    let ctx = mock.invocation();
    let s = Session::new(ctx.project().unwrap()).unwrap();
    let err = s
        .stream("delete", &[], &["C:/fixture/中文.txt".into()])
        .unwrap_err();
    assert!(err.contains("Unicode"));
    assert_eq!(mock.process_runs().len(), 1);
}
