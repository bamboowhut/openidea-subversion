"use strict";

const by = id => document.getElementById(id);
const labels = {
  normal: "Normal", modified: "Modified", added: "Added", deleted: "Deleted", unversioned: "Unversioned",
  ignored: "Ignored", missing: "Missing", conflicted: "Conflicted", replaced: "Replaced", external: "External working copy",
  obstructed: "Obstructed", incomplete: "Incomplete", none: "None"
};
const writable = new Set(["added", "modified", "deleted", "replaced"]);
const selected = new Set();
const openGroups = new Map();
let state = { changes: [] };
let active = "";
let currentTab = "changes";
let showIgnored = false;
let flat = true;
let waiting;
let dialogFocus;
const previewTimers = new Map();
let commitPath = "";
let changeDialogMode = "commit";
const drafts = { commit: "", shelve: "" };
const logs = Object.fromEntries(["repository", "incoming"].map(name => [name, { entries: [], next: null, root: "", loaded: false }]));
let seq = 0;
let languageSeq = 0;
let languageRequest = null;
let panelVisible = true;
// OpenIDEA broadcasts view messages to every window hosting this tool window.
const panel = Array.from(crypto.getRandomValues(new Uint32Array(4)),
  value => value.toString(16).padStart(8, "0")).join("");
let request = null;
let refreshTimer;
let invalidated = false;
let history = { entries: [], target: null, next: null, root: "" };
let conflict = null;
let propTarget = "";
let props = [];
let repoLocation = "";
let repoRoot = "";
let output = "";
let branchConfig = null;
let menuAnchor;

function el(tag, text, cls) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (cls) node.className = cls;
  return node;
}

function treeHeading(branch, select) {
  const heading = el("summary", undefined, "tree-heading");
  const disclosure = el("span", undefined, "disclosure");
  const collapsed = svnIcon("tree-right");
  const expanded = svnIcon("more");
  collapsed.classList.add("collapsed");
  expanded.classList.add("expanded");
  disclosure.append(collapsed, expanded);
  heading.append(disclosure);
  heading.onclick = event => {
    if (event.target.closest("input, button")) return;
    select?.();
    if (event.detail && !disclosure.contains(event.target)) event.preventDefault();
  };
  heading.ondblclick = event => {
    if (!event.target.closest("input, button") && !disclosure.contains(event.target)) branch.open = !branch.open;
  };
  heading.onkeydown = event => {
    if (event.target !== heading || !["ArrowLeft", "ArrowRight"].includes(event.key)) return;
    event.preventDefault();
    branch.open = event.key === "ArrowRight";
  };
  return heading;
}


function tab(name, requestData = true) {
  closeMenus();
  currentTab = name;
  document.querySelectorAll("main > section").forEach(section => { section.hidden = section.id !== name; });
  document.querySelectorAll("nav button[data-tab]").forEach(button => {
    if (button.dataset.tab === name) button.hidden = false;
    button.setAttribute("aria-selected", String(button.dataset.tab === name));
  });
  if (logs[name] && !logs[name].loaded && state.scope) loadLog(name);
  if (requestData && name === "shelf" && state.scope) send("shelves", {}, { quiet: true });
}

function status(text, error = false, visible = false) {
  by("status").textContent = text;
  by("status").classList.toggle("error", error);
  by("status").dataset.visible = String(visible);
}

function appendOutput(value) {
  if (!value || !value.trim()) return;
  output = (output + value.trimEnd() + "\n\n").slice(-(768 << 10));
  by("outputText").textContent = output;
}

function send(op, data = {}, meta = {}) {
  if (request) {
    if (meta.previewTo) return;
    if (op === "refresh") invalidated = true;
    else waiting = () => send(op, data, meta);
    return;
  }
  const serial = ++seq;
  request = { serial, op, data, ...meta };
  document.body.setAttribute("aria-busy", "true");
  if (!meta.quiet && !meta.previewTo) status(t("Running SVN…"), false, true);
  const project = op === "refresh" ? "" : state.project || "";
  window.openidea.postMessage(JSON.stringify({ op, seq: serial, panel, project, encoding: by("encoding").value, ...data }));
}

function scheduleRefresh() {
  invalidated = true;
  clearTimeout(refreshTimer);
  if (!request && document.visibilityState !== "hidden") {
    refreshTimer = setTimeout(() => {
      invalidated = false;
      send("refresh", {}, { quiet: true });
    }, 500);
  }
}

function requestLanguage() {
  if (!panelVisible || document.visibilityState === "hidden") return;
  const now = performance.now();
  if (languageRequest && (request || now - languageRequest.started < 5000)) return;
  languageRequest = { serial: ++seq, started: now };
  window.openidea.postMessage(JSON.stringify({ languageOnly: true, seq: languageRequest.serial, panel }));
}

function updatePanelLanguage(msg) {
  if (msg.seq < languageSeq) return;
  languageSeq = msg.seq;
  if (!applyLanguage(msg.language)) return;
  closeMenus();
  if (state.project && !(msg.data?.snapshot || msg.snapshot)) redrawChanges();
  for (const name of Object.keys(logs)) if (logs[name].loaded) renderLog(name);
  if (by("shelfList").children.length) renderShelves(shelves, false);
  if (by("commitDialog").open) {
    localizeCommitControls();
    const group = by("commitGroup").querySelector('option[value="Changes"]');
    if (group) group.textContent = t("Changes");
  }
  const group = by("unshelveGroups").querySelector('option[value="Changes"]');
  if (group) group.label = t("Changes");
  localizeForm();
}

function badge(change) {
  if (change.tree || change.item === "conflicted" || change.props === "conflicted") return "C";
  if (change.item === "ignored") return "I";
  if (change.item === "unversioned") return "?";
  if (change.item === "external") return "X";
  if (change.item === "missing" || change.item === "deleted") return "D";
  if (change.item === "added") return "A";
  if (change.locked && change.props !== "modified" && change.item === "normal") return "L";
  return "M";
}

function canCommit(change) {
  return badge(change) !== "C" && (change.item === "unversioned" || writable.has(change.item)
    || change.item === "normal" && change.props === "modified");
}


function canShelve(change) {
  return !change.directory && ["added", "deleted", "modified"].includes(change.item) && !change.tree && !change.copied
    && !change.from && !change.to && change.props !== "conflicted";
}

function target() {
  return active || state.scope || "";
}


function queuePreview(path, id) {
  clearTimeout(previewTimers.get(id));
  const pane = by(id);
  if (!path) {
    renderPreview(pane, { message: t("Nothing to show") });
    return;
  }
  renderPreview(pane, { message: t("Loading…") });
  const run = () => {
    if (path !== (id === "localDiff" ? active : commitPath)) return;
    if (request) { previewTimers.set(id, setTimeout(run, 150)); return; }
    send("diff", { path, preview: true }, { previewTo: id, quiet: true });
  };
  previewTimers.set(id, setTimeout(run, 100));
}

function localNavigation() {
  const all = [...by("changeList").querySelectorAll(".row[data-path]")];
  const group = all.find(row => row.dataset.path === active)?.closest("details[data-group]")?.dataset.group;
  const visible = group ? all.filter(row => row.closest("details[data-group]")?.dataset.group === group) : [];
  const index = visible.findIndex(row => row.dataset.path === active);
  by("previousFile").disabled = index < 1;
  by("nextFile").disabled = index < 0 || index >= visible.length - 1;
  by("diffFiles").disabled = index < 0;
  by("diffFiles").textContent = (index + 1) + "/" + visible.length + (visible.length === 1 ? t(" file") : t(" files"));
  const change = state.changes.find(change => change.path === active);
  by("jumpToSource").disabled = !change || change.directory || ["deleted", "missing"].includes(change.item);
  updateDiffToolbar();
  return { visible, index };
}

function choose(path, commit = false) {
  if (commit) {
    commitPath = path;
    queuePreview(path, "commitPreview");
  } else {
    active = path;
    by("shelveSelected").disabled = !state.changes.some(change => change.path === path && canShelve(change));
    by("revert").disabled = !path;
    by("showDiff").disabled = !path;
    localNavigation();
    if (!by("localPane").hidden) queuePreview(path, "localDiff");
  }
  by(commit ? "commitTree" : "changeList").querySelectorAll(".row[data-path]").forEach(row => {
    row.dataset.active = String(row.dataset.path === path);
  });
}

function openDiff(change) {
  if (!change) return;
  send(badge(change) === "C" ? "conflict" : "diff", { path: change.path });
}

