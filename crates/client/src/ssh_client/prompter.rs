//! Asking for a key passphrase or a password on the terminal while logging in, as `ssh`
//! does, for Jato's [`Prompter`]. Input is not echoed, and an empty answer declines.
//!
//! Answers are kept for the run in a [`PromptCache`], so provisioning several nodes through
//! the same server asks once. Within one connection a question asked again means the last
//! answer was wrong, so that one goes to the terminal.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use jato::{Node, Prompter, Secret};

/// Answers given during this run, by question.
#[derive(Clone, Debug, Default)]
pub struct PromptCache {
    answers: Arc<Mutex<HashMap<String, Secret>>>,
}

/// Asks on the terminal.
#[derive(Debug)]
struct TerminalPrompter;

/// The prompter for one connection attempt: cached answers first, then `inner`.
#[derive(Debug)]
struct CachedPrompter {
    answers: Arc<Mutex<HashMap<String, Secret>>>,
    asked: Mutex<HashSet<String>>,
    inner: Arc<dyn Prompter>,
}

/// A question Jato asks.
enum Question<'a> {
    Passphrase(&'a Path),
    Password(&'a Node, &'a str),
}

impl PromptCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// A prompter for one connection attempt, asking on the terminal.
    pub fn terminal(&self) -> Arc<dyn Prompter> {
        self.prompter(Arc::new(TerminalPrompter))
    }

    /// A prompter for one connection attempt, asking `inner` what is not cached.
    fn prompter(&self, inner: Arc<dyn Prompter>) -> Arc<dyn Prompter> {
        Arc::new(CachedPrompter {
            answers: Arc::clone(&self.answers),
            asked: Mutex::new(HashSet::new()),
            inner,
        })
    }
}

impl CachedPrompter {
    fn answer(&self, question: &Question<'_>) -> Option<Secret> {
        let key = question.key();
        let first_time = locked(&self.asked).insert(key.clone());
        if first_time && let Some(cached) = locked(&self.answers).get(&key) {
            return Some(copy(cached));
        }
        let answer = match question {
            Question::Passphrase(path) => self.inner.passphrase(path),
            Question::Password(node, username) => self.inner.password(node, username),
        }?;
        locked(&self.answers).insert(key, copy(&answer));
        Some(answer)
    }
}

impl Prompter for CachedPrompter {
    fn passphrase(&self, path: &Path) -> Option<Secret> {
        self.answer(&Question::Passphrase(path))
    }

    fn password(&self, node: &Node, username: &str) -> Option<Secret> {
        self.answer(&Question::Password(node, username))
    }
}

impl Prompter for TerminalPrompter {
    fn passphrase(&self, path: &Path) -> Option<Secret> {
        ask(&passphrase_question(path))
    }

    fn password(&self, node: &Node, username: &str) -> Option<Secret> {
        ask(&password_question(node, username))
    }
}

impl Question<'_> {
    /// What the answer is cached under.
    fn key(&self) -> String {
        match self {
            Question::Passphrase(path) => format!("passphrase {}", path.display()),
            Question::Password(node, username) => format!("password {username}@{node}"),
        }
    }
}

/// The lock's data; a thread that panicked holding it left nothing half-written.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn copy(secret: &Secret) -> Secret {
    Secret::new(secret.expose().to_string())
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
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Answers from a script and counts the questions.
    #[derive(Debug)]
    struct Scripted {
        answers: Mutex<VecDeque<Option<&'static str>>>,
        asked: Mutex<usize>,
    }

    impl Scripted {
        fn new(answers: &[Option<&'static str>]) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(answers.iter().copied().collect()),
                asked: Mutex::new(0),
            })
        }

        fn asked(&self) -> usize {
            *self.asked.lock().unwrap()
        }

        fn next(&self) -> Option<Secret> {
            *self.asked.lock().unwrap() += 1;
            let answer = self.answers.lock().unwrap().pop_front().flatten()?;
            Some(Secret::new(answer.to_string()))
        }
    }

    impl Prompter for Scripted {
        fn passphrase(&self, _path: &Path) -> Option<Secret> {
            self.next()
        }

        fn password(&self, _node: &Node, _username: &str) -> Option<Secret> {
            self.next()
        }
    }

    fn key() -> &'static Path {
        Path::new("/home/me/.ssh/id_ed25519")
    }

    #[test]
    fn a_later_connection_reuses_the_answer() {
        let inner = Scripted::new(&[Some("pass")]);
        let cache = PromptCache::new();
        let first = cache.prompter(inner.clone());
        assert_eq!(first.passphrase(key()).unwrap().expose(), "pass");
        let second = cache.prompter(inner.clone());
        assert_eq!(second.passphrase(key()).unwrap().expose(), "pass");
        assert_eq!(inner.asked(), 1);
    }

    #[test]
    fn asked_again_in_one_connection_means_the_answer_was_wrong() {
        let inner = Scripted::new(&[Some("wrong"), Some("right")]);
        let cache = PromptCache::new();
        let first = cache.prompter(inner.clone());
        assert_eq!(first.passphrase(key()).unwrap().expose(), "wrong");
        assert_eq!(first.passphrase(key()).unwrap().expose(), "right");
        let second = cache.prompter(inner.clone());
        assert_eq!(second.passphrase(key()).unwrap().expose(), "right");
        assert_eq!(inner.asked(), 2);
    }

    #[test]
    fn a_declined_question_is_asked_again_next_time() {
        let inner = Scripted::new(&[None, Some("pw")]);
        let cache = PromptCache::new();
        let node = Node::new("10.100.58.10", 22);
        assert!(
            cache
                .prompter(inner.clone())
                .password(&node, "me")
                .is_none()
        );
        let answer = cache.prompter(inner.clone()).password(&node, "me").unwrap();
        assert_eq!(answer.expose(), "pw");
        assert_eq!(inner.asked(), 2);
    }

    #[test]
    fn passphrases_and_passwords_are_kept_apart() {
        let inner = Scripted::new(&[Some("pass"), Some("pw")]);
        let cache = PromptCache::new();
        let node = Node::new("10.100.58.10", 22);
        let prompter = cache.prompter(inner.clone());
        assert_eq!(prompter.passphrase(key()).unwrap().expose(), "pass");
        assert_eq!(prompter.password(&node, "me").unwrap().expose(), "pw");
    }

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
