//! Enum-valued settings, ported from `net.sybyline.scarlet.util` and
//! `ScarletMobile.Severity`. Kept together here (rather than one file per
//! enum) since each is small and exists solely to back a `FileValuedEnum`
//! setting.
//!
//! String representations use the exact Java `Enum.name()` spelling (e.g.
//! `"ENABLED_ONLY_18_PLUS"`), preserved via explicit `serde(rename = ...)`
//! rather than a blanket case-conversion rule, so a `settings.json` carried
//! over from a Java install round-trips instead of silently resetting to
//! defaults.

use serde::{Deserialize, Serialize};

use super::file_valued::{FromStringValue, ToStringValue};

/// Port of `net.sybyline.scarlet.util.EnforcementAgeState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnforcementAgeState {
    #[serde(rename = "DISABLED")]
    Disabled,
    #[serde(rename = "ENABLED_ONLY_18_PLUS")]
    EnabledOnly18Plus,
    #[serde(rename = "ENABLED_NEVER_18_PLUS")]
    EnabledNever18Plus,
}

impl EnforcementAgeState {
    pub const ALL: [EnforcementAgeState; 3] =
        [EnforcementAgeState::Disabled, EnforcementAgeState::EnabledOnly18Plus, EnforcementAgeState::EnabledNever18Plus];

    pub fn label(&self) -> &'static str {
        match self {
            EnforcementAgeState::Disabled => "Disabled",
            EnforcementAgeState::EnabledOnly18Plus => "Enabled (only 18+)",
            EnforcementAgeState::EnabledNever18Plus => "Enabled (never 18+)",
        }
    }
}

impl ToStringValue for EnforcementAgeState {
    fn to_string_value(&self) -> String {
        match self {
            EnforcementAgeState::Disabled => "DISABLED",
            EnforcementAgeState::EnabledOnly18Plus => "ENABLED_ONLY_18_PLUS",
            EnforcementAgeState::EnabledNever18Plus => "ENABLED_NEVER_18_PLUS",
        }
        .to_string()
    }
}
impl FromStringValue for EnforcementAgeState {
    fn from_string_value(s: &str) -> Option<Self> {
        match s {
            "DISABLED" => Some(EnforcementAgeState::Disabled),
            "ENABLED_ONLY_18_PLUS" => Some(EnforcementAgeState::EnabledOnly18Plus),
            "ENABLED_NEVER_18_PLUS" => Some(EnforcementAgeState::EnabledNever18Plus),
            _ => None,
        }
    }
}

/// Port of `net.sybyline.scarlet.util.EnforcementListState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnforcementListState {
    #[serde(rename = "DISABLED")]
    Disabled,
    #[serde(rename = "ENABLED_WHITELIST")]
    EnabledWhitelist,
    #[serde(rename = "ENABLED_BLACKLIST")]
    EnabledBlacklist,
}

impl EnforcementListState {
    pub const ALL: [EnforcementListState; 3] =
        [EnforcementListState::Disabled, EnforcementListState::EnabledWhitelist, EnforcementListState::EnabledBlacklist];

    pub fn label(&self) -> &'static str {
        match self {
            EnforcementListState::Disabled => "Disabled",
            EnforcementListState::EnabledWhitelist => "Enabled (whitelist)",
            EnforcementListState::EnabledBlacklist => "Enabled (blacklist)",
        }
    }
}

impl ToStringValue for EnforcementListState {
    fn to_string_value(&self) -> String {
        match self {
            EnforcementListState::Disabled => "DISABLED",
            EnforcementListState::EnabledWhitelist => "ENABLED_WHITELIST",
            EnforcementListState::EnabledBlacklist => "ENABLED_BLACKLIST",
        }
        .to_string()
    }
}
impl FromStringValue for EnforcementListState {
    fn from_string_value(s: &str) -> Option<Self> {
        match s {
            "DISABLED" => Some(EnforcementListState::Disabled),
            "ENABLED_WHITELIST" => Some(EnforcementListState::EnabledWhitelist),
            "ENABLED_BLACKLIST" => Some(EnforcementListState::EnabledBlacklist),
            _ => None,
        }
    }
}

/// Port of `ScarletMobile.Severity` (the minimum severity that triggers a
/// mobile push notification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotifySeverity {
    #[serde(rename = "INFO")]
    Info,
    #[serde(rename = "WATCH")]
    Watch,
    #[serde(rename = "WARNING")]
    Warning,
    #[serde(rename = "CRITICAL")]
    Critical,
}

impl NotifySeverity {
    pub const ALL: [NotifySeverity; 4] =
        [NotifySeverity::Info, NotifySeverity::Watch, NotifySeverity::Warning, NotifySeverity::Critical];

    pub fn label(&self) -> &'static str {
        match self {
            NotifySeverity::Info => "Info",
            NotifySeverity::Watch => "Watch",
            NotifySeverity::Warning => "Warning",
            NotifySeverity::Critical => "Critical",
        }
    }
}

impl ToStringValue for NotifySeverity {
    fn to_string_value(&self) -> String {
        match self {
            NotifySeverity::Info => "INFO",
            NotifySeverity::Watch => "WATCH",
            NotifySeverity::Warning => "WARNING",
            NotifySeverity::Critical => "CRITICAL",
        }
        .to_string()
    }
}
impl FromStringValue for NotifySeverity {
    fn from_string_value(s: &str) -> Option<Self> {
        match s {
            "INFO" => Some(NotifySeverity::Info),
            "WATCH" => Some(NotifySeverity::Watch),
            "WARNING" => Some(NotifySeverity::Warning),
            "CRITICAL" => Some(NotifySeverity::Critical),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforcement_age_state_round_trips_through_java_name_strings() {
        for state in EnforcementAgeState::ALL {
            let s = state.to_string_value();
            assert_eq!(EnforcementAgeState::from_string_value(&s), Some(state));
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!("\"{s}\""));
            let back: EnforcementAgeState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }

    #[test]
    fn enforcement_list_state_round_trips_through_java_name_strings() {
        for state in EnforcementListState::ALL {
            let s = state.to_string_value();
            assert_eq!(EnforcementListState::from_string_value(&s), Some(state));
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!("\"{s}\""));
        }
    }

    #[test]
    fn notify_severity_round_trips_through_java_name_strings() {
        for severity in NotifySeverity::ALL {
            let s = severity.to_string_value();
            assert_eq!(NotifySeverity::from_string_value(&s), Some(severity));
            let json = serde_json::to_string(&severity).unwrap();
            assert_eq!(json, format!("\"{s}\""));
        }
    }

    #[test]
    fn unrecognized_string_is_rejected() {
        assert_eq!(EnforcementAgeState::from_string_value("NOT_A_REAL_VARIANT"), None);
    }
}
