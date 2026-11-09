//! The bus's TLS: a private CA of its own, and the server certificate it signs.
//!
//! `bus start` makes the CA once, in `tls/` in the host's state directory, and issues the server
//! certificate into the config directory the container mounts, next to `nats.conf`. The server
//! certificate names the address the bus listens on, which is the address the host and the guests
//! connect to. It is issued again when that address changes, when it is close to expiry, when the
//! CA is new, or when its files or the record of what it names are missing.
//!
//! Every client connects with TLS first and trusts only this CA: no system roots. The host's
//! commands read `tls/ca.pem`; a guest gets the CA certificate on the SSH stdin pipe that starts
//! it, and is pointed at it by `AGENT_BUS_CA`. There are no client certificates: `NKeys`
//! authenticate the clients.
//!
//! The CA's certificate is not parsed back to sign with: it is rebuilt from its key and its fixed
//! subject, which is all a certificate it issues takes from it.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use jiff::{SignedDuration, Timestamp};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::instrument;

use crate::files::{PRIVATE_FILE_MODE, private_dir, standard_state_dir, write_atomic};

/// The directory, in the state directory, holding the CA and the record of the server
/// certificate. It is not mounted into the container.
pub const TLS_DIR: &str = "tls";
/// The CA certificate, which every client trusts.
pub const CA_CERT_FILE: &str = "ca.pem";
const CA_KEY_FILE: &str = "ca-key.pem";
/// What the current server certificate names, and until when.
const ISSUED_FILE: &str = "server.toml";
/// The server certificate and key, in the config directory the container mounts.
pub const SERVER_CERT_FILE: &str = "server.pem";
pub const SERVER_KEY_FILE: &str = "server-key.pem";
/// The environment variable naming the CA certificate a client trusts, when it is not the
/// host's `tls/ca.pem`.
pub const CA_ENV: &str = "AGENT_BUS_CA";
/// Certificates are not secret: readable by everyone, like the config they sit next to.
const CERT_MODE: u32 = 0o644;

const CA_COMMON_NAME: &str = "agent-bus CA";
const SERVER_COMMON_NAME: &str = "agent-bus";
const ORGANIZATION: &str = "Sherpa agent-bus";
/// About ten years: the CA is made once, and replacing it means restarting every guest.
const CA_VALIDITY: SignedDuration = SignedDuration::from_hours(10 * 365 * 24);
const SERVER_VALIDITY: SignedDuration = SignedDuration::from_hours(365 * 24);
/// A server certificate expiring sooner than this is issued again by `bus start`.
const RENEW_BEFORE: SignedDuration = SignedDuration::from_hours(30 * 24);
/// Certificates start this long before they are made, so a guest whose clock is a little behind
/// the host's still accepts them.
const BACKDATE: SignedDuration = SignedDuration::from_hours(1);

/// Where the bus's TLS files are.
#[derive(Debug, Clone)]
pub struct TlsFiles {
    pub ca_cert: PathBuf,
    ca_key: PathBuf,
    issued: PathBuf,
    pub server_cert: PathBuf,
    pub server_key: PathBuf,
}

/// What the current server certificate names, and until when; kept so `bus start` can tell
/// whether to issue it again without parsing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Issued {
    addresses: Vec<IpAddr>,
    not_after: Timestamp,
}

/// Why the server certificate is issued again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reissue {
    /// There is no certificate, no key, or no record of what it names.
    Missing,
    /// The CA was just made, so the old certificate does not chain to it.
    NewCa,
    /// The bus listens on another address than the certificate names.
    Addresses,
    /// It expires within `RENEW_BEFORE`.
    Expiring,
}

/// What `ensure` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub ca_created: bool,
    pub reissued: Option<Reissue>,
}

impl fmt::Display for Reissue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "there was none"),
            Self::NewCa => write!(f, "the CA is new"),
            Self::Addresses => write!(f, "the listen address changed"),
            Self::Expiring => write!(f, "the old one was near expiry"),
        }
    }
}

impl TlsFiles {
    /// The CA and the record in `tls_dir`; the server certificate and key in `server_dir`, the
    /// directory the container mounts.
    pub fn new(tls_dir: &Path, server_dir: &Path) -> Self {
        Self {
            ca_cert: tls_dir.join(CA_CERT_FILE),
            ca_key: tls_dir.join(CA_KEY_FILE),
            issued: tls_dir.join(ISSUED_FILE),
            server_cert: server_dir.join(SERVER_CERT_FILE),
            server_key: server_dir.join(SERVER_KEY_FILE),
        }
    }
}

