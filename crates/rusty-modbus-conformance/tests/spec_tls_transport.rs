//! TLS Transport conformance tests.
//!
//! Verifies TLS configuration against the Modbus/TCP Security
//! Protocol Specification V36 (2021-07-30).

use rusty_modbus_tls::config::{AuthzDecision, TlsServerConfig};
use rusty_modbus_types::{MODBUS_ROLE_OID, MODBUS_TLS_MAX_FRAGMENT_SIZE};

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
    use rusty_modbus_tls::role::extract_role;
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

#[test]
fn spec_r31_authorize_helper_enforces_callback() {
    // R-24/R-31: the authorize() helper applies the configured callback, and a
    // NotAuthorized decision is honored. With no callback it allows all.
    use rusty_modbus_tls::config::AuthzRequest;
    use rusty_modbus_types::{FunctionCode, UnitId};
    use std::sync::Arc;

    let req = AuthzRequest {
        role: Some("operator".to_string()),
        function_code: FunctionCode::ReadHoldingRegisters,
        unit_id: UnitId(1),
        address: None,
        quantity: None,
    };

    // No callback → allow-all default.
    assert_eq!(
        TlsServerConfig::default().authorize(&req),
        AuthzDecision::Authorized
    );

    // Callback denying the "operator" role is honored by authorize().
    let config = TlsServerConfig {
        authz_callback: Some(Arc::new(|r: &AuthzRequest| {
            if r.role.as_deref() == Some("operator") {
                AuthzDecision::NotAuthorized
            } else {
                AuthzDecision::Authorized
            }
        })),
        ..TlsServerConfig::default()
    };
    assert_eq!(config.authorize(&req), AuthzDecision::NotAuthorized);
}

// ── Security Spec §5 — Port 802 ──────────────────────────────────

#[test]
fn spec_security_5_tls_port() {
    assert_eq!(rusty_modbus_types::MODBUS_TLS_PORT, 802);
}

#[test]
fn spec_security_6_2_max_fragment_length() {
    assert_eq!(MODBUS_TLS_MAX_FRAGMENT_SIZE, 512);
}
