//! Port of `ScarletVRChat.encodeVrchatBasicAuthComponent`/
//! `isVrchatBasicAuthUnescaped`: VRChat's API expects HTTP Basic Auth
//! credentials to be percent-encoded *before* base64-encoding, unlike
//! standard RFC 7617 (raw UTF-8 bytes). Their own web client does this
//! automatically; an RFC-7617-correct client fails to log in any account
//! whose password contains a character URL-encoding changes -- most
//! visibly `%`, `&`, `+`, or a space.
//!
//! Standard Basic Auth: `Base64(username + ":" + password)`.
//! VRChat's expected form: `Base64(url(username) + ":" + url(password))`.

use base64::Engine;

fn is_unescaped(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(b, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')')
}

/// Percent-encodes `value` using the same unreserved-character set as the
/// Java version (a superset of RFC 3986's unreserved set, matching what
/// VRChat's own web client sends).
pub fn encode_vrchat_basic_auth_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for &b in value.as_bytes() {
        if is_unescaped(b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    encoded
}

/// Builds the `Authorization: Basic ...` header value for VRChat's
/// non-standard Basic Auth scheme.
pub fn basic_auth_header(username: &str, password: &str) -> String {
    let encoded_username = encode_vrchat_basic_auth_component(username);
    let encoded_password = encode_vrchat_basic_auth_component(password);
    let combined = format!("{encoded_username}:{encoded_password}");
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(combined))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_unreserved_characters_alone() {
        assert_eq!(encode_vrchat_basic_auth_component("abcXYZ019-_.!~*'()"), "abcXYZ019-_.!~*'()");
    }

    #[test]
    fn percent_encodes_special_characters() {
        assert_eq!(encode_vrchat_basic_auth_component("p%ss&w+rd "), "p%25ss%26w%2Brd%20");
    }

    #[test]
    fn basic_auth_header_matches_expected_form() {
        // Base64("user:pass") for the trivial no-special-characters case,
        // to confirm the header shape ("Basic " + base64) is right.
        let header = basic_auth_header("user", "pass");
        assert_eq!(header, "Basic dXNlcjpwYXNz");
    }

    #[test]
    fn basic_auth_header_url_encodes_before_base64() {
        // A password containing '&' must be percent-encoded to %26 BEFORE
        // base64, not passed through raw -- this is the exact bug the
        // Java version's comment describes RFC-7617-correct clients hitting.
        let header = basic_auth_header("user", "a&b");
        let expected = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("user:a%26b"));
        assert_eq!(header, expected);
    }
}
