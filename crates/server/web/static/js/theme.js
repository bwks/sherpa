(function () {
  var root = document.documentElement;
  var stored = localStorage.getItem("theme");

  // Older releases offered several colour palettes; Soda Pop is now the only one.
  localStorage.removeItem("palette");

  function applyTheme(dark) {
    if (dark) {
      root.classList.add("dark");
    } else {
      root.classList.remove("dark");
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
})();
