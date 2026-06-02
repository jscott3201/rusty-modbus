//! `rustls` configuration builders for client and server.
//!
//! Enforces TLS 1.3 (stronger than the spec's minimum of TLS 1.2)
//! with mutual x.509v3 authentication.

use std::fs;
use std::io::BufReader;
use std::sync::Arc;

use std::sync::Once;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rusty_modbus_types::MODBUS_TLS_MAX_FRAGMENT_SIZE;
use tracing::warn;

static CRYPTO_INIT: Once = Once::new();

/// Ensure the ring crypto provider is installed exactly once.
fn ensure_crypto_provider() {
    CRYPTO_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

use crate::config::{TlsClientConfig, TlsServerConfig};
use crate::error::TlsError;

/// Build a `rustls::ClientConfig` for Modbus/TCP Security.
///
/// - TLS 1.3 enforced
/// - Mutual authentication: client always presents its certificate
/// - Server verified against the provided CA certificate
pub fn build_client_config(config: &TlsClientConfig) -> Result<ClientConfig, TlsError> {
    ensure_crypto_provider();
    let ca_certs = load_certs(&config.ca_cert)?;
    let client_certs = load_certs(&config.client_cert)?;
    let client_key = load_private_key(&config.client_key)?;

    let mut root_store = RootCertStore::empty();
    for cert in &ca_certs {
        root_store
            .add(cert.clone())
            .map_err(|e| TlsError::Certificate(format!("failed to add CA cert: {e}")))?;
    }

    let mut tls_config = ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(root_store)
        .with_client_auth_cert(client_certs, client_key)
        .map_err(|e| TlsError::Certificate(format!("client auth config failed: {e}")))?;
    tls_config.max_fragment_size = Some(MODBUS_TLS_MAX_FRAGMENT_SIZE);

    Ok(tls_config)
}

/// Build a `rustls::ServerConfig` for Modbus/TCP Security.
///
/// - TLS 1.3 enforced
/// - Mutual authentication: client certificate required (R-06)
/// - Client verified against the provided CA certificate
pub fn build_server_config(config: &TlsServerConfig) -> Result<ServerConfig, TlsError> {
    ensure_crypto_provider();
    let server_certs = load_certs(&config.server_cert)?;
    let server_key = load_private_key(&config.server_key)?;
    let ca_certs = load_certs(&config.ca_cert)?;

    let mut client_root_store = RootCertStore::empty();
    for cert in &ca_certs {
        client_root_store
            .add(cert.clone())
            .map_err(|e| TlsError::Certificate(format!("failed to add client CA cert: {e}")))?;
    }

    let client_verifier = if config.require_client_cert {
        WebPkiClientVerifier::builder(Arc::new(client_root_store))
            .build()
            .map_err(|e| TlsError::Certificate(format!("client verifier failed: {e}")))?
    } else {
        // R-06 mandates mutual authentication. Warn loudly when disabled — in
        // ALL build profiles, since a release server silently skipping client
        // auth is exactly the dangerous case operators need to see.
        warn!(
            spec_requirement = "R-06",
            "TLS server running without client certificate verification; this violates Modbus/TCP Security mutual authentication"
        );
        WebPkiClientVerifier::builder(Arc::new(client_root_store))
            .allow_unauthenticated()
            .build()
            .map_err(|e| TlsError::Certificate(format!("client verifier failed: {e}")))?
    };

    let mut tls_config = ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(server_certs, server_key)
        .map_err(|e| TlsError::Certificate(format!("server cert config failed: {e}")))?;
    tls_config.max_fragment_size = Some(MODBUS_TLS_MAX_FRAGMENT_SIZE);

    Ok(tls_config)
}

/// Load PEM-encoded certificates from a file.
fn load_certs(path: &std::path::Path) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let file = fs::File::open(path).map_err(|e| {
        TlsError::Certificate(format!("cannot open cert file {}: {e}", path.display()))
    })?;
    let mut reader = BufReader::new(file);
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            TlsError::Certificate(format!("cannot parse cert file {}: {e}", path.display()))
        })?;
    if certs.is_empty() {
        return Err(TlsError::Certificate(format!(
            "no certificates found in {}",
            path.display()
        )));
    }
    Ok(certs)
}

