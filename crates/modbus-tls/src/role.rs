//! Role extraction from x.509v3 certificate extensions.
//!
//! Looks for the Modbus role OID `1.3.6.1.4.1.50316.802.1` in certificate
//! extensions and decodes the value as a UTF-8 string.

use x509_parser::prelude::*;

/// The Modbus.org PEN OID for role assignment (Security Spec §8.4, R-21).
/// Encoded as DER OID bytes: 1.3.6.1.4.1.50316.802.1
const MODBUS_ROLE_OID_STR: &str = "1.3.6.1.4.1.50316.802.1";

/// Extract the Modbus role from a DER-encoded certificate's extensions.
///
/// Returns `None` if:
/// - The certificate cannot be parsed
/// - No extension with the Modbus role OID is present (R-23: NULL role)
/// - The extension value cannot be decoded as UTF-8
#[must_use]
pub fn extract_role(cert_der: &[u8]) -> Option<String> {
    let (_, cert) = X509Certificate::from_der(cert_der).ok()?;

    for ext in cert.extensions() {
        let oid_str = ext.oid.to_id_string();
        if oid_str == MODBUS_ROLE_OID_STR {
            // R-22: Value is ASN1:UTF8String. The extension value bytes
            // contain the DER encoding.
            let value = ext.value;
            // Skip ASN.1 tag (0x0C = UTF8String) and length byte if present.
            if value.len() >= 2 && value[0] == 0x0C {
                let len = value[1] as usize;
                if value.len() >= 2 + len {
                    return String::from_utf8(value[2..2 + len].to_vec()).ok();
                }
            }
            // Fallback: try the raw bytes as UTF-8.
            return String::from_utf8(value.to_vec()).ok();
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_role_in_standard_cert() {
        assert!(extract_role(&[]).is_none());
        assert!(extract_role(&[0x00, 0x01, 0x02]).is_none());
    }
}