/// The CA certificate a client trusts: `AGENT_BUS_CA`, then the host's `tls/ca.pem`.
///
/// # Errors
///
/// Fails when `AGENT_BUS_CA` is not set and there is no state directory.
pub fn client_ca() -> Result<PathBuf> {
    client_ca_in(std::env::var_os(CA_ENV), standard_state_dir)
}

/// `environment` when set, else `tls/ca.pem` in the state directory.
fn client_ca_in(
    environment: Option<OsString>,
    state_dir: impl FnOnce() -> Result<PathBuf>,
) -> Result<PathBuf> {
    match environment.filter(|path| !path.is_empty()) {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(state_dir()?.join(TLS_DIR).join(CA_CERT_FILE)),
    }
}

/// Make the CA if there is none, and issue the server certificate for `addresses` if
/// `reissue_reason` says so. Private keys are 0600, certificates 0644; `tls/` is 0700.
///
/// # Errors
///
/// Fails when only one of the CA's files exists, the CA key cannot be read, or a file cannot be
/// written.
#[instrument(level = "debug", skip(files), fields(ca = %files.ca_cert.display()))]
pub fn ensure(files: &TlsFiles, addresses: &[IpAddr]) -> Result<Outcome> {
    if let Some(dir) = files.ca_cert.parent() {
        private_dir(dir)?;
    }
    if let Some(dir) = files.server_cert.parent() {
        fs::create_dir_all(dir).with_context(|| format!("Unable to create {}", dir.display()))?;
    }
    let now = Timestamp::now();
    let (ca_key, ca_created) = load_or_create_ca(files, now)?;
    let present = files.server_cert.is_file() && files.server_key.is_file();
    let reason = reissue_reason(
        read_issued(&files.issued).as_ref(),
        present,
        ca_created,
        addresses,
        now,
    );
    if reason.is_some() {
        // The record goes first and comes back last: a start interrupted in between finds no
        // record, and issues again.
        remove_file(&files.issued)?;
        let (cert, key, issued) = issue_server(&ca_key, addresses, now)?;
        write_atomic(&files.server_key, &key, PRIVATE_FILE_MODE)?;
        write_atomic(&files.server_cert, &cert, CERT_MODE)?;
        let text = toml::to_string_pretty(&issued)
            .context("Unable to render the server certificate record")?;
        write_atomic(&files.issued, &text, PRIVATE_FILE_MODE)?;
    }
    Ok(Outcome {
        ca_created,
        reissued: reason,
    })
}

/// Why the server certificate must be issued again, or `None` to keep it. `issued` is the record
/// of the current one, `present` whether its certificate and key are both there, and `ca_created`
/// whether the CA was just made.
fn reissue_reason(
    issued: Option<&Issued>,
    present: bool,
    ca_created: bool,
    addresses: &[IpAddr],
    now: Timestamp,
) -> Option<Reissue> {
    let Some(issued) = issued.filter(|_| present) else {
        return Some(Reissue::Missing);
    };
    if ca_created {
        return Some(Reissue::NewCa);
    }
    let mut named = issued.addresses.clone();
    let mut wanted = addresses.to_vec();
    named.sort_unstable();
    named.dedup();
    wanted.sort_unstable();
    wanted.dedup();
    if named != wanted {
        return Some(Reissue::Addresses);
    }
    if issued.not_after.duration_since(now) < RENEW_BEFORE {
        return Some(Reissue::Expiring);
    }
    None
}

