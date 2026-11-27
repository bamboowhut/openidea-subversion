# Subversion for OpenIDEA

[English](README.md)

一个独立开发的 OpenIDEA 社区插件。通过 SVN 工具窗口和查找操作管理 Subversion 工作副本。使用 IDE 的原生文本差异查看器，并在工具窗口内提供冲突编辑、仓库浏览和历史记录。

- 插件 ID `bamboowhut.subversion`
- 验收环境为 OpenIDEA 0.1.51、插件 API 1.15、Windows x64
- 需要 SVN 1.14 或更高的 1.x 命令行客户端。Windows 中文路径使用 SlikSVN 1.14.5 通过命令行验证

## 安装

1. 从 GitHub Releases 下载或按下文构建 `bamboowhut.subversion-<版本>.oiplugin`。在 OpenIDEA 的 **设置 › 插件 › 齿轮 › 从插件包文件安装** 选择插件包。
2. 允许插件使用 `process:svn`。在插件的权限页可用 **选择可执行文件** 绑定到所需的 `svn.exe`。
3. 打开受信任的本地项目。在 **Tools** 菜单或 **Find Action** 中运行 **Subversion**。

在 Windows 上，某些 SVN 构建在没有控制台时会错误转换中文。有中文路径时，请选择经验证的 [SlikSVN 1.14.5](https://sliksvn.com/download/)，插件在传递这类参数前会先探测客户端行为。

### 界面语言

在 **设置 › 插件 › Subversion › 界面语言** 选择 **Follow IDE**（跟随 IDE）、**English**（英文）或 **Simplified Chinese**（简体中文）。默认跟随 IDE 的显示语言。IDE 使用其他语言时，插件采用英文。

工具窗口和已打开的插件表单会自动切换语言。OpenIDEA 原生菜单和查找操作使用 IDE 自己的显示语言。SVN 命令行输出保留客户端返回的原文。

## 使用

- 在 **Subversion › Local Changes** 中右键操作 SVN 变更。对当前工作范围内已打开的文件，可从 **查找操作** 运行 **Show History**、**Annotate** 等 Subversion 命令。
- **Local Changes** 按变更列表显示当前范围的文件。单击预览差异，双击打开原生差异查看器。通过标签栏的菜单可切换到项目内的子目录、独立工作副本或目录 external。**Update Project** 支持指定版本、深度、切换 URL、强制更新和忽略 externals。
- **Commit Changes** 提交对话框内勾选的变更。勾选 **Unversioned Files** 中的文件或目录即可在一次操作中加入 SVN 并提交。选中的新目录包含其未忽略的后代。目录复制、删除或替换可能涉及版本化后代，移动需要同时选择源和目标。失败后可用 **检查上次操作结果** 核对已加入但尚未提交的文件及仓库记录。
- **Repository** 显示当前范围的仓库日志，**Incoming** 显示实际待更新路径的提交。**Browse Subversion Repositories** 提供仓库目录浏览、书签、检出、导入、共享和分支操作。检出的目标是当前受信任项目内尚不存在的子目录。
- **Shelf** 可以搁置和恢复新增、删除或修改的文件，包括 SVN 识别为二进制的文件。含二进制内容的补丁最多 12 MiB，纯文本补丁最多 512 KiB。搁置数据按本地范围和仓库地址隔离。搁置或恢复前会验证所选路径和实际补丁结果。
- **历史**可按工作副本、文件或仓库地址翻页和查看版本差异。注解来自 SVN BASE，保存后的本地内容与 BASE 一致时也会显示在编辑器侧。
- **属性**支持查看、对比及编辑 SVN 属性，包括 `svn:ignore` 和 `svn:externals`。文本冲突显示三个输入及可编辑结果。属性和树冲突显示 SVN 给出的状态，手工调整后可标记已解决。
- **输出**保留 SVN 操作结果，提供文本编码选择。文件文本预览每侧最多 512 KiB，支持 UTF-8、带 BOM 的 Unicode 和用户所选的传统编码。二进制及特殊文件根据 SVN 能力显示状态或冲突选项。

## 权限与数据

| 权限 | 用途 |
| --- | --- |
| `process:svn` | 执行用户授权的本机 SVN 程序。用于仓库连接、差异、检出、提交及其他工作副本操作。 |
| `project.read` | 获取当前项目范围并读取文件元数据和差异内容。 |
| `project.write` | 在用户确认后，将文本冲突的编辑结果写入选中的工作副本文件。 |
| `editor.read` | 在当前窗口中检测未保存编辑，读取原生差异的本地文本。 |
| `events` | 在磁盘或保存事件到达时刷新面板。编辑已显示行注解的文件时清除旧注解。 |
| `storage` | 保存仓库书签、工作范围和中断操作的核对记录。 |

普通操作由 SVN 自身处理认证缓存。需要交互认证时，**认证**功能打开 IDE 终端运行 SVN，由客户端询问凭据或证书选择。插件进程只调用 `svn`，连接目标由用户填写的 SVN URL 或工作副本配置确定。插件没有独立的密码输入页。

## 构建与测试

需要 Rust stable、`wasm32-wasip2`、OpenIDEA v0.1.51 的插件 CLI，以及用于测试的 `svn`、`svnadmin` 和 Node.js。Windows 自动化测试包含中文路径，先将支持 Unicode 的客户端放在测试进程的 PATH 前面。官方提供 Linux x86-64 CLI，可在 Windows 的 WSL 内运行。 [OpenIDEA v0.1.51](https://github.com/l0ng-ai/openidea/releases/tag/v0.1.51)

```sh
rustup target add wasm32-wasip2
cargo fmt --check
cargo test --locked
cargo build --locked --release --target wasm32-wasip2
```

将 `target/wasm32-wasip2/release/openidea_subversion.wasm` 复制为 `dist/plugin.wasm`，再在 Linux 或 WSL 的项目目录执行。

```sh
mkdir -p release
openidea-plugin check .
openidea-plugin pack . --out release/bamboowhut.subversion-0.1.15.oiplugin
openidea-plugin check release/bamboowhut.subversion-0.1.15.oiplugin
```

SDK 固定到 OpenIDEA v0.1.51 的提交 `30e72ffa574bec988ada3ce2818298d99dfe83ad`。人工验收步骤见 [验收清单](docs/manual-testing.zh-CN.md)，开发和发布流程见 [贡献指南](CONTRIBUTING.md)。

## 许可

本项目采用 [Apache License 2.0](LICENSE)，图标和第三方资产声明见 [NOTICE](NOTICE)。插件工作流参考了 IntelliJ IDEA 的 Subversion 集成。
