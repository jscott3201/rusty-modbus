//! Role extraction from x.509v3 certificate extensions.
//!
//! Looks for the Modbus role OID `1.3.6.1.4.1.50316.802.1` in certificate
//! extensions and decodes the value as a UTF-8 string.

use x509_parser::asn1_rs::Utf8String;
use x509_parser::prelude::*;

/// The Modbus.org PEN OID for role assignment (Security Spec §8.4, R-21).
/// Encoded as DER OID bytes: 1.3.6.1.4.1.50316.802.1
const MODBUS_ROLE_OID_STR: &str = "1.3.6.1.4.1.50316.802.1";

/// Extract the Modbus role from a DER-encoded certificate's extensions.
///
/// Returns the **NULL role** (`None`) — per Security Spec R-23 — if:
/// - The certificate cannot be parsed
/// - No extension with the Modbus role OID is present
/// - The role extension value is not a well-formed, non-empty ASN.1
///   `UTF8String` (R-22)
#[must_use]
pub fn extract_role(cert_der: &[u8]) -> Option<String> {
    let (_, cert) = X509Certificate::from_der(cert_der).ok()?;

    for ext in cert.extensions() {
        if ext.oid.to_id_string() == MODBUS_ROLE_OID_STR {
            return parse_role_value(ext.value);
        }
    }

    None
}

/// Decode the Modbus role OID extension value.
///
/// R-22: the value is a DER-encoded ASN.1 `UTF8String`. We parse it with a real
/// ASN.1 decoder (handles short- *and* long-form lengths, enforces the
/// `UTF8String` tag, validates UTF-8, and rejects trailing data) rather than
/// hand-rolling the TLV. Anything that is not a well-formed, non-empty
/// `UTF8String` maps to the NULL role (`None`, R-23).
fn parse_role_value(value: &[u8]) -> Option<String> {
    let (rest, s) = Utf8String::from_der(value).ok()?;
    if !rest.is_empty() {
        return None; // trailing data after the UTF8String → not conformant
    }
    let role = s.as_ref();
    if role.is_empty() {
        None // empty role == NULL role (R-23)
    } else {
        Some(role.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_role_in_standard_cert() {
        assert!(extract_role(&[]).is_none());
        assert!(extract_role(&[0x00, 0x01, 0x02]).is_none());
    }

    // Direct coverage of the DER UTF8String parsing (R-22/R-23), independent of
    // certificate construction.

    fn utf8string_der(content: &[u8]) -> Vec<u8> {
        // Build a DER UTF8String (tag 0x0C) with short- or long-form length.
        let mut der = vec![0x0C];
        let len = content.len();
        if len < 0x80 {
            der.push(u8::try_from(len).unwrap());
        } else {
            // Long form: 0x81 = one length octet follows (len <= 255).
            der.push(0x81);
            der.push(u8::try_from(len).unwrap());
        }
        der.extend_from_slice(content);
        der
    }

    #[test]
    fn short_form_role_parses() {
        assert_eq!(
            parse_role_value(&utf8string_der(b"operator")),
            Some("operator".to_string())
        );
    }

    #[test]
    fn long_form_role_over_127_bytes_parses() {
        // 200-byte role uses DER long-form length — the old hand-parser broke here.
        let role = "A".repeat(200);
        assert_eq!(
            parse_role_value(&utf8string_der(role.as_bytes())),
            Some(role)
        );
    }

    #[test]
    fn empty_role_is_null_role() {
        assert_eq!(parse_role_value(&utf8string_der(b"")), None); // [0x0C, 0x00]
        assert_eq!(parse_role_value(&[]), None);
    }

    #[test]
    fn wrong_tag_is_null_role() {
        // OCTET STRING (0x04) instead of UTF8String → not conformant.
        assert_eq!(parse_role_value(&[0x04, 0x03, b'a', b'b', b'c']), None);
    }

    #[test]
    fn trailing_data_is_rejected() {
        let mut der = utf8string_der(b"hi");
        der.push(0xFF); // junk after the UTF8String
        assert_eq!(parse_role_value(&der), None);
    }

    #[test]
    fn invalid_utf8_is_null_role() {
        assert_eq!(parse_role_value(&[0x0C, 0x01, 0xFF]), None);
    }
}
