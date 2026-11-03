use anyhow::{Result, bail};

/// The longest hostname DNS allows.
const MAX_CLIENT_HOST_LENGTH: usize = 253;
/// Linux's `PATH_MAX`; Windows and macOS paths are shorter.
const MAX_CLIENT_FOLDER_LENGTH: usize = 4096;

/// Check the client hostname and folder an `up` request reports, which are stored with the lab
/// and shown back to users.
///
/// Either may be absent: labs started from the web UI or the API have neither. A present value
/// must not be empty, over its length limit, or hold control characters, which could rewrite
/// what a terminal shows.
pub fn validate_lab_origin(client_host: Option<&str>, client_folder: Option<&str>) -> Result<()> {
    if let Some(host) = client_host {
        check_text(host, "client_host", MAX_CLIENT_HOST_LENGTH)?;
    }
    if let Some(folder) = client_folder {
        check_text(folder, "client_folder", MAX_CLIENT_FOLDER_LENGTH)?;
    }
    Ok(())
}

fn check_text(value: &str, field: &str, max_length: usize) -> Result<()> {
    if value.is_empty() {
        bail!("{field} must not be empty");
    }
    if value.chars().count() > max_length {
        bail!("{field} must be at most {max_length} characters");
    }
    if value.chars().any(char::is_control) {
        bail!("{field} must not contain control characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_values_are_accepted() {
        assert!(validate_lab_origin(None, None).is_ok());
    }

    #[test]
    fn hostnames_and_folders_of_each_platform_are_accepted() {
        assert!(validate_lab_origin(Some("dev01"), Some("/home/user/labs/core")).is_ok());
        assert!(validate_lab_origin(Some("WIN-LAPTOP"), Some(r"C:\Users\user\labs\core")).is_ok());
        assert!(validate_lab_origin(Some("mac.local"), Some("/Users/user/My Labs/core")).is_ok());
    }

    #[test]
    fn empty_values_are_refused() {
        let error = validate_lab_origin(Some(""), None).unwrap_err().to_string();
        assert!(error.contains("client_host"), "{error}");
        let error = validate_lab_origin(None, Some("")).unwrap_err().to_string();
        assert!(error.contains("client_folder"), "{error}");
    }

    #[test]
    fn values_over_their_limit_are_refused() {
        let host = "a".repeat(MAX_CLIENT_HOST_LENGTH + 1);
        assert!(validate_lab_origin(Some(&host), None).is_err());
        let folder = format!("/{}", "a".repeat(MAX_CLIENT_FOLDER_LENGTH));
        assert!(validate_lab_origin(None, Some(&folder)).is_err());
        let folder = format!("/{}", "a".repeat(MAX_CLIENT_FOLDER_LENGTH - 1));
        assert!(validate_lab_origin(None, Some(&folder)).is_ok());
    }

    #[test]
    fn control_characters_are_refused() {
        let error = validate_lab_origin(Some("dev\u{1b}[2J"), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("control characters"), "{error}");
        assert!(validate_lab_origin(None, Some("/home/user\nlabs")).is_err());
    }
}
