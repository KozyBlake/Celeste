//! Port of `ScarletDiscordJDA.normalizeDiscordBotToken`/`fingerprintDiscordToken`:
//! cleans up a pasted bot token (BOM/zero-width characters, a leading "Bot "
//! prefix, surrounding quotes, stray control characters) and produces a
//! short non-reversible fingerprint safe to put in logs.

use sha2::{Digest, Sha256};

/// Normalizes a user-pasted Discord bot token. Returns `None` if the result
/// is empty (matching the Java version returning `null`).
pub fn normalize_discord_bot_token(token: Option<&str>) -> Option<String> {
    let token = token?;
    let mut normalized =
        token.replace(['\u{FEFF}', '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\r', '\n'], "");
    normalized = normalized.trim().to_string();
    if normalized.len() >= 4 && normalized[..4].eq_ignore_ascii_case("Bot ") {
        normalized = normalized[4..].trim().to_string();
    }
    let is_quoted = (normalized.starts_with('"') && normalized.ends_with('"'))
        || (normalized.starts_with('\'') && normalized.ends_with('\''));
    if is_quoted && normalized.len() >= 2 {
        normalized = normalized[1..normalized.len() - 1].trim().to_string();
    }
    normalized.retain(|c| !c.is_control());
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// A short, non-reversible fingerprint of a token, safe to log.
pub fn fingerprint_discord_token(token: Option<&str>) -> String {
    match token {
        None => "empty".to_string(),
        Some("") => "empty".to_string(),
        Some(t) => {
            let digest = Sha256::digest(t.as_bytes());
            digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_bot_prefix_and_quotes() {
        assert_eq!(normalize_discord_bot_token(Some("Bot \"abc123\"")), Some("abc123".to_string()));
        assert_eq!(normalize_discord_bot_token(Some("'abc123'")), Some("abc123".to_string()));
    }

    #[test]
    fn strips_zero_width_and_bom_characters() {
        assert_eq!(normalize_discord_bot_token(Some("\u{FEFF}abc\u{200B}123")), Some("abc123".to_string()));
    }

    #[test]
    fn empty_after_normalization_is_none() {
        assert_eq!(normalize_discord_bot_token(Some("   ")), None);
        assert_eq!(normalize_discord_bot_token(None), None);
    }

    #[test]
    fn fingerprint_is_stable_and_short() {
        let fp1 = fingerprint_discord_token(Some("my-token"));
        let fp2 = fingerprint_discord_token(Some("my-token"));
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 12);
        assert_ne!(fp1, fingerprint_discord_token(Some("other-token")));
        assert_eq!(fingerprint_discord_token(None), "empty");
    }
}
