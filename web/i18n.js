"use strict";

let panelLanguage = "en";
const staticText = [];

function t(source) {
  return panelLanguage === "zh-CN" && Object.hasOwn(chineseMessages, source)
    ? chineseMessages[source] : source;
}

// Capture only the original markup, including cloned log controls. Runtime
// filenames, repository data and text entered by a user never go through this.
function rememberStaticText() {
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const source = node.data.trim();
    if (!Object.hasOwn(chineseMessages, source)) continue;
    const original = node.data;
    staticText.push(() => { node.data = original.replace(source, () => t(source)); });
  }
  for (const node of document.querySelectorAll("[title], [aria-label], [placeholder]")) {
    for (const attr of ["title", "aria-label", "placeholder"]) {
      const source = node.getAttribute(attr);
      if (Object.hasOwn(chineseMessages, source)) staticText.push(() => node.setAttribute(attr, t(source)));
    }
  }
}

function applyLanguage(language) {
  if (!["en", "zh-CN"].includes(language) || language === panelLanguage) return false;
  panelLanguage = language;
  document.documentElement.lang = language;
  for (const update of staticText) update();
  return true;
}
