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
const diffOptions = { fold: true, align: true, sync: true, unified: false, words: true };

function renderPreview(pane, data) {
  const view = { data, index: 0, expanded: new Set(), annotations: null };
  if (!data.message) Object.assign(view, alignLines(data.left, data.right));
  previewViews.set(pane.id, view);
  drawPreview(pane.id, true);
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
      visible.push(...view.pairs.slice(i, head), { caption: (tail - head) + t(" unchanged lines"), start: head },
        ...view.pairs.slice(tail, to));
    } else visible.push(...view.pairs.slice(i, to));
    i = to;
  }
  if (end < view.pairs.length) visible.push({ caption: "…", start: -1 });
  return visible;
}

function highlightChange(text, other, node) {
  if (other == null || !diffOptions.words || text === other) {
    node.textContent = text;
    return;
  }
  let start = 0, end = 0;
  while (start < text.length && text[start] === other[start]) start++;
  while (end < Math.min(text.length, other.length) - start && text.at(-end - 1) === other.at(-end - 1)) end++;
  const last = text.length - end;
  node.append(text.slice(0, start), el("mark", text.slice(start, last)), text.slice(last));
}

function drawPreview(id, navigate = false) {
  const pane = by(id);
  const view = previewViews.get(id);
  if (!view) return;
  const { data } = view;
  const oldScroll = [...pane.querySelectorAll(".file-lines")];
  const focused = oldScroll.indexOf(document.activeElement?.closest(".file-lines"));
  pane.replaceChildren();
  if (data.message || data.left == null && data.right == null) {
    pane.append(el("div", data.message || t("Nothing to show"), "empty"));
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
    const shownPath = data.path ? relative(data.path) : "";
    const leftLabel = [t(data.leftLabel || "Base version"), data.revision ? "r" + data.revision : ""].filter(Boolean).join(" · ");
    const rightLabel = data.rightLabel ? t(data.rightLabel) : t("Current version");
    const caption = side === "left" ? leftLabel : side === "right" ? rightLabel : leftLabel + " → " + rightLabel;
    if (side === "left" || data.readOnly) header.append(svnIcon("lock"));
    const label = [caption, side === "right" && data.left != null ? "" : shownPath].filter(Boolean).join(" · ");
    header.append(el("span", label));
    header.title = [caption, data.path].filter(Boolean).join("\n");
    const content = el("div", undefined, "file-lines");
    content.tabIndex = 0;
    content.dataset.side = side;
    content.setAttribute("aria-label", label);
    content.style.setProperty("--gutter-width", Math.max(4, 2 + String(Math.max(view.a.length, view.b.length)).length) + "ch");
    const fragment = document.createDocumentFragment();
    for (const pair of rows) {
      if (!Array.isArray(pair)) {
        if (pair.start < 0) fragment.append(el("div", pair.caption, "file-line"));
        else {
          const fold = el("button", "···  " + pair.caption, "file-line folded");
          fold.title = pair.caption;
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
        if (number == null) line.dataset.placeholder = "true";
        if (changed) line.dataset.hunk = hunk;
        line.dataset.side = column;
        const lines = column === "left" ? view.a : view.b;
        const label = number == null ? "" : String(number + 1);
        line.append(el("span", label, "line-number"));
        const marker = el("span", kind === "inserted" ? "+" : kind === "removed" ? "−" : "", "line-marker");
        marker.setAttribute("aria-hidden", "true");
        line.append(marker);
        if (view.annotations && column === "right") {
          const annotation = !changed && left != null ? view.annotations.get(left + 1) : null;
          const author = el("span", annotation ? "r" + annotation.revision + " " + annotation.author : "", "line-author");
          if (annotation) author.title = author.textContent + " · " + annotation.date;
          line.append(author);
        }
        const text = el("span", undefined, "line-text");
        if (number != null) {
          const peer = column === "left" ? view.b[right] : view.a[left];
          highlightChange(lines[number], changed ? peer : null, text);
        }
        line.append(text);
        fragment.append(line);
      };
      if (unified) {
        if (changed && left != null) appendLine("left", left, "removed");
        if (right != null) appendLine("right", right, changed ? "inserted" : "");
      } else {
        const number = side === "left" ? left : right;
        const kind = left == null && right != null ? "inserted" : right == null && left != null ? "removed" : "";
        if (number != null || diffOptions.align) appendLine(side, number, number == null ? "" : kind);
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
        markDifference(id, view.index);
        updateDiffToolbar();
      }
    };
    content.onkeydown = event => {
      const contextKey = event.key === "ContextMenu" || event.shiftKey && event.key === "F10";
      if (contextKey) {
        const anchor = content.getBoundingClientRect();
        diffContextMenu(event, id, side === "unified" ? "right" : side, view.hunks.length ? view.index : undefined,
          anchor.left + 20, anchor.top + 20);
      } else if (event.key === "F7") {
        event.preventDefault();
        moveDifference(event.shiftKey ? -1 : 1, id);
      } else if (event.ctrlKey && event.altKey && event.key.toLowerCase() === "r" && !view.data.readOnly && view.cursorHunk !== undefined) {
        event.preventDefault();
        editDifference(view, view.cursorHunk, false);
      }
    };
    scrollers.push(content);
    panel.append(header, content);
    pane.append(panel);
  }
  for (const [index, scroller] of scrollers.entries()) {
    const previous = oldScroll.find(node => node.dataset.side === scroller.dataset.side) || oldScroll[index] || oldScroll[0];
    if (previous) {
      scroller.scrollTop = previous.scrollTop;
      scroller.scrollLeft = previous.scrollLeft;
    }
  }
  if (scrollers.length === 2 && diffOptions.sync && diffOptions.align) {
    const synchronized = new Map();
    for (const [i, source] of scrollers.entries()) {
      source.onscroll = () => {
        if (synchronized.get(source) === source.scrollTop) return;
        synchronized.delete(source);
        const peer = scrollers[1 - i];
        const top = Math.min(source.scrollTop, peer.scrollHeight - peer.clientHeight);
        if (Math.abs(peer.scrollTop - top) > 1) {
          peer.scrollTop = top;
          synchronized.set(peer, peer.scrollTop);
        }
      };
    }
  }
  if (focused >= 0) (scrollers[focused] || scrollers[0])?.focus({ preventScroll: true });
  if (navigate) focusDifference(id, view.index);
  else markDifference(id, view.index);
  updateDiffToolbar();
}

function markDifference(id, index) {
  for (const row of by(id).querySelectorAll("[data-hunk]")) {
    row.dataset.current = String(Number(row.dataset.hunk) === index);
  }
}

function focusDifference(id, index) {
  markDifference(id, index);
  for (const scroller of by(id).querySelectorAll(".file-lines")) {
    const row = scroller.querySelector('[data-hunk="' + index + '"]');
    if (row) {
      const offset = row.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
      if (offset < 0 || offset + row.offsetHeight > scroller.clientHeight) {
        scroller.scrollTop += offset - row.offsetHeight * 2;
      }
    }
  }
}

function editDifference(view, index, append) {
  const hunk = view.hunks?.[index];
  if (!view.data.editHash || !hunk || (append && (!hunk.leftLength || !hunk.rightLength))) return;
  send("editDifference", { path: view.data.path, hash: view.data.editHash, hunk: { ...hunk, append } });
}

function diffOptionActions(id = "localDiff") {
  const unified = previewViews.get(id)?.data.unified ?? diffOptions.unified;
  return [
    [t("Collapse Unchanged Fragments"), () => toggleDiffOption("fold"), { checked: diffOptions.fold }],
    [t("Highlight Changed Text"), () => toggleDiffOption("words"), { checked: diffOptions.words }],
    [t("Align Changes In Side by Side Diff"), !unified && (() => toggleDiffOption("align")), { checked: diffOptions.align }],
    [t("Synchronize Vertical Scrolling"), !unified && diffOptions.align && (() => toggleDiffOption("sync")), { checked: diffOptions.sync && diffOptions.align }]
  ];
}

function toggleDiffOption(key) {
  diffOptions[key] = !diffOptions[key];
  for (const [id, view] of previewViews) {
    if (key === "fold") view.expanded.clear();
    if (!by(id).closest("dialog:not([open])")) drawPreview(id, key === "unified");
  }
  by("foldDiff").setAttribute("aria-pressed", String(diffOptions.fold));
  by("unifiedDiff").setAttribute("aria-pressed", String(diffOptions.unified));
  by("sideBySideDiff").setAttribute("aria-pressed", String(!diffOptions.unified));
}

function diffContextMenu(event, id, side, hunkIndex, x = event.clientX, y = event.clientY) {
  event.preventDefault();
  const view = previewViews.get(id);
  if (!view?.pairs) return;
  const data = view.data;
  if (data.readOnly) {
    menuAt(x, y, diffOptionActions(id));
    return;
  }
  const hunk = view.hunks[hunkIndex];
  const editable = Boolean(hunk && data.editHash);
  if (hunk) {
    view.index = Number(hunkIndex);
    markDifference(id, view.index);
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
    ...diffOptionActions(id)
  ]);
}