function row(change, name, commit) {
  const line = el("div", undefined, "row");
  line.tabIndex = 0;
  line.dataset.active = String(change.path === (commit ? commitPath : active));
  line.dataset.path = change.path;
  if (commit) {
    const check = el("input");
    check.type = "checkbox";
    check.checked = selected.has(change.path);
    check.disabled = !(changeDialogMode === "shelve" ? canShelve(change) : canCommit(change));
    check.setAttribute("aria-label", name);
    check.onclick = event => event.stopPropagation();
    check.onchange = () => {
      if (check.checked && selected.size >= 200) {
        check.checked = false;
        return;
      }
      if (check.checked) selected.add(change.path);
      else selected.delete(change.path);
      const group = line.closest("[data-commit-group]");
      if (group) syncCommitCheckbox(group);
      if (change.item === "unversioned") choose(change.path, true);
      updateCommit();
    };
    check.dataset.commitPath = change.path;
    line.append(check);
  }
  const code = badge(change);
  const detail = [t(labels[change.item] || change.item), change.group].filter(Boolean).join(" · ");
  const nameNode = el("span", name, "narrow name-" + (code === "?" ? "unversioned" : code));
  const path = relative(change.path);
  const parent = path.slice(0, path.lastIndexOf("/"));
  line.title = path + "\n" + detail;
  line.append(fileIcon(change.path, change.directory || change.item === "external"), nameNode);
  if (flat && parent && !(commit && changeDialogMode === "shelve")) line.append(el("span", parent, "meta narrow"));
  line.onclick = () => choose(change.path, commit);
  line.ondblclick = () => openDiff(change);
  line.onkeydown = event => {
    if (event.target !== line) return;
    if (!commit && (event.key === "ContextMenu" || event.shiftKey && event.key === "F10")) {
      event.preventDefault();
      const rect = line.getBoundingClientRect();
      changeMenu(rect.left + 8, rect.bottom, change);
    } else if (!commit && event.key === "F4" && !["deleted", "missing"].includes(change.item)) {
      event.preventDefault();
      send("openSource", { path: change.path });
    } else if (event.key === "Enter" || event.ctrlKey && event.key.toLowerCase() === "d") {
      event.preventDefault();
      openDiff(change);
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      const rows = [...line.closest(".list").querySelectorAll(".row[data-path]")].filter(row => row.offsetParent);
      const next = rows[rows.indexOf(line) + (event.key === "ArrowDown" ? 1 : -1)];
      if (next) {
        event.preventDefault();
        next.focus();
        choose(next.dataset.path, commit);
      }
    }
  };
  if (!commit) line.oncontextmenu = event => {
    event.preventDefault();
    line.focus({ preventScroll: true });
    choose(change.path);
    changeMenu(event.clientX, event.clientY, change);
  };
  return line;
}

function relative(path) {
  return state.scope && path.toLowerCase().startsWith(state.scope.toLowerCase() + "/")
    ? path.slice(state.scope.length + 1) : path === state.scope ? "." : path;
}

function groupName(change) {
  if (change.item === "unversioned") return "Unversioned Files";
  if (change.item === "ignored") return "Ignored Files";
  return change.group || "Changes";
}

function visibleChanges() {
  const filter = by("filter").value.toLowerCase().trim();
  return state.changes.filter(change => (showIgnored || change.item !== "ignored")
    && (!filter || [change.path, t(labels[change.item] || change.item), change.item, change.group].some(x => (x || "").toLowerCase().includes(filter))));
}

function syncCommitCheckbox(group) {
  const box = group.querySelector("[data-commit-group-checkbox]");
  const paths = [...group.querySelectorAll("[data-commit-path]:enabled")];
  const included = paths.filter(item => selected.has(item.dataset.commitPath)).length;
  box.disabled = !paths.length;
  box.checked = paths.length > 0 && included === paths.length;
  box.indeterminate = included > 0 && included < paths.length;
}

function changeTree(container, changes, commit = false) {
  const top = new Map();
  for (const change of changes) {
    const name = groupName(change);
    if (!top.has(name)) top.set(name, []);
    top.get(name).push(change);
  }
  if (!commit && !top.has("Changes")) top.set("Changes", []);
  container.replaceChildren();
  const rank = name => name === "Changes" ? 0 : name === "Unversioned Files" ? 2 : name === "Ignored Files" ? 3 : 1;
  for (const [name, rows] of [...top].sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))) {
    const detail = el("details");
    detail.dataset.group = name;
    const summary = treeHeading(detail);
    const label = rank(name) === 1 ? name : t(name);
    if (commit) {
      detail.dataset.commitGroup = name;
      const box = el("input");
      box.type = "checkbox";
      box.dataset.commitGroupCheckbox = "";
      box.setAttribute("aria-label", t("Include ") + label);
      if (name === "Unversioned Files") box.title = t("Selected folders include their unignored contents");
      box.onclick = event => event.stopPropagation();
      box.onchange = () => {
        for (const option of detail.querySelectorAll("[data-commit-path]:enabled")) {
          const path = option.dataset.commitPath;
          if (!box.checked) selected.delete(path);
          else if (selected.size < 200) selected.add(path);
          option.checked = selected.has(path);
        }
        syncCommitCheckbox(detail);
        updateCommit();
      };
      summary.append(box);
    }
    const directoriesCount = rows.filter(change => change.directory).length;
    const filesCount = rows.length - directoriesCount;
    const count = [directoriesCount && directoriesCount + (directoriesCount === 1 ? t(" directory") : t(" directories")),
      filesCount && filesCount + (filesCount === 1 ? t(" file") : t(" files"))].filter(Boolean).join(t(" and "));
    summary.append(el(name === "Changes" ? "strong" : "span", label), el("span", count, "meta"));
    detail.append(summary);
    const groupKey = container.id + ":" + name;
    const filtering = !commit && by("filter").value.trim().length > 0;
    detail.open = filtering || (openGroups.get(groupKey) ??
      (name === "Changes" || commit && (name !== "Unversioned Files" || rows.some(change => selected.has(change.path)))));
    detail.ontoggle = () => {
      if (detail.isConnected && !filtering) openGroups.set(groupKey, detail.open);
      if (!commit && detail.isConnected) localNavigation();
    };
    container.append(detail);
    const directories = new Map();
    for (const change of rows) {
      const path = relative(change.path);
      const parts = path.split("/");
      const leaf = parts.pop();
      let parent = detail;
      let prefix = groupKey + "/";
      if (!flat) for (const part of parts) {
        prefix += part + "/";
        if (!directories.has(prefix)) {
          const folder = el("details");
          const key = prefix;
          folder.open = filtering || (openGroups.get(key) ?? true);
          folder.ontoggle = () => {
            if (folder.isConnected && !filtering) openGroups.set(key, folder.open);
            if (!commit && folder.isConnected) localNavigation();
          };
          const heading = treeHeading(folder);
          heading.append(svnIcon("folder"), part);
          folder.append(heading);
          parent.append(folder);
          directories.set(prefix, folder);
        }
        parent = directories.get(prefix);
      }
      parent.append(row(change, leaf, commit));
    }
    if (commit) syncCommitCheckbox(detail);
  }
}

function redrawChanges() {
  const list = by("changeList");
  const scroll = list.scrollTop;
  const focused = list.contains(document.activeElement) ? document.activeElement.dataset.path : null;
  if (state.scope) changeTree(list, visibleChanges());
  else list.replaceChildren(el("p", t("No Subversion working copy"), "caption"));
  list.scrollTop = scroll;
  if (focused) [...list.querySelectorAll(".row[data-path]")].find(row => row.dataset.path === focused)?.focus({ preventScroll: true });
  localNavigation();
  if (by("commitDialog").open) redrawCommit();
}

function redrawCommit() {
  const group = by("commitGroup").value;
  const changes = state.changes.filter(change => (change.item === "unversioned" || (change.group || "Changes") === group) &&
    (changeDialogMode === "shelve" ? canShelve(change) : change.item !== "ignored"));
  if (changeDialogMode === "shelve") {
    selectionTree(by("commitTree"), changes, change => row(change, relative(change.path).split("/").at(-1), true), selected, updateCommit);
  } else changeTree(by("commitTree"), changes, true);
  updateCommit();
}

function updateCommit() {
  if (changeDialogMode === "shelve") {
    syncShelfCheckboxes(by("commitTree"), selected);
    const counts = new Map();
    for (const change of state.changes) if (selected.has(change.path)) counts.set(change.item, (counts.get(change.item) || 0) + 1);
    by("shelveCounts").textContent = [...counts].map(([type, count]) => count + " " + t(labels[type] || type)).join(t(", "));
  }
  by("commit").disabled = selected.size === 0 || !state.scope || !by("note").value.trim() || ["commit", "shelve"].includes(request?.op);
}

