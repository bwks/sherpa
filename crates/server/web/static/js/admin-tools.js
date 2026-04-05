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
