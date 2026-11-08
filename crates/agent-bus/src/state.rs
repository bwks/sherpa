//! Where the bus is, for the commands that connect to it.
//!
//! `bus start` records the bus's URL (and the lab it serves, if started with `--lab`) in
//! `$XDG_STATE_HOME/agent-bus/bus.toml`, next to the server config. `send`, `listen` and `cancel`
//! read it when they are given no server.

use std::fs;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use shared::data::LabInfo;

/// The environment variable that names the bus.
pub const SERVER_ENV: &str = "AGENT_BUS_URL";

/// The lab the bus serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabRecord {
    pub id: String,
    pub name: String,
}

/// What `bus start` records about the bus it started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusRecord {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lab: Option<LabRecord>,
}

/// Read the lab's `lab-info.toml` in `lab_dir`.
///
/// # Errors
///
/// Fails, naming the file, when it is missing or is not a lab info file with a gateway.
pub fn read_lab_info(lab_dir: &Path) -> Result<LabInfo> {
    let path = lab_dir.join(shared::konst::LAB_FILE_NAME);
    let text = fs::read_to_string(&path)
        .with_context(|| format!("Unable to read {}: is it a lab directory?", path.display()))?;
    LabInfo::from_str(&text).with_context(|| {
        format!(
            "{} is not a lab info file with an ipv4_gateway",
            path.display()
        )
    })
}

/// The address to listen on for the lab in `lab_dir`: its management bridge address.
///
/// # Errors
///
/// Fails when the lab info cannot be read.
pub fn lab_gateway(lab_dir: &Path) -> Result<(Ipv4Addr, LabRecord)> {
    let info = read_lab_info(lab_dir)?;
    Ok((
        info.ipv4_gateway,
        LabRecord {
            id: info.id,
            name: info.name,
        },
    ))
}

/// Write the record to `path`, atomically: a reader sees the old file or the new one.
///
/// # Errors
///
/// Fails when the file cannot be written.
pub fn write(path: &Path, record: &BusRecord) -> Result<()> {
    let text = toml::to_string_pretty(record).context("Unable to render the bus record")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("Unable to create {}", dir.display()))?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, text)
        .with_context(|| format!("Unable to write {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| {
        let _ = fs::remove_file(&temporary);
        format!("Unable to replace {}", path.display())
    })
}

/// Read the record at `path`; `None` when there is no file.
///
/// # Errors
///
/// Fails when the file exists but cannot be read or parsed.
pub fn read(path: &Path) -> Result<Option<BusRecord>> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map(Some)
            .with_context(|| format!("{} is not a bus record", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Unable to read {}", path.display())),
    }
}

/// Remove the record at `path`. No file is fine.
///
/// # Errors
///
/// Fails when the file exists and cannot be removed.
pub fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Unable to remove {}", path.display())),
    }
}

/// Find the bus: the `--server` flag, then `AGENT_BUS_URL`, then the record `bus start` wrote at
/// `record_path`.
///
/// # Errors
///
/// Fails when none gives a server, or the record cannot be read.
pub fn find_server(
    flag: Option<String>,
    environment: Option<String>,
    record_path: &Path,
) -> Result<String> {
    if let Some(url) = flag.or(environment).filter(|url| !url.is_empty()) {
        return Ok(url);
    }
    match read(record_path)? {
        Some(record) => Ok(record.url),
        None => bail!(
            "No bus to connect to: run `agent-bus bus start`, or set --server or {SERVER_ENV} (looked for {})",
            record_path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAB_INFO: &str = r#"
id = "abc123"
name = "dev"
user = "sherpa"
ipv4_network = "192.0.2.0/24"
ipv4_gateway = "192.0.2.1"
ipv4_router = "192.0.2.2"
loopback_network = "198.51.100.0/24"
"#;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agent-bus-state-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(url: &str) -> BusRecord {
        BusRecord {
            url: url.to_owned(),
            lab: Some(LabRecord {
                id: "abc123".to_owned(),
                name: "dev".to_owned(),
            }),
        }
    }

    #[test]
    fn the_gateway_and_lab_come_from_lab_info() {
        let dir = scratch("gateway");
        fs::write(dir.join("lab-info.toml"), LAB_INFO).unwrap();
        let (gateway, lab) = lab_gateway(&dir).unwrap();
        assert_eq!(gateway, "192.0.2.1".parse::<Ipv4Addr>().unwrap());
        assert_eq!(lab.id, "abc123");
        assert_eq!(lab.name, "dev");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_gatewayless_lab_info_names_the_file() {
        let dir = scratch("missing");
        let path = dir.join("lab-info.toml");
        let error = format!("{:#}", lab_gateway(&dir).unwrap_err());
        assert!(error.contains(&path.display().to_string()), "{error}");

        fs::write(
            &path,
            LAB_INFO.replace("ipv4_gateway = \"192.0.2.1\"\n", ""),
        )
        .unwrap();
        let error = format!("{:#}", lab_gateway(&dir).unwrap_err());
        assert!(error.contains(&path.display().to_string()), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_record_is_written_read_and_removed() {
        let dir = scratch("record");
        let path = dir.join("state").join("bus.toml");
        assert_eq!(read(&path).unwrap(), None);
        write(&path, &record("nats://192.0.2.1:4222")).unwrap();
        assert_eq!(read(&path).unwrap(), Some(record("nats://192.0.2.1:4222")));
        // A second write replaces the first and leaves no temporary file.
        let without_lab = BusRecord {
            url: "nats://192.0.2.9:4222".to_owned(),
            lab: None,
        };
        write(&path, &without_lab).unwrap();
        assert_eq!(read(&path).unwrap(), Some(without_lab));
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        remove(&path).unwrap();
        remove(&path).unwrap();
        assert_eq!(read(&path).unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_server_is_the_flag_then_the_environment_then_the_record() {
        let dir = scratch("lookup");
        let path = dir.join("bus.toml");
        let some = |url: &str| Some(url.to_owned());

        // Nothing set: an error that says what to do.
        let error = format!("{:#}", find_server(None, None, &path).unwrap_err());
        assert!(
            error.contains("bus start") && error.contains(SERVER_ENV),
            "{error}"
        );

        write(&path, &record("nats://from-record:4222")).unwrap();
        assert_eq!(
            find_server(None, None, &path).unwrap(),
            "nats://from-record:4222"
        );
        assert_eq!(
            find_server(None, some("nats://from-env:4222"), &path).unwrap(),
            "nats://from-env:4222"
        );
        assert_eq!(
            find_server(
                some("nats://from-flag:4222"),
                some("nats://from-env:4222"),
                &path
            )
            .unwrap(),
            "nats://from-flag:4222"
        );
        // An empty variable counts as unset.
        assert_eq!(
            find_server(None, some(""), &path).unwrap(),
            "nats://from-record:4222"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