function selectCommitGroup(paths = []) {
  selected.clear();
  const group = by("commitGroup").value;
  const samePath = (change, path) => {
    const a = change.path.replaceAll("\\", "/").toLowerCase();
    const b = path.replaceAll("\\", "/").toLowerCase().replace(/\/$/, "");
    return a === b || a.startsWith(b + "/");
  };
  const candidates = state.changes.filter(change => (changeDialogMode === "shelve" ? canShelve(change) : canCommit(change)) &&
    (change.item === "unversioned"
      ? paths.some(path => samePath(change, path))
      : (change.group || "Changes") === group && (!paths.length || paths.some(path => samePath(change, path)))));
  for (const change of candidates.slice(0, 200)) selected.add(change.path);
  redrawCommit();
  choose(candidates[0]?.path || "", true);
}


function showPanelDialog(id) {
  dialogFocus = document.activeElement;
  by(id).setAttribute("open", "");
  by("dialogShade").hidden = false;
  for (const element of document.querySelectorAll("body > nav, body > main, #status, #pending")) element.inert = true;
}

function dismissDialog(id) {
  by(id).removeAttribute("open");
  by("dialogShade").hidden = true;
  for (const element of document.querySelectorAll("body > nav, body > main, #status, #pending")) element.inert = false;
  if (id === "commitDialog") {
    drafts[changeDialogMode] = by("note").value;
    commitPath = "";
    by("commitError").textContent = "";
  }
  dialogFocus?.focus({ preventScroll: true });
  dialogFocus = null;
}

function localizeCommitControls() {
  const shelve = changeDialogMode === "shelve";
  by("commitTitle").textContent = shelve ? t("Shelve Changes") : t("Commit Changes");
  by("commit").textContent = shelve ? t("Shelve Changes") : t("Commit");
}

function openCommit(paths = [], mode = "commit") {
  if (!state.scope || document.querySelector("dialog[open]")) return;
  changeDialogMode = mode;
  const shelve = mode === "shelve";
  by("commitDialog").dataset.mode = mode;
  delete by("commitDialog").dataset.error;
  localizeCommitControls();
  by("commitSvnOptions").hidden = shelve;
  by("shelveCounts").hidden = !shelve;
  by("shelveShowDiff").hidden = !shelve;
  by("shelveTreeActions").hidden = !shelve;
  by("shelveDiffToolbar").hidden = !shelve;
  by("note").value = drafts[mode];
  const select = by("commitGroup");
  const groups = new Set(["Changes", ...state.changes.map(change => change.group).filter(Boolean)]);
  select.replaceChildren(...[...groups].map(group => {
    const option = el("option", group === "Changes" ? t("Changes") : group);
    option.value = group;
    return option;
  }));
  const first = paths[0]?.replaceAll("\\", "/").toLowerCase();
  const match = first && state.changes.find(change => change.path.toLowerCase() === first);
  const group = match?.group || (!match && activeShelfGroups.get(state.scope)) || "Changes";
  select.value = groups.has(group) ? group : "Changes";
  if (!shelve && !drafts.commit) by("note").value = shelfComments.get(state.scope + ":" + select.value) || "";
  showPanelDialog("commitDialog");
  selectCommitGroup(paths);
  by("note").focus();
  by("note").select();
}

function snapshot(next) {
  if (!next) return;
  const changed = next.scope !== state.scope || next.project !== state.project;
  if (changed) {
    selected.clear();
    openGroups.clear();
    choose("");
    for (const log of Object.values(logs)) log.loaded = false;
    if (state.project) {
      by("note").value = "";
      drafts.commit = drafts.shelve = "";
      if (by("commitDialog").open) dismissDialog("commitDialog");
      if (by("unshelveDialog").open) dismissDialog("unshelveDialog");
      shelfSelection = null;
      renderShelves([]);
      propTarget = "";
      clearConflict();
    }
  }
  state = next;
  const selectFirst = changed && !active ? state.changes.find(change => change.item !== "unversioned" && canCommit(change))?.path : null;
  const valid = new Set(state.changes.filter(canCommit).map(x => x.path));
  for (const p of selected) if (!valid.has(p)) selected.delete(p);
  if (active && !state.changes.some(change => change.path === active)) choose("");
  by("scope").textContent = state.scope?.replaceAll("/", "\\") || t("No Subversion working copies");
  by("wcUrl").textContent = state.info?.url || "";
  by("wcDepth").textContent = state.info?.depth || "";
  by("wcRevision").textContent = state.info?.revision ?? "";
  for (const id of ["cleanupCopy", "configureBranches", "mergeFrom", "openCommit", "update"]) by(id).disabled = !state.scope;
  by("pending").hidden = !state.pending;
  by("pendingLabel").textContent = state.pending ? t("Pending · ") + state.pending.label : "";
  redrawChanges();
  if (selectFirst) choose(selectFirst);
  if (changed && logs[currentTab] && state.scope) loadLog(currentTab);
  if (changed && currentTab === "shelf" && state.scope) send("shelves", {}, { quiet: true });
}

function renderHistory(data, meta) {
  if (meta && meta.append) history.entries.push(...data.entries);
  else {
    history.entries = data.entries;
    by("historyPaths").replaceChildren(el("span", t("Select a revision to see its changed paths"), "caption"));
  }
  if (data.historyTarget) history.target = data.historyTarget;
  else if (meta && !meta.append) {
    const { path, url, peg } = meta.data;
    history.target = { path, url, peg };
  }
  history.next = data.next;
  history.root = data.historyRoot || history.root;
  by("historyTitle").textContent = history.target ? (history.target.url || history.target.path || state.scope || t("History")) : t("History");
  by("older").disabled = history.next == null;
  const list = by("historyList");
  list.replaceChildren();
  for (const entry of history.entries) {
    const line = el("div", undefined, "row");
    line.tabIndex = 0;
    line.append(el("strong", "r" + entry.revision), el("span", entry.message.split("\n")[0] || t("(No commit message)"), "grow"),
      el("span", entry.author + "  " + entry.date.slice(0, 10), "meta"));
    line.onclick = () => renderRevision(entry);
    line.onkeydown = e => { if (e.key === "Enter") renderRevision(entry); };
    list.append(line);
  }
  tab("history");
}

function renderRevision(entry, pane = by("historyPaths"), root = history.root, tree = false) {
  pane.replaceChildren();
  if (!tree) pane.append(el("strong", "r" + entry.revision + " · " + entry.author), el("pre", entry.message));
  const directories = new Map();
  for (const change of entry.changes) {
    const portions = change.path.split("/").filter(Boolean);
    const name = portions.pop();
    let parent = pane;
    if (tree && state.info?.url) {
      const prefix = decodeURIComponent(new URL(state.info.url).pathname).slice(decodeURIComponent(new URL(root).pathname).replace(/\/$/, "").length).split("/").filter(Boolean);
      if (prefix.every((part, index) => portions[index] === part)) portions.splice(0, prefix.length);
    }
    let key = "";
    if (tree) for (const part of portions) {
      key += "/" + part;
      if (!directories.has(key)) {
        const folder = el("details");
        folder.open = true;
        const heading = treeHeading(folder);
        heading.append(svnIcon("folder"), el("span", part));
        folder.append(heading);
        folder.dataset.name = part;
        parent.append(folder);
        directories.set(key, folder);
      }
      parent = directories.get(key);
    }
    const btn = el("button", undefined, tree ? "row name-" + change.action : "row");
    btn.append(fileIcon(change.path, change.kind === "dir"), tree ? name : change.action + "   " + change.path);
    btn.style.whiteSpace = "normal";
    if (change.from) btn.title = t("Copied from ") + change.from + " @ r" + change.from_rev;
    const open = () => {
      const inherited = entry.changes.filter(parent => parent !== change && parent.from && parent.kind === "dir"
        && change.path.startsWith(parent.path + "/")).sort((a, b) => b.path.length - a.path.length)[0];
      const current = !change.from && inherited && change.action !== "A"
        ? { ...change, from: inherited.from + change.path.slice(inherited.path.length), from_rev: inherited.from_rev }
        : change;
      send("historyDiff", { url: root, peg: entry.revision, change: current });
    };
    if (tree) {
      btn.ondblclick = open;
      btn.onkeydown = event => { if (event.key === "Enter" || event.ctrlKey && event.key === "d") open(); };
    } else btn.onclick = open;
    parent.append(btn);
  }
  if (tree) for (const folder of pane.querySelectorAll("details")) {
    while (folder.children.length === 2 && folder.lastElementChild.tagName === "DETAILS") {
      const child = folder.lastElementChild;
      folder.dataset.name += "\\" + child.dataset.name;
      folder.firstElementChild.lastElementChild.textContent = folder.dataset.name;
      child.replaceWith(...[...child.children].slice(1));
    }
  }
}

