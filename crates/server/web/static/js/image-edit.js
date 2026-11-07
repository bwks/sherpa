// Image edit page: ask before a save changes which version is the default.
(function () {
    var defaultCheckbox = document.getElementById("default");
    var initialDefaultState = defaultCheckbox.checked;

    // The form submits through htmx; `htmx:confirm` holds the request until we issue it.
    document.getElementById("edit-form").addEventListener("htmx:confirm", function (event) {
        var currentDefaultState = defaultCheckbox.checked;
        if (currentDefaultState === initialDefaultState) {
            return;
        }
        event.preventDefault();
        var message = currentDefaultState
            ? "Are you sure you want to set this version as the default? This will unset the current default version."
            : "Are you sure you want to unset this as the default version? At least one version must remain as default.";
        if (confirm(message)) {
            event.detail.issueRequest(true);
        }
    });
})();