/// Load a PEM-encoded private key from a file.
fn load_private_key(path: &std::path::Path) -> Result<PrivateKeyDer<'static>, TlsError> {
    let file = fs::File::open(path).map_err(|e| {
        TlsError::Certificate(format!("cannot open key file {}: {e}", path.display()))
    })?;
    let mut reader = BufReader::new(file);

    // Try PKCS8 first, then RSA, then EC.
    let key = rustls_pemfile::private_key(&mut reader)
        .map_err(|e| {
            TlsError::Certificate(format!("cannot parse key file {}: {e}", path.display()))
        })?
        .ok_or_else(|| {
            TlsError::Certificate(format!("no private key found in {}", path.display()))
        })?;

    Ok(key)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use rcgen::{CertificateParams, CertifiedIssuer, KeyPair};
    use tempfile::NamedTempFile;

    use super::*;
    use crate::config::{TlsClientConfig, TlsServerConfig};

    struct TestCerts {
        ca_cert: NamedTempFile,
        server_cert: NamedTempFile,
        server_key: NamedTempFile,
        client_cert: NamedTempFile,
        client_key: NamedTempFile,
    }

    fn generate_test_certs() -> TestCerts {
        let mut ca_params = CertificateParams::new(vec![String::from("Test CA")]).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().unwrap();
        let ca = CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();

        let mut server_params = CertificateParams::new(vec![String::from("127.0.0.1")]).unwrap();
        server_params
            .subject_alt_names
            .push(rcgen::SanType::IpAddress(std::net::IpAddr::V4(
                std::net::Ipv4Addr::LOCALHOST,
            )));
        let server_key = KeyPair::generate().unwrap();
        let server_cert = server_params.signed_by(&server_key, &*ca).unwrap();

        let client_params = CertificateParams::new(vec![String::from("Test Client")]).unwrap();
        let client_key = KeyPair::generate().unwrap();
        let client_cert = client_params.signed_by(&client_key, &*ca).unwrap();

        let mut ca_file = NamedTempFile::new().unwrap();
        ca_file.write_all(ca.pem().as_bytes()).unwrap();

        let mut server_cert_file = NamedTempFile::new().unwrap();
        server_cert_file
            .write_all(server_cert.pem().as_bytes())
            .unwrap();

        let mut server_key_file = NamedTempFile::new().unwrap();
        server_key_file
            .write_all(server_key.serialize_pem().as_bytes())
            .unwrap();

        let mut client_cert_file = NamedTempFile::new().unwrap();
        client_cert_file
            .write_all(client_cert.pem().as_bytes())
            .unwrap();

        let mut client_key_file = NamedTempFile::new().unwrap();
        client_key_file
            .write_all(client_key.serialize_pem().as_bytes())
            .unwrap();

        TestCerts {
            ca_cert: ca_file,
            server_cert: server_cert_file,
            server_key: server_key_file,
            client_cert: client_cert_file,
            client_key: client_key_file,
        }
    }

    #[test]
    fn builders_set_modbus_security_max_fragment_size() {
        let certs = generate_test_certs();

        let client_config = build_client_config(&TlsClientConfig {
            ca_cert: certs.ca_cert.path().to_path_buf(),
            client_cert: certs.client_cert.path().to_path_buf(),
            client_key: certs.client_key.path().to_path_buf(),
            ..TlsClientConfig::default()
        })
        .unwrap();
        assert_eq!(
            client_config.max_fragment_size,
            Some(MODBUS_TLS_MAX_FRAGMENT_SIZE)
        );

        let server_config = build_server_config(&TlsServerConfig {
            server_cert: certs.server_cert.path().to_path_buf(),
            server_key: certs.server_key.path().to_path_buf(),
            ca_cert: certs.ca_cert.path().to_path_buf(),
            require_client_cert: true,
            max_connections: 64,
            authz_callback: None,
        })
        .unwrap();
        assert_eq!(
            server_config.max_fragment_size,
            Some(MODBUS_TLS_MAX_FRAGMENT_SIZE)
        );
    }

    #[test]
    fn missing_files_fail_before_fragment_setting() {
        let missing = PathBuf::from("/definitely/not/a/certificate.pem");
        let err = build_client_config(&TlsClientConfig {
            ca_cert: missing.clone(),
            client_cert: missing.clone(),
            client_key: missing,
            ..TlsClientConfig::default()
        })
        .unwrap_err();
        assert!(matches!(err, TlsError::Certificate(_)));
    }
}