function renderRepo(data) {
  repoLocation = data.location;
  repoRoot = data.repository;
  by("url").value = data.location;
  by("repoLocation").textContent = data.location + " · r" + data.revision;
  const list = by("repoList");
  list.replaceChildren();
  for (const entry of data.entries) {
    const line = el("div", undefined, "row");
    line.tabIndex = 0;
    const directory = entry.kind === "dir";
    line.setAttribute("aria-label", entry.name + (directory ? t(" · directory") : t(" · file")));
    line.append(fileIcon(entry.name, directory), el("span", entry.name, "grow"),
      el("span", "r" + entry.revision + "  " + entry.author, "meta"));
    line.onclick = () => { by("url").value = entry.url; };
    const open = () => entry.kind === "dir" ? send("browse", { url: entry.url, revision: by("revision").value })
      : send("history", { url: entry.url, peg: numericRevision() });
    line.ondblclick = open;
    line.onkeydown = event => { if (event.key === "Enter") open(); };
    list.append(line);
  }
  if (data.entries.length === 0) list.append(el("p", t("The directory is empty"), "caption"));
  tab("browser");
}

function renderProps(data, meta) {
  props = data.properties;
  propTarget = meta?.data.path || data.propertyTarget || propTarget || state.scope || "";
  by("propertyTarget").textContent = propTarget || t("Current working directory");
  const list = by("propList");
  list.replaceChildren();
  for (const property of props) {
    const row = el("div", undefined, "row");
    row.append(el("strong", property.name), el("span", property.binary ? t("Binary property") : property.value, "grow"));
    if (!property.binary) {
      const edit = el("button", t("Edit…"));
      edit.onclick = () => propertyDialog(property.name, property.value);
      row.append(edit);
    }
    const remove = el("button", t("Delete…"));
    remove.onclick = () => send("propertyDelete", { path: propTarget, name: property.name });
    row.append(remove);
    list.append(row);
  }
  if (props.length === 0) list.append(el("p", t("This path has no explicit SVN properties"), "caption"));
  tab("properties");
}

function clearConflict() {
  conflict = null;
  by("mergeBody").hidden = true;
  by("conflictDetails").textContent = t("Select a conflicted file in Local Changes, then choose Resolve Conflicts.");
}

function renderConflict(data) {
  conflict = data;
  const c = data.conflict;
  const types = [c.text && t("Text conflict"), c.property && t("Property conflict"), c.tree && t("Tree conflict")].filter(Boolean).join(t(", "));
  by("conflictDetails").textContent = [data.path, types, c.operation, c.action, c.reason].filter(Boolean).join(" · ");
  for (const name of ["base", "mine", "incoming"]) by(name).textContent = data[name] ?? t("Text is unavailable for this version");
  by("mergeSides").hidden = !c.text;
  by("mergeText").value = data.working ?? "";
  by("editMerge").hidden = !c.text || data.working == null || c.tree || c.property;
  by("saveMerge").hidden = by("editMerge").hidden;
  by("takeMine").hidden = by("editMerge").hidden || data.mine == null;
  by("takeIncoming").hidden = by("editMerge").hidden || data.incoming == null;
  by("resolveMine").hidden = !c.text || c.tree;
  by("resolveIncoming").hidden = !c.text || c.tree;
  by("mergeBody").hidden = false;
  tab("conflict");
}


const logPart = (name, part) => by(name).querySelector('[data-log="' + part + '"]');

function loadLog(name, append = false) {
  const log = logs[name];
  if (!state.scope || (append && log.next == null)) return;
  if (!append) logPart(name, "list").replaceChildren(el("div", t("Loading…"), "empty"));
  const cursor = append ? log.next : undefined;
  send(name === "incoming" ? "incoming" : "history", { cursor }, { log: name, append });
}

function logDate(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return [t("Unknown date"), ""];
  const now = new Date();
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  const day = date.toDateString() === now.toDateString() ? t("Today") :
    date.toDateString() === yesterday.toDateString() ? t("Yesterday") : date.toLocaleDateString(panelLanguage);
  return [day, day + " " + date.toLocaleTimeString(panelLanguage, { hour: "2-digit", minute: "2-digit", hour12: false })];
}

function renderLog(name) {
  const log = logs[name];
  const search = logPart(name, "search").value.trim();
  const regexp = logPart(name, "regex").checked;
  let expression;
  if (search && regexp && search.length < 128) {
    try { expression = new RegExp(search, "i"); } catch { /* The input below reports an invalid pattern. */ }
  }
  const author = logPart(name, "author").value;
  const group = logPart(name, "group").value;
  const list = logPart(name, "list");
  list.replaceChildren();
  if (regexp && search && !expression) {
    list.append(el("div", t("Invalid regular expression"), "empty"));
    return;
  }
  const matches = log.entries.filter(entry => {
    if (author && entry.author !== author) return false;
    const text = entry.revision + " " + entry.author + " " + entry.message;
    return !search || (expression ? expression.test(text.slice(0, 512)) : text.toLowerCase().includes(search.toLowerCase()));
  });
  if (!matches.length) {
    list.append(el("div", name === "incoming" && !search && !author && log.next == null
      ? t("There are no incoming changes") : t("No changes to display"), "empty"));
  }
  const groups = new Map();
  for (const entry of matches) {
    const [day, time] = logDate(entry.date);
    const groupName = group === "Date" ? day : group === "Author" ? entry.author || t("Unknown author") : "";
    if (!groups.has(groupName)) {
      const section = groupName ? el("details") : el("div");
      if (groupName) {
        section.open = true;
        const heading = treeHeading(section);
        heading.append(el("span", groupName, "day"));
        section.append(heading);
      }
      groups.set(groupName, section);
      list.append(section);
    }
    const line = el("div", undefined, "row");
    line.tabIndex = 0;
    line.dataset.active = String(entry.revision === log.selected);
    line.title = entry.message;
    line.append(el("span", "#" + entry.revision, "revision"),
      el("span", entry.message.split("\n")[0], "grow narrow"), el("span", [entry.author, time].filter(Boolean).join(", "), "meta"));
    line.onclick = () => {
      log.selected = entry.revision;
      list.querySelectorAll(".row").forEach(row => { row.dataset.active = String(row === line); });
      renderRevision(entry, by(name).querySelector(".log-paths"), log.root, true);
    };
    line.onkeydown = event => {
      if (event.key === "Enter") line.click();
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        const rows = [...list.querySelectorAll(".row")].filter(row => row.offsetParent);
        const next = rows[rows.indexOf(line) + (event.key === "ArrowDown" ? 1 : -1)];
        if (next) { event.preventDefault(); next.focus(); next.click(); }
      }
    };
    groups.get(groupName).append(line);
  }
  logPart(name, "older").disabled = log.next == null;
}

function displayLog(name, data, append) {
  const log = logs[name];
  if (!append) {
    log.selected = null;
    logPart(name, "paths").replaceChildren(el("div", t("No differences"), "empty"));
  }
  log.entries = append ? log.entries.concat(data.entries) : data.entries;
  log.root = data.historyRoot || state.info?.repository || "";
  log.next = data.next;
  log.loaded = true;
  const author = logPart(name, "author");
  const previous = author.value;
  const authors = [...new Set(log.entries.map(entry => entry.author).filter(Boolean))].sort();
  author.replaceChildren(...[t("None"), ...authors].map((name, index) => {
    const option = el("option", name);
    option.value = index === 0 ? "" : name;
    return option;
  }));
  author.value = authors.includes(previous) ? previous : "";
  renderLog(name);
}



function displayUpdate(output) {
  const lines = (output || "").trimEnd().split(/\r?\n/);
  by("updateSummary").textContent = lines.findLast(line => /^(Updated to|At|Switched to) revision \d+\./.test(line))
    || lines.at(-1) || t("Update completed");
  const headings = { A: t("Added"), D: t("Deleted"), U: t("Updated"), G: t("Merged"), C: t("Conflicted"), E: t("Obstructed"), R: t("Replaced") };
  const groups = new Map();
  for (const line of lines) {
    const update = line.match(/^([ ADUGCER])([ ADUGCER])[ B]([ CT])\s+(.+)$/);
    if (!update) continue;
    const [, content, property, tree, path] = update;
    const code = tree === "C" || property === "C" ? "C" : content === " " ? property : content;
    if (!headings[code]) continue;
    if (!groups.has(code)) groups.set(code, []);
    groups.get(code).push(path);
  }
  const pane = by("updatedPaths");
  pane.replaceChildren();
  const directories = new Set(state.changes.filter(change => change.directory).map(change => relative(change.path)));
  for (const [code, paths] of groups) {
    const group = el("details");
    group.open = true;
    const heading = treeHeading(group);
    heading.append(headings[code] + " " + paths.length);
    group.append(heading);
    for (const path of paths) {
      const line = el("div", undefined, "row");
      line.append(fileIcon(path, directories.has(path.replaceAll("\\", "/"))), path);
      group.append(line);
    }
    pane.append(group);
  }
  if (!groups.size) pane.append(el("pre", output));
  tab("updateInfo");
}
by("closeUpdateInfo").onclick = () => {
  tab("changes");
  document.querySelector('[data-tab="updateInfo"]').hidden = true;
};

