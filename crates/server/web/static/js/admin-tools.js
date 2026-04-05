let pendingLabId = null;

function submitClean(event) {
    event.preventDefault();
    const labId = document.getElementById("lab-id").value.trim();
    if (!labId) return;

    pendingLabId = labId;
    document.getElementById("confirm-lab-id").textContent = labId;
    document.getElementById("clean-confirm").classList.remove("hidden");
    document.getElementById("clean-result").classList.add("hidden");
    document.getElementById("clean-btn").disabled = true;
}

function cancelClean() {
    pendingLabId = null;
    document.getElementById("clean-confirm").classList.add("hidden");
    document.getElementById("clean-btn").disabled = false;
}

async function executeClean() {
    if (!pendingLabId) return;

    const labId = pendingLabId;
    pendingLabId = null;
    document.getElementById("clean-confirm").classList.add("hidden");
    document.getElementById("clean-btn").disabled = true;
    document.getElementById("clean-btn").textContent = "Cleaning...";

    const resultDiv = document.getElementById("clean-result");
    resultDiv.classList.add("hidden");
    resultDiv.innerHTML = "";

    try {
        const response = await fetch("/admin/tools/clean/" + encodeURIComponent(labId), {
            method: "POST",
        });

        if (!response.ok) {
            const errorTpl = document.getElementById("clean-error-template").content.cloneNode(true);
            const text = await response.text();
            errorTpl.querySelector(".result-message").textContent = text || "Clean failed (HTTP " + response.status + ")";
            resultDiv.appendChild(errorTpl);
        } else {
            const data = await response.json();
            if (data.success) {
                const tpl = document.getElementById("clean-success-template").content.cloneNode(true);
                tpl.querySelector(".result-lab-name").textContent = data.lab_name;
                tpl.querySelector(".result-lab-id").textContent = data.lab_id;
                tpl.querySelector(".result-containers").textContent = data.summary.containers_destroyed.length + " destroyed";
                tpl.querySelector(".result-vms").textContent = data.summary.vms_destroyed.length + " destroyed";
                tpl.querySelector(".result-disks").textContent = data.summary.disks_deleted.length + " deleted";
                tpl.querySelector(".result-docker-nets").textContent = data.summary.docker_networks_destroyed.length + " destroyed";
                tpl.querySelector(".result-libvirt-nets").textContent = data.summary.libvirt_networks_destroyed.length + " destroyed";
                tpl.querySelector(".result-interfaces").textContent = data.summary.interfaces_deleted.length + " deleted";
                tpl.querySelector(".result-database").textContent = data.summary.database_records_deleted ? "cleaned" : "not found";
                tpl.querySelector(".result-directory").textContent = data.summary.lab_directory_deleted ? "deleted" : "not found";
                resultDiv.appendChild(tpl);
            } else {
                const tpl = document.getElementById("clean-partial-template").content.cloneNode(true);
                tpl.querySelector(".result-lab-name").textContent = data.lab_name;
                tpl.querySelector(".result-lab-id").textContent = data.lab_id;
                const errorList = tpl.querySelector(".result-errors");
                for (const err of data.errors) {
                    const li = document.createElement("li");
                    li.textContent = err.resource_type + " (" + err.resource_name + "): " + err.error_message;
                    errorList.appendChild(li);
                }
                resultDiv.appendChild(tpl);
            }
        }
    } catch (err) {
        const errorTpl = document.getElementById("clean-error-template").content.cloneNode(true);
        errorTpl.querySelector(".result-message").textContent = "Request failed: " + err.message;
        resultDiv.appendChild(errorTpl);
    }

    resultDiv.classList.remove("hidden");
    document.getElementById("clean-btn").disabled = false;
    document.getElementById("clean-btn").textContent = "Clean Lab";
    document.getElementById("lab-id").value = "";
}

// ============================================================================
// Image Scan
// ============================================================================

function submitScanImport() {
    document.getElementById("scan-confirm").classList.remove("hidden");
    document.getElementById("scan-result").classList.add("hidden");
    document.getElementById("scan-import-btn").disabled = true;
    document.getElementById("scan-btn").disabled = true;
}

function cancelScan() {
    document.getElementById("scan-confirm").classList.add("hidden");
    document.getElementById("scan-import-btn").disabled = false;
    document.getElementById("scan-btn").disabled = false;
}

