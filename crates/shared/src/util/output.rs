use std::io::{self, Write};

use anyhow::{Context, Result};

use crate::data::DestroyResponse;
use crate::util::emoji::Emoji;

/// Prints a surrounded message to the terminal
pub fn term_msg_surround(message: &str) {
    let msg_len = message.len();
    let surround = "━".repeat(msg_len);
    println!(
        r#"{surround}
{message}
{surround}"#,
    );
}

/// Prints an underlined message to the terminal
pub fn term_msg_underline(message: &str) {
    let msg_len = message.len();
    let underline = "─".repeat(msg_len);
    println!(
        r#"{message}
{underline}"#,
    );
}

/// Prints an highlighted message to the terminal
pub fn term_msg_highlight(message: &str) {
    println!("- {message} - ");
}

/// Display detailed destroy/clean results with success/failure tracking
pub fn display_destroy_results(response: &DestroyResponse) -> Result<()> {
    let summary = &response.summary;

    term_msg_surround("Destroy Results");

    // Containers
    if !summary.containers_destroyed.is_empty() {
        term_msg_underline(&format!(
            "{} Containers Destroyed ({})",
            Emoji::Success,
            summary.containers_destroyed.len()
        ));
        for container in &summary.containers_destroyed {
            println!("  - {}", container);
        }
        println!();
    }
    if !summary.containers_failed.is_empty() {
        term_msg_underline(&format!(
            "{} Containers Failed ({})",
            Emoji::Error,
            summary.containers_failed.len()
        ));
        for container in &summary.containers_failed {
            println!("  - {}", container);
        }
        println!();
    }

    // Virtual Machines
    if !summary.vms_destroyed.is_empty() {
        term_msg_underline(&format!(
            "{} Virtual Machines Destroyed ({})",
            Emoji::Success,
            summary.vms_destroyed.len()
        ));
        for vm in &summary.vms_destroyed {
            println!("  - {}", vm);
        }
        println!();
    }
    if !summary.vms_failed.is_empty() {
        term_msg_underline(&format!(
            "{} Virtual Machines Failed ({})",
            Emoji::Error,
            summary.vms_failed.len()
        ));
        for vm in &summary.vms_failed {
            println!("  - {}", vm);
        }
        println!();
    }

    // Disks
    if !summary.disks_deleted.is_empty() {
        term_msg_underline(&format!(
            "{} Disks Deleted ({})",
            Emoji::Success,
            summary.disks_deleted.len()
        ));
        for disk in &summary.disks_deleted {
            println!("  - {}", disk);
        }
        println!();
    }
    if !summary.disks_failed.is_empty() {
        term_msg_underline(&format!(
            "{} Disks Failed ({})",
            Emoji::Error,
            summary.disks_failed.len()
        ));
        for disk in &summary.disks_failed {
            println!("  - {}", disk);
        }
        println!();
    }

    // Libvirt Networks
    if !summary.libvirt_networks_destroyed.is_empty() {
        term_msg_underline(&format!(
            "{} Libvirt Networks Destroyed ({})",
            Emoji::Success,
            summary.libvirt_networks_destroyed.len()
        ));
        for network in &summary.libvirt_networks_destroyed {
            println!("  - {}", network);
        }
        println!();
    }
    if !summary.libvirt_networks_failed.is_empty() {
        term_msg_underline(&format!(
            "{} Libvirt Networks Failed ({})",
            Emoji::Error,
            summary.libvirt_networks_failed.len()
        ));
        for network in &summary.libvirt_networks_failed {
            println!("  - {}", network);
        }
        println!();
    }

    // Docker Networks
    if !summary.docker_networks_destroyed.is_empty() {
        term_msg_underline(&format!(
            "{} Docker Networks Destroyed ({})",
            Emoji::Success,
            summary.docker_networks_destroyed.len()
        ));
        for network in &summary.docker_networks_destroyed {
            println!("  - {}", network);
        }
        println!();
    }
    if !summary.docker_networks_failed.is_empty() {
        term_msg_underline(&format!(
            "{} Docker Networks Failed ({})",
            Emoji::Error,
            summary.docker_networks_failed.len()
        ));
        for network in &summary.docker_networks_failed {
            println!("  - {}", network);
        }
        println!();
    }

    // Interfaces
    if !summary.interfaces_deleted.is_empty() {
        term_msg_underline(&format!(
            "{} Interfaces Deleted ({})",
            Emoji::Success,
            summary.interfaces_deleted.len()
        ));
        for interface in &summary.interfaces_deleted {
            println!("  - {}", interface);
        }
        println!();
    }
    if !summary.interfaces_failed.is_empty() {
        term_msg_underline(&format!(
            "{} Interfaces Failed ({})",
            Emoji::Error,
            summary.interfaces_failed.len()
        ));
        for interface in &summary.interfaces_failed {
            println!("  - {}", interface);
        }
        println!();
    }

    // Database and filesystem
    if summary.database_records_deleted {
        println!("{} Database: Cleaned", Emoji::Success);
    } else {
        println!("{} Database: Failed to clean", Emoji::Error);
    }

    if summary.lab_directory_deleted {
        println!("{} Lab Directory: Deleted", Emoji::Success);
    } else {
        println!("{} Lab Directory: Failed to delete", Emoji::Error);
    }

    // Display error details if any
    if !response.errors.is_empty() {
        println!("\n{} Error Details:\n", Emoji::Warning);
        for error in &response.errors {
            println!(
                "  {} {}: {}",
                error.resource_type, error.resource_name, error.error_message
            );
        }
    }

    // Final status
    if response.success {
        println!(
            "\n{} Lab {}-{} destroyed successfully\n",
            Emoji::Success,
            response.lab_name,
            response.lab_id
        );
    } else {
        println!(
            "\n{} Lab {}-{} partially destroyed - review errors above\n",
            Emoji::Warning,
            response.lab_name,
            response.lab_id
        );
        println!(
            "{} Manual cleanup may be required for failed resources\n",
            Emoji::Warning
        );
    }

    Ok(())
}

/// Ask `question` and read the answer from stdin: `y` or `yes`, in any case, is yes; anything
/// else, or no answer, is no.
///
/// # Errors
///
/// Fails when the question cannot be shown or stdin cannot be read.
pub fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N]: ");
    io::stdout()
        .flush()
        .context("Failed to show the question")?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("Failed to read the answer")?;
    Ok(is_yes(&answer))
}

/// Whether `answer` is `y` or `yes`, in any case.
fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_y_or_yes_is_yes() {
        for answer in ["y\n", "Y", " yes ", "YES\r\n"] {
            assert!(is_yes(answer), "{answer:?}");
        }
        for answer in ["", "\n", "n", "no", "yep", "y es"] {
            assert!(!is_yes(answer), "{answer:?}");
        }
    }
}
