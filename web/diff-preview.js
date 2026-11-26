"use strict";

function alignLines(left, right) {
  const a = left?.replaceAll("\r\n", "\n").split("\n") ?? [];
  const b = right?.replaceAll("\r\n", "\n").split("\n") ?? [];
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let end = 0;
  while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) end++;
  const n = a.length - start - end;
  const m = b.length - start - end;
  const pairs = [];
  const hunks = [];
  for (let i = 0; i < start; i++) pairs.push([i, i, false]);
  const changed = (i, j, nextI, nextJ) => {
    if (i === nextI && j === nextJ) return;
    const index = hunks.length;
    hunks.push({
      row: pairs.length, leftStart: start + i, leftLength: nextI - i,
      rightStart: start + j, rightLength: nextJ - j,
    });
    for (let offset = 0; offset < Math.max(nextI - i, nextJ - j); offset++) {
      pairs.push([i + offset < nextI ? start + i + offset : null, j + offset < nextJ ? start + j + offset : null, true, index]);
    }
  };
  if (n && m && n * m <= 300000) {
    const width = m + 1;
    const table = new Uint32Array((n + 1) * width);
    for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) {
      table[i * width + j] = a[start + i] === b[start + j]
        ? 1 + table[(i + 1) * width + j + 1]
        : Math.max(table[(i + 1) * width + j], table[i * width + j + 1]);
    }
    let i = 0, j = 0, blockI = 0, blockJ = 0;
    while (i < n && j < m) {
      if (a[start + i] === b[start + j]) {
        changed(blockI, blockJ, i, j);
        pairs.push([start + i, start + j, false]);
        blockI = ++i;
        blockJ = ++j;
      } else if (table[(i + 1) * width + j] >= table[i * width + j + 1]) i++;
      else j++;
    }
    changed(blockI, blockJ, n, m);
  } else changed(0, 0, n, m);
  for (let i = end; i > 0; i--) pairs.push([a.length - i, b.length - i, false]);
  return { a, b, pairs, hunks };
}

const previewViews = new Map();
const diffOptions = { fold: true, align: false, sync: true, unified: false };

function renderPreview(pane, data) {
  const view = { data, index: 0, expanded: new Set(), annotations: null };
  if (!data.message) Object.assign(view, alignLines(data.left, data.right));
  previewViews.set(pane.id, view);
  drawPreview(pane.id);
}

function previewRows(view) {
  const center = view.hunks[view.index]?.row ?? 0;
  const start = Math.max(0, center - 300);
  const end = Math.min(view.pairs.length, start + 1800);
  const visible = [];
  if (start) visible.push({ caption: "…", start: -1 });
  for (let i = start; i < end;) {
    if (view.pairs[i][2] || !diffOptions.fold) {
      visible.push(view.pairs[i++]);
      continue;
    }
    let to = i + 1;
    while (to < end && !view.pairs[to][2]) to++;
    const head = i === 0 ? i : Math.min(i + 3, to);
    const tail = to === view.pairs.length ? to : Math.max(head, to - 3);
    if (tail - head > 2 && !view.expanded.has(head)) {
      visible.push(...view.pairs.slice(i, head), { caption: (tail - head) + " unchanged lines", start: head },
        ...view.pairs.slice(tail, to));
    } else visible.push(...view.pairs.slice(i, to));
    i = to;
  }
  if (end < view.pairs.length) visible.push({ caption: "…", start: -1 });
  return visible;
}