function handleData(data, meta) {
  if (data.snapshot) snapshot(data.snapshot);
  if (data.handoff) handleData(data.handoff, null);
  if (data.bookmarks) by("bookmarks").replaceChildren(...data.bookmarks.map(address => {
    const option = el("option"); option.value = address; return option;
  }));


  if (data.openAction) openAction(data.openAction, data.paths);
  if (data.targetUrl && meta?.branch) {
    form("Branch or Tag", [field("url", "Source URL:", data.targetUrl, true),
      field("to", "Destination URL:", branchConfig?.branches || "", true), field("revision", "Revision:", "HEAD", true),
      note("Commit Message:")], data => send("copy", data));
  }
  if (data.branchConfig) {
    branchConfig = data.branchConfig;
    if (meta?.op === "branches") form("Configure Branches",
      [field("url", "Trunk:", branchConfig.trunk), field("to", "Branch location:", branchConfig.branches)],
      data => send("setBranches", data));
  }
  if (data.localPreview && meta?.previewTo) {
    const expected = meta.previewTo === "localDiff" ? active : commitPath;
    const dataForPreview = meta.previewTo === "commitPreview" && changeDialogMode === "shelve"
      ? { ...data.localPreview, unified: shelveUnified, editHash: undefined } : data.localPreview;
    if (expected === meta.data.path) renderPreview(by(meta.previewTo), dataForPreview);
  }
  if (data.previewEdited) {
    if (active === data.previewEdited && !by("localPane").hidden) queuePreview(active, "localDiff");
    if (commitPath === data.previewEdited && by("commitDialog").open) queuePreview(commitPath, "commitPreview");
  }
  if (data.currentRevision) {
    const { path, revision } = data.currentRevision;
    form("Current Revision", [{ ...field("revision", relative(path), String(revision)), translateLabel: false }], () => {});
  }
  if (data.entries && data.location) renderRepo(data);
  else if (data.entries && meta?.log) displayLog(meta.log, data, meta.append);
  else if (data.entries) renderHistory(data, meta);
  handleShelfResult(data, meta);
  if (data.properties) {
    renderProps(data, meta);
    if (meta?.createExternal) by("externals").click();
  }
  if (data.annotations && meta?.annotatePreview) {
    const view = previewViews.get(meta.annotatePreview);
    if (view?.data.path === data.annotatedPath) {
      view.annotations = new Map(data.annotations.map(entry => [entry.line, entry]));
      drawPreview(meta.annotatePreview);
    }
  } else if (data.annotations) {
    by("annotationHead").hidden = false;
    by("annotations").hidden = false;
    by("annotatedPath").textContent = data.annotatedPath + (data.matchesEditor ? "" : t(" · BASE annotations"));
    by("annotations").replaceChildren(...data.annotations.map(a =>
      el("div", a.line + "   r" + a.revision + "   " + a.author + "   " + a.date.slice(0, 10), "row")));
    tab("history");
  }
  if (data.clearAnnotations) {
    by("annotationHead").hidden = true;
    by("annotations").hidden = true;
  }
  if (data.conflict) renderConflict(data);
  if (data.resolved) clearConflict();
  if (data.updated) displayUpdate(data.output);
  if (data.committed || data.updated) {
    if (data.committed && by("commitDialog").open) dismissDialog("commitDialog");
    selected.clear();
    for (const log of Object.values(logs)) log.loaded = false;
    if (logs[currentTab]) loadLog(currentTab);
    redrawChanges();
  }
  if (data.patch !== undefined) {
    if (meta?.patchAction) {
      if (data.truncated || !data.patch) status(data.truncated ? t("The patch exceeds the text limit") : t("No changes to export"), true);
      else if (meta.patchAction === "copy") copyText(data.patch);
      else window.openidea.saveFile({
        name: meta.patchPath.split("/").at(-1) + ".patch", data: data.patch,
        filters: [{ name: t("Patch"), extensions: ["patch", "diff"] }]
      }).catch(error => status(String(error), true));
    } else {
      appendOutput(data.patch || t("There are no differences"));
      if (data.truncated) appendOutput(t("Diff output was truncated"));
      tab("output");
    }
  }
  if (data.output) {
    appendOutput(data.output);
    if (data.preview || data.recovered) tab("output");
  }
  if (meta && ["propertySet", "propertyDelete"].includes(meta.op)) {
    send("properties", { path: propTarget });
  }
}


function receive(raw) {
  const msg = JSON.parse(raw);
  if (msg.languageChanged) {
    requestLanguage();
    return;
  }
  if (msg.invalidated) {
    if (msg.project === state.project) scheduleRefresh();
    return;
  }
  if (msg.panel !== panel) return;
  if (msg.languageOnly) {
    if (msg.seq !== languageRequest?.serial) return;
    languageRequest = null;
    updatePanelLanguage(msg);
    return;
  }
  if (msg.seq !== request?.serial) return;
  updatePanelLanguage(msg);
  const meta = request;
  if (meta) {
    request = null;
    document.body.removeAttribute("aria-busy");
  }
  if (msg.error) {
    snapshot(msg.snapshot);
    if (meta?.previewTo) {
      const expected = meta.previewTo === "localDiff" ? active : commitPath;
      if (expected === meta.data.path) renderPreview(by(meta.previewTo), { message: msg.error });
    } else {
      if (["commit", "shelve"].includes(meta?.op) && by("commitDialog").open) {
        by("commitError").textContent = msg.error;
        by("commitDialog").dataset.error = "true";
        updateCommit();
      }
      if (meta?.op === "unshelve" && by("unshelveDialog").open) {
        by("unshelveError").textContent = msg.error;
        by("applyUnshelve").disabled = false;
      }
      if (meta?.op === "shelfDiff" && meta.data.value === "preview" && currentShelf()?.id === meta.data.peg) {
        renderPreview(by("shelfPreview"), { message: msg.error });
      }
      if (meta?.log) logPart(meta.log, "list").replaceChildren(el("div", msg.error, "empty"));
      status(msg.error, true);
      appendOutput(msg.error);
    }
  } else if (msg.data) {
    if (!meta?.previewTo) {
      const message = msg.data.updateError || msg.data.refreshError || msg.data.output?.trim().split("\n").at(-1) || "";
      status(message, Boolean(msg.data.updateError || msg.data.refreshError), Boolean(message));
    }
    handleData(msg.data, meta);
  }
  if (!request && waiting) { const next = waiting; waiting = null; next(); }
  if (invalidated) scheduleRefresh();
}

const formText = new Map();

function localizeForm() {
  if (by("dialog").hasAttribute("open")) {
    for (const [node, source] of formText) node.textContent = t(source);
  }
}

function form(title, fields, submit, verb = "OK", layout = "") {
  if (document.querySelector("dialog[open]")) return;
  formText.clear();
  formText.set(by("dialogOk"), verb);
  formText.set(by("dialogTitle"), title);
  by("dialog").dataset.layout = layout;
  by("fields").replaceChildren();
  const inputs = {};
  for (const f of fields) {
    const label = el("label");
    const textNode = document.createTextNode(f.label);
    label.append(textNode);
    if (f.translateLabel !== false) formText.set(textNode, f.label);
    const input = el(f.type === "textarea" ? "textarea" : f.options ? "select" : "input");
    if (f.type === "checkbox") {
      input.type = "checkbox";
      input.checked = Boolean(f.value);
      label.classList.add("checkbox");
      label.prepend(input);
    }
    if (f.options) for (const [value, source] of f.options) {
      const option = el("option");
      option.value = value;
      formText.set(option, source);
      input.append(option);
    }
    input.required = Boolean(f.required);
    input.value = f.value ?? "";
    inputs[f.name] = input;
    if (f.type !== "checkbox") label.append(input);
    by("fields").append(label);
  }
  by("dialogOk").onclick = () => {
    if (Object.values(inputs).some(input => !input.reportValidity())) return;
    dismissDialog("dialog");
    submit(Object.fromEntries(Object.entries(inputs).map(([key, input]) => [key, input.type === "checkbox" ? input.checked : input.value])));
  };
  showPanelDialog("dialog");
  localizeForm();
  (Object.values(inputs).find(input => !input.disabled) || by("dialogOk")).focus();
  return inputs;
}

