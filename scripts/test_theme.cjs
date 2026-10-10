const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const themeScript = fs.readFileSync(
  path.join(__dirname, "../crates/server/web/static/js/theme.js"),
  "utf8",
);
const palettes = [
  "theme-ctp-frappe",
  "theme-dracula",
  "theme-gruvbox",
  "theme-nord",
  "theme-sodapop",
];

function loadTheme(preferences = {}, systemDark = false) {
  const storage = new Map(Object.entries(preferences));
  const classes = new Set();
  const selectors = [{ value: "" }, { value: "" }];
  const events = {};
  const media = {};
  const root = {
    classList: {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
      contains: (name) => classes.has(name),
      [Symbol.iterator]: () => classes[Symbol.iterator](),
    },
  };
  const context = {
    document: {
      documentElement: root,
      querySelector: () => null,
      querySelectorAll: () => selectors,
      addEventListener: (name, callback) => { events[name] = callback; },
    },
    window: {
      matchMedia: () => ({
        matches: systemDark,
        addEventListener: (name, callback) => { media[name] = callback; },
      }),
    },
    localStorage: {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
    },
    getComputedStyle: () => ({ getPropertyValue: () => "" }),
  };
  vm.runInNewContext(themeScript, context);
  events.DOMContentLoaded();
  return { classes, storage, selectors, media, window: context.window };
}

test("new visitors get Soda Pop and the system light/dark preference", () => {
  for (const dark of [false, true]) {
    const page = loadTheme({}, dark);
    assert.ok(page.classes.has("theme-sodapop"));
    assert.equal(page.classes.has("dark"), dark);
    assert.ok(page.selectors.every((selector) => selector.value === "theme-sodapop"));
  }
});

test("removed and unknown saved palettes migrate to Soda Pop", () => {
  for (const palette of ["theme-zinc-emerald", "theme-ctp-mocha", "unknown"]) {
    const page = loadTheme({ palette, theme: "dark" });
    assert.deepEqual([...page.classes].sort(), ["dark", "theme-sodapop"]);
    assert.equal(page.storage.get("palette"), "theme-sodapop");
    assert.ok(page.selectors.every((selector) => selector.value === "theme-sodapop"));
  }
});

test("all retained palettes preserve saved preferences", () => {
  for (const palette of palettes) {
    const page = loadTheme({ palette, theme: "light" }, true);
    assert.deepEqual([...page.classes], [palette]);
    assert.equal(page.storage.get("palette"), palette);
    assert.ok(page.selectors.every((selector) => selector.value === palette));
  }
});

test("palette switching replaces the active palette and updates all menus", () => {
  const page = loadTheme({ palette: "theme-nord", theme: "dark" });
  page.window.setPalette("theme-dracula");
  assert.deepEqual([...page.classes].sort(), ["dark", "theme-dracula"]);
  assert.equal(page.storage.get("palette"), "theme-dracula");
  assert.ok(page.selectors.every((selector) => selector.value === "theme-dracula"));
  page.window.setPalette("theme-ctp-latte");
  assert.deepEqual([...page.classes].sort(), ["dark", "theme-sodapop"]);
  assert.equal(page.storage.get("palette"), "theme-sodapop");
});

test("dark mode follows the system until explicitly overridden", () => {
  const page = loadTheme();
  page.media.change({ matches: true });
  assert.ok(page.classes.has("dark"));
  page.window.toggleTheme();
  assert.equal(page.storage.get("theme"), "light");
  page.media.change({ matches: true });
  assert.ok(!page.classes.has("dark"));
  assert.ok(page.classes.has("theme-sodapop"));
});

test("all theme menus offer only the five retained palettes", () => {
  for (const template of [
    "admin/partials/header-nav.html.jinja",
    "user/partials/header-nav.html.jinja",
    "snippets/sidebar.html.jinja",
  ]) {
    const html = fs.readFileSync(path.join(__dirname, "../crates/server/templates", template), "utf8");
    const values = [...html.matchAll(/<option value="(theme-[^"]+)"/g)].map((match) => match[1]);
    assert.deepEqual(values.sort(), palettes, template);
  }
});

test("source and built CSS contain only retained palettes and default to Soda Pop", () => {
  for (const asset of ["src/input.css", "static/css/tailwind.css"]) {
    const css = fs.readFileSync(path.join(__dirname, "../crates/server/web", asset), "utf8");
    const names = [...new Set([...css.matchAll(/\.theme-([\w-]+)/g)].map((match) => "theme-" + match[1]))];
    assert.deepEqual(names.sort(), palettes, asset);
    assert.match(css, /:root,\s*:root\.theme-sodapop\s*\{/);
    assert.match(css, /:root\.dark,\s*:root\.dark\.theme-sodapop\s*\{/);
    for (const palette of palettes) {
      assert.ok(css.includes(":root." + palette), asset + ": light " + palette);
      assert.ok(css.includes(":root.dark." + palette), asset + ": dark " + palette);
    }
  }
});
