//! One plain-language line per setting, describing what it does.
//!
//! The settings list used to print each setting's raw id under its name —
//! `ui_left_player_dim_percent` beneath "Dim players who left (%)". That
//! tells you how the setting is stored, which is the one thing a person
//! changing it does not need to know, while leaving the actual question
//! ("what happens if I turn this on?") unanswered. The Actions cards in the
//! same window already explained themselves; the settings rows did not.
//!
//! Copy rules, so these stay consistent as settings are added:
//! say what the setting *does*, in the present tense, from the point of
//! view of someone deciding whether to change it; name things as the
//! interface names them; one sentence; no "this setting…" preamble; no
//! restating the label.
//!
//! [`describe`] returning `None` is a gap, and the test at the bottom of
//! this file fails with the list of ids that have one.

/// What `id` does, as a sentence to show beneath its label.
pub fn describe(id: &str) -> Option<&'static str> {
    Some(match id {
        // ── Advisories: the line shown against a player in the table ────
        "advisory_show_watched_users" => "Flags players on your watched-users list when they join.",
        "advisory_show_watched_groups" => "Flags players who represent a watched group when they join.",
        "advisory_show_watched_avatars" => "Flags players wearing a watched avatar.",
        "advisory_show_new_players" => "Flags accounts younger than the age set below.",
        "advisory_show_suspicious_pronouns" => "Flags pronoun fields matching your bad-pronoun list.",
        "advisory_show_mixed_character_names" => "Flags display names that mix scripts, a common way to impersonate another member.",
        "advisory_show_votes_to_kick" => "Flags players the instance has started a vote-kick against.",
        "advisory_flag_visitor_rank" => "Flags players still on Visitor rank.",
        "advisory_flag_nuisance_rank" => "Flags players VRChat has marked as a probable nuisance.",
        "advisory_translate_endpoint" => "LibreTranslate server used to translate advisory text. Leave empty to skip translation.",
        "advisory_translate_api_key" => "API key for that translation server, if it needs one.",

        // ── Spoken announcements ─────────────────────────────────────────
        "tts_announce_watched_users" => "Speaks a warning when a watched user joins.",
        "tts_announce_watched_groups" => "Speaks a warning when someone representing a watched group joins.",
        "tts_announce_watched_avatars" => "Speaks a warning when someone wearing a watched avatar joins.",
        "tts_announce_new_players" => "Speaks a warning when an account younger than the age below joins.",
        "tts_announce_players_newer_than_days" => "How new an account has to be, in days, to count as a new player.",
        "tts_announce_suspicious_pronouns" => "Speaks a warning when a pronoun field matches your bad-pronoun list.",
        "tts_announce_mixed_character_names" => "Speaks a warning for display names that mix scripts.",
        "tts_announce_votes_to_kick" => "Speaks a warning when a vote-kick starts.",
        "tts_announce_visitor_rank" => "Speaks a warning when a Visitor-rank player joins.",
        "tts_announce_nuisance_rank" => "Speaks a warning when a probable-nuisance player joins.",
        "tts_flag_suspicious_pronouns" => "Reads the pronoun itself aloud as part of the warning, rather than only saying one was flagged.",
        "tts_volume_percent" => "How loud spoken callouts and the alert chime play through this computer, as a percentage of the engine's own level. Takes effect on the next announcement. Does not affect Discord voice, where the level belongs to the channel rather than to this machine.",
        "tts_voice_name" => "Which installed system voice to speak with. Leave empty for the default.",
        "tts_use_default_audio_device" => "Speaks through this computer's audio output. Turn off to speak into the configured Discord voice channel instead.",

        // ── Desktop notifications ────────────────────────────────────────
        "notify_desktop_enabled" => "Allows desktop notifications at all. The toggles below choose which events raise one.",
        "toast_notify_watched_users" => "Raises a desktop notification when a watched user joins.",
        "toast_notify_watched_groups" => "Raises one when someone representing a watched group joins.",
        "toast_notify_watched_avatars" => "Raises one when someone wearing a watched avatar joins.",
        "toast_notify_new_players" => "Raises one when a new account joins.",
        "toast_notify_suspicious_pronouns" => "Raises one for a flagged pronoun field.",
        "toast_notify_mixed_character_names" => "Raises one for a mixed-script display name.",
        "toast_notify_votes_to_kick" => "Raises one when a vote-kick starts.",
        "toast_notify_moderation" => "Raises one when a moderation action is logged.",
        "toast_notify_staff" => "Raises one when someone on your staff list joins.",

        // ── Instance enforcement ─────────────────────────────────────────
        "enforce_instances_18_plus" => "Closes group instances that are not age-gated to 18+.",
        "enforce_instances_worlds" => "Closes group instances opened in a world that is not on the list below.",
        "enforce_instances_world_list" => "World ids group instances are allowed to use, one per line.",
        "auto_close_bot_instance_minutes" => "Closes Celeste's own instance after this many minutes with nobody else in it. Zero leaves it open.",
        "instance_follow_fast_poll_seconds" => "How often to check for newly created group instances, in seconds.",
        "vrchat_client_launch_on_instance_create" => "Launches VRChat into a group instance as soon as one is created.",

        // ── Moderation ───────────────────────────────────────────────────
        "heuristicKickCount" => "How many instance kicks a player can collect within the period below before Celeste flags them for a ban.",
        "heuristicPeriodDays" => "How far back, in days, those kicks are counted.",
        "outstandingPeriodDays" => "How far back /outstanding-moderation looks for events still missing tags or a description.",
        "moderation_log_lookback_days" => "How far back /moderation-log reads a user's history, in days.",
        "moderation_summary_only_activity" => "Limits the moderation summary to periods that actually had activity.",
        "timed_bans_enabled" => "Shows the timed-ban buttons on moderation threads, which ban for a fixed period and lift automatically.",
        "audit_polling_interval" => "How often to poll VRChat for new group audit events, in seconds.",
        "audit_catchup_max_days" => "How much backlog to work through on startup, in days, after Celeste has been closed for a while.",
        "attempt_avatar_image_match" => "Identifies a player's avatar by matching its image when the name alone is ambiguous. Costs extra API calls.",
        "training_mode_enabled" => "Routes simulated moderation events to the training channel instead of the real log.",

        // ── Discord: pings on moderation events ──────────────────────────
        "discord_ping_instance_warn" => "Mentions the acting moderator when an instance warning is logged.",
        "discord_ping_instance_kick" => "Mentions them when an instance kick is logged.",
        "discord_ping_member_remove" => "Mentions them when a member is removed from the group.",
        "discord_ping_user_ban" => "Mentions them when a user is banned.",
        "discord_ping_user_unban" => "Mentions them when a user is unbanned.",
        "discord_ping_outstanding_instance_warn" => "Mentions them again if a warning is still untagged and undescribed.",
        "discord_ping_outstanding_instance_kick" => "Mentions them again if a kick is still untagged and undescribed.",
        "discord_ping_outstanding_member_remove" => "Mentions them again if a removal is still untagged and undescribed.",
        "discord_ping_outstanding_user_ban" => "Mentions them again if a ban is still untagged and undescribed.",
        "discord_ping_outstanding_user_unban" => "Mentions them again if an unban is still untagged and undescribed.",
        "discord_bundle_instance_kick_with_user_ban" => "Posts an instance kick inside the ban thread it caused, rather than as its own entry.",

        // ── Discord: server ──────────────────────────────────────────────
        "discord_kick_ban_enabled" => "Enables /discord-kick and /discord-ban for moderating the Discord server itself.",
        "verified_role_snowflake" => "Role given to someone who links their VRChat account.",
        "members_role_snowflake" => "Role given to someone confirmed as a group member.",
        "announce_verify_complete" => "Posts a message when someone finishes linking their account.",
        "auto_invite_group_on_verify" => "Sends a VRChat group invite once someone links their account.",
        "auto_invite_group_id" => "Which group that invite is for.",
        "link_vrchat_manual_verify_message" => "Message shown to someone who has to verify manually.",

        // ── VRChat reports ───────────────────────────────────────────────
        "vrchat_report_email" => "Address VRChat replies to when you file a report.",
        "vrchat_report_template_footer" => "Text appended to the bottom of every generated report.",

        // ── Avatar search ────────────────────────────────────────────────
        "custom_avatar_search_providers_enabled" => "Searches your chosen avatar databases instead of the built-in set.",
        "custom_avatar_search_providers" => "Which avatar databases to search.",

        // ── Evidence capture ─────────────────────────────────────────────
        "evidence_enabled" => "Enables evidence capture.",
        "evidence_capture_enabled" => "Presses the capture hotkey automatically after a moderation action.",
        "evidence_capture_hotkey" => "The key combination to press.",
        "evidence_file_path_format" => "Where captures are saved, and how they are named.",

        // ── Maintenance ──────────────────────────────────────────────────
        "cache_cleanup_enabled" => "Deletes old caches, logs and temporary audio on a schedule.",
        "cache_cleanup_days" => "How old a file has to be before cleanup removes it, in days.",

        // ── Interface ────────────────────────────────────────────────────
        "graphics_renderer" => "Chooses the graphics API used to draw Celeste. Automatic is recommended: it tries each API your machine has and falls back to software rendering on a virtual machine or a system with no graphics driver. Pick Software to force that fallback, or another API to work around a graphics-driver problem. Restart Celeste to apply.",
        "ui_left_player_dim_percent" => "How far to fade players who have left, so they stay readable without competing with the people still present.",
        "ui_accent_headers" => "Tints section headings with your accent colour.",
        "ui_confirm_group_invite" => "Asks before sending a group invite.",
        "ui_show_during_load" => "Opens the window while Celeste is still starting, rather than when it is ready.",
        "ui_show_cli_tab" => "Shows the CLI tab.",

        // ── Remembered answers, not really settings ──────────────────────
        "vibecoded_notice_acknowledged" => "Remembers that you have read the first-run notice.",
        "setup_profile_prompted" => "Remembers that you selected an initial setup type.",
        "tutorial_progress_step" => "Remembers how far you got in the guided tour so Help can offer to resume it.",
        "discord_setup_prompted" => "Remembers that you have been offered Discord setup.",
        "vrchat_setup_prompted" => "Remembers that you have been offered VRChat setup.",
        "discord_kick_ban_prompted" => "Remembers that you have been asked about the Discord moderation commands.",

        // ── Mobile companion (out of scope; hidden in the UI) ────────────
        "mobile_enabled" => "Enables the mobile companion.",
        "mobile_relay_endpoint" => "Relay server the companion connects through.",
        "mobile_fcm_service_account" => "Service account used to send push notifications.",
        "mobile_min_severity" => "Lowest severity worth sending to a phone.",
        "mobile_notify_watched_users" => "Pushes a notification when a watched user joins.",
        "mobile_notify_watched_groups" => "Pushes one when someone representing a watched group joins.",
        "mobile_notify_watched_avatars" => "Pushes one when someone wearing a watched avatar joins.",
        "mobile_notify_new_players" => "Pushes one when a new account joins.",
        "mobile_notify_suspicious_pronouns" => "Pushes one for a flagged pronoun field.",
        "mobile_notify_mixed_character_names" => "Pushes one for a mixed-script display name.",
        "mobile_notify_votes_to_kick" => "Pushes one when a vote-kick starts.",
        "mobile_notify_moderation" => "Pushes one when a moderation action is logged.",
        "mobile_notify_staff" => "Pushes one when a staff member joins.",

        _ => return None,
    })
}
