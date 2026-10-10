(function () {
  var root = document.documentElement;
  var stored = localStorage.getItem("theme");
  var defaultPalette = "theme-sodapop";
  var palettes = [
    "theme-ctp-frappe",
    "theme-dracula",
    "theme-gruvbox",
    "theme-nord",
    "theme-sodapop",
  ];

  function applyTheme(dark) {
    if (dark) {
      root.classList.add("dark");
    } else {
      root.classList.remove("dark");
    }
  }

  function applyPalette(name) {
    name = palettes.indexOf(name) !== -1 ? name : defaultPalette;
    for (var i = 0; i < palettes.length; i++) {
      root.classList.remove(palettes[i]);
    }
    root.classList.add(name);
    return name;
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
  var storedPalette = localStorage.getItem("palette");
  var currentPalette = applyPalette(storedPalette);
  if (storedPalette && storedPalette !== currentPalette) {
    localStorage.setItem("palette", currentPalette);
  }

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
    name = applyPalette(name);
    currentPalette = name;
    localStorage.setItem("palette", name);
    // Update any palette selectors on the page
    var selectors = document.querySelectorAll(".palette-selector");
    for (var i = 0; i < selectors.length; i++) {
      selectors[i].value = name;
    }
    updateFavicon();
  };

  // Sync palette selectors and favicon on page load
  function syncSelectors() {
    var selectors = document.querySelectorAll(".palette-selector");
    for (var i = 0; i < selectors.length; i++) {
      selectors[i].value = currentPalette;
    }
    updateFavicon();
  }

  document.addEventListener("DOMContentLoaded", syncSelectors);
})();
