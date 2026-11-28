"use strict";

let shelves = [];
let shelfSelection = null;
let shelfPreviewTimer;
let showRecycledShelves = false;
let shelfUnified = true;
let shelveUnified = false;
let unshelveSelection;
let unshelveFile = "";
let groupUnshelveFiles = true;
let removeAppliedFiles = false;
const activeShelfGroups = new Map();
const shelfComments = new Map();

function fileCount(count) {
  return count + (count === 1 ? t(" file") : t(" files"));
}

function shelfDate(seconds) {
  if (!seconds) return "";
  const date = new Date(seconds * 1000);
  const today = new Date();
  const time = date.toLocaleTimeString(panelLanguage, { hour: "numeric", minute: "2-digit" });
  if (date.toDateString() === today.toDateString()) return t("Today ") + time;
  today.setDate(today.getDate() - 1);
  if (date.toDateString() === today.toDateString()) return t("Yesterday ") + time;
  return date.toLocaleDateString(panelLanguage, { month: "short", day: "numeric", year: "numeric" }) + " " + time;
}

function currentShelf() {
  return shelves.find(shelf => shelf.id === shelfSelection?.id);
}

function shelfFile(shelf, path) {
  const item = shelf.added?.includes(path) ? "added" : shelf.deleted?.includes(path) ? "deleted" : "modified";
  return { path, item };
}

function shelfFileRow(shelf, path, showParent = true) {
  const name = relative(path);
  const parts = name.split("/");
  const label = parts.pop();
  const change = shelfFile(shelf, path);
  const line = el("div", undefined, "row");
  line.tabIndex = 0;
  line.append(fileIcon(path, false), el("span", label, "name-" + badge(change)));
  if (showParent && parts.length) line.append(el("span", parts.join("/"), "meta"));
  return line;
}

function shelfHeading(shelf, expanded) {
  const item = el("details");
  item.dataset.shelf = shelf.id;
  item.open = expanded;
  const heading = treeHeading(item, () => selectShelf(shelf));
  heading.dataset.shelfId = shelf.id;
  heading.dataset.path = "";
  const detail = [fileCount(shelf.paths.length), shelfDate(shelf.removed || shelf.created)].filter(Boolean).join(", ");
  heading.append(el("span", shelf.name, shelf.restored ? "restored narrow" : "narrow"), el("span", detail, "meta"));
  bindShelfRow(heading, shelf);
  item.append(heading);
  for (const path of shelf.paths) {
    const line = shelfFileRow(shelf, path);
    line.dataset.shelfId = shelf.id;
    line.dataset.path = path;
    line.onclick = () => selectShelf(shelf, path);
    line.ondblclick = () => showShelfDiff("native", shelf, path);
    bindShelfRow(line, shelf, path);
    item.append(line);
  }
  return item;
}

function renderShelves(entries, refreshPreview = true) {
  shelves = entries;
  const list = by("shelfList");
  const expanded = new Set([...list.querySelectorAll("details[open]")].map(item => item.dataset.shelf));
  const visible = [...entries].sort((a, b) => (b.created || b.id) - (a.created || a.id));
  const regular = visible.filter(shelf => !shelf.removed && (showRecycledShelves || !shelf.restored));
  const deleted = visible.filter(shelf => shelf.removed).sort((a, b) => b.removed - a.removed);
  list.replaceChildren(...regular.map(shelf => shelfHeading(shelf, expanded.has(String(shelf.id)))));
  const group = el("details");
  group.dataset.shelf = "deleted";
  group.open = expanded.has("deleted");
  const heading = treeHeading(group);
  heading.append(el("span", t("Recently Deleted")));
  group.append(heading, ...deleted.map(shelf => shelfHeading(shelf, expanded.has(String(shelf.id)))));
  if (!deleted.length) {
    heading.classList.add("no-children");
    heading.onclick = event => event.preventDefault();
  }
  list.append(group);
  const selected = [...regular, ...deleted].find(shelf => shelf.id === shelfSelection?.id);
  selectShelf(selected, selected?.paths.includes(shelfSelection?.path) ? shelfSelection.path : "", refreshPreview);
}

