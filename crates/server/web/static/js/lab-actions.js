// ============================================================================
// Labs grid — icon button actions (used on /labs and /admin/labs)
// ============================================================================

async function labAction(labId, btn, action) {
    btn.disabled = true;
    btn.classList.add("animate-spin");

    try {
        var response = await fetch(`/labs/${encodeURIComponent(labId)}/${action}`, {
            method: "POST",
        });

        if (!response.ok) {
            var text = await response.text();
            showLabNotification(labId, "error", text || `${action} failed (HTTP ${response.status})`);
            return;
        }

        var data = await response.json();
        var failed = data.results.filter(function(r) { return !r.success; });

        if (failed.length === 0) {
            var verb = action === "start" ? "Started" : "Stopped";
            showLabNotification(labId, "success", `${verb} ${data.results.length} node(s)`);
        } else {
            var msgs = failed.map(function(r) { return `${r.name}: ${r.message}`; });
            showLabNotification(labId, "error", msgs.join(", "));
        }
    } catch (err) {
        showLabNotification(labId, "error", `Request failed: ${err.message}`);
    } finally {
        btn.disabled = false;
        btn.classList.remove("animate-spin");
    }
}

function labStart(labId, btn) {
    labAction(labId, btn, "start");
}

function labStop(labId, btn) {
    labAction(labId, btn, "stop");
}

function showLabNotification(labId, type, message) {
    var existing = document.getElementById(`lab-notification-${labId}`);
    if (existing) {
        existing.remove();
    }

    var row = document.getElementById(`lab-row-${labId}`);
    if (!row) return;

    var tr = document.createElement("tr");
    tr.id = `lab-notification-${labId}`;

    var td = document.createElement("td");
    td.colSpan = 5;
    td.className = "px-4 py-2";

    var div = document.createElement("div");
    div.className = type === "success"
        ? "text-sm text-success-text bg-success rounded-md px-3 py-2"
        : "text-sm text-danger-solid bg-danger rounded-md px-3 py-2";
    div.textContent = message;

    td.appendChild(div);
    tr.appendChild(td);
    row.after(tr);

    setTimeout(function() { tr.remove(); }, 5000);
}

// ============================================================================
// Lab detail page — text button actions (used on /labs/{id})
// ============================================================================

async function labDetailAction(labId, action, btn) {
    var originalText = btn.textContent;
    btn.disabled = true;
    btn.textContent = action === "start" ? "Starting..." : "Stopping...";

    var resultDiv = document.getElementById("lab-action-result");
    resultDiv.classList.add("hidden");
    resultDiv.textContent = "";

    try {
        var response = await fetch(`/labs/${encodeURIComponent(labId)}/${action}`, {
            method: "POST",
        });

        if (!response.ok) {
            var text = await response.text();
            showDetailResult("error", text || `${action} failed (HTTP ${response.status})`);
            return;
        }

        var data = await response.json();
        var succeeded = data.results.filter(function(r) { return r.success; });
        var failed = data.results.filter(function(r) { return !r.success; });

        if (failed.length === 0) {
            var names = succeeded.map(function(r) { return r.name; }).join(", ");
            var verb = action === "start" ? "Started" : "Stopped";
            showDetailResult("success", `${verb} ${succeeded.length} node(s): ${names}`);
        } else {
            var msgs = failed.map(function(r) { return `${r.name}: ${r.message}`; });
            if (succeeded.length > 0) {
                var okNames = succeeded.map(function(r) { return r.name; }).join(", ");
                var verb = action === "start" ? "Started" : "Stopped";
                msgs.unshift(`${verb}: ${okNames}`);
            }
            showDetailResult("error", msgs.join(" | "));
        }
    } catch (err) {
        showDetailResult("error", `Request failed: ${err.message}`);
    } finally {
        btn.disabled = false;
        btn.textContent = originalText;
    }
}

// ============================================================================
// Node actions — per-node stop/start/redeploy (used on /labs/{id} nodes table)
// ============================================================================

async function nodeAction(labId, nodeName, action, btn) {
    btn.disabled = true;
    btn.classList.add("animate-spin");

    try {
        var url = `/labs/${encodeURIComponent(labId)}/nodes/${encodeURIComponent(nodeName)}/${action}`;
        var response = await fetch(url, { method: "POST" });

        if (!response.ok) {
            var text = await response.text();
            showNodeNotification(nodeName, "error",
                text || `${action} failed (HTTP ${response.status})`);
            return;
        }

        var data = await response.json();
        var result = data.results[0];

        if (result && result.success) {
            showNodeNotification(nodeName, "success", result.message);
        } else {
            showNodeNotification(nodeName, "error",
                result ? result.message : "Unknown error");
        }
    } catch (err) {
        showNodeNotification(nodeName, "error", `Request failed: ${err.message}`);
    } finally {
        btn.disabled = false;
        btn.classList.remove("animate-spin");
    }
}

async function nodeRedeploy(labId, nodeName, btn) {
    btn.disabled = true;
    btn.classList.add("animate-spin");

    try {
        var url = `/labs/${encodeURIComponent(labId)}/nodes/${encodeURIComponent(nodeName)}/redeploy`;
        var response = await fetch(url, { method: "POST" });

        if (!response.ok) {
            var text = await response.text();
            showNodeNotification(nodeName, "error",
                text || `Redeploy failed (HTTP ${response.status})`);
            return;
        }

        var data = await response.json();

        if (data.success) {
            showNodeNotification(nodeName, "success", data.message);
        } else {
            showNodeNotification(nodeName, "error", data.message);
        }
    } catch (err) {
        showNodeNotification(nodeName, "error", `Request failed: ${err.message}`);
    } finally {
        btn.disabled = false;
        btn.classList.remove("animate-spin");
    }
}

function showNodeNotification(nodeName, type, message) {
    var existing = document.getElementById(`node-notification-${nodeName}`);
    if (existing) {
        existing.remove();
    }

    var row = document.getElementById(`node-row-${nodeName}`);
    if (!row) return;

    var tr = document.createElement("tr");
    tr.id = `node-notification-${nodeName}`;

    var td = document.createElement("td");
    td.colSpan = 9;
    td.className = "px-6 py-2";

    var div = document.createElement("div");
    div.className = type === "success"
        ? "text-sm text-success-text bg-success rounded-md px-3 py-2"
        : "text-sm text-danger-solid bg-danger rounded-md px-3 py-2";
    div.textContent = message;

    td.appendChild(div);
    tr.appendChild(td);
    row.after(tr);

    setTimeout(function() { tr.remove(); }, 5000);
}

function showDetailResult(type, message) {
    var resultDiv = document.getElementById("lab-action-result");
    resultDiv.className = type === "success"
        ? "mt-4 text-sm text-success-text bg-success rounded-lg px-4 py-3"
        : "mt-4 text-sm text-danger-solid bg-danger rounded-lg px-4 py-3";
    resultDiv.textContent = message;
    resultDiv.classList.remove("hidden");

    setTimeout(function() {
        resultDiv.classList.add("hidden");
    }, 8000);
}