function updateDiffToolbar() {
  const view = previewViews.get("localDiff");
  const count = view?.hunks?.length || 0;
  by("differenceCount").textContent = view?.hunks ? differencePosition(view) : "";
  by("previousDifference").disabled = !(view?.index > 0);
  by("nextDifference").disabled = !(view?.index < count - 1);
  const shelf = previewViews.get("shelfPreview");
  by("shelfDifferenceCount").textContent = shelf?.hunks ? differencePosition(shelf) : "";
  by("shelfPreviousDifference").disabled = !(shelf?.index > 0);
  by("shelfNextDifference").disabled = !(shelf?.index < (shelf?.hunks?.length || 0) - 1);
  updateShelveToolbar();
}

function differencePosition(view) {
  const count = view.hunks.length;
  return (count ? (view.index + 1) + "/" : "") + count + (count === 1 ? t(" difference") : t(" differences"));
}

function moveDifference(step, id = "localDiff") {
  const view = previewViews.get(id);
  const next = view?.index + step;
  if (!view?.hunks || next < 0 || next >= view.hunks.length) return;
  if (by(id).querySelector('[data-hunk="' + next + '"]')) {
    view.index = next;
    view.cursorHunk = next;
    focusDifference(id, next);
    updateDiffToolbar();
  } else {
    view.index = next;
    view.cursorHunk = next;
    drawPreview(id, true);
  }
}
