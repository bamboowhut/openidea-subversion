# Subversion for OpenIDEA 人工验收

使用 OpenIDEA 0.1.51 和隔离的测试仓库。Windows 如需测试中文路径，请在插件权限中选择 SlikSVN 1.14.5 的 `svn.exe`。安装开发包时，IDE 会提示该包未经发布者签名。

## 准备测试仓库

在一个普通 PowerShell 窗口内执行。以下命令在系统临时目录新建测试仓库和工作副本。

```powershell
$testRoot = Join-Path $env:TEMP ('openidea-svn-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$repository = Join-Path $testRoot 'repo'
svnadmin create $repository
$svnUrl = ([uri]$repository).AbsoluteUri
svn mkdir "$svnUrl/trunk" "$svnUrl/branches" "$svnUrl/tags" -m 'Initial layout'
svn checkout "$svnUrl/trunk" (Join-Path $testRoot 'fixture-a')
svn checkout "$svnUrl/trunk" (Join-Path $testRoot 'fixture-b')
$testRoot
$svnUrl
```

先在 `fixture-a` 创建 `module/a.txt`、`module/b.txt`、`sibling/keep.txt`，将目录与文件加入 SVN 再提交为测试基线，在 `fixture-b` 更新。初次使用可通过下列日常操作完成。第二个工作副本可使用终端或另一个 IDE 窗口。

## 验收项

| 项目 | 操作与通过标准 |
| --- | --- |
| 安装和权限 | 安装包可被原版 IDE 识别。打开受信任目录，授权 SVN 进程后可以打开 Subversion 面板并刷新。 |
| 界面语言 | 在跟随 IDE 模式下切换 OpenIDEA 的英文和简体中文。保持工具窗口和更新对话框打开，检查标签及表单是否自动切换并保留输入。再指定与 IDE 不同的插件语言，核对工具窗口是否跟随指定语言。 |
| 仓库与检出 | 在测试父目录中打开 **Browse Subversion Repositories**，保存仓库地址、浏览目录，指定版本检出为新目录，完成后打开。用已存在的目标再检出应提示冲突。验证包含中文和空格的新目录。 |
| 项目隔离 | 在两个窗口分别打开 `fixture-a/module` 和 `fixture-b`，在两边同时刷新 **Local Changes**。A 只显示 `module` 内的变更，B 显示自己工作副本的变更。提交对话框仅显示当前工作副本的文件，提交后未勾选文件仍有变更。 |
| Incoming 和更新 | 在 B 提交后，A 的 **Incoming** 显示提交记录和对应路径。执行 **Update Project**，从 **Update Info** 核对版本和文件，确认 A 原有的其他编辑仍在。 |
| 直接提交未跟踪文件 | 在 **Commit Changes** 中仅勾选一个未跟踪文件和一个未跟踪目录。提交后核对仓库中的文件、目录后代和提交说明。未勾选及忽略的文件仍保留在工作副本。 |
| Shelf | 选中文本改动和 SVN 已识别的二进制文件，执行 **Shelve Changes…**。验证修改、新增和删除三种状态。恢复后比较二进制文件的哈希值，并核对 SVN 状态和 MIME 属性。 |
| 本地变更 | 查看目录和列表，添加、移动、删除、还原并更新到指定版本。用中文及含 `@` 的文件名，验证路径原样保存。移动后单独勾选一侧，应提示补齐移动关联。 |
| 差异与历史 | 双击普通文件打开原生差异。打开未保存的本地修改，差异右侧应显示当前编辑内容，提交前要求先保存。检查旧版本和复制、改名历史。 |
| 中文提交说明 | 提交包含中文、两行及 emoji 的说明，使用插件历史和 `svn log --xml` 核对内容。 |
| 分支、标签和合并 | 从 `trunk` 创建 `branches/demo` 和 `tags/demo`。切换第二工作副本后提交一个改动。第一副本预演合并时磁盘内容保持原样，执行后显示待提交变更。提交合并、更新后用负版本号预演反向合并。 |
| 属性、忽略和锁 | 设置测试文件属性并查看属性差异。忽略一个文件，切换显示已忽略，再取消直接匹配的规则。锁定和解锁同一个版本化文件。外部目录先作为单独工作目录再更新。 |
| 冲突 | 在两个工作副本修改同一行，先提交第二份，再更新第一份。打开冲突页，核对基线、本地和传入版本并编辑保存。分别修改同一属性制造属性冲突，以及一方删除且一方修改文件制造树冲突，检查状态和确认入口。 |
| 认证与失败恢复 | 在获准使用的测试 SVN 服务上触发认证失败，通过 **认证** 终端登录后重试。用并发提交触发版本过期，查看本地状态与仓库记录。取消一次长操作，检查工作副本再重试。 |
| 导入、共享和迁移 | 将小目录导入测试仓库的新地址。从另一个未受版本管理的测试目录发起共享，确认原文件成为可显式添加的变更。用同一测试仓库的另一个访问地址验证迁移时的 UUID 校验。 |

记录失败时附上 OpenIDEA 版本、SVN 客户端版本、操作、目标类型和输出中的 SVN 错误编号。仓库真实凭据和业务代码可省略。

## 通过后的发布材料

从通过验收的源码版本构建插件，使用自己的 `bamboowhut` Ed25519 发布密钥签名。将签名包发布到 `BambooWhut/openidea-subversion` 的版本 Release。登记 PR 使用 `publishers/bamboowhut.toml` 和 `plugins/bamboowhut.subversion.toml`，填入公钥、owner、源码和下载地址、版本及签名包的 SHA-256。发布密钥文件应放在仓库外部。 [OpenIDEA 登记规则](https://github.com/l0ng-ai/openidea-plugins#readme)
