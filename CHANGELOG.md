# Changelog

## 1.0.0

- Adapt the tool window and operation dialogs to compact layouts. Add keyboard navigation for tabs, keep dialog actions visible, and restore focus when closing a dialog.
- Share file and difference navigation between commit and shelve previews. Show the number of selected paths, retain the commit message after an error, and clear it after a successful commit.
- Improve embedded diffs with current-difference markers, inline change highlighting, aligned side-by-side rows, and synchronized vertical scrolling. Expose separate actions for inline previews and editor diffs.
- Add independent font size settings for the tool window and embedded diffs, both following OpenIDEA by default.
- Keep repository and incoming log details in sync with the visible revisions. Dismiss progress feedback when the user cancels an SVN operation.

## 0.1.15

- Fix **Commit File…** preselection and changelist lookup for files whose names differ only by case in a case-sensitive working copy.

## 0.1.14

- Sync the tool window and open plugin forms when the display language changes, without waiting for an SVN refresh.

## 0.1.13

- Added English and Simplified Chinese UI with an optional language override. Subversion follows the OpenIDEA display language by default.
- Added GitHub CI for Rust tests, web script syntax checks, and validation of the installable plugin package.
- Added English and Chinese documentation, contribution guidance, and issue and pull request templates.

## 0.1.12

- Working-copy operations, repository browsing, diffs, history, annotations, shelves, and conflict resolution.
