(function(){
  var d = localStorage.getItem("theme");
  var dark = d === "dark" || (!d && window.matchMedia("(prefers-color-scheme: dark)").matches);
  if (dark) {
    document.documentElement.style.colorScheme = "dark";
    document.documentElement.classList.add("dark");
  } else {
    document.documentElement.style.colorScheme = "light";
  }
})();
