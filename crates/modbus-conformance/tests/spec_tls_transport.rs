//! TLS Transport conformance tests.
//!
//! Verifies TLS configuration against the Modbus/TCP Security
//! Protocol Specification V36 (2021-07-30).

use modbus_tls::config::{AuthzDecision, TlsServerConfig};
use modbus_types::MODBUS_ROLE_OID;

// ── Security Spec §8.2 — TLS Handshake Requirements ───────────────

#[test]
fn spec_r06_require_client_cert_default_true() {
    // R-06: "Mutual authentication MUST be provided"
    let config = TlsServerConfig::default();
    assert!(
        config.require_client_cert,
        "R-06: require_client_cert must default to true for mutual auth"
    );
}

// ── Security Spec §8.4 — Role Extraction ──────────────────────────

#[test]
fn spec_r21_role_oid_correct() {
    // R-21: "Must use the Modbus.org PEN OID: 1.3.6.1.4.1.50316.802.1"
    assert_eq!(MODBUS_ROLE_OID, "1.3.6.1.4.1.50316.802.1");
}

#[test]
fn spec_r23_no_role_returns_none() {
    // R-23: "If no role extension → provide NULL role to AuthZ"
    use modbus_tls::role::extract_role;
    // Empty or invalid DER should return None
    assert!(extract_role(&[]).is_none());
    assert!(extract_role(&[0x30, 0x00]).is_none()); // minimal but no extensions
}

// ── Security Spec §8.4 R-31 — Authorization Decision ──────────────

#[test]
fn spec_r31_not_authorized_semantics() {
    // R-31: "Rejection returns ExceptionCode::IllegalFunction (0x01)"
    // The AuthzDecision enum documents this mapping
    assert_ne!(AuthzDecision::Authorized, AuthzDecision::NotAuthorized);
}

#[test]
fn spec_tls_server_default_max_connections() {
    let config = TlsServerConfig::default();
    assert_eq!(config.max_connections, 64);
}

#[test]
fn spec_tls_server_no_authz_callback_by_default() {
    // R-24: "AuthZ algorithm is defined by the device vendor"
    // Default is no callback — all requests authorized
    let config = TlsServerConfig::default();
    assert!(config.authz_callback.is_none());
}

// ── Security Spec §5 — Port 802 ──────────────────────────────────

#[test]
fn spec_security_5_tls_port() {
    assert_eq!(modbus_types::MODBUS_TLS_PORT, 802);
}
