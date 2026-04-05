/**
 * Lab Create Form — dynamic node/link/bridge management and manifest assembly.
 *
 * Row markup lives in <template> elements rendered by Askama in the page.
 * This script clones those templates and wires up add/remove/submit behaviour.
 */
(function () {
  "use strict";

  var counters = { node: 0, link: 0, bridge: 0 };

  function nextId(prefix) {
    counters[prefix] = (counters[prefix] || 0) + 1;
    return prefix + "-" + counters[prefix];
  }

  // ── clone a <template> and assign a unique id ───────────────────────────
  function cloneRow(templateId, prefix) {
    var tpl = document.getElementById(templateId);
    var clone = tpl.content.firstElementChild.cloneNode(true);
    var id = nextId(prefix);
    clone.id = id;
    var removeBtn = clone.querySelector("[data-remove]");
    if (removeBtn) {
      removeBtn.setAttribute("data-remove", id);
    }
    return clone;
  }

  // ── tab switching ───────────────────────────────────────────────────────
  function switchTab(activeTab) {
    var tabs = ["build", "upload"];
    for (var i = 0; i < tabs.length; i++) {
      var tab = tabs[i];
      var btn = document.getElementById("tab-" + tab);
      var panel = document.getElementById("panel-" + tab);
      if (tab === activeTab) {
        btn.classList.add("border-accent", "text-accent");
        btn.classList.remove("border-transparent", "text-muted");
        panel.classList.remove("hidden");
      } else {
        btn.classList.remove("border-accent", "text-accent");
        btn.classList.add("border-transparent", "text-muted");
        panel.classList.add("hidden");
      }
    }
    document.getElementById("active-tab").value = activeTab;
  }

  // ── empty state visibility ──────────────────────────────────────────────
  function updateEmptyStates() {
    var linksEmpty = document.getElementById("links-empty");
    if (linksEmpty) {
      linksEmpty.classList.toggle(
        "hidden",
        document.querySelectorAll("#links-list > div").length > 0,
      );
    }
    var bridgesEmpty = document.getElementById("bridges-empty");
    if (bridgesEmpty) {
      bridgesEmpty.classList.toggle(
        "hidden",
        document.querySelectorAll("#bridges-list > div").length > 0,
      );
    }
  }

  // ── TOML builder ────────────────────────────────────────────────────────
  function escapeTomlString(s) {
    return s.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  }

  function buildToml() {
    var lines = [];
    var name = document.getElementById("lab-name").value.trim();
    if (!name) return null;
    lines.push('name = "' + escapeTomlString(name) + '"');
    lines.push("");

    var nodeRows = document.querySelectorAll("#nodes-list > div");
    if (nodeRows.length === 0) return null;

    for (var i = 0; i < nodeRows.length; i++) {
      var row = nodeRows[i];
      var nodeName = row.querySelector('[data-field="node-name"]').value.trim();
      var nodeModel = row.querySelector('[data-field="node-model"]').value;
      if (!nodeName || !nodeModel) return null;
      lines.push("[[nodes]]");
      lines.push('name = "' + escapeTomlString(nodeName) + '"');
      lines.push('model = "' + escapeTomlString(nodeModel) + '"');
      lines.push("");
    }

    var linkRows = document.querySelectorAll("#links-list > div");
    for (var i = 0; i < linkRows.length; i++) {
      var row = linkRows[i];
      var src = row.querySelector('[data-field="link-src"]').value.trim();
      var dst = row.querySelector('[data-field="link-dst"]').value.trim();
      if (!src || !dst) return null;
      lines.push("[[links]]");
      lines.push('src = "' + escapeTomlString(src) + '"');
      lines.push('dst = "' + escapeTomlString(dst) + '"');
      lines.push("");
    }

    var bridgeRows = document.querySelectorAll("#bridges-list > div");
    for (var i = 0; i < bridgeRows.length; i++) {
      var row = bridgeRows[i];
      var bname = row.querySelector('[data-field="bridge-name"]').value.trim();
      var blinks = row
        .querySelector('[data-field="bridge-links"]')
        .value.trim();
      if (!bname || !blinks) return null;
      var parts = blinks.split(",").map(function (s) {
        return s.trim();
      });
      lines.push("[[bridges]]");
      lines.push('name = "' + escapeTomlString(bname) + '"');
      lines.push(
        "links = [" +
          parts
            .map(function (p) {
              return '"' + escapeTomlString(p) + '"';
            })
            .join(", ") +
          "]",
      );
      lines.push("");
    }

    return lines.join("\n");
  }

  // ── file upload handling ────────────────────────────────────────────────
  function handleFileUpload(file) {
    var reader = new FileReader();
    reader.onload = function (e) {
      document.getElementById("manifest").value = e.target.result;
    };
    reader.readAsText(file);
  }

  // ── init ────────────────────────────────────────────────────────────────
  document.addEventListener("DOMContentLoaded", function () {
    document.getElementById("tab-build").addEventListener("click", function () {
      switchTab("build");
    });
    document
      .getElementById("tab-upload")
      .addEventListener("click", function () {
        switchTab("upload");
      });

    document
      .getElementById("add-node-btn")
      .addEventListener("click", function () {
        document
          .getElementById("nodes-list")
          .appendChild(cloneRow("tpl-node-row", "node"));
      });

    document
      .getElementById("add-link-btn")
      .addEventListener("click", function () {
        document
          .getElementById("links-list")
          .appendChild(cloneRow("tpl-link-row", "link"));
        updateEmptyStates();
      });

    document
      .getElementById("add-bridge-btn")
      .addEventListener("click", function () {
        document
          .getElementById("bridges-list")
          .appendChild(cloneRow("tpl-bridge-row", "bridge"));
        updateEmptyStates();
      });

    // Remove buttons (delegated)
    document.addEventListener("click", function (e) {
      var btn = e.target.closest("[data-remove]");
      if (btn) {
        var targetId = btn.getAttribute("data-remove");
        var el = document.getElementById(targetId);
        if (el) el.remove();
        updateEmptyStates();
      }
    });

    var fileInput = document.getElementById("toml-file");
    if (fileInput) {
      fileInput.addEventListener("change", function () {
        if (this.files.length > 0) {
          handleFileUpload(this.files[0]);
        }
      });
    }

    // Form submit — build TOML from form or use uploaded file
    document
      .getElementById("create-lab-form")
      .addEventListener("htmx:configRequest", function (e) {
        var activeTab = document.getElementById("active-tab").value;
        if (activeTab === "build") {
          var toml = buildToml();
          if (!toml) {
            e.preventDefault();
            alert(
              "Please fill in all required fields: lab name, at least one node with name and model.",
            );
            return;
          }
          e.detail.parameters["manifest"] = toml;
        }
      });

    // Seed one empty node row
    document
      .getElementById("nodes-list")
      .appendChild(cloneRow("tpl-node-row", "node"));
  });
})();