function selectShelf(shelf, path = "", refreshPreview = true) {
  shelfSelection = shelf ? { id: shelf.id, path } : null;
  for (const row of by("shelfList").querySelectorAll("[data-shelf-id]")) {
    row.dataset.active = String(Number(row.dataset.shelfId) === shelf?.id && row.dataset.path === path);
  }
  by("unshelve").disabled = !shelf;
  const index = shelf ? Math.max(0, shelf.paths.indexOf(path)) : -1;
  by("shelfPreviousFile").disabled = index < 1;
  by("shelfNextFile").disabled = index < 0 || index + 1 >= shelf.paths.length;
  by("shelfFileCount").textContent = !shelf ? "" : shelf.paths.length === 1 ? t("1 file") : `${index + 1}/${fileCount(shelf.paths.length)}`;
  if (refreshPreview && !by("shelfPane").hidden) queueShelfPreview(shelf, path);
}

function queueShelfPreview(shelf, path) {
  clearTimeout(shelfPreviewTimer);
  renderPreview(by("shelfPreview"), { message: shelf ? t("Loading…") : t("Nothing to show") });
  if (!shelf) return;
  const run = () => {
    if (by("shelfPane").hidden || currentTab !== "shelf") return;
    if (request) { shelfPreviewTimer = setTimeout(run, 120); return; }
    showShelfDiff("preview", shelf, path || shelf.paths[0]);
  };
  shelfPreviewTimer = setTimeout(run, 90);
}

function showShelfDiff(value, shelf = currentShelf(), path = shelfSelection?.path || "") {
  if (shelf) send("shelfDiff", { peg: shelf.id, path, value }, { quiet: value === "preview" });
}

