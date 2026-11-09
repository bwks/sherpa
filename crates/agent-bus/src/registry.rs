//! The registry of enrolled guests: `<state>/enrolled.toml` on the host, managed by
//! `agent-bus enroll` and `unenroll`, never by hand. It has the agents file's format, with only
//! guests, and holds only public keys; it is still 0600 and written atomically, because what it
//! says decides who may connect.
//!
//! The server config is rendered from the agents file and the registry together (`merge`). The
//! agents file wins a name clash: `enroll` refuses a name the agents file lists, and a registry
//! entry whose name the agents file has since taken is left out of the config, with a warning.
//!
//! Every change to the registry, and every render of the config from it, happens under an
//! exclusive lock on `<state>/enroll.lock` (`lock`), so two commands cannot lose each other's
//! change.

use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use nix::fcntl::{Flock, FlockArg};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::agents::{Agent, Agents, Role, check_entries, parse_error};
use crate::files::{PRIVATE_FILE_MODE, private_dir, write_atomic};

/// The first line of the file.
const HEADER: &str =
    "# Guests enrolled with `agent-bus enroll`. Managed by agent-bus: do not edit.\n";

/// The enrolled guests, in the order they were first enrolled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    agents: Vec<Agent>,
}

/// The agents file and the registry together.
#[derive(Debug)]
pub struct Merged {
    pub agents: Agents,
    /// Enrolled guests left out because the agents file lists their name
    pub shadowed: Vec<String>,
}

/// The held registry lock. Dropping it releases it.
#[derive(Debug)]
pub struct RegistryLock {
    _lock: Flock<File>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    #[serde(default)]
    agent: Vec<Agent>,
}

impl Registry {
    /// Read the registry at `path`. A missing file is an empty registry.
    ///
    /// # Errors
    ///
    /// Fails when the file exists and cannot be read or is not valid.
    #[instrument(level = "debug", skip_all, fields(path = %path.display()))]
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).with_context(|| format!("Invalid {}", path.display())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("Unable to read {}", path.display())),
        }
    }

    /// Parse and validate the text of a registry: the agents file's rules, and only guests.
    ///
    /// # Errors
    ///
    /// Fails when the text is not valid TOML, lists a host, or breaks an agents file rule.
    pub fn parse(text: &str) -> Result<Self> {
        let file: RegistryFile = toml::from_str(text)
            .map_err(|error| parse_error("the registry of enrolled guests", text, &error))?;
        check_entries(&file.agent)?;
        if let Some(agent) = file.agent.iter().find(|agent| agent.role != Role::Guest) {
            return Err(anyhow!(
                "Agent '{}' in the registry is not a guest; only guests are enrolled",
                agent.name
            ));
        }
        Ok(Self { agents: file.agent })
    }

    /// The registry as the file holds it.
    ///
    /// # Errors
    ///
    /// Fails when it cannot be serialized.
    pub fn render(&self) -> Result<String> {
        let body = toml::to_string_pretty(&RegistryFile {
            agent: self.agents.clone(),
        })
        .context("Unable to render the registry of enrolled guests")?;
        Ok(format!("{HEADER}{body}"))
    }

    /// Write the registry to `path`: 0600, atomically, in a 0700 directory.
    ///
    /// # Errors
    ///
    /// Fails when the directory or the file cannot be written.
    #[instrument(level = "debug", skip_all, fields(path = %path.display()))]
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = self.render()?;
        if let Some(dir) = path.parent() {
            private_dir(dir)?;
        }
        write_atomic(path, &text, PRIVATE_FILE_MODE)
    }

    /// Enroll the guest `name` with the public key `key`, replacing its previous key, which is
    /// returned. A new guest goes last.
    pub fn enroll(&mut self, name: &str, key: &str) -> Option<String> {
        if let Some(agent) = self.agents.iter_mut().find(|agent| agent.name == name) {
            return Some(std::mem::replace(&mut agent.key, key.to_owned()));
        }
        self.agents.push(Agent {
            name: name.to_owned(),
            key: key.to_owned(),
            role: Role::Guest,
        });
        None
    }

    /// Remove the guest `name`, returning its key; `None` when it was not enrolled.
    pub fn unenroll(&mut self, name: &str) -> Option<String> {
        let index = self.agents.iter().position(|agent| agent.name == name)?;
        Some(self.agents.remove(index).key)
    }
}

