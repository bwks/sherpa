//! The lab certificate authority: one CA per lab, which issues every node a certificate for
//! its names and management addresses.
//!
//! The CA is saved in the lab's `certs` directory, so later operations (`redeploy`) rebuild it
//! from its key and issue certificates that chain to the same CA certificate.

use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use anyhow::{Context, Result};
use tracing::instrument;

use shared::konst::{
    LAB_CA_CERT_FILE, LAB_CA_KEY_FILE, LAB_CERT_VALIDITY_DAYS, SHERPA_DOMAIN_NAME,
};

use super::generator::{
    CaSpec, KeyAlgorithm, PemPair, ServerSubject, generate_ca, issue_server_certificate,
    write_file_with_permissions,
};

/// Organization of the lab CA and of the certificates it issues.
const LAB_CA_ORGANIZATION: &str = "Sherpa";

/// A lab's CA: its PEM certificate and key, and the subject to rebuild it with.
#[derive(Clone, Debug)]
pub struct LabCa {
    common_name: String,
    /// The CA certificate and key, PEM. The certificate is handed to clients that should trust
    /// lab nodes.
    pub pair: PemPair,
}

/// Paths to a node's TLS files on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeCertPaths {
    pub ca_cert: String,
    pub node_cert: String,
    pub node_key: String,
}

/// The names and addresses a node's certificate is issued for.
#[derive(Clone, Copy, Debug)]
pub struct NodeIdentity<'a> {
    pub name: &'a str,
    pub lab_id: &'a str,
    pub ipv4: Ipv4Addr,
    pub ipv6: Option<Ipv6Addr>,
}

fn ca_common_name(lab_id: &str) -> String {
    format!("Sherpa Lab CA - {lab_id}")
}

fn lab_ca_spec(common_name: &str) -> CaSpec<'_> {
    CaSpec {
        common_name: Some(common_name),
        organization: Some(LAB_CA_ORGANIZATION),
        key_algorithm: KeyAlgorithm::EcdsaP256,
    }
}

/// Generate a lab's CA and save it in `certs_dir`: the certificate world-readable, the key
/// readable only by the server.
#[instrument(fields(%lab_id, certs_dir = %certs_dir.display()))]
pub fn create_lab_ca(certs_dir: &Path, lab_id: &str) -> Result<LabCa> {
    let common_name = ca_common_name(lab_id);
    let pair = generate_ca(&lab_ca_spec(&common_name), LAB_CERT_VALIDITY_DAYS)
        .with_context(|| format!("Failed to generate the CA of lab {lab_id}"))?;
    let ca = LabCa { common_name, pair };
    write_file_with_permissions(
        &certs_dir.join(LAB_CA_CERT_FILE),
        ca.pair.cert.as_bytes(),
        0o644,
    )
    .context("Failed to write lab CA certificate")?;
    write_file_with_permissions(
        &certs_dir.join(LAB_CA_KEY_FILE),
        ca.pair.key.as_bytes(),
        0o600,
    )
    .context("Failed to write lab CA key")?;
    Ok(ca)
}

/// Load the CA `create_lab_ca` saved for a lab.
#[instrument(level = "debug", fields(%lab_id, certs_dir = %certs_dir.display()))]
pub fn load_lab_ca(certs_dir: &Path, lab_id: &str) -> Result<LabCa> {
    let cert_path = certs_dir.join(LAB_CA_CERT_FILE);
    let key_path = certs_dir.join(LAB_CA_KEY_FILE);
    Ok(LabCa {
        common_name: ca_common_name(lab_id),
        pair: PemPair {
            cert: fs::read_to_string(&cert_path)
                .with_context(|| format!("Failed to read {}", cert_path.display()))?,
            key: fs::read_to_string(&key_path)
                .with_context(|| format!("Failed to read {}", key_path.display()))?,
        },
    })
}

/// A node's certificate names: its name, its name in the lab and its FQDN, and its management
/// addresses.
pub fn node_certificate_names(node: NodeIdentity) -> (Vec<String>, Vec<IpAddr>) {
    let dns_names = vec![
        node.name.to_owned(),
        format!("{}.{}", node.name, node.lab_id),
        format!("{}.{}.{SHERPA_DOMAIN_NAME}", node.name, node.lab_id),
    ];
    let mut ip_addresses = vec![IpAddr::V4(node.ipv4)];
    if let Some(ipv6) = node.ipv6 {
        ip_addresses.push(IpAddr::V6(ipv6));
    }
    (dns_names, ip_addresses)
}

