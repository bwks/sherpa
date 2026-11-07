use std::io::{self, Write};

use anyhow::{Context, Result};

use crate::data::DestroyResponse;
use crate::util::emoji::Emoji;

/// Prints a surrounded message to the terminal
pub fn term_msg_surround(message: &str) {
    let msg_len = message.len();
    let surround = "━".repeat(msg_len);
    println!(
        r"{surround}
{message}
{surround}",
    );
}

/// Prints an underlined message to the terminal
pub fn term_msg_underline(message: &str) {
    let msg_len = message.len();
    let underline = "─".repeat(msg_len);
    println!(
        r"{message}
{underline}",
    );
}

/// Prints an highlighted message to the terminal
pub fn term_msg_highlight(message: &str) {
    println!("- {message} - ");
}

/// Display detailed destroy/clean results with success/failure tracking
///
/// # Errors
///
/// Never fails today; the `Result` is kept for callers.
pub fn display_destroy_results(response: &DestroyResponse) -> Result<()> {
    let summary = &response.summary;

    term_msg_surround("Destroy Results");

    let sections: [(Emoji, &str, &[String]); 12] = [
        (
            Emoji::Success,
            "Containers Destroyed",
            &summary.containers_destroyed,
        ),
        (
            Emoji::Error,
            "Containers Failed",
            &summary.containers_failed,
        ),
        (
            Emoji::Success,
            "Virtual Machines Destroyed",
            &summary.vms_destroyed,
        ),
        (Emoji::Error, "Virtual Machines Failed", &summary.vms_failed),
        (Emoji::Success, "Disks Deleted", &summary.disks_deleted),
        (Emoji::Error, "Disks Failed", &summary.disks_failed),
        (
            Emoji::Success,
            "Libvirt Networks Destroyed",
            &summary.libvirt_networks_destroyed,
        ),
        (
            Emoji::Error,
            "Libvirt Networks Failed",
            &summary.libvirt_networks_failed,
        ),
        (
            Emoji::Success,
            "Docker Networks Destroyed",
            &summary.docker_networks_destroyed,
        ),
        (
            Emoji::Error,
            "Docker Networks Failed",
            &summary.docker_networks_failed,
        ),
        (
            Emoji::Success,
            "Interfaces Deleted",
            &summary.interfaces_deleted,
        ),
        (
            Emoji::Error,
            "Interfaces Failed",
            &summary.interfaces_failed,
        ),
    ];
    for (emoji, title, items) in sections {
        if items.is_empty() {
            continue;
        }
        term_msg_underline(&format!("{emoji} {title} ({})", items.len()));
        for item in items {
            println!("  - {item}");
        }
        println!();
    }

    print_destroy_outcome(response);

    Ok(())
}

/// Print whether the lab's database records and folder went, the errors, and the outcome.
fn print_destroy_outcome(response: &DestroyResponse) {
    let summary = &response.summary;

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
