//! Asking for a key passphrase or a password on the terminal while logging in, as `ssh`
//! does, for Jato's [`Prompter`]. Input is not echoed. An empty answer declines, so Jato
//! moves on instead of trying an empty secret.

use std::path::Path;

use jato::{Node, Prompter, Secret};

/// Asks on the terminal; Jato calls it before the remote shell puts the terminal in raw mode.
#[derive(Debug)]
pub(super) struct TerminalPrompter;

impl Prompter for TerminalPrompter {
    fn passphrase(&self, path: &Path) -> Option<Secret> {
        ask(&passphrase_question(path))
    }

    fn password(&self, node: &Node, username: &str) -> Option<Secret> {
        ask(&password_question(node, username))
    }
}

/// Ask `question` without echoing the answer. A terminal that cannot be read declines.
fn ask(question: &str) -> Option<Secret> {
    match rpassword::prompt_password(question) {
        Ok(typed) => answer(typed),
        Err(error) => {
            tracing::debug!(%error, "Failed to read an answer from the terminal");
            None
        }
    }
}

/// The question OpenSSH asks for a key's passphrase.
fn passphrase_question(path: &Path) -> String {
    format!("Enter passphrase for key '{}': ", path.display())
}

/// The question OpenSSH asks for a password.
fn password_question(node: &Node, username: &str) -> String {
    format!("{username}@{}'s password: ", node.host())
}

/// The typed answer as a secret, or `None` if nothing was typed.
fn answer(typed: String) -> Option<Secret> {
    if typed.is_empty() {
        None
    } else {
        Some(Secret::new(typed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_passphrase_question_names_the_key() {
        assert_eq!(
            passphrase_question(Path::new("/home/me/.ssh/id_ed25519")),
            "Enter passphrase for key '/home/me/.ssh/id_ed25519': "
        );
    }

    #[test]
    fn the_password_question_names_the_user_and_host() {
        let node = Node::new("10.100.58.10", 22);
        assert_eq!(
            password_question(&node, "alice"),
            "alice@10.100.58.10's password: "
        );
    }

    #[test]
    fn an_empty_answer_declines() {
        assert!(answer(String::new()).is_none());
    }

    #[test]
    fn an_answer_is_kept_as_typed() {
        let secret = answer(" s3cret ".to_string()).unwrap();
        assert_eq!(secret.expose(), " s3cret ");
    }
}
