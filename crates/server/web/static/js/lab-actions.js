// ============================================================================
// P2p links a resume or redeploy left unattached
// ============================================================================

// Each of a node's links that is not attached, as text. A link waiting for its peer is
// expected; a failed one carries no traffic until either end is resumed or redeployed.
function linkNotes(links) {
    return (links || [])
        .filter(function(l) { return l.state !== "attached"; })
        .map(function(l) {
            var why = l.state === "peer_down"
                ? `${l.peer} is down; it is attached when ${l.peer} starts`
                : l.reason;
            return `link ${l.link} not attached: ${why}`;
        });
}

// Whether any of the results' links failed, so the notification shows as an error.
function anyLinkFailed(results) {
    return results.some(function(r) {
        return (r.links || []).some(function(l) { return l.state === "failed"; });
    });
}

// The link notes of every result, each prefixed by its node's name.
function resultLinkNotes(results) {
    return results.flatMap(function(r) {
        return linkNotes(r.links).map(function(note) { return `${r.name}: ${note}`; });
    });
}

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
            var notes = resultLinkNotes(data.results);
            showLabNotification(labId, anyLinkFailed(data.results) ? "error" : "success",
                [`${verb} ${data.results.length} node(s)`].concat(notes).join(" | "));
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
            var notes = resultLinkNotes(data.results);
            showDetailResult(anyLinkFailed(data.results) ? "error" : "success",
                [`${verb} ${succeeded.length} node(s): ${names}`].concat(notes).join(" | "));
        } else {
            var msgs = failed.map(function(r) { return `${r.name}: ${r.message}`; });
            if (succeeded.length > 0) {
                var okNames = succeeded.map(function(r) { return r.name; }).join(", ");
                var verb = action === "start" ? "Started" : "Stopped";
                msgs.unshift(`${verb}: ${okNames}`);
            }
            showDetailResult("error", msgs.concat(resultLinkNotes(data.results)).join(" | "));
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
            showNodeNotification(nodeName, anyLinkFailed([result]) ? "error" : "success",
                [result.message].concat(linkNotes(result.links)).join(" | "));
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
            showNodeNotification(nodeName, anyLinkFailed([data]) ? "error" : "success",
                [data.message].concat(linkNotes(data.links)).join(" | "));
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

// ============================================================================
// Buttons name their action and target in data- attributes; one listener serves
// every page. Partials that htmx swaps in load this script again, so bind once.
// ============================================================================

if (!window.labActionsBound) {
    window.labActionsBound = true;
    document.addEventListener("click", function (event) {
        var btn = event.target.closest("[data-lab-action], [data-lab-detail-action], [data-node-action]");
        if (!btn) {
            return;
        }
        var data = btn.dataset;
        if (data.labAction) {
            labAction(data.labId, btn, data.labAction);
        } else if (data.labDetailAction) {
            labDetailAction(data.labId, data.labDetailAction, btn);
        } else if (data.nodeAction === "redeploy") {
            nodeRedeploy(data.labId, data.nodeName, btn);
        } else {
            nodeAction(data.labId, data.nodeName, data.nodeAction, btn);
        }
    });
}
