//! Port of `net.sybyline.scarlet.Features`: build-time feature flags.
//!
//! At build time we embed `resources/celeste-features.properties` (via
//! `include_str!`, the Rust equivalent of the Java version's bundled
//! classpath resource — there's no classpath to probe at runtime, but the
//! "ships an empty properties file, custom builds can edit it before
//! building" intent is the same either way). Its boolean entries decide
//! which optional subsystems are active; when a key is absent or blank,
//! the flag falls back to its compile-time default.
//!
//! Default values encode "what should happen if no properties file
//! entries override it": every flag defaults to `true` so the standard
//! build preserves full functionality.
//!
//! Unlike the Java version's `public static final boolean` fields (which
//! the JIT can constant-fold), these are [`std::sync::LazyLock`] statics —
//! Rust has no classfile-style build-time constant folding across an
//! `include_str!`, so the properties are parsed once, lazily, on first
//! access instead. The externally visible behavior (flag value, `is_enabled`
//! dispatch, unknown-key handling) matches the Java version exactly.

use std::collections::HashMap;
use std::sync::LazyLock;

const RAW_PROPERTIES: &str = include_str!("../resources/celeste-features.properties");

static PROPS: LazyLock<HashMap<String, String>> = LazyLock::new(load_properties);

fn load_properties() -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in RAW_PROPERTIES.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    if map.is_empty() {
        tracing::info!("No feature flag entries in celeste-features.properties; using compile-time defaults for all feature flags");
    } else {
        tracing::info!("Loaded {} feature flag entries from celeste-features.properties", map.len());
    }
    map
}

fn load_flag(key: &str, default_value: bool) -> bool {
    match PROPS.get(key) {
        None => default_value,
        Some(raw) => {
            let raw = raw.trim();
            if raw.is_empty() {
                default_value
            } else {
                // Matches Java's Boolean.parseBoolean: case-insensitive
                // "true" is true, everything else is false.
                raw.eq_ignore_ascii_case("true")
            }
        }
    }
}

/// Whether the built-in Discord `/discord-warn`, `/discord-kick`, and
/// `/discord-ban` slash commands are registered and their settings toggle
/// (`discordKickBanEnabled`) is exposed in the UI. Orthogonal to the runtime
/// toggle. Default `true`.
pub fn discord_kick_ban_enabled() -> bool {
    load_flag("discord.kick_ban.enabled", true)
}

/// Whether the `/watched-avatar` command family and the watched-avatar
/// detection branch of the event listener are active. Default `true`.
pub fn watched_avatars_enabled() -> bool {
    load_flag("watched.avatars.enabled", true)
}

/// Whether the `ScarletCalendar` subsystem (event scheduling, periodic
/// reports, `/schedule` command) is active. Default `true`.
pub fn calendar_enabled() -> bool {
    load_flag("calendar.enabled", true)
}

/// Whether the `/aux-webhooks` and `/set-audit-aux-webhooks` commands + the
/// aux-webhook routing they drive are active. Default `true`.
pub fn aux_webhooks_enabled() -> bool {
    load_flag("aux_webhooks.enabled", true)
}

/// Whether the `/vrchat-animated-emoji` slash/message command and its
/// GIF -> Discord-sprite-sheet backend are registered. Default `true`.
pub fn animated_emoji_enabled() -> bool {
    load_flag("animated_emoji.enabled", true)
}

/// Whether DAVE (Discord Audio & Video Encryption) is wired into the voice
/// stack. Default `true`.
pub fn dave_enabled() -> bool {
    load_flag("dave.enabled", true)
}

/// Whether moderation-case evidence and attachment tracking are active.
/// Default `true`.
pub fn evidence_enabled() -> bool {
    load_flag("evidence.enabled", true)
}

/// Whether the avatar-search subsystem is active. Default `true`.
pub fn avatar_search_enabled() -> bool {
    load_flag("avatar_search.enabled", true)
}

/// Whether the pronoun-validation subsystem is active. Default `true`.
pub fn pronouns_enabled() -> bool {
    load_flag("pronouns.enabled", true)
}

/// Whether the VRChat-reports subsystem is active. Default `true`.
pub fn vrchat_reports_enabled() -> bool {
    load_flag("vrchat_reports.enabled", true)
}

/// Whether Celeste's interactive CLI is active. Default `true`.
pub fn cli_commands_enabled() -> bool {
    load_flag("cli_commands.enabled", true)
}

/// Resolve a feature flag by its properties-file key. Used by declarative
/// registration gates such as Discord commands. Unknown keys log a warning
/// and return `false`, matching the Java version.
pub fn is_enabled(key: &str) -> bool {
    match key {
        "discord.kick_ban.enabled" => discord_kick_ban_enabled(),
        "watched.avatars.enabled" => watched_avatars_enabled(),
        "calendar.enabled" => calendar_enabled(),
        "aux_webhooks.enabled" => aux_webhooks_enabled(),
        "animated_emoji.enabled" => animated_emoji_enabled(),
        "dave.enabled" => dave_enabled(),
        "evidence.enabled" => evidence_enabled(),
        "avatar_search.enabled" => avatar_search_enabled(),
        "pronouns.enabled" => pronouns_enabled(),
        "vrchat_reports.enabled" => vrchat_reports_enabled(),
        "cli_commands.enabled" => cli_commands_enabled(),
        other => {
            tracing::warn!("Unknown feature flag key `{}` requested; defaulting to false", other);
            false
        }
    }
}

/// Returns the raw properties map loaded from the embedded properties file
/// (never absent — may be empty). Useful for diagnostic logging.
pub fn properties() -> &'static HashMap<String, String> {
    &PROPS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_flags_default_true_with_the_shipped_empty_properties_file() {
        assert!(discord_kick_ban_enabled());
        assert!(watched_avatars_enabled());
        assert!(calendar_enabled());
        assert!(aux_webhooks_enabled());
        assert!(animated_emoji_enabled());
        assert!(dave_enabled());
        assert!(evidence_enabled());
        assert!(avatar_search_enabled());
        assert!(pronouns_enabled());
        assert!(vrchat_reports_enabled());
        assert!(cli_commands_enabled());
    }

    #[test]
    fn is_enabled_dispatches_by_key() {
        assert_eq!(is_enabled("dave.enabled"), dave_enabled());
        assert!(!is_enabled("not.a.real.key"));
    }
}
