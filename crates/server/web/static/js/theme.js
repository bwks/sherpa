(function () {
  var root = document.documentElement;
  var stored = localStorage.getItem("theme");
  var palettes = [
    "theme-zinc-emerald",
    "theme-stone-amber",
    "theme-neon-cyber",
    "theme-matrix",
    "theme-retro-nes",
    "theme-pixel-quest",
    "theme-c64",
    "theme-ctp-latte",
    "theme-ctp-frappe",
    "theme-ctp-macchiato",
    "theme-ctp-mocha",
    "theme-gruvbox",
    "theme-fizzy",
    "theme-nord",
    "theme-rose-pine",
    "theme-rose-pine-moon",
    "theme-rose-pine-dawn",
    "theme-tokyo-night",
    "theme-tokyo-storm",
    "theme-dracula",
    "theme-ant-bloody",
  ];

  function applyTheme(dark) {
    if (dark) {
      root.classList.add("dark");
    } else {
      root.classList.remove("dark");
    }
  }

  function applyPalette(name) {
    for (var i = 0; i < palettes.length; i++) {
      root.classList.remove(palettes[i]);
    }
    if (name && name !== "theme-zinc-emerald") {
      root.classList.add(name);
    }
  }

  // Apply dark/light immediately to prevent flash
  if (stored === "dark") {
    applyTheme(true);
  } else if (stored === "light") {
    applyTheme(false);
  } else {
    applyTheme(window.matchMedia("(prefers-color-scheme: dark)").matches);
  }

  // Apply palette immediately
  var storedPalette = localStorage.getItem("palette") || "theme-zinc-emerald";
  applyPalette(storedPalette);

  // Listen for OS theme changes (only when no explicit override)
  window
    .matchMedia("(prefers-color-scheme: dark)")
    .addEventListener("change", function (e) {
      if (!localStorage.getItem("theme")) {
        applyTheme(e.matches);
      }
    });

  // Build an SVG favicon coloured with the current accent
  function updateFavicon() {
    var style = getComputedStyle(root);
    var accent = style.getPropertyValue("--t-accent").trim();
    if (!accent) {
      return;
    }
    var svg =
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">' +
      '<rect width="32" height="32" rx="7" fill="' + accent + '"/>' +
      '<text x="16" y="23" text-anchor="middle" font-family="Arial,Helvetica,sans-serif" font-weight="bold" font-size="20" fill="#fff">S</text>' +
      '</svg>';
    var link = document.querySelector('link[rel="icon"]');
    if (link) {
      link.href = "data:image/svg+xml," + encodeURIComponent(svg);
    }
  }

  // Global toggle function for dark/light
  window.toggleTheme = function () {
    var isDark = root.classList.contains("dark");
    applyTheme(!isDark);
    localStorage.setItem("theme", isDark ? "light" : "dark");
    updateFavicon();
  };

  // Global palette setter
  window.setPalette = function (name) {
    applyPalette(name);
    localStorage.setItem("palette", name);
    // Update any palette selectors on the page
    var selectors = document.querySelectorAll(".palette-selector");
    for (var i = 0; i < selectors.length; i++) {
      selectors[i].value = name;
    }
    updateFavicon();
  };

  // Sync palette selectors and favicon on page load
  document.addEventListener("DOMContentLoaded", function () {
    var selectors = document.querySelectorAll(".palette-selector");
    for (var i = 0; i < selectors.length; i++) {
      selectors[i].value = storedPalette;
    }
    updateFavicon();
  });
})();