function drawPreview(id) {
  const pane = by(id);
  const view = previewViews.get(id);
  if (!view) return;
  const { data } = view;
  pane.replaceChildren();
  if (data.message) {
    pane.append(el("div", data.message, "empty"));
    updateDiffToolbar();
    return;
  }
  const rows = previewRows(view);
  const unified = (data.unified ?? diffOptions.unified) && data.left != null && data.right != null;
  const sides = unified ? ["unified"] : ["left", "right"].filter(side => data[side] != null);
  const scrollers = [];
  for (const side of sides) {
    const panel = el("div", undefined, "preview-side");
    panel.dataset.added = String(data.left == null);
    panel.dataset.deleted = String(data.right == null);
    const header = el("header");
    if (side !== "right" && data.path) {
      header.id = id === "localDiff" ? "previewName" : "";
      if (data.left != null) header.append(svnIcon("lock"));
      header.append([data.revision, relative(data.path)].filter(Boolean).join("  "));
    } else header.textContent = data[side + "Label"] || t("Current version");
    header.title = data.path || "";
    const content = el("div", undefined, "file-lines");
    content.tabIndex = 0;
    content.dataset.side = side;
    const fragment = document.createDocumentFragment();
    for (const pair of rows) {
      if (!Array.isArray(pair)) {
        if (pair.start < 0) fragment.append(el("div", pair.caption, "file-line"));
        else {
          const fold = el("button", "···  " + pair.caption, "file-line folded");
          fold.onclick = () => { view.expanded.add(pair.start); drawPreview(id); };
          fragment.append(fold);
        }
        continue;
      }
      const [left, right, changed, hunk] = pair;
      const appendLine = (column, number, kind = "") => {
        const line = el("div", undefined, "file-line");
        line.dataset.changed = String(changed && !kind);
        if (kind) line.dataset[kind] = "true";
        if (changed) line.dataset.hunk = hunk;
        line.dataset.side = column;
        const lines = column === "left" ? view.a : view.b;
        const label = number == null ? "" : String(number + 1);
        line.append(el("span", label, "line-number"));
        if (view.annotations && column === "right") {
          const annotation = !changed && left != null ? view.annotations.get(left + 1) : null;
          const author = el("span", annotation ? "r" + annotation.revision + " " + annotation.author : "", "line-author");
          if (annotation) author.title = annotation.date;
          line.append(author);
        }
        line.append(el("span", number == null ? "" : lines[number], "line-text"));
        fragment.append(line);
      };
      if (unified) {
        if (changed && left != null) appendLine("left", left, "removed");
        if (right != null) appendLine("right", right, changed ? "inserted" : "");
      } else {
        const number = side === "left" ? left : right;
        if (number != null || diffOptions.align) appendLine(side, number);
      }
    }
    content.append(fragment);
    content.oncontextmenu = event => {
      const line = event.target.closest(".file-line");
      const column = line?.dataset.side || (side === "unified" ? "right" : side);
      diffContextMenu(event, id, column, line?.dataset.hunk);
    };
    content.onclick = event => {
      const hunk = event.target.closest("[data-hunk]")?.dataset.hunk;
      view.cursorHunk = hunk;
      if (hunk !== undefined) {
        view.index = Number(hunk);
        updateDiffToolbar();
      }
    };
    content.onkeydown = event => {
      const contextKey = event.key === "ContextMenu" || event.shiftKey && event.key === "F10";
      if (contextKey) {
        const anchor = content.getBoundingClientRect();
        diffContextMenu(event, id, side === "unified" ? "right" : side, view.hunks.length ? view.index : undefined,
          anchor.left + 20, anchor.top + 20);
      } else if (event.ctrlKey && event.altKey && event.key.toLowerCase() === "r" && view.cursorHunk !== undefined) {
        event.preventDefault();
        editDifference(view, view.cursorHunk, false);
      }
    };
    scrollers.push(content);
    panel.append(header, content);
    if (side === "unified" && data.readOnly) {
      const shelved = el("header", data.rightLabel || t("Shelved Version"));
      shelved.prepend(svnIcon("lock"));
      panel.insertBefore(shelved, content);
    }
    pane.append(panel);
  }
  if (scrollers.length === 2) for (const [i, source] of scrollers.entries()) {
    source.onscroll = () => {
      const peer = scrollers[1 - i];
      if (diffOptions.sync && peer.scrollTop !== source.scrollTop) {
        const top = Math.min(source.scrollTop, peer.scrollHeight - peer.clientHeight);
        if (Math.abs(peer.scrollTop - top) > 1) peer.scrollTop = top;
      }
    };
  }
  focusDifference(id, view.index);
  updateDiffToolbar();
}

