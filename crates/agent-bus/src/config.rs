//! The nats-server configuration, rendered from the agents file. nats-server accepts JSON, so it
//! is built from structs. It holds public keys and subjects only; a seed has no way in.
//!
//! The bus has one account, so each agent's permissions are what keeps it to its own inbox:
//!
//! - A guest may publish to the host's inbox, and use its own consumer: read its info, pull
//!   messages and acknowledge them. It may create no stream or consumer and send to no one else.
//! - The host may publish to every inbox and use the `JetStream` API, which it needs to create
//!   the stream and the consumers.
//!
//! Replies to requests come back on `_INBOX.>`, so every agent may subscribe to that and nothing
//! else.
//!
//! nats-server allows one user per key, so guests that share a key become one user with the
//! union of their permissions. Each of them can then read and act on the others' inboxes, so
//! sharing a key is for testing only, until enrollment (#494) gives every agent its own key.

use std::fs;
use std::io::Write;
use std::iter;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::agents::Agents;
use crate::channel::{STREAM, SUBJECT_PREFIX};
use crate::files::private_dir;

/// The mode of the config file: it holds nothing secret, and the container's user, whoever the
/// image runs as, must be able to read it. The file itself is bind-mounted, so the container
/// never looks up the state directory, which stays 0700.
const FILE_MODE: u32 = 0o644;

#[derive(Debug, Serialize)]
struct Config {
    authorization: Authorization,
}

#[derive(Debug, Serialize)]
struct Authorization {
    users: Vec<User>,
}

#[derive(Debug, Serialize)]
struct User {
    nkey: String,
    permissions: Permissions,
}

#[derive(Debug, Serialize)]
struct Permissions {
    publish: Allow,
    subscribe: Allow,
}

#[derive(Debug, Serialize)]
struct Allow {
    allow: Vec<String>,
}

/// Render the nats-server configuration for `agents`, as JSON: the host first, then one user per
/// distinct guest key, in the order each key first appears.
///
/// # Errors
///
/// Fails when the configuration cannot be serialized.
pub fn render(agents: &Agents) -> Result<String> {
    let mut guests: Vec<(&str, Vec<&str>)> = Vec::new();
    for guest in &agents.guests {
        if let Some((_, names)) = guests.iter_mut().find(|(key, _)| *key == guest.key) {
            names.push(&guest.name);
        } else {
            guests.push((&guest.key, vec![&guest.name]));
        }
    }
    let users = iter::once(host_user(agents))
        .chain(
            guests
                .into_iter()
                .map(|(key, names)| guest_user(key, &names, agents)),
        )
        .collect::<Vec<_>>();
    let config = Config {
        authorization: Authorization { users },
    };
    serde_json::to_string_pretty(&config).context("Unable to render the nats-server configuration")
}

