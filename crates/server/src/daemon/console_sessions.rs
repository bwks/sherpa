//! The node console sessions sherpad relays: one per console, serial or VNC.
//!
//! A serial console serves one client. sherpad holds the session open on each and answers a
//! second at once, naming who holds it, instead of leaving it connected and silent as the
//! socket would. A VNC console is held the same way, so one viewer at a time sees and drives
//! the node's screen. The owner can take a console over from a session of their own left open
//! elsewhere (`sherpa console --force`, `sherpa vnc --force`); the session taken over is told
//! and closed.

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use jiff::Timestamp;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::services::console::ConsoleKind;

/// The console sessions open now, by lab and node.
#[derive(Clone, Default)]
pub struct ConsoleSessions {
    held: Arc<DashMap<ConsoleKey, Holder>>,
}

/// A node console: its lab, the node's name and which of its consoles.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConsoleKey {
    lab_id: String,
    node_name: String,
    kind: ConsoleKind,
}

/// Who asks for a console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimant {
    pub username: String,
    /// The address the request came from, when the server knows it.
    pub client: Option<SocketAddr>,
}

/// The session that holds a console.
#[derive(Debug, Clone)]
pub struct Holder {
    pub claimant: Claimant,
    pub since: Timestamp,
    id: Uuid,
    /// Cancelled when another session takes the console over.
    taken_over: CancellationToken,
}

/// A console held by the session that claimed it, until this is dropped.
#[must_use = "the console is let go when the claim is dropped"]
pub struct ConsoleClaim {
    held: Arc<DashMap<ConsoleKey, Holder>>,
    key: ConsoleKey,
    id: Uuid,
    taken_over: CancellationToken,
    /// The session this one took the console over from, if any.
    pub displaced: Option<Holder>,
}

impl ConsoleSessions {
    /// Claim the `kind` console of `node_name` in lab `lab_id` for `claimant`. A console another
    /// session holds is refused with that session, unless `force` is set and the session is
    /// the claimant's own: it is then taken over, told so, and named in
    /// [`ConsoleClaim::displaced`].
    ///
    /// # Errors
    ///
    /// The session holding the console, when it is refused.
    pub fn claim(
        &self,
        lab_id: &str,
        node_name: &str,
        kind: ConsoleKind,
        claimant: Claimant,
        force: bool,
    ) -> Result<ConsoleClaim, Holder> {
        let key = ConsoleKey {
            lab_id: lab_id.to_owned(),
            node_name: node_name.to_owned(),
            kind,
        };
        let holder = Holder {
            claimant,
            since: Timestamp::now(),
            id: Uuid::new_v4(),
            taken_over: CancellationToken::new(),
        };
        let id = holder.id;
        let taken_over = holder.taken_over.clone();
        // The entry locks its part of the map until the match ends; the claim, whose drop takes
        // that lock, is made after.
        let displaced = match self.held.entry(key.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(holder);
                None
            }
            Entry::Occupied(mut entry) => {
                let held = entry.get();
                if !force || held.claimant.username != holder.claimant.username {
                    return Err(held.clone());
                }
                let displaced = entry.insert(holder);
                displaced.taken_over.cancel();
                Some(displaced)
            }
        };
        Ok(ConsoleClaim {
            held: Arc::clone(&self.held),
            key,
            id,
            taken_over,
            displaced,
        })
    }

    /// The session holding the `kind` console of `node_name` in lab `lab_id`, if any.
    #[must_use]
    pub fn holder(&self, lab_id: &str, node_name: &str, kind: ConsoleKind) -> Option<Holder> {
        self.held
            .get(&ConsoleKey {
                lab_id: lab_id.to_owned(),
                node_name: node_name.to_owned(),
                kind,
            })
            .map(|holder| holder.clone())
    }
}

impl ConsoleClaim {
    /// Resolves once another session takes the console over.
    pub async fn taken_over(&self) {
        self.taken_over.cancelled().await;
    }
}

impl Drop for ConsoleClaim {
    /// Let the console go, unless another session has taken it over since.
    fn drop(&mut self) {
        self.held
            .remove_if(&self.key, |_, holder| holder.id == self.id);
    }
}

