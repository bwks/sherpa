(function () {
  var input = document.getElementById("combobox-input");
  var list = document.getElementById("combobox-list");
  var hidden = document.getElementById("model");

  if (!input || !list || !hidden) {
    return;
  }

  var items = list.querySelectorAll("li");
  var activeIndex = -1;

  function open() {
    list.classList.remove("hidden");
    filter(input.value);
  }

  function close() {
    list.classList.add("hidden");
    activeIndex = -1;
    clearHighlight();
  }

  function isOpen() {
    return !list.classList.contains("hidden");
  }

  function select(item) {
    hidden.value = item.getAttribute("data-value");
    input.value = item.textContent;
    close();
  }

  function filter(query) {
    var q = query.toLowerCase();
    var visibleCount = 0;
    activeIndex = -1;
    clearHighlight();

    for (var i = 0; i < items.length; i++) {
      var text = items[i].textContent.toLowerCase();
      var match = q === "" || text.indexOf(q) !== -1;
      items[i].classList.toggle("hidden", !match);
      if (match) {
        visibleCount++;
      }
    }
  }

  function getVisibleItems() {
    var visible = [];
    for (var i = 0; i < items.length; i++) {
      if (!items[i].classList.contains("hidden")) {
        visible.push(items[i]);
      }
    }
    return visible;
  }

  function clearHighlight() {
    for (var i = 0; i < items.length; i++) {
      items[i].classList.remove("bg-accent/15", "text-accent");
    }
  }

  function highlight(index) {
    var visible = getVisibleItems();
    clearHighlight();
    if (index >= 0 && index < visible.length) {
      activeIndex = index;
      visible[index].classList.add("bg-accent/15", "text-accent");
      visible[index].scrollIntoView({ block: "nearest" });
    }
  }

  // Open on click or focus
  input.addEventListener("focus", open);
  input.addEventListener("click", function () {
    if (!isOpen()) {
      open();
    }
  });

  // Filter as user types
  input.addEventListener("input", function () {
    if (!isOpen()) {
      open();
    }
    filter(input.value);
  });

  // Keyboard navigation
  input.addEventListener("keydown", function (e) {
    var visible = getVisibleItems();
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (!isOpen()) {
        open();
      }
      var next = activeIndex + 1;
      if (next < visible.length) {
        highlight(next);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      var prev = activeIndex - 1;
      if (prev >= 0) {
        highlight(prev);
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (activeIndex >= 0 && activeIndex < visible.length) {
        select(visible[activeIndex]);
      }
    } else if (e.key === "Escape") {
      close();
      input.blur();
    }
  });

  // Select on click
  list.addEventListener("click", function (e) {
    var target = e.target;
    if (target.tagName === "LI") {
      select(target);
    }
  });

  // Close on outside click
  document.addEventListener("click", function (e) {
    if (!input.contains(e.target) && !list.contains(e.target)) {
      close();
    }
  });
})();
