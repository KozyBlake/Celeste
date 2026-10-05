//! Port of `net.sybyline.scarlet.ScarletSettings`.
//!
//! Split across submodules along the same lines the Java class mixes
//! together in one file:
//! - [`store`] — the JSON-file-backed `getJson`/`getString`/`getObject`/...
//!   half (`SettingsStore`).
//! - [`registry`] — the `Preferences`-backed `Registry*` values, replaced
//!   with a flat file store plus [`crate::secure_store::EncryptedPrefs`]
//!   for the encrypted ones (see that module's docs for why).
//! - [`file_valued`] — the `FileValued<T>` typed-settings registry.
//!
//! ## Not ported here: interactive dialogs and the backup-warning popup
//!
//! `requireInput`/`requireConfirmYesNo`/`requireSelect` (and their
//! `*Async` variants) show a Swing dialog when a GUI is available, falling
//! back to console I/O when headless. Since there's no Swing in Rust, this
//! module defines [`UserPrompt`] as the seam: [`ConsolePrompt`] ports the
//! headless fallback path faithfully (it's plain stdin/stdout, no
//! Java-specific dependency), and a GUI-backed implementation belongs in
//! `scarlet-ui` once that crate's toolkit is chosen (see PORTING.md).
//! Likewise, `warnBackupBeforeSecureStoreChanges`'s message content and
//! ack-file gating are ported (any UI can reuse
//! [`backup_warning::is_acknowledged`]/[`backup_warning::acknowledge`]),
//! but showing it is left to whichever UI (console or graphical) is
//! actually running.

pub mod backup_warning;
pub mod enums;
pub mod file_valued;
pub mod doc;
pub mod help;
pub mod prompt;
pub mod registry;
pub mod store;

use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::secure_store::EncryptedPrefs;
use enums::{EnforcementAgeState, EnforcementListState, NotifySeverity};
use file_valued::{FileValued, SettingCategory, SettingsRegistry};
use registry::{RegistryStore, RegistryStringValued};
use store::SettingsStore;

