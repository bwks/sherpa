use anyhow::{Context, Result};
use rand::RngCore;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, PKCS_ED25519, SanType, SerialNumber,
};
use shared::util::write_file_with_permissions;
use std::net::IpAddr;
use std::path::Path;
use tracing::instrument;

/// Write a self-signed server certificate for `key_pair` to `cert_path`, valid from now for
/// `validity_days` and naming `san` (entries `IP:<address>`, `DNS:<name>` or a bare DNS name),
/// localhost and 127.0.0.1. The file is written to a temporary file and renamed over the old one,
/// so it is never left half written.
#[instrument(level = "debug", skip(key_pair), fields(cert_path = %cert_path.display()))]
pub fn write_self_signed_certificate(
    cert_path: &Path,
    key_pair: &KeyPair,
    san: &[String],
    validity_days: u32,
) -> Result<()> {
    let mut params =
        CertificateParams::new(vec![]).context("Failed to create certificate parameters")?;

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Sherpa Server");
    dn.push(DnType::OrganizationName, "Sherpa");
    params.distinguished_name = dn;

    for san_entry in san {
        if let Some(ip_str) = san_entry.strip_prefix("IP:") {
            if let Ok(ip) = ip_str.parse() {
                params.subject_alt_names.push(SanType::IpAddress(ip));
            } else {
                tracing::warn!("Invalid IP in SAN: {}", ip_str);
            }
        } else {
            let dns = san_entry.strip_prefix("DNS:").unwrap_or(san_entry);
            params.subject_alt_names.push(SanType::DnsName(
                dns.to_string()
                    .try_into()
                    .with_context(|| format!("Invalid DNS SAN: {dns}"))?,
            ));
        }
    }

    if !san.iter().any(|s| s.contains("localhost")) {
        params.subject_alt_names.push(SanType::DnsName(
            "localhost"
                .to_string()
                .try_into()
                .context("Failed to create localhost SAN")?,
        ));
    }
    if !san.iter().any(|s| s.contains("127.0.0.1")) {
        params
            .subject_alt_names
            .push(SanType::IpAddress(IpAddr::from([127, 0, 0, 1])));
    }

    set_validity(&mut params, validity_days)?;
    params.serial_number = Some(random_serial_number());

    let cert = params
        .self_signed(key_pair)
        .context("Failed to generate self-signed certificate")?;
    write_file_with_permissions(cert_path, cert.pem().as_bytes(), 0o644)
        .context("Failed to write certificate file")?;
    tracing::info!(
        cert_path = %cert_path.display(),
        validity_days,
        "Wrote a new self-signed server certificate"
    );
    Ok(())
}

/// A random positive 128-bit serial number. Without one, rcgen derives the serial from the
/// public key, so every certificate sherpad made for its kept key would share one serial, which
/// RFC 5280 forbids and browsers refuse.
fn random_serial_number() -> SerialNumber {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes[0] &= 0x7F;
    SerialNumber::from_slice(&bytes)
}

/// Key algorithm for generated keys and certificates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAlgorithm {
    EcdsaP256,
    Ed25519,
}

/// A certificate authority Sherpa can rebuild from its stored key.
///
/// A certificate it issues only needs the CA's subject and key, so the CA certificate itself
/// does not have to be parsed back.
#[derive(Clone, Copy, Debug)]
pub struct CaSpec<'a> {
    /// Common name (CN) of the CA subject, before the organization.
    pub common_name: Option<&'a str>,
    /// Organization (O) of the CA subject. With no common name either, the subject is empty.
    pub organization: Option<&'a str>,
    pub key_algorithm: KeyAlgorithm,
}

/// Subject of a client certificate.
#[derive(Clone, Copy, Debug)]
pub struct ClientSubject<'a> {
    /// Organization (O); Talos and Kubernetes read the client's role or group from it.
    pub organization: &'a str,
    pub common_name: Option<&'a str>,
}

/// Subject and names of a server certificate.
#[derive(Clone, Copy, Debug)]
pub struct ServerSubject<'a> {
    pub common_name: &'a str,
    pub organization: &'a str,
    /// DNS names and IP addresses the certificate is valid for (SANs).
    pub dns_names: &'a [String],
    pub ip_addresses: &'a [IpAddr],
}

/// A PEM certificate and its PKCS#8 PEM private key.
#[derive(Clone, Debug)]
pub struct PemPair {
    pub cert: String,
    pub key: String,
}

/// Generate a key pair with the given algorithm.
pub fn generate_key_pair(algorithm: KeyAlgorithm) -> Result<KeyPair> {
    let signature = match algorithm {
        KeyAlgorithm::EcdsaP256 => &PKCS_ECDSA_P256_SHA256,
        KeyAlgorithm::Ed25519 => &PKCS_ED25519,
    };
    KeyPair::generate_for(signature).context("Failed to generate key pair")
}