/// The CA's key, loaded, or made with the CA's certificate when neither file exists; and
/// whether it was made.
fn load_or_create_ca(files: &TlsFiles, now: Timestamp) -> Result<(KeyPair, bool)> {
    match (files.ca_key.is_file(), files.ca_cert.is_file()) {
        (true, true) => {
            let pem = fs::read_to_string(&files.ca_key)
                .with_context(|| format!("Unable to read {}", files.ca_key.display()))?;
            let key = KeyPair::from_pem(&pem)
                .with_context(|| format!("{} is not a private key", files.ca_key.display()))?;
            Ok((key, false))
        }
        (false, false) => {
            let (cert, key) = new_ca(now)?;
            write_atomic(&files.ca_key, &key.serialize_pem(), PRIVATE_FILE_MODE)?;
            write_atomic(&files.ca_cert, &cert, CERT_MODE)?;
            Ok((key, true))
        }
        (has_key, _) => {
            let (there, missing) = if has_key {
                (&files.ca_key, &files.ca_cert)
            } else {
                (&files.ca_cert, &files.ca_key)
            };
            bail!(
                "{} exists but {} does not. Remove {} too to make a new bus CA, then start every \
                 guest agent again so it gets the new CA certificate",
                there.display(),
                missing.display(),
                there.display()
            )
        }
    }
}

/// The record at `path`; `None` when it is missing or unreadable, which issues the certificate
/// again.
fn read_issued(path: &Path) -> Option<Issued> {
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("Unable to remove {}", path.display())),
    }
}

/// The CA's subject and constraints; the same every time, so the CA can be rebuilt from its key.
fn ca_params() -> Result<CertificateParams> {
    let mut params =
        CertificateParams::new(vec![]).context("Unable to create the CA certificate parameters")?;
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, CA_COMMON_NAME);
    name.push(DnType::OrganizationName, ORGANIZATION);
    params.distinguished_name = name;
    // It signs server certificates only, never another CA.
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    Ok(params)
}

/// A new CA: its certificate, as PEM, and its key.
fn new_ca(now: Timestamp) -> Result<(String, KeyPair)> {
    let key = KeyPair::generate().context("Unable to generate the CA key")?;
    let mut params = ca_params()?;
    set_validity(&mut params, now, CA_VALIDITY)?;
    let cert = params
        .self_signed(&key)
        .context("Unable to make the CA certificate")?;
    Ok((cert.pem(), key))
}

/// The CA rebuilt from its key, to sign with.
fn rebuilt_ca(key: &KeyPair) -> Result<Certificate> {
    ca_params()?
        .self_signed(key)
        .context("Unable to rebuild the CA certificate from its key")
}

/// A server certificate for `addresses` signed by the CA with `ca_key`: the certificate and key,
/// as PEM, and its record.
fn issue_server(
    ca_key: &KeyPair,
    addresses: &[IpAddr],
    now: Timestamp,
) -> Result<(String, String, Issued)> {
    let ca = rebuilt_ca(ca_key)?;
    let key = KeyPair::generate().context("Unable to generate the server key")?;
    let mut params = CertificateParams::new(vec![])
        .context("Unable to create the server certificate parameters")?;
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, SERVER_COMMON_NAME);
    name.push(DnType::OrganizationName, ORGANIZATION);
    params.distinguished_name = name;
    params.subject_alt_names = addresses
        .iter()
        .map(|address| SanType::IpAddress(*address))
        .collect();
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    let not_after = set_validity(&mut params, now, SERVER_VALIDITY)?;
    let cert = params
        .signed_by(&key, &ca, ca_key)
        .context("Unable to sign the server certificate")?;
    Ok((
        cert.pem(),
        key.serialize_pem(),
        Issued {
            addresses: addresses.to_vec(),
            not_after,
        },
    ))
}

