# Manual verification

Use OpenIDEA 0.1.51 and a disposable SVN repository. On Windows, select an SVN client with Unicode command-line support, such as SlikSVN 1.14.5.

## Create a fixture

Run the following from PowerShell. It creates a repository and two working copies under the system temporary directory.

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

In `fixture-a`, create `module/a.txt`, `module/b.txt`, and `sibling/keep.txt`. Add and commit the files, then update `fixture-b`.

## Workflows

| Area | Expected behavior |
| --- | --- |
| Install and permissions | The IDE accepts the package. Granting SVN process access lets the tool window refresh in a trusted project. |
| Display language | Set the plugin to **Follow IDE** and switch the IDE between English and Simplified Chinese. With Subversion already open, check that its tabs and any open Update dialog switch automatically and preserve the entered values. Set the plugin to the opposite language and verify the tool window follows the override. |
| Browse and checkout | Save a repository URL, browse directories and check out a specific revision. Checkout accepts a new path with spaces and Unicode characters. It reports a conflict if the destination already exists. |
| Scope isolation | Open `fixture-a/module` and `fixture-b` in separate windows. Refresh both, then commit a subset in the first. Only the selected paths in that scope are committed. |
| Incoming and update | Commit in B. A shows the incoming revision and paths. After updating A, check Update Info and verify unrelated local edits remain. |
| Commit unversioned paths | Select an unversioned file and directory in Commit Changes. The repository receives only selected paths and their unignored descendants. Check Unicode and multiline commit messages in `svn log --xml`. |
| Shelve and unshelve | Shelve and restore added, deleted, and modified files. Compare the restored binary file hash, SVN status, and MIME property. |
| Local changes | Add, move, delete, revert, and update to a specific revision. Try Unicode file names and `@`. A move requires both sides when committing. |
| Diff and history | Double-click a file to open the native diff. Unsaved editor text appears on the local side. A commit requires saving first. Inspect history across a rename or copy. |
| Branch, tag, and merge | Create a branch and tag. Commit on the branch, preview a merge into trunk, and confirm preview leaves local files intact. Apply it, then preview a reverse merge. |
| Properties, ignore, and locks | Set and compare a property. Ignore a file, show ignored files, and remove the rule. Lock and unlock a versioned file. Update an external in its own scope. |
| Conflicts | Create a text conflict by changing the same line in both working copies and updating the second one after the first commits. Review all three sides, edit the result, and mark it resolved. Repeat with conflicting property edits and with a delete versus edit to inspect property and tree conflicts. |
| Authentication and recovery | Use a test service to trigger an authentication challenge and retry after authenticating in the IDE terminal. Trigger an out-of-date commit and check the pending recovery record and repository history. |
| Import, share, and relocate | Import a small directory, share an unversioned project, and verify its files remain available for explicit addition. Relocate a working copy to another URL for the same repository and check UUID validation. |

Record the IDE, plugin, and SVN versions, the target type, and any SVN error codes. Redact credentials, private URLs, and project content.