/// Set a certificate's validity to start now and last `validity_days`.
fn set_validity(params: &mut CertificateParams, validity_days: u32) -> Result<()> {
    let now = jiff::Timestamp::now();
    let future = now
        .checked_add(jiff::Span::new().hours(i64::from(validity_days) * 24))
        .context("Failed to calculate certificate expiration date")?;
    params.not_before = time::OffsetDateTime::from_unix_timestamp(now.as_second())
        .context("Failed to convert start time to OffsetDateTime")?;
    params.not_after = time::OffsetDateTime::from_unix_timestamp(future.as_second())
        .context("Failed to convert end time to OffsetDateTime")?;
    Ok(())
}

fn ca_params(spec: &CaSpec, validity_days: u32) -> Result<CertificateParams> {
    let mut params =
        CertificateParams::new(vec![]).context("Failed to create CA certificate parameters")?;
    let mut dn = DistinguishedName::new();
    if let Some(common_name) = spec.common_name {
        dn.push(DnType::CommonName, common_name);
    }
    if let Some(organization) = spec.organization {
        dn.push(DnType::OrganizationName, organization);
    }
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
    ];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    set_validity(&mut params, validity_days)?;
    Ok(params)
}

/// Generate a self-signed CA in memory.
pub fn generate_ca(spec: &CaSpec, validity_days: u32) -> Result<PemPair> {
    let key_pair = generate_key_pair(spec.key_algorithm)?;
    let cert = ca_params(spec, validity_days)?
        .self_signed(&key_pair)
        .context("Failed to generate CA certificate")?;
    Ok(PemPair {
        cert: cert.pem(),
        key: key_pair.serialize_pem(),
    })
}

/// Issue a client-authentication certificate from a CA rebuilt from its PEM key.
pub fn issue_client_certificate(
    ca: &CaSpec,
    ca_key_pem: &str,
    subject: ClientSubject,
    key_algorithm: KeyAlgorithm,
    validity_days: u32,
) -> Result<PemPair> {
    let ca_key = KeyPair::from_pem(ca_key_pem).context("Failed to load CA private key")?;
    let ca_cert = ca_params(ca, validity_days)?
        .self_signed(&ca_key)
        .context("Failed to rebuild CA certificate")?;

    let key_pair = generate_key_pair(key_algorithm)?;
    let mut params =
        CertificateParams::new(vec![]).context("Failed to create client certificate parameters")?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, subject.organization);
    if let Some(common_name) = subject.common_name {
        dn.push(DnType::CommonName, common_name);
    }
    params.distinguished_name = dn;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    set_validity(&mut params, validity_days)?;
    let cert = params
        .signed_by(&key_pair, &ca_cert, &ca_key)
        .context("Failed to sign client certificate")?;
    Ok(PemPair {
        cert: cert.pem(),
        key: key_pair.serialize_pem(),
    })
}