function field(name, label, value = "", required = false) { return { name, label, value, required }; }
function note(label = "Commit message", required = true) { return { ...field("note", label, "", required), type: "textarea" }; }
function repoURL() { return by("url").value.trim() || state.info?.url || ""; }
function numericRevision() {
  const s = by("revision").value.trim();
  return /^\d+$/.test(s) && Number.isSafeInteger(Number(s)) ? Number(s) : undefined;
}
function propertyDialog(name, value, path = propTarget || target()) {
  form("Set Property", [field("name", "Property name:", name, true), { ...field("value", "Property value:", value), type: "textarea" }],
    data => { propTarget = path; send("propertySet", { ...data, path }); });
}


function closeMenus(restoreFocus = false) {
  const wasOpen = !by("menu").hidden;
  by("menu").hidden = by("submenu").hidden = true;
  if (restoreFocus && wasOpen) menuAnchor?.focus({ preventScroll: true });
}

function menuAt(x, y, actions, parent = null) {
  const menu = by(parent ? "submenu" : "menu");
  if (!parent) {
    closeMenus();
    menuAnchor = document.activeElement;
  }
  menu.replaceChildren();
  for (const item of actions) {
    if (!item) {
      const separator = el("hr");
      separator.setAttribute("role", "separator");
      menu.append(separator);
      continue;
    }
    const [label, action, options = {}] = item;
    const button = el("button");
    button.setAttribute("role", options.checked === undefined ? "menuitem" : "menuitemcheckbox");
    if (options.checked !== undefined) button.setAttribute("aria-checked", String(options.checked));
    if (options.submenu) button.setAttribute("aria-haspopup", "menu");
    button.disabled = !action && !options.submenu;
    button.append(
      options.file ? fileIcon(options.file) : options.icon ? svnIcon(options.icon) : el("span", options.checked ? "✓" : ""),
      el("span", label, "menu-label"),
      options.submenu ? svnIcon("tree-right") : el("span", options.shortcut || "", "shortcut")
    );
    button.openSubmenu = options.submenu && (() => {
      const anchor = button.getBoundingClientRect();
      menuAt(anchor.right - 3, anchor.top, options.submenu, button);
    });
    button.onclick = () => {
      if (button.openSubmenu) button.openSubmenu();
      else {
        closeMenus(true);
        action();
      }
    };
    button.onpointerenter = () => {
      if (button.openSubmenu) button.openSubmenu();
      else if (!parent) by("submenu").hidden = true;
    };
    menu.append(button);
  }
  menu.parentItem = parent;
  menu.hidden = false;
  if (parent && x + menu.offsetWidth > innerWidth) x = parent.getBoundingClientRect().left - menu.offsetWidth + 3;
  menu.style.left = Math.max(0, Math.min(x, innerWidth - menu.offsetWidth - 2)) + "px";
  menu.style.top = Math.max(0, Math.min(y, innerHeight - menu.offsetHeight - 2)) + "px";
  menu.querySelector("button:not(:disabled)")?.focus();
}

function buttonMenu(button, actions) {
  const rect = button.getBoundingClientRect();
  menuAt(rect.left, rect.bottom, actions);
}

async function readClipboard(consume) {
  try {
    const value = await navigator.clipboard.readText();
    consume(value);
  } catch {
    form("Compare with Clipboard", [note("Paste clipboard text:", false)], ({ note }) => consume(note), "Compare");
  }
}

async function copyText(value) {
  try {
    await navigator.clipboard.writeText(value);
  } catch {
    const inputs = form("Copy to Clipboard", [{ ...note("Copy the selected text:", false), value }], () => {});
    inputs?.note.select();
  }
}

function openRepositories() {
  tab("browser");
  if (!repoLocation && repoURL()) by("browse").click();
  else by("url").focus();
}


function showUpdate(paths = [], url = "") {
  const one = paths.length === 1 ? paths[0] : "";
  const title = one ? "Update File" : paths.length > 1 ? "Update Files" : "Update Project";
  const checkbox = (name, label) => ({ name, label, type: "checkbox" });
  const branches = [...new Set([branchConfig?.trunk, branchConfig?.branches].filter(Boolean))];
  const fields = [
    checkbox("switchUrl", "Update/Switch to specific URL:"),
    { name: "branch", label: "Use branch:", options: [["", ""], ...branches.map(url => [url, url])] },
    field("url", "URL:", url || state.info?.url, true),
    checkbox("useRevision", "Update/Switch to specific revision:"),
    field("revision", "Revision:", "HEAD", true),
    { name: "value", label: "Depth:", value: "", options: [["", "working copy"], ["infinity", "fully recursive"],
      ["immediates", "immediate children"], ["files", "files only"], ["empty", "this item only"]] },
    checkbox("force", "Force update"),
    checkbox("ignoreExternals", "Ignore Externals")
  ];
  const inputs = form(title, fields, ({ switchUrl, useRevision, branch, ...data }) => send("update", {
    ...data, paths, url: switchUrl ? data.url : "", revision: useRevision ? data.revision : "HEAD"
  }), undefined, "update");
  if (!inputs) return;
  const toggle = () => {
    inputs.url.disabled = inputs.branch.disabled = !inputs.switchUrl.checked;
    inputs.revision.disabled = !inputs.useRevision.checked;
  };
  inputs.branch.onchange = () => { if (inputs.branch.value) inputs.url.value = inputs.branch.value; };
  inputs.switchUrl.onchange = inputs.useRevision.onchange = toggle;
  toggle();
}

function openAction(action, paths = []) {
  const path = paths[0] || target();
  switch (action) {
    case "commit": openCommit(paths); break;
    case "repositories": openRepositories(); break;
    case "update": showUpdate(); break;
    case "update-file": if (paths.length) showUpdate(paths); break;
    case "lock": form("Lock File", [note("Comment:", false)], data => send("lock", { ...data, paths })); break;
    case "ignore": form("Ignore", [], () => send("ignore", { paths })); break;
    case "set-property": propertyDialog("", "", path); break;
    case "compare-latest": send("compareLocal", { path, revision: "HEAD" }); break;
    case "compare-revision":
      form("Compare with Revision", [field("revision", "Revision:", "HEAD", true)], data => send("compareLocal", { ...data, path }));
      break;
    case "branch-tag":
    case "relocate": {
      by("repoAction").value = action === "branch-tag" ? "copy" : action;
      if (action === "branch-tag") {
        send("targetUrl", { path }, { branch: true });
      } else by("runRepoAction").click();
      break;
    }
  }
}

