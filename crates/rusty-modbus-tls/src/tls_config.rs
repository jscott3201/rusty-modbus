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

    let tls_config = ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_root_certificates(root_store)
        .with_client_auth_cert(client_certs, client_key)
        .map_err(|e| TlsError::Certificate(format!("client auth config failed: {e}")))?;

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
        // R-06 mandates mutual authentication. Warn loudly when disabled.
        #[cfg(debug_assertions)]
        eprintln!(
            "WARNING: TLS server running WITHOUT client certificate verification. \
             This violates Modbus/TCP Security spec R-06 (mutual authentication)."
        );
        WebPkiClientVerifier::builder(Arc::new(client_root_store))
            .allow_unauthenticated()
            .build()
            .map_err(|e| TlsError::Certificate(format!("client verifier failed: {e}")))?
    };

    let tls_config = ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(server_certs, server_key)
        .map_err(|e| TlsError::Certificate(format!("server cert config failed: {e}")))?;

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
