(function () {
  var root = document.documentElement;
  var stored = localStorage.getItem("theme");
  var palettes = [
    "theme-zinc-emerald",
    "theme-slate-indigo",
    "theme-slate-sky",
    "theme-stone-amber",
    "theme-neon-cyber",
    "theme-neon-matrix",
    "theme-retro-nes",
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

  // Global toggle function for dark/light
  window.toggleTheme = function () {
    var isDark = root.classList.contains("dark");
    applyTheme(!isDark);
    localStorage.setItem("theme", isDark ? "light" : "dark");
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
  };

  // Sync palette selectors on page load
  document.addEventListener("DOMContentLoaded", function () {
    var selectors = document.querySelectorAll(".palette-selector");
    for (var i = 0; i < selectors.length; i++) {
      selectors[i].value = storedPalette;
    }
  });
})();
