# Subversion for OpenIDEA

[简体中文](README.zh-CN.md)

Subversion is a community plugin for [OpenIDEA](https://github.com/l0ng-ai/openidea). Browse SVN repositories, review and commit changes, and resolve conflicts from the IDE using your existing Subversion CLI.

## Requirements

- OpenIDEA 0.1.51 or later with plugin API 1.15
- SVN 1.14 or a later 1.x command-line client
- Windows users working with non-ASCII paths need a client that accepts Unicode arguments without a console. [SlikSVN 1.14.5](https://sliksvn.com/download/) passed this check on Windows x64.

## Install

1. Download `bamboowhut.subversion-<version>.oiplugin` from a GitHub Release, or build the package below.
2. In OpenIDEA, choose **Settings › Plugins › gear menu › Install Plugin from Package File…** and select the package. Grant the SVN process permission and choose your executable in the plugin's permissions settings if needed.
3. Open a trusted local project and run **Tools › Subversion** or find **Subversion** in Find Action.

### Language

Open **Settings › Plugins › Subversion › Language**. Choose **Follow IDE** (the default), **English**, or **Simplified Chinese**. In **Follow IDE** mode, a Chinese IDE locale selects Simplified Chinese and all other locales use English.

The tool window switches automatically, including open plugin forms. OpenIDEA localizes its own menus and Find Action entries using the IDE display language.

## Features

- Review changes grouped by changelist or directory. Preview text diffs, open native diffs, commit selected changes, and add selected unversioned files during a commit.
- Browse repositories, save bookmarks, check out a working copy, and manage branches, tags, merges, repository imports and relocation.
- Browse repository history, inspect incoming revisions, compare files and revisions, and show BASE annotations in the editor.
- Shelve or restore local changes, including SVN-recognized binary files, and import or export patches.
- Edit SVN properties and ignore rules, manage locks, and review text, property, and tree conflicts. The text merge editor supports editing and saving a resolution.

## Permissions and data

| Permission | Use |
| --- | --- |
| `process:svn` | Run your approved SVN client for working-copy and repository operations. |
| `project.read` | Read working-copy metadata and file content for review. |
| `project.write` | Apply confirmed text resolutions and edits from the diff viewer. |
| `editor.read` | Read open buffer content for diffs and check for unsaved edits. |
| `events` | Refresh on changes and invalidate outdated annotations. |
| `storage` | Keep bookmarks, scope preferences, shelves, and recovery records locally. |
| `clipboard` (optional) | Use clipboard text in supported diff and copy actions. |

SVN handles authentication with its own configuration and credential cache. **Authenticate…** opens an IDE terminal where the SVN client can prompt for credentials or certificates. Repository connections are made by the approved SVN executable. The tool window displays SVN CLI output as returned by that client.

## Build and test

The build uses Rust stable with `wasm32-wasip2` and the SDK pinned to OpenIDEA v0.1.51. Install `svn` and `svnadmin` for tests, and use the `openidea-plugin` CLI from the [OpenIDEA v0.1.51 release](https://github.com/l0ng-ai/openidea/releases/tag/v0.1.51) to check and pack the plugin. The release provides a Linux x86-64 CLI. On Windows, the CLI can run in x86-64 WSL while Rust builds on Windows.

```sh
rustup target add wasm32-wasip2
cargo fmt --check
cargo test --locked
cargo build --locked --release --target wasm32-wasip2
```

Copy `target/wasm32-wasip2/release/openidea_subversion.wasm` to `dist/plugin.wasm`. Run the following from the project directory on Linux or WSL.

```sh
mkdir -p release
openidea-plugin check .
openidea-plugin pack . --out release/bamboowhut.subversion-0.1.14.oiplugin
openidea-plugin check release/bamboowhut.subversion-0.1.14.oiplugin
```

See the [manual testing guide](docs/manual-testing.md) for IDE workflows and [CONTRIBUTING.md](CONTRIBUTING.md) for development and releases.

## License

[Apache License 2.0](LICENSE). See [NOTICE](NOTICE) for artwork and third-party attribution. The IntelliJ IDEA Subversion integration served as a workflow reference.