/// Issue a node's certificate and save it in `certs_dir` as `<node>.crt` and `<node>.key`,
/// replacing any earlier one.
#[instrument(skip(ca), fields(node = %node.name, certs_dir = %certs_dir.display()))]
pub fn issue_node_certificate(
    ca: &LabCa,
    certs_dir: &Path,
    node: NodeIdentity,
) -> Result<NodeCertPaths> {
    let (dns_names, ip_addresses) = node_certificate_names(node);
    let pair = issue_server_certificate(
        &lab_ca_spec(&ca.common_name),
        &ca.pair.key,
        ServerSubject {
            common_name: node.name,
            organization: LAB_CA_ORGANIZATION,
            dns_names: &dns_names,
            ip_addresses: &ip_addresses,
        },
        KeyAlgorithm::EcdsaP256,
        LAB_CERT_VALIDITY_DAYS,
    )
    .with_context(|| format!("Failed to issue the certificate of node '{}'", node.name))?;
    let cert_path = certs_dir.join(format!("{}.crt", node.name));
    let key_path = certs_dir.join(format!("{}.key", node.name));
    write_file_with_permissions(&cert_path, pair.cert.as_bytes(), 0o644)
        .with_context(|| format!("Failed to write the certificate of node '{}'", node.name))?;
    write_file_with_permissions(&key_path, pair.key.as_bytes(), 0o600)
        .with_context(|| format!("Failed to write the key of node '{}'", node.name))?;
    Ok(NodeCertPaths {
        ca_cert: certs_dir
            .join(LAB_CA_CERT_FILE)
            .to_string_lossy()
            .into_owned(),
        node_cert: cert_path.to_string_lossy().into_owned(),
        node_key: key_path.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;
    use x509_parser::pem::parse_x509_pem;

    fn identity(ipv6: Option<Ipv6Addr>) -> NodeIdentity<'static> {
        NodeIdentity {
            name: "vault01",
            lab_id: "abcd1234",
            ipv4: Ipv4Addr::new(172, 31, 1, 21),
            ipv6,
        }
    }

    #[test]
    fn node_certificates_name_the_node_in_the_lab_and_its_addresses() {
        let (dns_names, ip_addresses) =
            node_certificate_names(identity(Some("fd00:b00b:0:1::15".parse().unwrap())));
        assert_eq!(
            dns_names,
            [
                "vault01",
                "vault01.abcd1234",
                "vault01.abcd1234.sherpa.lab.local"
            ]
        );
        assert_eq!(
            ip_addresses,
            [
                IpAddr::V4(Ipv4Addr::new(172, 31, 1, 21)),
                "fd00:b00b:0:1::15".parse::<IpAddr>().unwrap(),
            ]
        );
        let (_, without_ipv6) = node_certificate_names(identity(None));
        assert_eq!(without_ipv6.len(), 1);
    }

    #[test]
    fn a_reloaded_ca_issues_certificates_that_chain_to_the_saved_ca() {
        let dir = TempDir::new().unwrap();
        let created = create_lab_ca(dir.path(), "abcd1234").unwrap();
        let mode = fs::metadata(dir.path().join(LAB_CA_KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        let loaded = load_lab_ca(dir.path(), "abcd1234").unwrap();
        assert_eq!(loaded.pair.cert, created.pair.cert);
        let paths = issue_node_certificate(&loaded, dir.path(), identity(None)).unwrap();
        assert_eq!(
            paths.ca_cert,
            dir.path().join(LAB_CA_CERT_FILE).to_string_lossy()
        );
        let key_mode = fs::metadata(&paths.node_key).unwrap().permissions().mode();
        assert_eq!(key_mode & 0o777, 0o600);

        let (_, ca_pem) = parse_x509_pem(created.pair.cert.as_bytes()).unwrap();
        let ca_cert = ca_pem.parse_x509().unwrap();
        let node_cert_pem = fs::read_to_string(&paths.node_cert).unwrap();
        let (_, node_pem) = parse_x509_pem(node_cert_pem.as_bytes()).unwrap();
        let node_cert = node_pem.parse_x509().unwrap();
        assert_eq!(node_cert.issuer().as_raw(), ca_cert.subject().as_raw());
        node_cert
            .verify_signature(Some(ca_cert.public_key()))
            .unwrap();
    }

    #[test]
    fn loading_a_lab_without_a_ca_fails() {
        let dir = TempDir::new().unwrap();
        assert!(load_lab_ca(dir.path(), "abcd1234").is_err());
    }
}
