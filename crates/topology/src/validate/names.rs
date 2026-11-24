use anyhow::{Result, bail};

use crate::Node;

/// The longest node or lab name: one DNS label.
const NAME_MAX_LEN: usize = 63;

/// A word no node name may contain, in any case: the `sonic_linux` image treats a host whose name
/// contains it as a chassis supervisor card.
const RESERVED_NODE_NAME_WORD: &str = "supervisor";

/// Check `name`, the name of a `what`, is a plain name: 1 to 63 ASCII letters, digits, `-` or
/// `_`, starting with a letter or digit.
fn check_plain_name(what: &str, name: &str) -> Result<()> {
    let starts_well = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let plain = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !starts_well || !plain || name.len() > NAME_MAX_LEN {
        bail!(
            "Manifest - {what} name '{name}' is not allowed: use 1 to {NAME_MAX_LEN} letters, \
             digits, '-' or '_', starting with a letter or digit"
        );
    }
    Ok(())
}

/// Check every node name is a plain name: 1 to 63 ASCII letters, digits, `-` or `_`, starting
/// with a letter or digit, without the reserved word `supervisor` in any case.
///
/// Node names become host names, container and domain names, file names in the lab folder and
/// values in the web UI, so they cannot carry path separators, quotes or markup.
///
/// # Errors
///
/// Fails on the first node whose name breaks the rule.
pub fn check_node_names(devices: &[Node]) -> Result<()> {
    for device in devices {
        check_plain_name("node", &device.name)?;
        if device
            .name
            .to_ascii_lowercase()
            .contains(RESERVED_NODE_NAME_WORD)
        {
            bail!(
                "Manifest - node name '{}' is not allowed: '{RESERVED_NODE_NAME_WORD}' is a \
                 reserved word in node names, because the SONiC image treats a host whose name \
                 contains it as a chassis supervisor card",
                device.name
            );
        }
    }
    Ok(())
}

/// Check the lab's name is a plain name, by the rule node names follow.
///
/// The lab name goes into status messages, log fields and the file name of the web UI's lab
/// download, so it cannot carry path separators, quotes, control characters or markup.
///
/// # Errors
///
/// Fails when the name breaks the rule.
pub fn check_lab_name(name: &str) -> Result<()> {
    check_plain_name("lab", name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFUSED: [&str; 10] = [
        "",
        "-r1",
        "_r1",
        "../etc",
        "r1/x",
        "r1.core",
        "r1 x",
        "x',alert(1),'",
        "<b>",
        "r\u{e9}1",
    ];

    fn named(name: &str) -> Node {
        Node {
            name: name.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn node_naming_001_a_plain_name_is_taken() {
        let longest = "a".repeat(NAME_MAX_LEN);
        for name in ["r1", "Spine-01", "leaf_2", "9k", longest.as_str()] {
            assert!(check_node_names(&[named(name)]).is_ok(), "{name}");
        }
    }

    #[test]
    fn node_naming_002_a_name_with_a_path_a_quote_or_markup_is_refused() {
        let too_long = "a".repeat(NAME_MAX_LEN + 1);
        for name in REFUSED.into_iter().chain([too_long.as_str()]) {
            let error = check_node_names(&[named("ok1"), named(name)])
                .expect_err(&format!("'{name}' should be refused"));
            let error = error.to_string();
            assert!(
                error.contains(&format!("node name '{name}' is not allowed")),
                "{error}"
            );
            assert!(error.contains("letters, digits, '-' or '_'"), "{error}");
        }
    }

    #[test]
    fn node_naming_006_a_name_containing_supervisor_in_any_case_is_refused_as_reserved() {
        for name in [
            "supervisor",
            "supervisor1",
            "my-Supervisor",
            "R1_SUPERVISOR_2",
        ] {
            let error = check_node_names(&[named("ok1"), named(name)])
                .expect_err(&format!("'{name}' should be refused"))
                .to_string();
            assert!(
                error.contains(&format!("node name '{name}' is not allowed")),
                "{error}"
            );
            assert!(error.contains("'supervisor' is a reserved word"), "{error}");
            assert!(error.contains("chassis supervisor card"), "{error}");
        }
        for name in ["super", "visor1", "super-visor"] {
            assert!(check_node_names(&[named(name)]).is_ok(), "{name}");
        }
    }

    #[test]
    fn lab_settings_006_a_plain_lab_name_is_taken() {
        let longest = "a".repeat(NAME_MAX_LEN);
        for name in ["lab1", "Core-Lab", "dc_2", "9k", longest.as_str()] {
            assert!(check_lab_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn lab_settings_007_an_empty_path_like_shell_like_too_long_or_non_ascii_lab_name_is_refused() {
        let too_long = "a".repeat(NAME_MAX_LEN + 1);
        for name in REFUSED
            .into_iter()
            .chain(["../../x y;$(id)", "lab\nx", too_long.as_str()])
        {
            let error = check_lab_name(name)
                .expect_err(&format!("'{name}' should be refused"))
                .to_string();
            assert!(
                error.contains(&format!("lab name '{name}' is not allowed")),
                "{error}"
            );
            assert!(error.contains("letters, digits, '-' or '_'"), "{error}");
        }
    }
}
