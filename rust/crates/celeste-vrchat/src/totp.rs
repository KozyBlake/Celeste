//! RFC 6238 TOTP support for Java-compatible unattended VRChat login.
//!
//! Java stores the authenticator's Base32 seed (not a six-digit one-time
//! code) and generates a fresh SHA-1 code using VRChat's server clock.

use hmac::{Hmac, Mac};
use sha1::Sha1;

const JAVA_SECRET_LEN: usize = 32;

/// Normalizes the common human-readable form of a VRChat authenticator
/// setup key. Celeste Java only accepts a 32-character Base32 seed, so the
/// Rust port deliberately preserves that compatibility constraint.
pub fn normalize_secret(input: &str) -> Result<String, String> {
    let normalized: String = input
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '-')
        .map(|character| character.to_ascii_uppercase())
        .collect();
    if normalized.len() != JAVA_SECRET_LEN {
        return Err(format!(
            "Authenticator setup key must contain {JAVA_SECRET_LEN} Base32 characters"
        ));
    }
    if !normalized
        .chars()
        .all(|character| matches!(character, 'A'..='Z' | '2'..='7'))
    {
        return Err(
            "Authenticator setup key may only contain Base32 letters A-Z and digits 2-7"
                .to_string(),
        );
    }
    Ok(normalized)
}

/// Generates the six-digit RFC 6238 code used by VRChat at `unix_seconds`.
pub fn generate_code(secret: &str, unix_seconds: i64) -> Result<String, String> {
    if unix_seconds < 0 {
        return Err("Server time was before the Unix epoch".to_string());
    }
    let secret = normalize_secret(secret)?;
    let key = decode_base32(&secret)?;
    let counter = (unix_seconds as u64 / 30).to_be_bytes();
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).map_err(|error| error.to_string())?;
    mac.update(&counter);
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let value = (u32::from(digest[offset] & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    Ok(format!("{:06}", value % 1_000_000))
}

fn decode_base32(input: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(input.len() * 5 / 8);
    let mut buffer = 0_u32;
    let mut bits = 0_u8;
    for character in input.bytes() {
        let value = match character {
            b'A'..=b'Z' => character - b'A',
            b'2'..=b'7' => character - b'2' + 26,
            _ => return Err("Invalid Base32 character in authenticator setup key".to_string()),
        };
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1_u32 << bits) - 1;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_a_human_readable_setup_key() {
        assert_eq!(
            normalize_secret("jbsw y3dp-ehpk3pxp jbsw y3dp ehpk3pxp").unwrap(),
            "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP"
        );
    }

    #[test]
    fn rejects_a_six_digit_one_time_code() {
        assert!(normalize_secret("123456").is_err());
    }

    #[test]
    fn matches_the_rfc_6238_sha1_test_vector() {
        // RFC 6238's seed is the ASCII text "12345678901234567890", whose
        // Base32 representation is exactly the 32 characters Java accepts.
        assert_eq!(
            generate_code("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59).unwrap(),
            "287082"
        );
    }
}