/// Make `params` valid from `BACKDATE` before `now` for `validity`, and return its end. rcgen
/// takes `time::OffsetDateTime`, so the times are converted through Unix seconds.
fn set_validity(
    params: &mut CertificateParams,
    now: Timestamp,
    validity: SignedDuration,
) -> Result<Timestamp> {
    let start = now
        .checked_sub(BACKDATE)
        .context("Unable to calculate the certificate's start")?;
    let end = now
        .checked_add(validity)
        .context("Unable to calculate the certificate's expiry")?;
    params.not_before = OffsetDateTime::from_unix_timestamp(start.as_second())
        .context("Unable to convert the certificate's start")?;
    params.not_after = OffsetDateTime::from_unix_timestamp(end.as_second())
        .context("Unable to convert the certificate's expiry")?;
    Timestamp::from_second(end.as_second()).context("Unable to round the certificate's expiry")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::net::Ipv4Addr;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use async_nats::rustls::RootCertStore;
    use async_nats::rustls::client::WebPkiServerVerifier;
    use async_nats::rustls::client::danger::ServerCertVerifier;
    use async_nats::rustls::crypto::aws_lc_rs;
    use async_nats::rustls::pki_types::pem::PemObject;
    use async_nats::rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    const LISTEN: IpAddr = IpAddr::V4(Ipv4Addr::new(172, 31, 3, 1));

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agent-bus-tls-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn files_in(dir: &Path) -> TlsFiles {
        TlsFiles::new(&dir.join(TLS_DIR), &dir.join("nats"))
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn certificate(path: &Path) -> CertificateDer<'static> {
        CertificateDer::from_pem_file(path).unwrap()
    }

    /// Verify the server certificate in `files` for `address` as a client trusting only the CA
    /// certificate at `ca` does.
    fn verify(ca: &Path, files: &TlsFiles, address: IpAddr) -> Result<()> {
        let mut roots = RootCertStore::empty();
        roots.add(certificate(ca)).unwrap();
        let verifier = WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(aws_lc_rs::default_provider()),
        )
        .build()
        .unwrap();
        verifier
            .verify_server_cert(
                &certificate(&files.server_cert),
                &[],
                &ServerName::IpAddress(address.into()),
                &[],
                UnixTime::now(),
            )
            .map(drop)
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    fn issued(addresses: &[IpAddr], not_after: Timestamp) -> Issued {
        Issued {
            addresses: addresses.to_vec(),
            not_after,
        }
    }

    #[test]
    fn the_server_certificate_chains_to_the_ca_for_the_listen_address_only() {
        let dir = scratch("chain");
        let files = files_in(&dir);
        let outcome = ensure(&files, &[LISTEN]).unwrap();
        assert_eq!(
            outcome,
            Outcome {
                ca_created: true,
                reissued: Some(Reissue::Missing),
            }
        );
        verify(&files.ca_cert, &files, LISTEN).unwrap();
        // Another address is not in the certificate.
        let other = IpAddr::V4(Ipv4Addr::new(172, 31, 3, 2));
        let error = verify(&files.ca_cert, &files, other)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not valid for name"), "{error}");
        // Another bus's CA does not vouch for it. Every bus CA has the same subject, so the
        // other CA is found as the issuer and its signature does not match.
        let stranger = scratch("chain-stranger");
        let stranger_files = files_in(&stranger);
        ensure(&stranger_files, &[LISTEN]).unwrap();
        let error = verify(&stranger_files.ca_cert, &files, LISTEN)
            .unwrap_err()
            .to_string();
        assert!(error.contains("BadSignature"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(&stranger).unwrap();
    }

    #[test]
    fn keys_are_private_and_certificates_readable() {
        let dir = scratch("modes");
        let files = files_in(&dir);
        ensure(&files, &[LISTEN]).unwrap();
        assert_eq!(mode(&files.ca_key), PRIVATE_FILE_MODE);
        assert_eq!(mode(&files.server_key), PRIVATE_FILE_MODE);
        assert_eq!(mode(&files.ca_cert), CERT_MODE);
        assert_eq!(mode(&files.server_cert), CERT_MODE);
        assert_eq!(mode(&dir.join(TLS_DIR)), 0o700);
        // No key is in a certificate file, and the CA key is not in the mounted directory.
        assert!(
            !fs::read_to_string(&files.server_cert)
                .unwrap()
                .contains("PRIVATE KEY")
        );
        assert!(!dir.join("nats").join(CA_KEY_FILE).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_later_start_keeps_the_ca_and_the_certificate_until_the_address_changes() {
        let dir = scratch("reuse");
        let files = files_in(&dir);
        ensure(&files, &[LISTEN]).unwrap();
        let ca = fs::read_to_string(&files.ca_cert).unwrap();
        let server = fs::read_to_string(&files.server_cert).unwrap();
        let outcome = ensure(&files, &[LISTEN]).unwrap();
        assert_eq!(
            outcome,
            Outcome {
                ca_created: false,
                reissued: None,
            }
        );
        assert_eq!(fs::read_to_string(&files.server_cert).unwrap(), server);
        // A new address: a new certificate from the same CA, which a client trusting the old
        // CA certificate accepts.
        let moved = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let outcome = ensure(&files, &[moved]).unwrap();
        assert_eq!(outcome.reissued, Some(Reissue::Addresses));
        assert!(!outcome.ca_created);
        assert_eq!(fs::read_to_string(&files.ca_cert).unwrap(), ca);
        verify(&files.ca_cert, &files, moved).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lost_server_key_or_record_issues_again_and_half_a_ca_is_refused() {
        let dir = scratch("lost");
        let files = files_in(&dir);
        ensure(&files, &[LISTEN]).unwrap();
        fs::remove_file(&files.server_key).unwrap();
        assert_eq!(
            ensure(&files, &[LISTEN]).unwrap().reissued,
            Some(Reissue::Missing)
        );
        fs::remove_file(&files.issued).unwrap();
        assert_eq!(
            ensure(&files, &[LISTEN]).unwrap().reissued,
            Some(Reissue::Missing)
        );
        verify(&files.ca_cert, &files, LISTEN).unwrap();
        fs::remove_file(&files.ca_cert).unwrap();
        let error = ensure(&files, &[LISTEN]).unwrap_err().to_string();
        assert!(error.contains(CA_CERT_FILE), "{error}");
        assert!(error.contains("start every guest agent again"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_new_ca_issues_the_server_certificate_again() {
        let dir = scratch("new-ca");
        let files = files_in(&dir);
        ensure(&files, &[LISTEN]).unwrap();
        fs::remove_file(&files.ca_cert).unwrap();
        fs::remove_file(&files.ca_key).unwrap();
        let outcome = ensure(&files, &[LISTEN]).unwrap();
        assert_eq!(
            outcome,
            Outcome {
                ca_created: true,
                reissued: Some(Reissue::NewCa),
            }
        );
        verify(&files.ca_cert, &files, LISTEN).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_reissue_decision() {
        let now = Timestamp::from_second(1_800_000_000).unwrap();
        let later = |days: i64| {
            now.checked_add(SignedDuration::from_hours(days * 24))
                .unwrap()
        };
        let current = issued(&[LISTEN], later(200));
        let reason = |issued: Option<&Issued>, present, ca_created, addresses: &[IpAddr]| {
            reissue_reason(issued, present, ca_created, addresses, now)
        };
        assert_eq!(reason(Some(&current), true, false, &[LISTEN]), None);
        assert_eq!(reason(None, true, false, &[LISTEN]), Some(Reissue::Missing));
        assert_eq!(
            reason(Some(&current), false, false, &[LISTEN]),
            Some(Reissue::Missing)
        );
        assert_eq!(
            reason(Some(&current), true, true, &[LISTEN]),
            Some(Reissue::NewCa)
        );
        let other = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        assert_eq!(
            reason(Some(&current), true, false, &[other]),
            Some(Reissue::Addresses)
        );
        assert_eq!(
            reason(Some(&current), true, false, &[LISTEN, other]),
            Some(Reissue::Addresses)
        );
        // The order and repeats of the addresses do not matter.
        let both = issued(&[other, LISTEN], later(200));
        assert_eq!(
            reason(Some(&both), true, false, &[LISTEN, other, LISTEN]),
            None
        );
        // Within 30 days of expiry, or past it, it is issued again; 31 days out it is kept.
        assert_eq!(
            reason(Some(&issued(&[LISTEN], later(31))), true, false, &[LISTEN]),
            None
        );
        assert_eq!(
            reason(Some(&issued(&[LISTEN], later(29))), true, false, &[LISTEN]),
            Some(Reissue::Expiring)
        );
        assert_eq!(
            reason(Some(&issued(&[LISTEN], later(-1))), true, false, &[LISTEN]),
            Some(Reissue::Expiring)
        );
    }

    #[test]
    fn the_client_trusts_the_named_ca_else_the_hosts() {
        let state = || Ok(PathBuf::from("/home/u/.local/state/agent-bus"));
        let hosts = PathBuf::from("/home/u/.local/state/agent-bus/tls/ca.pem");
        assert_eq!(client_ca_in(None, state).unwrap(), hosts);
        assert_eq!(client_ca_in(Some(OsString::new()), state).unwrap(), hosts);
        // A named CA needs no state directory.
        let none = || anyhow::bail!("no state directory");
        assert_eq!(
            client_ca_in(Some(OsString::from("/tmp/bus-ca.pem")), none).unwrap(),
            PathBuf::from("/tmp/bus-ca.pem")
        );
        assert!(client_ca_in(None, none).is_err());
    }
}
