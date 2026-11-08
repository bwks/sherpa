//! The agents file: which agents may use the bus, and in which role. It is TOML and holds only
//! public keys, so it is not secret.
//!
//! ```toml
//! [[agent]]
//! name = "host"
//! key = "UA..."
//! role = "host"
//!
//! [[agent]]
//! name = "dev"
//! key = "UB..."
//! role = "guest"
//! ```

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Deserialize;

use crate::keys::check_public_key;
use crate::message::check_agent_name;

/// A word this long is a key or a seed, not prose.
const MIN_KEY_LENGTH: usize = 40;

/// What an agent may do on the bus.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Sends tasks to every agent and reads its own inbox
    Host,
    /// Reads its own inbox and replies to the host
    Guest,
}

/// One entry of the agents file.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub name: String,
    /// The agent's `NKey` user public key
    pub key: String,
    pub role: Role,
}

/// A validated agents file: exactly one host, and the guests.
#[derive(Debug, PartialEq, Eq)]
pub struct Agents {
    pub host: Agent,
    pub guests: Vec<Agent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    agent: Vec<Agent>,
}

/// Describe a TOML error without quoting the file: `toml`'s own display shows the offending
/// line, which could be a seed pasted in the wrong place. Its message can quote a value too, so
/// anything the length of a key is hidden.
fn parse_error(text: &str, error: &toml::de::Error) -> anyhow::Error {
    let line = error.span().map_or(0, |span| {
        text.get(..span.start)
            .map_or(0, |before| before.matches('\n').count())
            + 1
    });
    let message = error
        .message()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(|word| {
            if word.len() >= MIN_KEY_LENGTH {
                "<hidden>"
            } else {
                word
            }
        })
        .collect::<Vec<_>>();
    anyhow!(
        "Unable to parse the agents file at line {line}: {}",
        message.join(" ")
    )
}

impl Agents {
    /// Read and validate the agents file at `path`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or is not valid.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Unable to read the agents file {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("Invalid agents file {}", path.display()))
    }

    /// Parse and validate the text of an agents file. Names must be valid and unique, keys must
    /// be user public keys, and there must be exactly one host, whose key no guest may use.
    ///
    /// Guests may share a key. Agents that share a key can read and act on each other's inboxes,
    /// because the bus authorizes keys, not names, so sharing is for testing only, until
    /// enrollment (#494) gives every agent its own key.
    ///
    /// # Errors
    ///
    /// Fails when the text is not valid TOML or breaks one of those rules.
    pub fn parse(text: &str) -> Result<Self> {
        let file: File = toml::from_str(text).map_err(|error| parse_error(text, &error))?;
        let mut names = HashSet::new();
        for agent in &file.agent {
            check_agent_name(&agent.name)?;
            check_public_key(&agent.key)
                .with_context(|| format!("The key of agent '{}' is not valid", agent.name))?;
            ensure!(
                names.insert(agent.name.as_str()),
                "Agent '{}' is listed twice",
                agent.name
            );
        }
        let (hosts, guests): (Vec<Agent>, Vec<Agent>) = file
            .agent
            .into_iter()
            .partition(|agent| agent.role == Role::Host);
        let mut hosts = hosts.into_iter();
        let (Some(host), None) = (hosts.next(), hosts.next()) else {
            bail!("The agents file must list exactly one agent with the role 'host'");
        };
        if let Some(guest) = guests.iter().find(|guest| guest.key == host.key) {
            bail!(
                "Guest '{}' uses the key of the host '{}'; the host's key must never be shared",
                guest.name,
                host.name
            );
        }
        Ok(Self { host, guests })
    }

    /// Every agent: the host first, then the guests.
    pub fn all(&self) -> impl Iterator<Item = &Agent> {
        std::iter::once(&self.host).chain(self.guests.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::generate;

    fn entry(name: &str, key: &str, role: &str) -> String {
        format!("[[agent]]\nname = \"{name}\"\nkey = \"{key}\"\nrole = \"{role}\"\n")
    }

    fn key() -> String {
        generate().unwrap().public_key
    }

    #[test]
    fn a_host_and_guests_parse() {
        let (host, dev, third) = (key(), key(), key());
        let file = [
            entry("host", &host, "host"),
            entry("dev", &dev, "guest"),
            entry("test-1", &third, "guest"),
        ]
        .concat();
        let agents = Agents::parse(&file).unwrap();
        assert_eq!(agents.host.name, "host");
        assert_eq!(agents.host.key, host);
        assert_eq!(agents.guests.len(), 2);
        assert_eq!(agents.guests[1].name, "test-1");
        assert_eq!(agents.guests[1].role, Role::Guest);
        let names: Vec<&str> = agents.all().map(|agent| agent.name.as_str()).collect();
        assert_eq!(names, ["host", "dev", "test-1"]);
    }

    #[test]
    fn exactly_one_host_is_required() {
        let none = entry("dev", &key(), "guest");
        assert!(Agents::parse(&none).is_err());
        let two = [entry("a", &key(), "host"), entry("b", &key(), "host")].concat();
        assert!(Agents::parse(&two).is_err());
        assert!(Agents::parse("").is_err());
    }

    #[test]
    fn names_must_be_valid_and_unique() {
        let bad = [
            entry("host", &key(), "host"),
            entry("Dev.>", &key(), "guest"),
        ]
        .concat();
        assert!(Agents::parse(&bad).is_err());
        let twice = [
            entry("host", &key(), "host"),
            entry("dev", &key(), "guest"),
            entry("dev", &key(), "guest"),
        ]
        .concat();
        assert!(Agents::parse(&twice).is_err());
    }

    #[test]
    fn the_host_key_is_never_shared_with_a_guest() {
        let shared = key();
        let same = [
            entry("dev", &shared, "guest"),
            entry("host", &shared, "host"),
        ]
        .concat();
        let error = format!("{:#}", Agents::parse(&same).unwrap_err());
        assert!(error.contains("Guest 'dev' uses the key of the host 'host'"));
    }

    #[test]
    fn guests_may_share_a_key() {
        let shared = key();
        let file = [
            entry("host", &key(), "host"),
            entry("dev", &shared, "guest"),
            entry("agent-2", &shared, "guest"),
        ]
        .concat();
        let agents = Agents::parse(&file).unwrap();
        assert_eq!(agents.guests.len(), 2);
        assert_eq!(agents.guests[0].key, agents.guests[1].key);
    }

    #[test]
    fn keys_must_be_user_public_keys() {
        let junk = [entry("host", &key(), "host"), entry("dev", "UABC", "guest")].concat();
        assert!(Agents::parse(&junk).is_err());
    }

    #[test]
    fn a_seed_in_the_file_is_refused_and_never_repeated() {
        let seed = generate().unwrap().seed;
        let in_key = [entry("host", &key(), "host"), entry("dev", &seed, "guest")].concat();
        let error = format!("{:#}", Agents::parse(&in_key).unwrap_err());
        assert!(!error.contains(&seed));
        let extra = format!("{}seed = \"{seed}\"\n", entry("host", &key(), "host"));
        let error = format!("{:#}", Agents::parse(&extra).unwrap_err());
        assert!(!error.contains(&seed));
    }

    #[test]
    fn an_unknown_role_is_refused() {
        let text = entry("host", &key(), "admin");
        assert!(Agents::parse(&text).is_err());
    }
}
