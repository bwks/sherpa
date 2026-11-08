//! The nats-server configuration, rendered from the agents file and the registry of enrolled
//! guests. nats-server accepts JSON, so it is built from structs. It holds public keys and
//! subjects only; a seed has no way in.
//!
//! The bus has one account, so each agent's permissions are what keeps it to its own inbox:
//!
//! - A guest may publish to the host's inbox, and use its own consumer: read its info, pull
//!   messages and acknowledge them. It may also publish its own status, under its name in the
//!   status bucket (`status`); it cannot read the bucket. It may create no stream or consumer
//!   and send to no one else.
//! - The host may publish to every inbox and use the `JetStream` API, which it needs to create
//!   the stream and the consumers.
//!
//! Replies to requests come back on `_INBOX.>`, so every agent may subscribe to that and nothing
//! else. Every agent has its own key (`agents::check_entries`), so each is one user.
//!
//! The file is written atomically into its own directory, which the container bind-mounts: a
//! bind-mounted single file would keep showing the container the old file after a rename.

use std::fs;
use std::iter;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::agents::Agents;
use crate::channel::{STREAM, SUBJECT_PREFIX};
use crate::files::{private_dir, write_atomic};
use crate::status::BUCKET;

/// The mode of the config file and its directory: they hold nothing secret, and the container's
/// user, whoever the image runs as, must be able to read them. The directory itself is
/// bind-mounted, so the container never looks up the state directory above it, which stays 0700.
const FILE_MODE: u32 = 0o644;
const DIR_MODE: u32 = 0o755;

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
/// guest, in order.
///
/// # Errors
///
/// Fails when the configuration cannot be serialized.
pub fn render(agents: &Agents) -> Result<String> {
    let users = iter::once(host_user(agents))
        .chain(
            agents
                .guests
                .iter()
                .map(|guest| guest_user(&guest.key, &guest.name, agents)),
        )
        .collect::<Vec<_>>();
    let config = Config {
        authorization: Authorization { users },
    };
    serde_json::to_string_pretty(&config).context("Unable to render the nats-server configuration")
}

/// Write the rendered configuration to `path`, atomically, readable by everyone. Its directory
/// (the one the container mounts) is 0755, and the directory above it, the state directory,
/// 0700.
///
/// # Errors
///
/// Fails when the directories or the file cannot be written.
pub fn write(path: &Path, agents: &Agents) -> Result<()> {
    let text = render(agents)?;
    if let Some(dir) = path.parent() {
        if let Some(state) = dir.parent() {
            private_dir(state)?;
        }
        fs::create_dir_all(dir).with_context(|| format!("Unable to create {}", dir.display()))?;
        fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))
            .with_context(|| format!("Unable to set the mode of {}", dir.display()))?;
    }
    write_atomic(path, &text, FILE_MODE)
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

/// The user for the guest `name`: it may send to the host, use its own consumer and write its
/// own status.
fn guest_user(key: &str, name: &str, agents: &Agents) -> User {
    user(
        key,
        vec![
            format!("{SUBJECT_PREFIX}.{}", agents.host.name),
            format!("$JS.API.CONSUMER.INFO.{STREAM}.{name}"),
            format!("$JS.API.CONSUMER.MSG.NEXT.{STREAM}.{name}"),
            format!("$JS.ACK.{STREAM}.{name}.>"),
            format!("$KV.{BUCKET}.{name}"),
        ],
    )
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
    use std::os::unix::fs::MetadataExt;

    use serde_json::Value;

    use super::*;
    use crate::files::DIR_MODE as STATE_DIR_MODE;
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
                "$JS.ACK.AGENTS.dev.>",
                "$KV.AGENT_STATUS.dev"
            ])
        );
        assert_eq!(
            guest["permissions"]["subscribe"]["allow"],
            serde_json::json!(["_INBOX.>"])
        );
    }

    #[test]
    fn a_guest_may_write_only_its_own_status_and_read_none() {
        let fixture = fixture();
        let config = rendered(&fixture);
        let guest = user_of(&config, &fixture.dev);
        let publish: Vec<&str> = guest["permissions"]["publish"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .map(|subject| subject.as_str().unwrap())
            .collect();
        let status: Vec<&str> = publish
            .iter()
            .copied()
            .filter(|subject| subject.contains(BUCKET))
            .collect();
        // Its own key, exactly: no wildcard, and not another guest's or the host's.
        assert_eq!(status, ["$KV.AGENT_STATUS.dev"]);
        // Reading a status goes through the JetStream API, which a guest may use only for its
        // own consumer.
        assert!(
            publish
                .iter()
                .filter(|subject| subject.starts_with("$JS.API."))
                .all(|subject| subject.ends_with(".AGENTS.dev")),
            "{publish:?}"
        );
        let test_key = Agents::parse(&fixture.text).unwrap().guests[1].key.clone();
        let other = user_of(&config, &test_key);
        assert!(
            other["permissions"]["publish"]["allow"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("$KV.AGENT_STATUS.test"))
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
        let keys: Vec<&str> = config["authorization"]["users"]
            .as_array()
            .unwrap()
            .iter()
            .map(|user| user["nkey"].as_str().unwrap())
            .collect();
        assert_eq!(keys.len(), 3);
        assert_eq!(keys[..2], [fixture.host.as_str(), fixture.dev.as_str()]);
        assert_eq!(config.as_object().unwrap().len(), 1);
        assert_eq!(config["authorization"].as_object().unwrap().len(), 1);
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
        let root = std::env::temp_dir().join(format!("agent-bus-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let state = root.join("agent-bus");
        let path = state.join("nats").join("nats.conf");
        write(&path, &agents).unwrap();
        // A second write replaces the first, even over a file that is too strict, and leaves no
        // temporary file.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        write(&path, &agents).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), FILE_MODE);
        assert_eq!(mode(path.parent().unwrap()), DIR_MODE);
        assert_eq!(mode(&state), STATE_DIR_MODE);
        assert_eq!(fs::read_to_string(&path).unwrap(), render(&agents).unwrap());
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        // Replaced by a rename: the container sees the new file through the mounted directory.
        assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
        fs::remove_dir_all(&root).unwrap();
    }
}
