"use strict";

const { webcrypto } = require("node:crypto");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { runInNewContext } = require("node:vm");

const input = JSON.parse(readFileSync(0, "utf8"));
const noop = () => {};
const elements = new Map();
const element = () => ({
  dataset: {}, classList: { toggle: noop }, content: { cloneNode: noop }, querySelector: () => ({}), querySelectorAll: () => [],
  append: noop, focus: noop, select: noop, replaceChildren: noop, setAttribute: noop
});
const document = {
  body: element(), visibilityState: "hidden", createElement: element, addEventListener: noop,
  querySelector: () => null, querySelectorAll: () => [],
  getElementById(id) {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  }
};
const messages = [];
const context = {
  input, document, crypto: webcrypto, t: value => value, rememberStaticText: noop, initShelf: noop,
  activeShelfGroups: new Map(), shelfComments: new Map(), setInterval: noop,
  IntersectionObserver: class { observe() {} },
  window: { addEventListener: noop, openidea: { onMessage: noop, postMessage: raw => messages.push(JSON.parse(raw)) } }
};

const script = readFileSync(join(__dirname, "../web/panel.js"), "utf8");
// Skip DOM rendering and previews while retaining dialog selection and the
// production click handler and message sent to the plugin.
runInNewContext(script + `
  request = null;
  state = input.state;
  redrawCommit = updateCommit;
  choose = () => {};
  openAction("commit", [input.target]);
  by("note").value = input.note;
  by("note").oninput();
  by("commit").onclick();
`, context);
process.stdout.write(JSON.stringify(messages.find(message => message.op === "commit") ?? null));