impl fmt::Display for Holder {
    /// `<user> from <address> since <time> UTC`, the address only when known.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.claimant.username)?;
        if let Some(client) = self.claimant.client {
            write!(f, " from {}", client.ip())?;
        }
        write!(f, " since {} UTC", self.since.strftime("%Y-%m-%d %H:%M:%S"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIAL: ConsoleKind = ConsoleKind::Serial;

    fn alice(port: u16) -> Claimant {
        Claimant {
            username: "alice".to_owned(),
            client: Some(SocketAddr::from(([192, 0, 2, 10], port))),
        }
    }

    #[test]
    fn a_console_is_held_by_one_session_and_let_go_when_it_ends() {
        let sessions = ConsoleSessions::default();
        let first = sessions
            .claim("lab1", "alp01", SERIAL, alice(1), false)
            .unwrap();
        assert!(first.displaced.is_none());
        let refused = sessions
            .claim("lab1", "alp01", SERIAL, alice(2), false)
            .err()
            .unwrap();
        assert_eq!(refused.claimant, alice(1));
        // The refused claim did not let go of the holder's console.
        assert_eq!(
            sessions.holder("lab1", "alp01", SERIAL).unwrap().claimant,
            alice(1)
        );
        // Another node's console, or the same node in another lab, is its own.
        let _other = sessions
            .claim("lab1", "srx01", SERIAL, alice(3), false)
            .unwrap();
        let _elsewhere = sessions
            .claim("lab2", "alp01", SERIAL, alice(4), false)
            .unwrap();
        // The node's VNC console is held apart from its serial console.
        let vnc = sessions
            .claim("lab1", "alp01", ConsoleKind::Vnc, alice(5), false)
            .unwrap();
        assert!(
            sessions
                .claim("lab1", "alp01", ConsoleKind::Vnc, alice(6), false)
                .is_err()
        );
        drop(vnc);
        assert!(sessions.holder("lab1", "alp01", ConsoleKind::Vnc).is_none());
        assert_eq!(
            sessions.holder("lab1", "alp01", SERIAL).unwrap().claimant,
            alice(1)
        );
        drop(first);
        assert!(sessions.holder("lab1", "alp01", SERIAL).is_none());
        let _again = sessions
            .claim("lab1", "alp01", SERIAL, alice(2), false)
            .unwrap();
    }

    #[tokio::test]
    async fn force_takes_over_the_owners_own_session_and_tells_it() {
        let sessions = ConsoleSessions::default();
        let first = sessions
            .claim("lab1", "alp01", SERIAL, alice(1), false)
            .unwrap();
        let second = sessions
            .claim("lab1", "alp01", SERIAL, alice(2), true)
            .unwrap();
        assert_eq!(second.displaced.as_ref().unwrap().claimant, alice(1));
        first.taken_over().await;
        // The session taken over ends after the new one holds the console: it must not let go
        // of the new one's.
        drop(first);
        assert_eq!(
            sessions.holder("lab1", "alp01", SERIAL).unwrap().claimant,
            alice(2)
        );
        drop(second);
        assert!(sessions.holder("lab1", "alp01", SERIAL).is_none());
    }

    #[test]
    fn force_does_not_take_over_another_users_session() {
        let sessions = ConsoleSessions::default();
        let _first = sessions
            .claim("lab1", "alp01", SERIAL, alice(1), false)
            .unwrap();
        let bob = Claimant {
            username: "bob".to_owned(),
            client: None,
        };
        let refused = sessions
            .claim("lab1", "alp01", SERIAL, bob, true)
            .err()
            .unwrap();
        assert_eq!(refused.claimant, alice(1));
    }

    #[test]
    fn a_holder_is_named_by_user_address_and_time() {
        let holder = Holder {
            claimant: alice(40000),
            since: "2026-11-18T09:08:07.654Z".parse().unwrap(),
            id: Uuid::nil(),
            taken_over: CancellationToken::new(),
        };
        assert_eq!(
            holder.to_string(),
            "alice from 192.0.2.10 since 2026-11-18 09:08:07 UTC"
        );
        let unknown = Holder {
            claimant: Claimant {
                username: "alice".to_owned(),
                client: None,
            },
            ..holder
        };
        assert_eq!(unknown.to_string(), "alice since 2026-11-18 09:08:07 UTC");
    }
}