/// Resolves the password protecting the local encrypted secret store.
/// Port of `Celeste`'s `globalPW`/`fallbackGlobalPw`: `SCARLET_GLOBAL_PW`
/// env var first (Java also checked the `scarlet.global.pw` system
/// property, which Rust has no equivalent of -- env var covers the same
/// "override from the launch environment" use case), then a persisted
/// random 256-bit key at `<data_dir>/global-prefs.key`, generated once and
/// reused thereafter.
pub fn resolve_global_password(data_dir: &Path) -> String {
    if let Ok(pw) = std::env::var("SCARLET_GLOBAL_PW") {
        return pw;
    }
    let key_path = data_dir.join("global-prefs.key");
    if let Ok(existing) = std::fs::read_to_string(&key_path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    use base64::Engine;
    use rand::RngCore;
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let encoded = base64::engine::general_purpose::STANDARD.encode(secret);
    let _ = std::fs::create_dir_all(data_dir);
    if std::fs::write(&key_path, &encoded).is_err() {
        tracing::error!("Failed to persist local fallback encryption password at {}", key_path.display());
    }
    encoded
}

pub struct Settings {
    /// Root Celeste data directory, used by subsystems such as Discord log export.
    pub data_dir: std::path::PathBuf,
    pub store: Arc<SettingsStore>,
    pub registry: SettingsRegistry,
    pub registry_store: Arc<RegistryStore>,
    pub encrypted: Arc<EncryptedPrefs>,

    pub last_run_version: RegistryStringValued<String>,
    pub last_run_time: RegistryStringValued<DateTime<Utc>>,
    pub last_audit_query: RegistryStringValued<DateTime<Utc>>,
    pub last_instances_check: RegistryStringValued<DateTime<Utc>>,
    pub last_auth_refresh: RegistryStringValued<DateTime<Utc>>,
    pub next_poll_action: RegistryStringValued<DateTime<Utc>>,
    pub next_mod_summary: RegistryStringValued<DateTime<Utc>>,
    pub next_outstanding_mod: RegistryStringValued<DateTime<Utc>>,
    pub last_cache_cleanup: RegistryStringValued<DateTime<Utc>>,
    pub last_instance_joined: RegistryStringValued<chrono::NaiveDateTime>,
    pub ui_bounds: RegistryStringValued<registry::Rectangle>,

    pub heuristic_kick_count: Arc<FileValued<i32>>,
    pub heuristic_period_days: Arc<FileValued<i32>>,
    pub outstanding_period_days: Arc<FileValued<i32>>,
    pub cache_cleanup_enabled: Arc<FileValued<bool>>,
    pub cache_cleanup_days: Arc<FileValued<i32>>,

    // --- Scarlet.java: General/UI ---
    pub confirm_group_invite: Arc<FileValued<bool>>,
    pub show_ui_during_load: Arc<FileValued<bool>>,
    pub show_cli_tab: Arc<FileValued<bool>>,
    pub ui_accent_headers: Arc<FileValued<bool>>,
    pub ui_language: Arc<FileValued<String>>,
    pub ui_left_player_dim_percent: Arc<FileValued<i32>>,

    // --- Scarlet.java: Moderation ---
    pub training_mode: Arc<FileValued<bool>>,
    pub enforce_instances_18_plus: Arc<FileValued<EnforcementAgeState>>,
    pub enforce_instances_worlds: Arc<FileValued<EnforcementListState>>,
    pub enforce_instances_world_list: Arc<FileValued<Vec<String>>>,
    pub moderation_log_lookback_days: Arc<FileValued<i32>>,
    pub timed_bans_enabled: Arc<FileValued<bool>>,
    pub evidence_capture_enabled: Arc<FileValued<bool>>,
    pub evidence_capture_hotkey: Arc<FileValued<String>>,

    // --- Scarlet.java: Discord ---
    pub discord_kick_ban_enabled: Arc<FileValued<bool>>,
    pub discord_kick_ban_prompted: Arc<FileValued<bool>>,
    pub verified_role_sf: Arc<FileValued<String>>,
    pub members_role_sf: Arc<FileValued<String>>,

    // --- Scarlet.java: Integrations ---
    pub auto_invite_on_verify: Arc<FileValued<bool>>,
    pub not_verified_link_message: Arc<FileValued<String>>,
    pub auto_invite_group_id: Arc<FileValued<String>>,
    pub auto_close_bot_instance_minutes: Arc<FileValued<i32>>,
    pub audit_polling_interval: Arc<FileValued<i32>>,
    pub audit_catchup_max_days: Arc<FileValued<i32>>,
    pub instance_follow_fast_poll_seconds: Arc<FileValued<i32>>,
    pub advisory_translate_endpoint: Arc<FileValued<String>>,
    pub advisory_translate_api_key: Arc<FileValued<String>>,

    // --- Scarlet.java: Notifications ---
    pub announce_verify_complete: Arc<FileValued<bool>>,

    // --- Scarlet.java: System ---
    pub vibecoded_notice_acknowledged: Arc<FileValued<bool>>,
    pub discord_setup_prompted: Arc<FileValued<bool>>,

    // --- ScarletDiscordJDA.java: Integrations (VRChat Help Desk reporting) ---
    pub requesting_email: Arc<FileValued<String>>,
    pub append_template_footer: Arc<FileValued<bool>>,
    pub vrchat_client_launch_on_instance_create: Arc<FileValued<bool>>,
    pub avatar_search_providers_enabled: Arc<FileValued<bool>>,
    pub avatar_search_providers: Arc<FileValued<Vec<String>>>,

    // --- ScarletDiscordJDA.java: Discord (moderation pings) ---
    pub bundle_moderations_instance_kick_to_user_ban: Arc<FileValued<bool>>,
    pub moderation_summary_only_activity: Arc<FileValued<bool>>,
    pub ping_on_moderation_instance_warn: Arc<FileValued<bool>>,
    pub ping_on_moderation_instance_kick: Arc<FileValued<bool>>,
    pub ping_on_moderation_member_remove: Arc<FileValued<bool>>,
    pub ping_on_moderation_user_ban: Arc<FileValued<bool>>,
    pub ping_on_moderation_user_unban: Arc<FileValued<bool>>,
    pub ping_on_outstanding_moderation_instance_warn: Arc<FileValued<bool>>,
    pub ping_on_outstanding_moderation_instance_kick: Arc<FileValued<bool>>,
    pub ping_on_outstanding_moderation_member_remove: Arc<FileValued<bool>>,
    pub ping_on_outstanding_moderation_user_ban: Arc<FileValued<bool>>,
    pub ping_on_outstanding_moderation_user_unban: Arc<FileValued<bool>>,

    // --- ScarletDiscordJDA.java: Moderation (evidence capture) ---
    pub evidence_enabled: Arc<FileValued<bool>>,
    pub evidence_file_path_format: Arc<FileValued<String>>,

    // --- ScarletEventListener.java: Notifications (TTS + advisory) ---
    pub tts_voice_name: Arc<FileValued<String>>,
    pub tts_use_default_audio_device: Arc<FileValued<bool>>,
    pub announce_watched_users: Arc<FileValued<bool>>,
    pub announce_watched_groups: Arc<FileValued<bool>>,
    pub announce_watched_avatars: Arc<FileValued<bool>>,
    pub announce_new_players: Arc<FileValued<bool>>,
    pub announce_mixed_character_names: Arc<FileValued<bool>>,
    pub announce_votes_to_kick: Arc<FileValued<bool>>,
    pub flag_suspicious_pronouns: Arc<FileValued<bool>>,
    pub announce_suspicious_pronouns: Arc<FileValued<bool>>,
    pub announce_players_newer_than_days: Arc<FileValued<i32>>,
    pub advisory_show_watched_users: Arc<FileValued<bool>>,
    pub advisory_show_watched_groups: Arc<FileValued<bool>>,
    pub advisory_show_watched_avatars: Arc<FileValued<bool>>,
    pub advisory_show_new_players: Arc<FileValued<bool>>,
    pub advisory_show_mixed_character_names: Arc<FileValued<bool>>,
    pub advisory_show_votes_to_kick: Arc<FileValued<bool>>,
    pub advisory_show_suspicious_pronouns: Arc<FileValued<bool>>,
    pub advisory_flag_nuisance_rank: Arc<FileValued<bool>>,
    pub advisory_flag_visitor_rank: Arc<FileValued<bool>>,
    pub announce_nuisance_rank: Arc<FileValued<bool>>,
    pub announce_visitor_rank: Arc<FileValued<bool>>,

    // --- ScarletEventListener.java: Moderation ---
    pub attempt_avatar_image_match: Arc<FileValued<bool>>,

    // --- ScarletMobile.java: Integrations (mobile companion pairing) ---
    pub mobile_enabled: Arc<FileValued<bool>>,
    pub mobile_fcm_service_account: Arc<FileValued<String>>,
    pub mobile_relay_endpoint: Arc<FileValued<String>>,

    // --- ScarletMobile.java: Notifications (mobile push + desktop toast) ---
    pub mobile_min_severity: Arc<FileValued<NotifySeverity>>,
    pub mobile_notify_watched_users: Arc<FileValued<bool>>,
    pub mobile_notify_watched_groups: Arc<FileValued<bool>>,
    pub mobile_notify_watched_avatars: Arc<FileValued<bool>>,
    pub mobile_notify_votes_to_kick: Arc<FileValued<bool>>,
    pub mobile_notify_moderation: Arc<FileValued<bool>>,
    pub mobile_notify_staff: Arc<FileValued<bool>>,
    pub mobile_notify_new_players: Arc<FileValued<bool>>,
    pub mobile_notify_mixed_character_names: Arc<FileValued<bool>>,
    pub mobile_notify_suspicious_pronouns: Arc<FileValued<bool>>,
    pub notify_desktop_enabled: Arc<FileValued<bool>>,
    pub toast_notify_watched_users: Arc<FileValued<bool>>,
    pub toast_notify_watched_groups: Arc<FileValued<bool>>,
    pub toast_notify_watched_avatars: Arc<FileValued<bool>>,
    pub toast_notify_votes_to_kick: Arc<FileValued<bool>>,
    pub toast_notify_moderation: Arc<FileValued<bool>>,
    pub toast_notify_staff: Arc<FileValued<bool>>,
    pub toast_notify_new_players: Arc<FileValued<bool>>,
    pub toast_notify_mixed_character_names: Arc<FileValued<bool>>,
    pub toast_notify_suspicious_pronouns: Arc<FileValued<bool>>,

    settings_file: std::path::PathBuf,
    settings_file_last_modified: Option<std::time::SystemTime>,
    has_version_changed_since_last_run: std::sync::Mutex<Option<bool>>,
}

impl Settings {
    pub fn open(data_dir: &Path) -> Self {
        let settings_path = data_dir.join("settings.json");
        let settings_file_last_modified = std::fs::metadata(&settings_path).and_then(|m| m.modified()).ok();
        let store = Arc::new(SettingsStore::new(settings_path.clone()));
        let registry_store = Arc::new(RegistryStore::open(data_dir));
        let global_password = resolve_global_password(data_dir);
        let encrypted = Arc::new(EncryptedPrefs::open(data_dir, &global_password));
        let registry = SettingsRegistry::new(store.clone());

        Settings {
            data_dir: data_dir.to_path_buf(),
            last_run_version: registry::registry_string(registry_store.clone(), "lastRunVersion"),
            last_run_time: registry::registry_offset_date_time(registry_store.clone(), "lastRunTime"),
            last_audit_query: registry::registry_offset_date_time(registry_store.clone(), "lastAuditQuery"),
            last_instances_check: registry::registry_offset_date_time(registry_store.clone(), "lastInstancesCheck"),
            last_auth_refresh: registry::registry_offset_date_time(registry_store.clone(), "lastAuthRefresh"),
            next_poll_action: registry::registry_offset_date_time(registry_store.clone(), "nextPollAction"),
            next_mod_summary: registry::registry_offset_date_time(registry_store.clone(), "nextModSummary"),
            next_outstanding_mod: registry::registry_offset_date_time(registry_store.clone(), "nextOutstandingMod"),
            last_cache_cleanup: registry::registry_offset_date_time(registry_store.clone(), "lastCacheCleanup"),
            last_instance_joined: registry::registry_local_date_time(registry_store.clone(), "lastInstanceJoined"),
            ui_bounds: registry::registry_rectangle(registry_store.clone(), "uiBounds"),

            heuristic_kick_count: registry.int_range("heuristicKickCount", "Heuristic Kick Count", SettingCategory::Moderation, 3, 1, 10),
            heuristic_period_days: registry.int_range(
                "heuristicPeriodDays",
                "Heuristic Period (days)",
                SettingCategory::Moderation,
                3,
                1,
                30,
            ),
            outstanding_period_days: registry.int_range(
                "outstandingPeriodDays",
                "Outstanding Period (days)",
                SettingCategory::Moderation,
                3,
                1,
                30,
            ),
            cache_cleanup_enabled: registry.boolean(
                "cache_cleanup_enabled",
                "Auto-delete old cached files",
                SettingCategory::System,
                true,
            ),
            cache_cleanup_days: registry.int_range(
                "cache_cleanup_days",
                "Delete files older than (days)",
                SettingCategory::System,
                7,
                1,
                30,
            ),

            // --- Scarlet.java: General/UI ---
            confirm_group_invite: registry.boolean(
                "ui_confirm_group_invite",
                "Confirmation dialog for group invites",
                SettingCategory::General,
                false,
            ),
            show_ui_during_load: registry.boolean(
                "ui_show_during_load",
                "Show UI during load",
                SettingCategory::General,
                false,
            ),
            show_cli_tab: registry.boolean("ui_show_cli_tab", "Show CLI tab", SettingCategory::General, true),
            ui_accent_headers: registry.boolean(
                "ui_accent_headers",
                "Colour section headers with accent",
                SettingCategory::General,
                false,
            ),
            ui_language: registry.string_choice(
                "ui_language",
                "Desktop UI language (restart to apply)",
                SettingCategory::General,
                "system",
                || {
                    ["system", "en", "de", "es", "id", "ja", "ru"].into_iter().map(str::to_string).collect()
                },
            ),
            ui_left_player_dim_percent: registry.int_range(
                "ui_left_player_dim_percent",
                "Dim players who left (%)",
                SettingCategory::General,
                35,
                0,
                80,
            ),

            // --- Scarlet.java: Moderation ---
            training_mode: registry.boolean(
                "training_mode_enabled",
                "Training mode (enable the event simulator)",
                SettingCategory::Moderation,
                false,
            ),
            enforce_instances_18_plus: registry.enum_setting(
                "enforce_instances_18_plus",
                "Instances: enforce 18+",
                SettingCategory::Moderation,
                EnforcementAgeState::Disabled,
                &EnforcementAgeState::ALL,
            ),
            enforce_instances_worlds: registry.enum_setting(
                "enforce_instances_worlds",
                "Instances: enforce worlds",
                SettingCategory::Moderation,
                EnforcementListState::Disabled,
                &EnforcementListState::ALL,
            ),
            enforce_instances_world_list: registry.string_array_pattern(
                "enforce_instances_world_list",
                "Instances: enforce world list",
                SettingCategory::Moderation,
                Vec::new(),
                Some(r"wr?ld_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"),
                true,
            ),
            moderation_log_lookback_days: registry.int_range(
                "moderation_log_lookback_days",
                "Moderation log: include history from the past N days (0 = all time)",
                SettingCategory::Moderation,
                0,
                0,
                3650,
            ),
            timed_bans_enabled: registry.boolean(
                "timed_bans_enabled",
                "Discord: show timed-ban buttons on moderation posts",
                SettingCategory::Moderation,
                true,
            ),
            evidence_capture_enabled: registry.boolean(
                "evidence_capture_enabled",
                "Evidence: trigger capture hotkey on kick/ban",
                SettingCategory::Moderation,
                false,
            ),
            evidence_capture_hotkey: registry.string_pattern(
                "evidence_capture_hotkey",
                "Evidence: capture hotkey (e.g. CTRL+SHIFT+F9) for OBS/Medal",
                SettingCategory::Moderation,
                "F9",
                None,
                true,
            ),

            // --- Scarlet.java: Discord ---
            discord_kick_ban_enabled: registry.boolean(
                "discord_kick_ban_enabled",
                "Enable built-in Discord moderation commands",
                SettingCategory::Discord,
                false,
            ),
            discord_kick_ban_prompted: registry.boolean(
                "discord_kick_ban_prompted",
                "Discord moderation prompt shown",
                SettingCategory::Discord,
                false,
            ),
            verified_role_sf: registry.string_pattern(
                "verified_role_snowflake",
                "Discord role snowflake that triggers VRChat group auto-invite",
                SettingCategory::Discord,
                "1234567890",
                Some(r"\d{1,20}"),
                true,
            ),
            members_role_sf: registry.string_pattern(
                "members_role_snowflake",
                "Discord role snowflake that members must have before being prompted to link their VRChat account",
                SettingCategory::Discord,
                "1234567890",
                Some(r"\d{1,20}"),
                true,
            ),

            // --- Scarlet.java: Integrations ---
            auto_invite_on_verify: registry.boolean(
                "auto_invite_group_on_verify",
                "Auto-invite to VRChat group when the verified role is added",
                SettingCategory::Integrations,
                true,
            ),
            not_verified_link_message: registry.string_pattern(
                "link_vrchat_manual_verify_message",
                "Message shown when a member can't be auto-verified and needs manual verification (blank to disable)",
                SettingCategory::Integrations,
                "Please open a ticket so an Orchard administrator can verify you manually.",
                None,
                true,
            ),
            auto_invite_group_id: registry.string_pattern(
                "auto_invite_group_id",
                "VRChat group ID to auto-invite verified members to",
                SettingCategory::Integrations,
                "grp_abc12345-6789-0abc-def0-123456789abc",
                Some(r"grp_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"),
                true,
            ),
            auto_close_bot_instance_minutes: registry.int_range(
                "auto_close_bot_instance_minutes",
                "Auto-close the bot's own instance this many minutes after everyone else leaves (0 = off)",
                SettingCategory::Integrations,
                0,
                0,
                1440,
            ),
            audit_polling_interval: registry.int_range(
                "audit_polling_interval",
                "Audit polling interval seconds (10-300 inclusive)",
                SettingCategory::Integrations,
                60,
                10,
                300,
            ),
            audit_catchup_max_days: registry.int_range(
                "audit_catchup_max_days",
                "After being offline, catch up on at most this many days of audit log (1-90)",
                SettingCategory::Integrations,
                14,
                1,
                90,
            ),
            instance_follow_fast_poll_seconds: registry.int_range(
                "instance_follow_fast_poll_seconds",
                "Instance follow: fast detect interval (seconds, 0=off)",
                SettingCategory::Integrations,
                30,
                0,
                300,
            ),
            advisory_translate_endpoint: registry.string_pattern(
                "advisory_translate_endpoint",
                "Advisory translation endpoint (optional; blank uses the free built-in service)",
                SettingCategory::Integrations,
                "",
                None,
                true,
            ),
            advisory_translate_api_key: registry.string_pattern(
                "advisory_translate_api_key",
                "Translation API key, or a MyMemory email to raise the free limit (optional)",
                SettingCategory::Integrations,
                "",
                None,
                true,
            ),

            // --- Scarlet.java: Notifications ---
            announce_verify_complete: registry.boolean(
                "announce_verify_complete",
                "Announce when a verified member is auto-invited (staff log channel + the user's ticket)",
                SettingCategory::Notifications,
                true,
            ),

            // --- Scarlet.java: System ---
            vibecoded_notice_acknowledged: registry.boolean(
                "vibecoded_notice_acknowledged",
                "Acknowledged the \"vibe-coded project\" notice",
                SettingCategory::System,
                false,
            ),
            // Not a direct Java field -- see scarlet_ui::setup module docs
            // for why the Discord setup step is gated on this instead of
            // Java's own "is a token stored" check.
            discord_setup_prompted: registry.boolean(
                "discord_setup_prompted",
                "Discord setup wizard has run",
                SettingCategory::System,
                false,
            ),

            // --- ScarletDiscordJDA.java: Integrations ---
            requesting_email: registry.string_pattern(
                "vrchat_report_email",
                "VRChat Help Desk report email",
                SettingCategory::Integrations,
                "",
                Some(r".+@.+"),
                false,
            ),
            append_template_footer: registry.boolean(
                "vrchat_report_template_footer",
                "Append footer to template",
                SettingCategory::Integrations,
                true,
            ),
            vrchat_client_launch_on_instance_create: registry.boolean(
                "vrchat_client_launch_on_instance_create",
                "VRChat Client: Launch on Instance Create",
                SettingCategory::Integrations,
                false,
            ),
            avatar_search_providers_enabled: registry.boolean(
                "custom_avatar_search_providers_enabled",
                "Use custom avatar search providers",
                SettingCategory::Integrations,
                false,
            ),
            avatar_search_providers: registry.string_array_pattern(
                "custom_avatar_search_providers",
                "VRCX-compatible avatar search providers",
                SettingCategory::Integrations,
                vec![
                    "https://api.avtrdb.com/v2/avatar/search/vrcx".to_string(),
                    "https://vrcavatarsearch.nekosunevr.co.uk/vrcx_search".to_string(),
                    "https://vrcx.vrcdb.com/avatars/Avatar/VRCX".to_string(),
                    "https://paw-api.amelia.fun/vrcx_search".to_string(),
                    "https://avtr.fumikoecho.net/api/integrations/avatars/vrcx".to_string(),
                ],
                Some(r"https?://.+"),
                false,
            ),

            // --- ScarletDiscordJDA.java: Discord (moderation pings) ---
            bundle_moderations_instance_kick_to_user_ban: registry.boolean(
                "discord_bundle_instance_kick_with_user_ban",
                "Discord: Bundle Instance Kicks with causing User Ban",
                SettingCategory::Discord,
                true,
            ),
            moderation_summary_only_activity: registry.boolean(
                "moderation_summary_only_activity",
                "Moderation summary: only list staff with activity",
                SettingCategory::Discord,
                true,
            ),
            ping_on_moderation_instance_warn: registry.boolean(
                "discord_ping_instance_warn",
                "Discord: Ping on Instance Warn",
                SettingCategory::Discord,
                false,
            ),
            ping_on_moderation_instance_kick: registry.boolean(
                "discord_ping_instance_kick",
                "Discord: Ping on Instance Kick",
                SettingCategory::Discord,
                false,
            ),
            ping_on_moderation_member_remove: registry.boolean(
                "discord_ping_member_remove",
                "Discord: Ping on Member Remove",
                SettingCategory::Discord,
                true,
            ),
            ping_on_moderation_user_ban: registry.boolean(
                "discord_ping_user_ban",
                "Discord: Ping on User Ban",
                SettingCategory::Discord,
                true,
            ),
            ping_on_moderation_user_unban: registry.boolean(
                "discord_ping_user_unban",
                "Discord: Ping on User Unban",
                SettingCategory::Discord,
                false,
            ),
            ping_on_outstanding_moderation_instance_warn: registry.boolean(
                "discord_ping_outstanding_instance_warn",
                "Discord: Ping on outstanding Instance Warn",
                SettingCategory::Discord,
                false,
            ),
            ping_on_outstanding_moderation_instance_kick: registry.boolean(
                "discord_ping_outstanding_instance_kick",
                "Discord: Ping on outstanding Instance Kick",
                SettingCategory::Discord,
                true,
            ),
            ping_on_outstanding_moderation_member_remove: registry.boolean(
                "discord_ping_outstanding_member_remove",
                "Discord: Ping on outstanding Member Remove",
                SettingCategory::Discord,
                false,
            ),
            ping_on_outstanding_moderation_user_ban: registry.boolean(
                "discord_ping_outstanding_user_ban",
                "Discord: Ping on outstanding User Ban",
                SettingCategory::Discord,
                true,
            ),
            ping_on_outstanding_moderation_user_unban: registry.boolean(
                "discord_ping_outstanding_user_unban",
                "Discord: Ping on outstanding User Unban",
                SettingCategory::Discord,
                false,
            ),

            // --- ScarletDiscordJDA.java: Moderation (evidence capture) ---
            evidence_enabled: registry.boolean("evidence_enabled", "Evidence submission", SettingCategory::Moderation, false),
            evidence_file_path_format: registry.string_pattern(
                "evidence_file_path_format",
                "Evidence file path format",
                SettingCategory::Moderation,
                "",
                Some(r".+"),
                false,
            ),

            // --- ScarletEventListener.java: Notifications (TTS + advisory) ---
            tts_voice_name: registry.string_choice(
                "tts_voice_name",
                "TTS: Voice name",
                SettingCategory::Notifications,
                "",
                installed_tts_voices,
            ),
            tts_use_default_audio_device: registry.boolean(
                "tts_use_default_audio_device",
                "TTS: Use default system audio device",
                SettingCategory::Notifications,
                false,
            ),
            announce_watched_users: registry.boolean(
                "tts_announce_watched_users",
                "TTS: Announce watched users",
                SettingCategory::Notifications,
                true,
            ),
            announce_watched_groups: registry.boolean(
                "tts_announce_watched_groups",
                "TTS: Announce watched groups",
                SettingCategory::Notifications,
                true,
            ),
            announce_watched_avatars: registry.boolean(
                "tts_announce_watched_avatars",
                "TTS: Announce watched avatars",
                SettingCategory::Notifications,
                true,
            ),
            announce_new_players: registry.boolean(
                "tts_announce_new_players",
                "TTS: Announce new players",
                SettingCategory::Notifications,
                true,
            ),
            announce_mixed_character_names: registry.boolean(
                "tts_announce_mixed_character_names",
                "TTS: Announce mixed-character names",
                SettingCategory::Notifications,
                true,
            ),
            announce_votes_to_kick: registry.boolean(
                "tts_announce_votes_to_kick",
                "TTS: Announce Votes-to-Kick",
                SettingCategory::Notifications,
                true,
            ),
            flag_suspicious_pronouns: registry.boolean(
                "tts_flag_suspicious_pronouns",
                "TTS: Flag suspicious pronouns",
                SettingCategory::Notifications,
                true,
            ),
            announce_suspicious_pronouns: registry.boolean(
                "tts_announce_suspicious_pronouns",
                "TTS: Announce suspicious pronouns",
                SettingCategory::Notifications,
                true,
            ),
            announce_players_newer_than_days: registry.int_range(
                "tts_announce_players_newer_than_days",
                "TTS: Announce players newer than (days)",
                SettingCategory::Notifications,
                30,
                1,
                365,
            ),
            advisory_show_watched_users: registry.boolean(
                "advisory_show_watched_users",
                "Advisory: watched users",
                SettingCategory::Notifications,
                true,
            ),
            advisory_show_watched_groups: registry.boolean(
                "advisory_show_watched_groups",
                "Advisory: watched groups",
                SettingCategory::Notifications,
                true,
            ),
            advisory_show_watched_avatars: registry.boolean(
                "advisory_show_watched_avatars",
                "Advisory: watched avatars",
                SettingCategory::Notifications,
                true,
            ),
            advisory_show_new_players: registry.boolean(
                "advisory_show_new_players",
                "Advisory: new players",
                SettingCategory::Notifications,
                true,
            ),
            advisory_show_mixed_character_names: registry.boolean(
                "advisory_show_mixed_character_names",
                "Advisory: mixed-character names",
                SettingCategory::Notifications,
                true,
            ),
            advisory_show_votes_to_kick: registry.boolean(
                "advisory_show_votes_to_kick",
                "Advisory: votes-to-kick",
                SettingCategory::Notifications,
                true,
            ),
            advisory_show_suspicious_pronouns: registry.boolean(
                "advisory_show_suspicious_pronouns",
                "Advisory: suspicious pronouns",
                SettingCategory::Notifications,
                true,
            ),
            advisory_flag_nuisance_rank: registry.boolean(
                "advisory_flag_nuisance_rank",
                "Advisory: flag Nuisance trust rank",
                SettingCategory::Notifications,
                true,
            ),
            advisory_flag_visitor_rank: registry.boolean(
                "advisory_flag_visitor_rank",
                "Advisory: flag Visitor trust rank",
                SettingCategory::Notifications,
                false,
            ),
            announce_nuisance_rank: registry.boolean(
                "tts_announce_nuisance_rank",
                "TTS: announce Nuisance rank (with alarm chime)",
                SettingCategory::Notifications,
                true,
            ),
            announce_visitor_rank: registry.boolean(
                "tts_announce_visitor_rank",
                "TTS: announce Visitor rank",
                SettingCategory::Notifications,
                false,
            ),

            // --- ScarletEventListener.java: Moderation ---
            attempt_avatar_image_match: registry.boolean(
                "attempt_avatar_image_match",
                "Attempt avatar image match",
                SettingCategory::Moderation,
                false,
            ),

            // --- ScarletMobile.java: Integrations (mobile companion pairing) ---
            mobile_enabled: registry.boolean("mobile_enabled", "Mobile companion", SettingCategory::Integrations, false),
            mobile_fcm_service_account: registry.string_pattern(
                "mobile_fcm_service_account",
                "Mobile FCM service account JSON",
                SettingCategory::Integrations,
                "",
                None,
                false,
            ),
            mobile_relay_endpoint: registry.string_pattern(
                "mobile_relay_endpoint",
                "Mobile relay endpoint (optional, https)",
                SettingCategory::Integrations,
                "https://peachpuff-swan-183728.hostingersite.com/scarlet/mobile/event",
                Some(r"https://.+"),
                false,
            ),

            // --- ScarletMobile.java: Notifications (mobile push + desktop toast) ---
            mobile_min_severity: registry.enum_setting(
                "mobile_min_severity",
                "Mobile minimum severity",
                SettingCategory::Notifications,
                NotifySeverity::Watch,
                &NotifySeverity::ALL,
            ),
            mobile_notify_watched_users: registry.boolean(
                "mobile_notify_watched_users",
                "Mobile: watched users",
                SettingCategory::Notifications,
                true,
            ),
            mobile_notify_watched_groups: registry.boolean(
                "mobile_notify_watched_groups",
                "Mobile: watched groups",
                SettingCategory::Notifications,
                true,
            ),
            mobile_notify_watched_avatars: registry.boolean(
                "mobile_notify_watched_avatars",
                "Mobile: watched avatars",
                SettingCategory::Notifications,
                true,
            ),
            mobile_notify_votes_to_kick: registry.boolean(
                "mobile_notify_votes_to_kick",
                "Mobile: votes-to-kick",
                SettingCategory::Notifications,
                true,
            ),
            mobile_notify_moderation: registry.boolean(
                "mobile_notify_moderation",
                "Mobile: moderation",
                SettingCategory::Notifications,
                true,
            ),
            mobile_notify_staff: registry.boolean(
                "mobile_notify_staff",
                "Mobile: staff joins/leaves",
                SettingCategory::Notifications,
                false,
            ),
            mobile_notify_new_players: registry.boolean(
                "mobile_notify_new_players",
                "Mobile: new players",
                SettingCategory::Notifications,
                false,
            ),
            mobile_notify_mixed_character_names: registry.boolean(
                "mobile_notify_mixed_character_names",
                "Mobile: mixed-character names",
                SettingCategory::Notifications,
                false,
            ),
            mobile_notify_suspicious_pronouns: registry.boolean(
                "mobile_notify_suspicious_pronouns",
                "Mobile: suspicious pronouns",
                SettingCategory::Notifications,
                true,
            ),
            notify_desktop_enabled: registry.boolean(
                "notify_desktop_enabled",
                "Desktop notifications",
                SettingCategory::Notifications,
                false,
            ),
            toast_notify_watched_users: registry.boolean(
                "toast_notify_watched_users",
                "Toast: watched users",
                SettingCategory::Notifications,
                true,
            ),
            toast_notify_watched_groups: registry.boolean(
                "toast_notify_watched_groups",
                "Toast: watched groups",
                SettingCategory::Notifications,
                true,
            ),
            toast_notify_watched_avatars: registry.boolean(
                "toast_notify_watched_avatars",
                "Toast: watched avatars",
                SettingCategory::Notifications,
                true,
            ),
            toast_notify_votes_to_kick: registry.boolean(
                "toast_notify_votes_to_kick",
                "Toast: votes-to-kick",
                SettingCategory::Notifications,
                true,
            ),
            toast_notify_moderation: registry.boolean(
                "toast_notify_moderation",
                "Toast: moderation",
                SettingCategory::Notifications,
                true,
            ),
            toast_notify_staff: registry.boolean(
                "toast_notify_staff",
                "Toast: staff joins/leaves",
                SettingCategory::Notifications,
                false,
            ),
            toast_notify_new_players: registry.boolean(
                "toast_notify_new_players",
                "Toast: new players",
                SettingCategory::Notifications,
                false,
            ),
            toast_notify_mixed_character_names: registry.boolean(
                "toast_notify_mixed_character_names",
                "Toast: mixed-character names",
                SettingCategory::Notifications,
                false,
            ),
            toast_notify_suspicious_pronouns: registry.boolean(
                "toast_notify_suspicious_pronouns",
                "Toast: suspicious pronouns",
                SettingCategory::Notifications,
                true,
            ),

            registry,
            store,
            registry_store,
            encrypted,
            settings_file: settings_path,
            settings_file_last_modified,
            has_version_changed_since_last_run: std::sync::Mutex::new(None),
        }
    }

    /// Port of `checkHasVersionChangedSinceLastRun`.
    pub fn check_has_version_changed_since_last_run(&self, current_version: &str) -> bool {
        if let Some(cached) = *self.has_version_changed_since_last_run.lock().unwrap_or_else(|e| e.into_inner()) {
            return cached;
        }
        let changed = match self.last_run_time.get_or_null() {
            None => true,
            Some(last_run_time) => {
                // Java's `File.lastModified()` returns `0` (epoch) when the
                // file doesn't exist, not "now" -- falling back to
                // `Utc::now()` here would flip the meaning of "no settings
                // file yet" from "definitely not modified after last run"
                // to "just modified this instant", spuriously tripping the
                // check below on every slow startup (PBKDF2 key derivation
                // alone can take over a second in an unoptimized build).
                let settings_modified: DateTime<Utc> =
                    self.settings_file_last_modified.map(DateTime::<Utc>::from).unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
                if settings_modified > last_run_time + chrono::Duration::milliseconds(1_000) {
                    true
                } else {
                    self.last_run_version.get_or_null().as_deref() != Some(current_version)
                }
            }
        };
        *self.has_version_changed_since_last_run.lock().unwrap_or_else(|e| e.into_inner()) = Some(changed);
        changed
    }

    /// Port of `updateRunVersionAndTime`.
    pub fn update_run_version_and_time(&self, current_version: &str) {
        self.last_run_version.set(Some(current_version.to_string()));
        self.last_run_time.set(Some(Utc::now()));
    }

    pub fn settings_file(&self) -> &Path {
        &self.settings_file
    }
}

/// Enumerates voices from the platform engine used by the Rust desktop TTS
/// service. Kept here so `FileValuedStringChoice` validates the same values
/// the Settings UI offers instead of rejecting every non-empty voice.
/// Every installed speech voice, as `engine/voice`.
///
/// Previously this asked only eSpeak, and returned on the first engine that
/// answered — so a machine with eSpeak NG installed never saw Flite or
/// Pico voices at all. [`scarlet_core::tts_voices`] ports Java's full
/// discovery.
pub fn installed_tts_voices() -> Vec<String> {
    crate::tts_voices::installed()
}


#[cfg(test)]
mod tests {
    /// Every registered setting needs a description, or the list goes back
    /// to showing a bare label with nothing explaining it. Failing here
    /// names the ids that were added without copy.
    #[test]
    fn every_setting_has_a_description() {
        let dir = std::env::temp_dir().join(format!(
            "celeste-settings-help-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = Settings::open(&dir);
        let missing: Vec<String> = settings
            .registry
            .all()
            .iter()
            .map(|descriptor| descriptor.id().to_string())
            .filter(|id| super::help::describe(id).is_none())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(missing.is_empty(), "settings with no description: {missing:?}");
    }

    use super::*;

    #[test]
    fn fresh_install_has_changed_version() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        assert!(settings.check_has_version_changed_since_last_run("0.4.20"));
    }

    #[test]
    fn matching_version_and_recent_run_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        settings.update_run_version_and_time("0.4.20");
        // Re-check must not be cached from the earlier fresh-install call
        // in this same instance, so open a second handle onto the same dir.
        let settings2 = Settings::open(dir.path());
        assert!(!settings2.check_has_version_changed_since_last_run("0.4.20"));
    }

    #[test]
    fn different_version_is_changed() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        settings.update_run_version_and_time("0.4.19");
        let settings2 = Settings::open(dir.path());
        assert!(settings2.check_has_version_changed_since_last_run("0.4.20"));
    }

    #[test]
    fn global_password_persists_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let a = resolve_global_password(dir.path());
        let b = resolve_global_password(dir.path());
        assert_eq!(a, b);
    }

    #[test]
    fn env_var_overrides_persisted_key() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("SCARLET_GLOBAL_PW", "explicit-password");
        assert_eq!(resolve_global_password(dir.path()), "explicit-password");
        std::env::remove_var("SCARLET_GLOBAL_PW");
    }

    #[test]
    fn heuristic_settings_have_javas_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        assert_eq!(settings.heuristic_kick_count.get(), 3);
        assert_eq!(settings.heuristic_period_days.get(), 3);
        assert_eq!(settings.outstanding_period_days.get(), 3);
        assert!(settings.cache_cleanup_enabled.get());
        assert_eq!(settings.cache_cleanup_days.get(), 7);
    }

    #[test]
    fn registry_has_every_category_populated_with_no_duplicate_ids() {
        // Guards the big block of FileValued registrations in `open()`:
        // every category the Settings tab groups by actually has at least
        // one setting (nothing silently mis-typed into a category that's
        // never checked), and `SettingsRegistry::register_id`'s panic on a
        // duplicate id would already fail `Settings::open` above, but this
        // also confirms the count matches expectations so a future typo
        // that merges two settings under one id doesn't shrink silently.
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::open(dir.path());
        let all = settings.registry.all();

        let mut ids = std::collections::HashSet::new();
        for descriptor in &all {
            assert!(ids.insert(descriptor.id().to_string()), "duplicate setting id: {}", descriptor.id());
        }

        for category in file_valued::SettingCategory::ALL {
            let count = all.iter().filter(|d| d.category() == category).count();
            assert!(count > 0, "no settings registered under {:?}", category);
        }

        // Sanity floor rather than an exact count, so this doesn't need
        // updating every time a setting is added -- but it does fail hard
        // if the big registration block regresses to only the original 5.
        assert!(all.len() > 80, "expected the full settings inventory, only found {}", all.len());
    }
}
