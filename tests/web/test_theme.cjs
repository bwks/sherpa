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
  const listeners = {};
  const context = {
    document: {
      documentElement: root,
      addEventListener: (name, callback) => { listeners[name] = callback; },
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
  // A click on an element inside a `data-theme-toggle` button, or anywhere else.
  const click = (onToggle) =>
    listeners.click({ target: { closest: (selector) => (onToggle && selector === "[data-theme-toggle]" ? {} : null) } });
  return { classes, storage, media, clickToggle: () => click(true), clickElsewhere: () => click(false) };
}

test("web_layout_008_a_new_visitor_gets_the_light_or_dark_theme_of_their_system", () => {
  for (const dark of [false, true]) {
    const page = loadTheme({}, dark);
    assert.deepEqual([...page.classes], dark ? ["dark"] : []);
  }
});

test("web_layout_009_a_saved_choice_of_light_or_dark_wins_over_the_systems", () => {
  assert.deepEqual([...loadTheme({ theme: "light" }, true).classes], []);
  assert.deepEqual([...loadTheme({ theme: "dark" }, false).classes], ["dark"]);
});

test("web_layout_010_the_theme_follows_the_system_until_the_user_toggles_it", () => {
  const page = loadTheme();
  page.media.change({ matches: true });
  assert.ok(page.classes.has("dark"));
  page.clickElsewhere();
  assert.ok(page.classes.has("dark"));
  assert.ok(!page.storage.has("theme"));
  page.clickToggle();
  assert.equal(page.storage.get("theme"), "light");
  page.media.change({ matches: true });
  assert.ok(!page.classes.has("dark"));
});