function focusDifference(id, index) {
  for (const scroller of by(id).querySelectorAll(".file-lines")) {
    const row = scroller.querySelector('[data-hunk="' + index + '"]');
    if (row) scroller.scrollTop = row.offsetTop - scroller.offsetTop;
  }
}

function editDifference(view, index, append) {
  const hunk = view.hunks?.[index];
  if (!view.data.editHash || !hunk || (append && (!hunk.leftLength || !hunk.rightLength))) return;
  send("editDifference", { path: view.data.path, hash: view.data.editHash, hunk: { ...hunk, append } });
}

function diffOptionActions() {
  return [
    [t("Collapse Unchanged Fragments"), () => toggleDiffOption("fold"), { checked: diffOptions.fold }],
    [t("Align Changes In Side by Side Diff"), !diffOptions.unified && (() => toggleDiffOption("align")), { checked: diffOptions.align }],
    [t("Synchronize Scrolling"), !diffOptions.unified && (() => toggleDiffOption("sync")), { checked: diffOptions.sync }]
  ];
}

function toggleDiffOption(key) {
  diffOptions[key] = !diffOptions[key];
  for (const [id, view] of previewViews) {
    if (key === "fold") view.expanded.clear();
    if (!by(id).closest("dialog:not([open])")) drawPreview(id);
  }
  by("foldDiff").setAttribute("aria-pressed", String(diffOptions.fold));
  by("unifiedDiff").setAttribute("aria-pressed", String(diffOptions.unified));
  by("sideBySideDiff").setAttribute("aria-pressed", String(!diffOptions.unified));
}

function diffContextMenu(event, id, side, hunkIndex, x = event.clientX, y = event.clientY) {
  event.preventDefault();
  const view = previewViews.get(id);
  if (!view?.pairs || view.data.readOnly) return;
  const data = view.data;
  const hunk = view.hunks[hunkIndex];
  const editable = Boolean(hunk && data.editHash);
  if (hunk) {
    view.index = Number(hunkIndex);
    updateDiffToolbar();
  }
  view.cursorHunk = hunkIndex;
  const compare = () => readClipboard(value => send("compareClipboard", { path: data.path, name: side, value }));
  menuAt(x, y, [
    [t("Revert"), editable && (() => editDifference(view, hunkIndex, false)), { icon: "apply-change", shortcut: "Ctrl+Alt+R" }],
    [t("Append"), editable && hunk.leftLength && hunk.rightLength && (() => editDifference(view, hunkIndex, true)), { icon: "append-change" }],
    null,
    [t("Compare with Clipboard"), data.path && compare, { icon: "clipboard-diff" }],
    ...(side === "right" && data.left != null && data.right != null ? [
      [t("Annotate"), () => {
        if (view.annotations) { view.annotations = null; drawPreview(id); }
        else send("annotate", { path: data.path }, { annotatePreview: id });
      }, { checked: Boolean(view.annotations) }]
    ] : []),
    null,
    ...diffOptionActions()
  ]);
}

function updateDiffToolbar() {
  const view = previewViews.get("localDiff");
  const count = view?.hunks?.length || 0;
  by("differenceCount").textContent = view?.hunks ? count + (count === 1 ? t(" difference") : t(" differences")) : "";
  by("previousDifference").disabled = !(view?.index > 0) && by("previousFile").disabled;
  by("nextDifference").disabled = !(view?.index < count - 1) && by("nextFile").disabled;
  const shelf = previewViews.get("shelfPreview");
  by("shelfPreviousDifference").disabled = !(shelf?.index > 0);
  by("shelfNextDifference").disabled = !(shelf?.index < (shelf?.hunks?.length || 0) - 1);
  updateShelveToolbar();
}

function moveDifference(step) {
  const view = previewViews.get("localDiff");
  const next = view?.index + step;
  if (!view?.hunks || next < 0 || next >= view.hunks.length) return movePreview(step);
  if (by("localDiff").querySelector('[data-hunk="' + next + '"]')) {
    view.index = next;
    view.cursorHunk = next;
    focusDifference("localDiff", next);
    updateDiffToolbar();
  } else {
    view.index = next;
    view.cursorHunk = next;
    drawPreview("localDiff");
  }
}