function changeMenu(x, y, change) {
  const path = change.path;
  const paths = [path];
  const run = op => () => send(op, { path, paths });
  const action = op => () => openAction(op, paths);
  const unversioned = change.item === "unversioned";
  const ignored = change.item === "ignored";
  const tracked = !unversioned && !ignored && change.item !== "external";
  const exists = !["deleted", "missing"].includes(change.item);
  const hasConflict = badge(change) === "C";
  const diff = () => openDiff(change);
  const changelist = (title, value) => () => form(title,
    [field("name", "Name:", value, true)], data => send("changelist", { ...data, paths }));
  const patch = mode => () => send("createPatch", { paths }, { patchAction: mode, patchPath: path });
  const urls = (title, label, submit, more = []) => () => form(title,
    [field("url", label, "", true), field("revision", "Revision:", "HEAD", true), ...more], submit);

  const subversion = [
    [t("Create External…"), tracked && change.directory && (() => send("properties", { path }, { createExternal: true }))],
    null,
    [t("Commit File…"), canCommit(change) && action("commit")],
    [t("Update File…"), tracked && action("update-file")],
    [t("Integrate File…"), tracked && exists && urls("Integrate File", "Source file URL:", ({ mode, ...data }) =>
      send("merge", { ...data, path, preview: mode !== "apply" }), [
      field("value", "Revisions (optional):", ""),
      { name: "mode", label: "Mode:", value: "preview", options: [["preview", "Preview"], ["apply", "Apply"]] }
    ])],
    null,
    [t("Edit Properties"), tracked && run("properties")],
    [t("Set Property…"), tracked && action("set-property")],
    [t("Revert…"), tracked && run("revert"), { icon: "revert", shortcut: "Ctrl+Alt+Z" }],
    [t("Resolve Text Conflict…"), hasConflict && run("conflict")],
    [t("Mark Resolved…"), hasConflict && (() => send("resolve", { path, value: "working" }))],
    [t("Cleanup"), tracked && run("cleanup")],
    null,
    [t("Show Current Revision"), tracked && run("currentRevision")],
    [t("Compare with Same Repository Version"), tracked && diff, { icon: "diff", shortcut: "Ctrl+D" }],
    [t("Compare with Latest Repository Version"), tracked && action("compare-latest")],
    [t("Compare with Revision…"), tracked && action("compare-revision")],
    [t("Compare with Branch…"), tracked && exists && !change.directory &&
      urls("Compare with Branch", "Corresponding file URL:", data => send("compareLocal", { ...data, path }))],
    [t("Show History"), tracked && run("history"), { icon: "history" }],
    [t("Annotate"), tracked && exists && !change.directory && run("annotate")],
    [t("Branch or Tag…"), tracked && action("branch-tag")],
    null,
    [t("Lock…"), tracked && exists && !change.directory && !change.locked && action("lock")],
    [t("Unlock"), change.locked && run("unlock")],
    [t("Relocate…"), tracked && (() => { by("repoAction").value = "relocate"; by("runRepoAction").click(); })],
    [t("Browse Changes…"), tracked && run("history")],
    ...(change.group ? [[t("Remove from Changelist"), run("removeChangelist")]] : [])
  ];

  menuAt(x, y, [
    [t("Commit File…"), canCommit(change) && action("commit")],
    [t("Revert…"), tracked && run("revert"), { icon: "revert", shortcut: "Ctrl+Alt+Z" }],
    [t("Move to Another Changelist…"), tracked && !change.directory && changelist("Move to Another Changelist", change.group), { shortcut: "Alt+Shift+M" }],
    [t("Show Diff"), diff, { icon: "diff", shortcut: "Ctrl+D" }],
    [t("Show Diff in a New Tab"), diff, { icon: "diff" }],
    [t("Jump to Source"), exists && !change.directory && run("openSource"), { icon: "edit", shortcut: "F4" }],
    [t("Copy Path/Reference…"), null, { submenu: [
      [t("Absolute Path"), () => copyText(path)],
      [t("Path From Content Root"), () => copyText(relative(path))],
      [t("File Name"), () => copyText(path.split("/").at(-1))]
    ] }],
    null,
    [t("Delete…"), tracked && exists && run("delete"), { shortcut: "Delete" }],
    ...(unversioned ? [
      [t("Add to VCS"), run("add"), { shortcut: "Ctrl+Alt+A" }],
      [t("Ignore"), null, { submenu: [[path.split("/").at(-1), action("ignore")]] }]
    ] : ignored ? [[t("Remove from Ignore"), run("unignore")]] : []),
    null,
    ...(tracked && !change.directory ? [[t("New Changelist…"), changelist("New Changelist", ""), { icon: "add" }]] : []),
    [t("Create Patch from Local Changes…"), tracked && patch("save"), { icon: "patch" }],
    [t("Copy as Patch to Clipboard"), tracked && patch("copy")],
    [t("Shelve Changes…"), canShelve(change) && (() => openCommit(paths, "shelve")), { icon: "shelve" }],
    null,
    [t("Refresh"), () => send("refresh"), { icon: "refresh" }],
    null,
    ["Subversion", null, { submenu: subversion }]
  ]);
}