/// The agents to render the server config from: the agents file's, then the enrolled guests
/// whose names the agents file does not list.
///
/// # Errors
///
/// Fails when an enrolled guest uses a key the agents file also uses.
pub fn merge(fixed: Agents, registry: &Registry) -> Result<Merged> {
    let (enrolled, shadowed): (Vec<&Agent>, Vec<&Agent>) = registry
        .agents
        .iter()
        .partition(|agent| !fixed.has(&agent.name));
    let mut guests = fixed.guests;
    guests.extend(enrolled.into_iter().cloned());
    let agents = Agents {
        host: fixed.host,
        guests,
    };
    check_entries(agents.all())
        .context("The agents file and the registry of enrolled guests clash")?;
    Ok(Merged {
        agents,
        shadowed: shadowed
            .into_iter()
            .map(|agent| agent.name.clone())
            .collect(),
    })
}

/// Take the registry lock at `path`, waiting for another holder to finish.
///
/// # Errors
///
/// Fails when the file cannot be created or locked.
#[instrument(level = "debug", skip_all, fields(path = %path.display()))]
pub fn lock(path: &Path) -> Result<RegistryLock> {
    if let Some(dir) = path.parent() {
        private_dir(dir)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(PRIVATE_FILE_MODE)
        .open(path)
        .with_context(|| format!("Unable to open {}", path.display()))?;
    let lock = Flock::lock(file, FlockArg::LockExclusive).map_err(|(_, errno)| {
        anyhow!(errno).context(format!("Unable to lock {}", path.display()))
    })?;
    Ok(RegistryLock { _lock: lock })
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    use serde_json::Value;

    use super::*;
    use crate::config::render;
    use crate::files::DIR_MODE;
    use crate::keys::generate;

    fn key() -> String {
        generate().unwrap().public_key
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agent-bus-registry-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn fixed(guests: &[(&str, &str)]) -> Agents {
        let mut text = format!(
            "[[agent]]\nname = \"host\"\nkey = \"{}\"\nrole = \"host\"\n",
            key()
        );
        for (name, key) in guests {
            writeln!(
                text,
                "[[agent]]\nname = \"{name}\"\nkey = \"{key}\"\nrole = \"guest\""
            )
            .unwrap();
        }
        Agents::parse(&text).unwrap()
    }

    fn names(agents: &Agents) -> Vec<&str> {
        agents.all().map(|agent| agent.name.as_str()).collect()
    }

    #[test]
    fn enroll_adds_then_replaces_and_unenroll_removes() {
        let mut registry = Registry::default();
        let (first, second, other) = (key(), key(), key());
        assert_eq!(registry.enroll("dev", &first), None);
        assert_eq!(registry.enroll("agent-2", &other), None);
        // Re-enrolling replaces the key in place and returns the old one.
        assert_eq!(registry.enroll("dev", &second), Some(first));
        let entries: Vec<(&str, &str, Role)> = registry
            .agents
            .iter()
            .map(|agent| (agent.name.as_str(), agent.key.as_str(), agent.role))
            .collect();
        assert_eq!(
            entries,
            [
                ("dev", second.as_str(), Role::Guest),
                ("agent-2", other.as_str(), Role::Guest)
            ]
        );
        assert_eq!(registry.unenroll("dev"), Some(second));
        assert_eq!(registry.unenroll("dev"), None);
        assert_eq!(registry.agents.len(), 1);
    }

    #[test]
    fn the_file_round_trips_and_a_missing_file_is_empty() {
        let dir = scratch("round-trip");
        let path = dir.join("agent-bus").join("enrolled.toml");
        assert_eq!(Registry::load(&path).unwrap(), Registry::default());
        let mut registry = Registry::default();
        registry.enroll("dev", &key());
        registry.enroll("agent-2", &key());
        registry.save(&path).unwrap();
        assert_eq!(Registry::load(&path).unwrap(), registry);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(HEADER) && text.contains("role = \"guest\""));
        // An emptied registry is saved and read back as empty.
        registry.unenroll("dev");
        registry.unenroll("agent-2");
        registry.save(&path).unwrap();
        assert_eq!(Registry::load(&path).unwrap(), Registry::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_file_is_written_atomically_0600_in_a_0700_directory() {
        let dir = scratch("mode");
        let state = dir.join("agent-bus");
        let path = state.join("enrolled.toml");
        let mut registry = Registry::default();
        registry.enroll("dev", &key());
        registry.save(&path).unwrap();
        assert_eq!(mode(&path), PRIVATE_FILE_MODE);
        assert_eq!(mode(&state), DIR_MODE);
        // A looser file is replaced by a 0600 one, by a rename, leaving no temporary file.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let before = fs::metadata(&path).unwrap().ino();
        registry.enroll("dev", &key());
        registry.save(&path).unwrap();
        assert_eq!(mode(&path), PRIVATE_FILE_MODE);
        assert_ne!(fs::metadata(&path).unwrap().ino(), before);
        assert_eq!(fs::read_dir(&state).unwrap().count(), 1);
        assert_eq!(Registry::load(&path).unwrap(), registry);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_host_a_shared_key_or_a_seed_in_the_registry_is_refused() {
        let host = format!(
            "[[agent]]\nname = \"boss\"\nkey = \"{}\"\nrole = \"host\"\n",
            key()
        );
        assert!(Registry::parse(&host).is_err());
        let shared = key();
        let twice = format!(
            "[[agent]]\nname = \"a\"\nkey = \"{shared}\"\nrole = \"guest\"\n\
             [[agent]]\nname = \"b\"\nkey = \"{shared}\"\nrole = \"guest\"\n"
        );
        assert!(Registry::parse(&twice).is_err());
        let seed = generate().unwrap().seed;
        let leaked = format!("[[agent]]\nname = \"a\"\nkey = \"{seed}\"\nrole = \"guest\"\n");
        let error = format!("{:#}", Registry::parse(&leaked).unwrap_err());
        assert!(!error.contains(&seed));
    }

    #[test]
    fn the_merge_adds_enrolled_guests_after_the_agents_file() {
        let fixed_key = key();
        let mut registry = Registry::default();
        registry.enroll("agent-2", &key());
        registry.enroll("agent-3", &key());
        let merged = merge(fixed(&[("dev", &fixed_key)]), &registry).unwrap();
        assert_eq!(names(&merged.agents), ["host", "dev", "agent-2", "agent-3"]);
        assert_eq!(merged.shadowed, Vec::<String>::new());
        assert_eq!(merged.agents.guests[0].key, fixed_key);
        // The merged list renders one user per agent.
        let config: Value = serde_json::from_str(&render(&merged.agents).unwrap()).unwrap();
        assert_eq!(
            config["authorization"]["users"].as_array().unwrap().len(),
            4
        );
    }

    #[test]
    fn the_agents_file_wins_a_name_clash() {
        let fixed_key = key();
        let mut registry = Registry::default();
        registry.enroll("dev", &key());
        registry.enroll("agent-2", &key());
        let merged = merge(fixed(&[("dev", &fixed_key)]), &registry).unwrap();
        assert_eq!(names(&merged.agents), ["host", "dev", "agent-2"]);
        assert_eq!(merged.agents.guests[0].key, fixed_key);
        assert_eq!(merged.shadowed, ["dev"]);
        // An enrolled guest named like the host is left out too.
        let mut registry = Registry::default();
        registry.enroll("host", &key());
        let merged = merge(fixed(&[]), &registry).unwrap();
        assert_eq!(names(&merged.agents), ["host"]);
        assert_eq!(merged.shadowed, ["host"]);
    }

    #[test]
    fn an_enrolled_key_the_agents_file_also_uses_is_refused() {
        let shared = key();
        let mut registry = Registry::default();
        registry.enroll("agent-2", &shared);
        let error = format!(
            "{:#}",
            merge(fixed(&[("dev", &shared)]), &registry).unwrap_err()
        );
        assert!(error.contains("clash"), "{error}");
        assert!(!error.contains(&shared));
    }

    #[test]
    fn the_lock_is_exclusive() {
        let dir = scratch("lock");
        let path = dir.join("enroll.lock");
        let held = lock(&path).unwrap();
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        assert!(Flock::lock(file, FlockArg::LockExclusiveNonblock).is_err());
        drop(held);
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        assert!(Flock::lock(file, FlockArg::LockExclusiveNonblock).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }
}
