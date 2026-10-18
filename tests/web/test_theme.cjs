const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const themeScript = fs.readFileSync(
  path.join(__dirname, "../../crates/server/web/static/js/theme.js"),
  "utf8",
);

function loadTheme(preferences = {}, systemDark = false) {
  const storage = new Map(Object.entries(preferences));
  const classes = new Set();
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
      removeItem: (key) => storage.delete(key),
    },
  };
  vm.runInNewContext(themeScript, context);
  return { classes, storage, media, window: context.window };
}

test("new visitors get the system light/dark preference", () => {
  for (const dark of [false, true]) {
    const page = loadTheme({}, dark);
    assert.deepEqual([...page.classes], dark ? ["dark"] : []);
  }
});

test("a saved light/dark choice wins over the system preference", () => {
  assert.deepEqual([...loadTheme({ theme: "light" }, true).classes], []);
  assert.deepEqual([...loadTheme({ theme: "dark" }, false).classes], ["dark"]);
});

test("a palette saved by an older release is forgotten", () => {
  for (const palette of ["theme-nord", "theme-sodapop", "unknown"]) {
    const page = loadTheme({ palette, theme: "dark" });
    assert.deepEqual([...page.classes], ["dark"]);
    assert.ok(!page.storage.has("palette"));
  }
});

test("dark mode follows the system until explicitly overridden", () => {
  const page = loadTheme();
  page.media.change({ matches: true });
  assert.ok(page.classes.has("dark"));
  page.window.toggleTheme();
  assert.equal(page.storage.get("theme"), "light");
  page.media.change({ matches: true });
  assert.ok(!page.classes.has("dark"));
});

test("no template offers a palette menu", () => {
  for (const template of [
    "admin/partials/header-nav.html.jinja",
    "user/partials/header-nav.html.jinja",
    "snippets/sidebar.html.jinja",
  ]) {
    const html = fs.readFileSync(path.join(__dirname, "../../crates/server/templates", template), "utf8");
    assert.ok(!html.includes("setPalette"), template);
    assert.ok(!/<option value="theme-/.test(html), template);
  }
});

test("source and built CSS define only the Soda Pop light and dark themes", () => {
  for (const asset of ["src/input.css", "static/css/tailwind.css"]) {
    const css = fs.readFileSync(path.join(__dirname, "../../crates/server/web", asset), "utf8");
    assert.deepEqual([...css.matchAll(/\.theme-[\w-]+/g)], [], asset);
    assert.match(css, /:root\s*\{[^}]*--t-accent:\s*#00A6FB/i, asset);
    assert.match(css, /:root\.dark\s*\{[^}]*--t-accent:\s*#00A6FB/i, asset);
  }
});