async function executeScan(dryRun) {
    document.getElementById("scan-confirm").classList.add("hidden");
    document.getElementById("scan-btn").disabled = true;
    document.getElementById("scan-import-btn").disabled = true;

    const activeBtn = dryRun ? "scan-btn" : "scan-import-btn";
    const originalText = document.getElementById(activeBtn).textContent;
    document.getElementById(activeBtn).textContent = "Scanning...";

    const resultDiv = document.getElementById("scan-result");
    resultDiv.classList.add("hidden");
    resultDiv.innerHTML = "";

    try {
        const formData = new URLSearchParams();
        formData.append("dry_run", dryRun ? "true" : "false");

        const response = await fetch("/admin/tools/scan", {
            method: "POST",
            headers: { "Content-Type": "application/x-www-form-urlencoded" },
            body: formData.toString(),
        });

        if (!response.ok) {
            const errorTpl = document.getElementById("scan-error-template").content.cloneNode(true);
            const text = await response.text();
            errorTpl.querySelector(".scan-error-message").textContent = text || "Scan failed (HTTP " + response.status + ")";
            resultDiv.appendChild(errorTpl);
        } else {
            const data = await response.json();
            if (data.scanned.length === 0) {
                const tpl = document.getElementById("scan-empty-template").content.cloneNode(true);
                resultDiv.appendChild(tpl);
            } else {
                const tpl = document.getElementById("scan-success-template").content.cloneNode(true);
                if (dryRun) {
                    tpl.querySelector(".scan-title").textContent = "Dry run complete";
                    tpl.querySelector(".scan-summary").textContent =
                        "Found " + data.scanned.length + " image(s). No changes were made.";
                } else {
                    tpl.querySelector(".scan-title").textContent = "Scan & import complete";
                    tpl.querySelector(".scan-summary").textContent =
                        "Found " + data.scanned.length + " image(s), " + data.total_imported + " imported.";
                }

                const table = buildScanTable(data.scanned);
                tpl.querySelector(".scan-table-container").appendChild(table);
                resultDiv.appendChild(tpl);
            }
        }
    } catch (err) {
        const errorTpl = document.getElementById("scan-error-template").content.cloneNode(true);
        errorTpl.querySelector(".scan-error-message").textContent = "Request failed: " + err.message;
        resultDiv.appendChild(errorTpl);
    }

    resultDiv.classList.remove("hidden");
    document.getElementById("scan-btn").disabled = false;
    document.getElementById("scan-import-btn").disabled = false;
    document.getElementById(activeBtn).textContent = originalText;
}

function buildScanTable(scanned) {
    const table = document.createElement("table");
    table.className = "min-w-full divide-y divide-border text-sm";

    const thead = document.createElement("thead");
    thead.className = "bg-table-head";
    thead.innerHTML =
        '<tr>' +
        '<th class="px-4 py-2 text-left text-xs font-medium text-table-head-text uppercase tracking-wider">Model</th>' +
        '<th class="px-4 py-2 text-left text-xs font-medium text-table-head-text uppercase tracking-wider">Version</th>' +
        '<th class="px-4 py-2 text-left text-xs font-medium text-table-head-text uppercase tracking-wider">Kind</th>' +
        '<th class="px-4 py-2 text-left text-xs font-medium text-table-head-text uppercase tracking-wider">Status</th>' +
        '</tr>';
    table.appendChild(thead);

    const tbody = document.createElement("tbody");
    tbody.className = "bg-card divide-y divide-border";
    for (const img of scanned) {
        const tr = document.createElement("tr");
        tr.className = "hover:bg-hover transition-colors";
        tr.innerHTML =
            '<td class="px-4 py-2 text-body font-mono">' + escapeHtml(img.model) + '</td>' +
            '<td class="px-4 py-2 text-body">' + escapeHtml(img.version) + '</td>' +
            '<td class="px-4 py-2 text-body">' + escapeHtml(img.kind) + '</td>' +
            '<td class="px-4 py-2 text-body">' + escapeHtml(img.status) + '</td>';
        tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    return table;
}

function escapeHtml(text) {
    const div = document.createElement("div");
    div.textContent = text;
    return div.innerHTML;
}