/// Write the rendered configuration to `path`, replacing the file, readable by everyone, in a
/// directory private to the user.
///
/// # Errors
///
/// Fails when the directory or the file cannot be written.
pub fn write(path: &Path, agents: &Agents) -> Result<()> {
    let text = render(agents)?;
    if let Some(dir) = path.parent() {
        private_dir(dir)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(FILE_MODE)
        .open(path)
        .with_context(|| format!("Unable to write {}", path.display()))?;
    file.write_all(text.as_bytes())
        .with_context(|| format!("Unable to write {}", path.display()))?;
    // `mode` is masked by the umask and ignored for a file that exists.
    fs::set_permissions(path, fs::Permissions::from_mode(FILE_MODE))
        .with_context(|| format!("Unable to set the mode of {}", path.display()))
}

fn host_user(agents: &Agents) -> User {
    user(
        &agents.host.key,
        vec![
            format!("{SUBJECT_PREFIX}.>"),
            "$JS.API.>".to_owned(),
            format!("$JS.ACK.{STREAM}.>"),
        ],
    )
}

/// The user for a guest key and the names of every guest using it. Each of those guests may use
/// every one of their consumers, so guests sharing a key can act on each other's inboxes.
fn guest_user(key: &str, names: &[&str], agents: &Agents) -> User {
    let mut publish = vec![format!("{SUBJECT_PREFIX}.{}", agents.host.name)];
    for name in names {
        publish.extend([
            format!("$JS.API.CONSUMER.INFO.{STREAM}.{name}"),
            format!("$JS.API.CONSUMER.MSG.NEXT.{STREAM}.{name}"),
            format!("$JS.ACK.{STREAM}.{name}.>"),
        ]);
    }
    user(key, publish)
}

fn user(key: &str, publish: Vec<String>) -> User {
    User {
        nkey: key.to_owned(),
        permissions: Permissions {
            publish: Allow { allow: publish },
            subscribe: Allow {
                allow: vec!["_INBOX.>".to_owned()],
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::keys::generate;

    struct Fixture {
        text: String,
        host: String,
        dev: String,
        seeds: Vec<String>,
    }

    fn fixture() -> Fixture {
        let (host, dev, third) = (
            generate().unwrap(),
            generate().unwrap(),
            generate().unwrap(),
        );
        let text = format!(
            "[[agent]]\nname = \"host\"\nkey = \"{}\"\nrole = \"host\"\n\
             [[agent]]\nname = \"dev\"\nkey = \"{}\"\nrole = \"guest\"\n\
             [[agent]]\nname = \"test\"\nkey = \"{}\"\nrole = \"guest\"\n",
            host.public_key, dev.public_key, third.public_key
        );
        Fixture {
            text,
            host: host.public_key,
            dev: dev.public_key,
            seeds: vec![host.seed, dev.seed, third.seed],
        }
    }

    fn rendered(fixture: &Fixture) -> Value {
        let agents = Agents::parse(&fixture.text).unwrap();
        serde_json::from_str(&render(&agents).unwrap()).unwrap()
    }

    fn user_of<'a>(config: &'a Value, key: &str) -> &'a Value {
        config["authorization"]["users"]
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["nkey"] == key)
            .unwrap()
    }

    #[test]
    fn a_guest_may_use_only_its_own_consumer_and_reply_to_the_host() {
        let fixture = fixture();
        let config = rendered(&fixture);
        let guest = user_of(&config, &fixture.dev);
        assert_eq!(
            guest["permissions"]["publish"]["allow"],
            serde_json::json!([
                "agents.host",
                "$JS.API.CONSUMER.INFO.AGENTS.dev",
                "$JS.API.CONSUMER.MSG.NEXT.AGENTS.dev",
                "$JS.ACK.AGENTS.dev.>"
            ])
        );
        assert_eq!(
            guest["permissions"]["subscribe"]["allow"],
            serde_json::json!(["_INBOX.>"])
        );
    }

    #[test]
    fn the_host_may_send_to_every_inbox_and_manage_jetstream() {
        let fixture = fixture();
        let config = rendered(&fixture);
        let host = user_of(&config, &fixture.host);
        assert_eq!(
            host["permissions"]["publish"]["allow"],
            serde_json::json!(["agents.>", "$JS.API.>", "$JS.ACK.AGENTS.>"])
        );
        assert_eq!(
            host["permissions"]["subscribe"]["allow"],
            serde_json::json!(["_INBOX.>"])
        );
    }

    #[test]
    fn every_agent_is_a_user_and_nothing_else_is_configured() {
        let fixture = fixture();
        let config = rendered(&fixture);
        assert_eq!(
            config["authorization"]["users"].as_array().unwrap().len(),
            3
        );
        assert_eq!(config.as_object().unwrap().len(), 1);
        assert_eq!(config["authorization"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn guests_sharing_a_key_are_one_user_with_both_inboxes() {
        let (host, shared, third) = (
            generate().unwrap(),
            generate().unwrap(),
            generate().unwrap(),
        );
        let text = format!(
            "[[agent]]\nname = \"host\"\nkey = \"{}\"\nrole = \"host\"\n\
             [[agent]]\nname = \"dev\"\nkey = \"{}\"\nrole = \"guest\"\n\
             [[agent]]\nname = \"test\"\nkey = \"{}\"\nrole = \"guest\"\n\
             [[agent]]\nname = \"agent-2\"\nkey = \"{}\"\nrole = \"guest\"\n",
            host.public_key, shared.public_key, third.public_key, shared.public_key
        );
        let agents = Agents::parse(&text).unwrap();
        let config: Value = serde_json::from_str(&render(&agents).unwrap()).unwrap();
        let keys: Vec<&str> = config["authorization"]["users"]
            .as_array()
            .unwrap()
            .iter()
            .map(|user| user["nkey"].as_str().unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                host.public_key.as_str(),
                shared.public_key.as_str(),
                third.public_key.as_str()
            ]
        );
        let guest = user_of(&config, &shared.public_key);
        assert_eq!(
            guest["permissions"]["publish"]["allow"],
            serde_json::json!([
                "agents.host",
                "$JS.API.CONSUMER.INFO.AGENTS.dev",
                "$JS.API.CONSUMER.MSG.NEXT.AGENTS.dev",
                "$JS.ACK.AGENTS.dev.>",
                "$JS.API.CONSUMER.INFO.AGENTS.agent-2",
                "$JS.API.CONSUMER.MSG.NEXT.AGENTS.agent-2",
                "$JS.ACK.AGENTS.agent-2.>"
            ])
        );
        assert_eq!(
            guest["permissions"]["subscribe"]["allow"],
            serde_json::json!(["_INBOX.>"])
        );
    }

    #[test]
    fn no_seed_reaches_the_config() {
        let fixture = fixture();
        let agents = Agents::parse(&fixture.text).unwrap();
        let text = render(&agents).unwrap();
        for seed in &fixture.seeds {
            assert!(!text.contains(seed));
            assert!(!text.contains(&seed[2..]));
        }
    }

    #[test]
    fn the_file_is_written_readable_by_the_container_user() {
        let fixture = fixture();
        let agents = Agents::parse(&fixture.text).unwrap();
        let dir = std::env::temp_dir().join(format!("agent-bus-config-{}", std::process::id()));
        let path = dir.join("nats.conf");
        write(&path, &agents).unwrap();
        // A second write replaces the first, even over a file that is too strict.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        write(&path, &agents).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, FILE_MODE);
        assert_eq!(fs::read_to_string(&path).unwrap(), render(&agents).unwrap());
        fs::remove_dir_all(&dir).unwrap();
    }
}