function shelfMenu(x, y, shelf, path) {
  const edit = value => send("editShelf", { peg: shelf.id, value, path: value === "delete" ? path : "" });
  const exportPatch = patchAction => send("shelfDiff", { peg: shelf.id, path, value: "patch" }, { patchAction, patchPath: shelf.name.replaceAll(/[\\/:*?"<>|]/g, "_") });
  menuAt(x, y, [
    [t("Unshelve…"), () => openUnshelve(shelf, path), { icon: "unshelve", shortcut: "Ctrl+Shift+U" }],
    [t("Restore"), shelf.removed && (() => edit("restore"))],
    [t("Show Diff"), () => { showShelfPreview(true); selectShelf(shelf, path); }, { icon: "diff", shortcut: "Ctrl+D" }],
    [t("Show Diff in a New Tab"), () => showShelfDiff("native", shelf, path), { icon: "diff" }],
    [t("Compare with Local"), () => showShelfDiff("local", shelf, path)],
    [t("Create Patch…"), () => exportPatch("save"), { icon: "add" }],
    [t("Copy as Patch to Clipboard"), () => exportPatch("copy")],
    [t("Import Patches…"), () => by("importShelfFile").click()],
    null,
    [t("Rename…"), !path && (() => renameShelf(shelf)), { shortcut: "F2" }],
    [t("Delete…"), !shelf.removed && (() => edit("delete")), { shortcut: "Delete" }]
  ]);
}

function renameShelf(shelf) {
  form("Rename Shelved Changelist", [field("name", "Name:", shelf.name, true)], ({ name }) =>
    send("editShelf", { peg: shelf.id, value: "rename", name }));
}

function bindShelfRow(row, shelf, path = "") {
  row.oncontextmenu = event => {
    event.preventDefault();
    event.stopPropagation();
    row.focus();
    selectShelf(shelf, path);
    shelfMenu(event.clientX, event.clientY, shelf, path);
  };
  row.addEventListener("keydown", event => {
    if (event.target !== row) return;
    if (event.key === "ContextMenu" || event.shiftKey && event.key === "F10") {
      event.preventDefault();
      const rect = row.getBoundingClientRect();
      shelfMenu(rect.left + 8, rect.bottom, shelf, path);
    } else if (event.key === "F2" && !path) {
      event.preventDefault();
      renameShelf(shelf);
    } else if (event.key === "Delete" && !shelf.removed) {
      event.preventDefault();
      send("editShelf", { peg: shelf.id, value: "delete", path });
    }
  });
}

function showShelfPreview(show) {
  by("shelf").dataset.preview = String(show);
  by("shelfPane").hidden = !show;
  by("shelfTogglePreview").setAttribute("aria-pressed", String(show));
  if (show) queueShelfPreview(currentShelf(), shelfSelection?.path);
}

function toggleBranches(id, open) {
  for (const branch of by(id).querySelectorAll("details")) branch.open = open;
}

function selectionTree(container, changes, makeRow, selected, sync, grouped = true) {
  container.replaceChildren();
  const tree = new Map();
  for (const change of changes) {
    const parts = relative(change.path).split("/");
    parts.pop();
    let children = tree;
    if (grouped) for (const part of parts) {
      if (!children.has(part)) children.set(part, new Map());
      children = children.get(part);
    }
    children.set(change, null);
  }
  function branch(name, tree, icon) {
    const group = el("details");
    group.open = true;
    const heading = treeHeading(group);
    const box = el("input");
    box.type = "checkbox";
    box.setAttribute("aria-label", t("Include ") + name);
    box.onclick = event => event.stopPropagation();
    box.onchange = () => {
      for (const option of group.querySelectorAll(".row input[type=checkbox]:enabled")) {
        if (box.checked && selected.size < 200) selected.add(option.dataset.path || option.dataset.commitPath);
        if (!box.checked) selected.delete(option.dataset.path || option.dataset.commitPath);
      }
      sync();
    };
    heading.append(box, svnIcon(icon), el("span", name), el("span", "", "meta"));
    group.append(heading);
    for (const [item, children] of tree) group.append(children ? branch(item, children, "folder") : makeRow(item));
    heading.lastElementChild.textContent = fileCount(group.querySelectorAll(".row").length);
    return group;
  }
  container.append(branch(state.scope.split("/").at(-1), tree, "folder"));
}

function syncShelfCheckboxes(container, selected) {
  for (const option of container.querySelectorAll(".row input[type=checkbox]")) {
    option.checked = selected.has(option.dataset.path || option.dataset.commitPath);
  }
  for (const branch of container.querySelectorAll("details")) {
    const options = [...branch.querySelectorAll(".row input[type=checkbox]:enabled")];
    const checked = options.filter(option => option.checked).length;
    const box = branch.querySelector("summary input");
    box.disabled = !options.length;
    box.checked = checked > 0 && checked === options.length;
    box.indeterminate = checked > 0 && checked < options.length;
  }
}

function drawUnshelve() {
  const { shelf, paths } = unshelveSelection;
  const tree = by("unshelveTree");
  function sync() {
    syncShelfCheckboxes(tree, paths);
    by("applyUnshelve").disabled = !paths.size || request?.op === "unshelve";
  }
  selectionTree(tree, shelf.paths.map(path => shelfFile(shelf, path)), change => {
    const line = shelfFileRow(shelf, change.path, !groupUnshelveFiles);
    line.dataset.path = change.path;
    const check = el("input");
    check.type = "checkbox";
    check.dataset.path = change.path;
    check.setAttribute("aria-label", relative(change.path));
    check.onchange = () => { if (check.checked) paths.add(change.path); else paths.delete(change.path); sync(); };
    line.prepend(check);
    line.append(el("span", t(labels[change.item] || change.item), "meta"));
    line.dataset.active = String(unshelveFile === change.path);
    line.onclick = () => {
      unshelveFile = change.path;
      for (const row of tree.querySelectorAll(".row")) row.dataset.active = String(row === line);
    };
    line.ondblclick = () => showShelfDiff("native", shelf, change.path);
    line.onkeydown = event => {
      if (event.target === line && event.key === "Enter") {
        event.preventDefault();
        showShelfDiff("native", shelf, change.path);
      }
    };
    return line;
  }, paths, sync, groupUnshelveFiles);
  sync();
}

function openUnshelve(shelf = currentShelf(), path = shelfSelection?.path || "") {
  if (!shelf || document.querySelector("dialog[open]")) return;
  unshelveSelection = { shelf, paths: new Set(path ? [path] : shelf.paths) };
  unshelveFile = path || shelf.paths[0];
  const groups = new Set(["Changes", ...state.changes.map(change => change.group).filter(Boolean)]);
  by("unshelveGroups").replaceChildren(...[...groups].map(group => {
    const option = el("option", group === "Changes" ? t("Changes") : group);
    option.value = group;
    return option;
  }));
  by("unshelveGroup").value = activeShelfGroups.get(state.scope) || "Changes";
  by("unshelveComment").value = shelf.name;
  by("unshelveSetActive").checked = false;
  by("removeApplied").checked = removeAppliedFiles;
  by("unshelveDiff").disabled = !unshelveFile;
  by("unshelveError").textContent = "";
  drawUnshelve();
  showPanelDialog("unshelveDialog");
  by("unshelveGroup").focus();
  by("unshelveGroup").select();
}

function handleShelfResult(data, meta) {
  if (data.shelves) renderShelves(data.shelves);
  if (data.shelved != null) {
    if (by("commitDialog").open) dismissDialog("commitDialog");
    const shelf = shelves.find(entry => entry.id === data.shelved);
    if (shelf) { drafts.shelve = shelf.name; selectShelf(shelf); }
    tab("shelf", false);
  }
  if (data.unshelved != null && by("unshelveDialog").open) {
    if (by("unshelveSetActive").checked) activeShelfGroups.set(state.scope, by("unshelveGroup").value.trim());
    if (meta?.data?.name) shelfComments.set(state.scope + ":" + meta.data.name, by("unshelveComment").value);
    dismissDialog("unshelveDialog");
  }
  if (data.shelfPreview && meta?.data?.value === "preview" && currentShelf()?.id === meta.data.peg && !by("shelfPane").hidden) {
    const path = shelfSelection.path || currentShelf().paths[0];
    if (path === meta.data.path) renderPreview(by("shelfPreview"), { ...data.shelfPreview, readOnly: true, unified: shelfUnified });
  } else if (data.shelfPreview?.message) status(data.shelfPreview.message, false, true);
}

function shelvePreviewFiles() {
  return [...by("commitTree").querySelectorAll(".row[data-path]")].map(row => row.dataset.path);
}

function updateShelveToolbar() {
  if (by("shelveDiffToolbar").hidden) return;
  const files = shelvePreviewFiles();
  const index = files.indexOf(commitPath);
  const view = previewViews.get("commitPreview");
  const count = view?.hunks?.length;
  by("shelvePreviousFile").disabled = index < 1;
  by("shelveNextFile").disabled = index < 0 || index + 1 >= files.length;
  by("shelveFileCount").textContent = index < 0 ? "" : files.length === 1 ? t("1 file") : `${index + 1}/${fileCount(files.length)}`;
  by("shelvePreviousDifference").disabled = !(view?.index > 0);
  by("shelveNextDifference").disabled = !(view?.index < count - 1);
  by("shelveDifferenceCount").textContent = count === undefined ? "" : differencePosition(view);
}

function initShelf() {
  by("unshelve").onclick = () => openUnshelve();
  by("shelfOptions").onclick = event => buttonMenu(event.currentTarget, [
    [t("Show Already Unshelved"), () => { showRecycledShelves = !showRecycledShelves; renderShelves(shelves); }, { checked: showRecycledShelves }],
    [t("Import Patches…"), () => by("importShelfFile").click()]
  ]);
  by("shelfList").oncontextmenu = event => menuAt(event.clientX, event.clientY, [[t("Import Patches…"), () => by("importShelfFile").click()]]);
  by("shelfExpand").onclick = () => toggleBranches("shelfList", true);
  by("shelfCollapse").onclick = () => toggleBranches("shelfList", false);
  by("shelfTogglePreview").onclick = () => showShelfPreview(by("shelfPane").hidden);
  function moveFile(step) {
    const shelf = currentShelf();
    if (shelf) selectShelf(shelf, shelf.paths[Math.max(0, shelf.paths.indexOf(shelfSelection.path)) + step]);
  }
  by("shelfPreviousFile").onclick = () => moveFile(-1);
  by("shelfNextFile").onclick = () => moveFile(1);
  function layout(unified) {
    shelfUnified = unified;
    by("shelfUnified").setAttribute("aria-pressed", String(unified));
    by("shelfSideBySide").setAttribute("aria-pressed", String(!unified));
    const view = previewViews.get("shelfPreview");
    if (view) { view.data.unified = unified; drawPreview("shelfPreview"); }
  }
  by("shelfUnified").onclick = () => layout(true);
  by("shelfSideBySide").onclick = () => layout(false);
  by("shelfPreviousDifference").onclick = () => moveDifference(-1, "shelfPreview");
  by("shelfNextDifference").onclick = () => moveDifference(1, "shelfPreview");
  by("shelveExpand").onclick = () => toggleBranches("commitTree", true);
  by("shelveCollapse").onclick = () => toggleBranches("commitTree", false);
  function moveShelveFile(step) {
    const files = shelvePreviewFiles();
    const path = files[files.indexOf(commitPath) + step];
    if (path) {
      const row = [...by("commitTree").querySelectorAll(".row[data-path]")].find(item => item.dataset.path === path);
      for (let branch = row.closest("details"); branch; branch = branch.parentElement.closest("details")) branch.open = true;
      choose(path, true);
      row.scrollIntoView({ block: "nearest" });
    }
  }
  by("shelvePreviousFile").onclick = () => moveShelveFile(-1);
  by("shelveNextFile").onclick = () => moveShelveFile(1);
  by("shelvePreviousDifference").onclick = () => moveDifference(-1, "commitPreview");
  by("shelveNextDifference").onclick = () => moveDifference(1, "commitPreview");
  function switchShelveLayout(unified) {
    if (changeDialogMode === "shelve") shelveUnified = unified;
    else commitUnified = unified;
    by("shelveSideBySide").setAttribute("aria-pressed", String(!unified));
    by("shelveUnified").setAttribute("aria-pressed", String(unified));
    const view = previewViews.get("commitPreview");
    if (view) { view.data.unified = unified; drawPreview("commitPreview", true); }
  }
  by("shelveSideBySide").onclick = () => switchShelveLayout(false);
  by("shelveUnified").onclick = () => switchShelveLayout(true);
  by("shelveDiffSettings").onclick = event => buttonMenu(event.currentTarget, diffOptionActions("commitPreview"));
  by("unshelveDiff").onclick = () => showShelfDiff("native", unshelveSelection.shelf, unshelveFile);
  by("unshelveExpand").onclick = () => toggleBranches("unshelveTree", true);
  by("unshelveCollapse").onclick = () => toggleBranches("unshelveTree", false);
  by("unshelveGroupBy").onclick = () => {
    groupUnshelveFiles = !groupUnshelveFiles;
    by("unshelveGroupBy").setAttribute("aria-pressed", String(groupUnshelveFiles));
    drawUnshelve();
  };
  by("closeUnshelve").onclick = by("cancelUnshelve").onclick = () => dismissDialog("unshelveDialog");
  by("removeApplied").onchange = () => { removeAppliedFiles = by("removeApplied").checked; };
  by("applyUnshelve").onclick = () => {
    const { shelf, paths } = unshelveSelection;
    if (!paths.size) return;
    by("unshelveError").textContent = "";
    send("unshelve", { peg: shelf.id, paths: [...paths], name: by("unshelveGroup").value.trim(), removeApplied: removeAppliedFiles });
    by("applyUnshelve").disabled = true;
  };
  by("importShelfFile").onchange = async event => {
    const file = event.target.files[0];
    event.target.value = "";
    if (!file) return;
    if (file.size > 12 << 20) { status(t("The patch exceeds 12 MiB"), true, true); return; }
    try { send("importShelf", { name: file.name.replace(/\.(patch|diff)$/i, ""), value: await file.text() }); }
    catch (error) { status(String(error), true, true); }
  };
}