document.querySelectorAll("nav button[data-tab]").forEach(button => { button.onclick = () => tab(button.dataset.tab); });
for (const name of Object.keys(logs)) {
  by(name).append(by("logView").content.cloneNode(true));
  logPart(name, "refresh").onclick = () => loadLog(name);
  logPart(name, "browse").hidden = name === "incoming";
  logPart(name, "browse").onclick = openRepositories;
  logPart(name, "older").onclick = () => loadLog(name, true);
  for (const field of ["author", "group", "search", "regex"]) logPart(name, field).oninput = () => renderLog(name);
}
document.querySelectorAll("[data-icon]").forEach(node => node.prepend(svnIcon(node.dataset.icon)));
rememberStaticText();
by("more").onclick = event => buttonMenu(event.currentTarget, [
  [t("Browse Subversion Repositories"), openRepositories],
  [t("Update Project…"), state.scope && (() => showUpdate())],
  [t("Select Working Directory…"), () => send("chooseScope")],
  [t("Reset Working Directory"), () => send("resetScope")],
  [t("Authenticate…"), () => form("SVN Authentication", [field("url", "URL:", repoURL(), true)], data => send("authenticate", data))],
  [t("Console"), () => tab("output")]
]);
by("treeSettings").onclick = event => buttonMenu(event.currentTarget, [
  [flat ? t("Group by Directory") : t("Flatten Directories"), () => { flat = !flat; redrawChanges(); }],
  [showIgnored ? t("Hide Ignored Files") : t("Show Ignored Files"), () => { showIgnored = !showIgnored; redrawChanges(); }],
  [t("Filter Changes"), () => { by("filterBar").hidden = !by("filterBar").hidden; if (!by("filterBar").hidden) by("filter").focus(); }],
  [t("Expand All"), () => by("changeList").querySelectorAll("details").forEach(group => { group.open = true; })],
  [t("Collapse All"), () => by("changeList").querySelectorAll("details").forEach(group => { group.open = false; })]
]);
document.addEventListener("contextmenu", event => event.preventDefault(), true);
for (const type of ["pointerdown", "mousedown", "mouseup", "auxclick"]) {
  document.addEventListener(type, event => {
    if (event.button === 3 || event.button === 4) event.preventDefault();
  }, true);
}
document.addEventListener("pointerdown", event => { if (!event.target.closest(".action-menu")) closeMenus(); });
document.addEventListener("keydown", event => {
  if (["BrowserBack", "BrowserForward"].includes(event.key) ||
    event.altKey && !event.ctrlKey && ["ArrowLeft", "ArrowRight"].includes(event.key)) {
    event.preventDefault();
    closeMenus(true);
    return;
  }
  if (event.defaultPrevented) return;
  const menu = by("menu");
  const submenu = by("submenu");
  if (!menu.hidden) {
    const popup = submenu.hidden ? menu : submenu;
    if (event.key === "Escape" || event.key === "ArrowLeft" && popup === submenu) {
      event.preventDefault();
      if (popup === submenu) { submenu.hidden = true; submenu.parentItem?.focus(); }
      else closeMenus(true);
      return;
    }
    if (event.key === "ArrowRight" && document.activeElement?.openSubmenu) {
      event.preventDefault();
      document.activeElement.openSubmenu();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const items = [...popup.querySelectorAll("button:enabled")];
      const index = items.indexOf(document.activeElement);
      items[(index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
      return;
    }
  }
  const dialog = document.querySelector("dialog[open]");
  if (dialog && event.key === "Enter" && (event.ctrlKey || event.target.matches("input:not([type=checkbox])"))) {
    event.preventDefault();
    by(dialog.id === "commitDialog" ? "commit" : dialog.id === "unshelveDialog" ? "applyUnshelve" : "dialogOk").click();
    return;
  }
  if (dialog && (event.key === "Escape" || event.key === "Tab")) {
    if (event.key === "Escape") {
      event.preventDefault();
      dismissDialog(dialog.id);
    } else {
      const tabStops = [...dialog.querySelectorAll("button:enabled, input:enabled, select:enabled, textarea:enabled")].filter(node => node.offsetParent);
      const index = tabStops.indexOf(document.activeElement);
      if (index < 0 || (event.shiftKey ? index === 0 : index === tabStops.length - 1)) {
        event.preventDefault();
        tabStops[event.shiftKey ? tabStops.length - 1 : 0]?.focus();
      }
    }
    return;
  }
  if (!dialog && currentTab === "shelf" && event.ctrlKey && !event.altKey) {
    if (event.shiftKey && event.key.toLowerCase() === "u") { event.preventDefault(); openUnshelve(); }
    if (!event.shiftKey && event.key.toLowerCase() === "d" && currentShelf()) { event.preventDefault(); showShelfPreview(true); }
  }
  if (event.ctrlKey && !event.shiftKey && !event.altKey && state.scope && !by("dialog").open && !by("commitDialog").open) {
    const key = event.key.toLowerCase();
    if (key === "k" || key === "t") {
      event.preventDefault();
      closeMenus();
      if (key === "k") openCommit();
      else showUpdate();
      return;
    }
  }
  if (currentTab === "changes" && !by("localPane").hidden && !dialog && event.key === "F7") {
    event.preventDefault();
    moveDifference(event.shiftKey ? -1 : 1);
  }
});
initShelf();
by("shelveSelected").onclick = () => { if (active) openCommit([active], "shelve"); };
by("shelveShowDiff").onclick = () => {
  const change = state.changes.find(change => change.path === commitPath);
  if (change) openDiff(change);
};
by("dialogCancel").onclick = () => dismissDialog("dialog");
by("refresh").onclick = by("copiesRefresh").onclick = by("commitRefresh").onclick = () => send("refresh");
by("recover").onclick = () => send("recover");
by("filter").oninput = redrawChanges;
by("openCommit").onclick = () => openCommit();
by("update").onclick = () => showUpdate();
by("copiesRefresh").onclick = () => send("refresh");
by("cleanupCopy").onclick = () => send("cleanup", { paths: [] });
by("mergeFrom").onclick = () => { by("repoAction").value = "merge"; by("runRepoAction").click(); };
by("configureBranches").onclick = () => send("branches");
by("revert").onclick = () => { if (active) send("revert", { paths: [active] }); };
by("showDiff").onclick = () => openDiff(state.changes.find(change => change.path === active));
by("previewToggle").onclick = () => {
  const hidden = !by("localPane").hidden;
  by("localPane").hidden = hidden;
  by("changeSplitter").hidden = hidden;
  by("changes").dataset.noPreview = String(hidden);
  by("previewToggle").setAttribute("aria-pressed", String(!hidden));
  if (!hidden) queuePreview(active, "localDiff");
};
function movePreview(step) {
  const { visible, index } = localNavigation();
  const row = visible[index + step];
  if (row) selectPreviewFile(row);
}
function selectPreviewFile(row) {
  for (let branch = row.closest("details"); branch; branch = branch.parentElement.closest("details")) branch.open = true;
  choose(row.dataset.path);
  row.scrollIntoView({ block: "nearest" });
}
by("previousFile").onclick = () => movePreview(-1);
by("nextFile").onclick = () => movePreview(1);
by("previousDifference").onclick = () => moveDifference(-1);
by("nextDifference").onclick = () => moveDifference(1);
by("jumpToSource").onclick = () => {
  const view = previewViews.get("localDiff");
  const cursor = view?.hunks?.[view.index]?.rightStart ?? 0;
  send("openSource", { path: active, cursor });
};
by("diffFiles").onclick = event => {
  const { visible } = localNavigation();
  buttonMenu(event.currentTarget, visible.map(row => {
    const path = row.dataset.path;
    return [relative(path), () => selectPreviewFile(row), { file: path }];
  }));
};
by("foldDiff").onclick = () => toggleDiffOption("fold");
by("sideBySideDiff").onclick = () => { if (diffOptions.unified) toggleDiffOption("unified"); };
by("unifiedDiff").onclick = () => { if (!diffOptions.unified) toggleDiffOption("unified"); };
by("diffSettings").onclick = event => buttonMenu(event.currentTarget, diffOptionActions());
const splitter = by("changeSplitter");
function resizeTree(position) {
  const width = by("changes").clientWidth;
  const size = Math.max(90, Math.min(width - 135, position));
  by("changes").style.setProperty("--tree-width", size + "px");
}
splitter.onpointerdown = event => splitter.setPointerCapture(event.pointerId);
splitter.onpointermove = event => { if (splitter.hasPointerCapture(event.pointerId)) resizeTree(event.clientX - 32); };
splitter.onkeydown = event => {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  event.preventDefault();
  resizeTree(splitter.offsetLeft - 32 + (event.key === "ArrowRight" ? 20 : -20));
};
by("commitGroup").onchange = () => selectCommitGroup();
by("note").oninput = updateCommit;
by("cancelCommit").onclick = by("closeCommit").onclick = () => dismissDialog("commitDialog");
by("commit").onclick = () => {
  if (by("commit").disabled) return;
  by("commit").disabled = true;
  by("commitError").textContent = "";
  if (changeDialogMode === "shelve") send("shelve", { paths: [...selected], name: by("note").value });
  else send("commit", { paths: [...selected], note: by("note").value,
    autoUpdate: by("autoUpdate").checked, keepLocked: by("keepLocked").checked });
};
by("browse").onclick = () => send("browse", { url: repoURL(), revision: by("revision").value });
by("url").onkeydown = event => { if (event.key === "Enter") by("browse").click(); };
by("repoHistory").onclick = () => send("history", { url: repoURL(), peg: numericRevision() });
by("bookmark").onclick = () => send("bookmark", { url: repoURL() });
by("unbookmark").onclick = () => send("unbookmark", { url: repoURL() });
by("up").onclick = () => {
  if (!repoLocation || repoLocation.replace(/\/$/, "") === repoRoot.replace(/\/$/, "")) return;
  const url = repoLocation.replace(/\/$/, "").replace(/\/[^/]+$/, "");
  if (url.startsWith(repoRoot.replace(/\/$/, ""))) send("browse", { url, revision: by("revision").value });
};
by("checkout").onclick = () => {
  let name = "";
  try { name = decodeURIComponent(new URL(repoURL()).pathname.replace(/\/$/, "").split("/").at(-1)); } catch {}
  form("Check Out SVN Project", [field("url", "Repository directory", repoURL(), true), field("revision", "Destination revision", by("revision").value || "HEAD"),
    field("name", "New directory name", name, true),
    { name: "value", label: "Checkout depth", value: "infinity", options: [["infinity", "Fully recursive"], ["immediates", "Immediate children"], ["files", "Files only"], ["empty", "Directory only"]] }],
    data => send("checkout", data));
};
by("runRepoAction").onclick = () => {
  const op = by("repoAction").value;
  const rev = by("revision").value || "HEAD";
  const from = field("url", "Source URL:", op === "merge" ? branchConfig?.branches || repoURL() : repoURL(), true);
  const to = field("to", "Destination URL", "", true);
  const revision = field("revision", "Source revision", rev);
  const path = field("path", "Local directory in the project (blank for the project root)");
  const url = field("url", "Destination URL", repoURL(), true);
  const dialog = (title, fields, extra = {}) => form(title, fields, data => send(op, { ...data, ...extra }));
  switch (op) {
    case "compare": dialog("Compare SVN URLs or Revisions", [from, revision, { ...to, value: state.info?.url || repoURL() }, field("toRevision", "Destination revision", "HEAD")]); break;
    case "copy": dialog("Create SVN Branch or Tag", [from, revision, to, note()]); break;
    case "switch": dialog("Switch Working Copy Branch", [url, field("revision", "Destination revision", rev)]); break;
    case "merge": form("Merge Branch into Current Scope", [from, revision, field("value", "Comma-separated revisions (negative to reverse, blank for automatic)"),
      { name: "mode", label: "Mode", value: "preview", options: [["preview", "Preview"], ["apply", "Apply Merge"]] }],
      ({ mode, ...data }) => send(op, { ...data, preview: mode === "preview" })); break;
    case "import": dialog("Import Local Directory", [path, url, note()]); break;
    case "share": form("Share Local Project", [path, url, note("Commit message for creating the repository directory", false),
      { name: "mode", label: "Sharing mode", value: "create", options: [["create", "Create a repository directory"], ["resume", "Use an existing empty directory"]] }],
      ({ mode, ...data }) => send(op, { ...data, preview: mode === "resume" })); break;
    case "relocate": dialog("Relocate SVN Repository", [field("url", "New repository root URL", "", true)]); break;
  }
};
by("older").onclick = () => {
  if (history.target && history.next != null) send("history", { ...history.target, cursor: history.next }, { append: true });
};
by("hideAnnotations").onclick = () => send("clearAnnotations");
by("propReload").onclick = () => send("properties", { path: propTarget || target() });
by("propDiff").onclick = () => send("propertyDiff", { path: propTarget || target() });
by("propNew").onclick = () => propertyDialog("", "");
by("externals").onclick = () => propertyDialog("svn:externals", props.find(p => p.name === "svn:externals" && !p.binary)?.value || "");
by("takeMine").onclick = () => { if (conflict?.mine != null) by("mergeText").value = conflict.mine; };
by("takeIncoming").onclick = () => { if (conflict?.incoming != null) by("mergeText").value = conflict.incoming; };
const resolve = (choice, merged) => {
  if (conflict) send("resolve", { path: conflict.path, hash: conflict.hash || "", value: choice, merged });
};
by("saveMerge").onclick = () => resolve("working", by("mergeText").value);
by("resolveWorking").onclick = () => resolve("working");
by("resolveMine").onclick = () => resolve("mine-full");
by("resolveIncoming").onclick = () => resolve("theirs-full");
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "hidden") {
    requestLanguage();
    scheduleRefresh();
  }
});
window.addEventListener("focus", requestLanguage);
window.openidea.onMessage(raw => {
  try { receive(raw); } catch (error) { status(t("Could not read the SVN panel response · ") + error.message, true); }
});
// OpenIDEA 0.1.51 signals plugin preference changes, but not IDE locale changes.
// The IDE keeps previously opened tool windows mounted even when hidden.
new IntersectionObserver(([entry]) => {
  panelVisible = entry.isIntersecting;
  if (panelVisible) requestLanguage();
}).observe(document.documentElement);
setInterval(requestLanguage, 1000);
requestLanguage();
send("refresh", {}, { quiet: true });