/// Issue a certificate for a server, also usable as a client certificate, from a CA rebuilt
/// from its PEM key.
pub fn issue_server_certificate(
    ca: &CaSpec,
    ca_key_pem: &str,
    subject: ServerSubject,
    key_algorithm: KeyAlgorithm,
    validity_days: u32,
) -> Result<PemPair> {
    let ca_key = KeyPair::from_pem(ca_key_pem).context("Failed to load CA private key")?;
    let ca_cert = ca_params(ca, validity_days)?
        .self_signed(&ca_key)
        .context("Failed to rebuild CA certificate")?;

    let key_pair = generate_key_pair(key_algorithm)?;
    let mut params =
        CertificateParams::new(vec![]).context("Failed to create server certificate parameters")?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, subject.common_name);
    dn.push(DnType::OrganizationName, subject.organization);
    params.distinguished_name = dn;
    for name in subject.dns_names {
        params.subject_alt_names.push(SanType::DnsName(
            name.clone()
                .try_into()
                .with_context(|| format!("Invalid DNS name for a certificate: {name}"))?,
        ));
    }
    for address in subject.ip_addresses {
        params.subject_alt_names.push(SanType::IpAddress(*address));
    }
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    set_validity(&mut params, validity_days)?;
    let cert = params
        .signed_by(&key_pair, &ca_cert, &ca_key)
        .context("Failed to sign server certificate")?;
    Ok(PemPair {
        cert: cert.pem(),
        key: key_pair.serialize_pem(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pem: &str) -> x509_parser::pem::Pem {
        let (_, pem) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).unwrap();
        pem
    }

    #[test]
    fn generated_cas_use_the_requested_key_and_pkcs8() {
        for (algorithm, oid) in [
            (KeyAlgorithm::Ed25519, "1.3.101.112"),
            (KeyAlgorithm::EcdsaP256, "1.2.840.10045.2.1"),
        ] {
            let spec = CaSpec {
                common_name: None,
                organization: Some("talos"),
                key_algorithm: algorithm,
            };
            let ca = generate_ca(&spec, 3650).unwrap();
            assert!(ca.key.starts_with("-----BEGIN PRIVATE KEY-----"));
            let pem = parse(&ca.cert);
            let cert = pem.parse_x509().unwrap();
            assert!(cert.is_ca());
            assert_eq!(cert.subject().to_string(), "O=talos");
            assert_eq!(
                cert.public_key().algorithm.algorithm.to_id_string(),
                oid,
                "{algorithm:?}"
            );
        }
    }

    #[test]
    fn client_certificates_chain_to_the_rebuilt_ca() {
        let spec = CaSpec {
            common_name: None,
            organization: Some("kubernetes"),
            key_algorithm: KeyAlgorithm::EcdsaP256,
        };
        let ca = generate_ca(&spec, 3650).unwrap();
        let client = issue_client_certificate(
            &spec,
            &ca.key,
            ClientSubject {
                organization: "system:masters",
                common_name: Some("sherpa-admin"),
            },
            KeyAlgorithm::EcdsaP256,
            365,
        )
        .unwrap();
        let ca_pem = parse(&ca.cert);
        let ca_cert = ca_pem.parse_x509().unwrap();
        let client_pem = parse(&client.cert);
        let client_cert = client_pem.parse_x509().unwrap();
        assert_eq!(client_cert.issuer(), ca_cert.subject());
        assert_eq!(
            client_cert.subject().to_string(),
            "O=system:masters, CN=sherpa-admin"
        );
        assert!(!client_cert.is_ca());
        client_cert
            .verify_signature(Some(ca_cert.public_key()))
            .unwrap();
    }

    #[test]
    fn server_certificates_carry_their_names_and_chain_to_the_rebuilt_ca() {
        let spec = CaSpec {
            common_name: Some("Sherpa Lab CA - lab1"),
            organization: Some("Sherpa"),
            key_algorithm: KeyAlgorithm::EcdsaP256,
        };
        let ca = generate_ca(&spec, 365).unwrap();
        let dns_names = [
            "vault01".to_owned(),
            "vault01.lab1.sherpa.lab.local".to_owned(),
        ];
        let ip_addresses: [IpAddr; 2] =
            ["172.31.1.21".parse().unwrap(), "fd00::15".parse().unwrap()];
        let server = issue_server_certificate(
            &spec,
            &ca.key,
            ServerSubject {
                common_name: "vault01",
                organization: "Sherpa",
                dns_names: &dns_names,
                ip_addresses: &ip_addresses,
            },
            KeyAlgorithm::EcdsaP256,
            365,
        )
        .unwrap();
        let ca_pem = parse(&ca.cert);
        let ca_cert = ca_pem.parse_x509().unwrap();
        assert_eq!(
            ca_cert.subject().to_string(),
            "CN=Sherpa Lab CA - lab1, O=Sherpa"
        );
        let server_pem = parse(&server.cert);
        let server_cert = server_pem.parse_x509().unwrap();
        assert_eq!(server_cert.issuer(), ca_cert.subject());
        server_cert
            .verify_signature(Some(ca_cert.public_key()))
            .unwrap();
        let usage = server_cert.extended_key_usage().unwrap().unwrap().value;
        assert!(usage.server_auth && usage.client_auth);
        let names = server_cert
            .subject_alternative_name()
            .unwrap()
            .unwrap()
            .value;
        let rendered: Vec<String> = names
            .general_names
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        assert_eq!(
            rendered,
            [
                "DNSName(vault01)",
                "DNSName(vault01.lab1.sherpa.lab.local)",
                "IPAddress(ac:1f:01:15)",
                "IPAddress(fd:00:00:00:00:00:00:00:00:00:00:00:00:00:00:15)",
            ]
        );
    }

    #[test]
    fn upgraded_labs_002_certificates_issued_now_chain_to_lab_cas_made_before_ca_specs() {
        // Labs created before the lab CA moved to `CaSpec` have a CA whose subject is
        // CN then O, with an rcgen default key. Reissuing node certificates from its saved key
        // must produce the same issuer name.
        let key_pair = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![]).unwrap();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "Sherpa Lab CA - old1");
        dn.push(DnType::OrganizationName, "Sherpa");
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let old_ca = params.self_signed(&key_pair).unwrap();

        let spec = CaSpec {
            common_name: Some("Sherpa Lab CA - old1"),
            organization: Some("Sherpa"),
            key_algorithm: KeyAlgorithm::EcdsaP256,
        };
        let server = issue_server_certificate(
            &spec,
            &key_pair.serialize_pem(),
            ServerSubject {
                common_name: "node01",
                organization: "Sherpa",
                dns_names: &["node01".to_owned()],
                ip_addresses: &[],
            },
            KeyAlgorithm::EcdsaP256,
            365,
        )
        .unwrap();
        let old_pem = old_ca.pem();
        let old_parsed = parse(&old_pem);
        let old_cert = old_parsed.parse_x509().unwrap();
        let server_pem = parse(&server.cert);
        let server_cert = server_pem.parse_x509().unwrap();
        assert_eq!(server_cert.issuer().as_raw(), old_cert.subject().as_raw());
        server_cert
            .verify_signature(Some(old_cert.public_key()))
            .unwrap();
    }
}
