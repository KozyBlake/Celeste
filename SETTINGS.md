# Celeste settings reference

**This file is generated.** It is built from the settings registry by
`crates/celeste-core/src/settings/doc.rs`, and a test fails if it falls out
of date. To regenerate after adding or changing a setting:

```sh
cd rust && UPDATE_SETTINGS_MD=1 cargo test -p celeste-core settings_md_is_current
```

Edit the description in `settings/help.rs`, not this file.

## Where settings live

| | |
|---|---|
| Linux | `$XDG_DATA_HOME/KozyBlake/Celeste/settings.json`, or `~/.local/share/KozyBlake/Celeste/settings.json` |
| Windows | `%LOCALAPPDATA%\KozyBlake\Celeste\settings.json` |

Celeste also discovers the legacy `KozyBlake/Scarlet` locations and adopts an
existing install rather than starting empty. `CELESTE_HOME` (or the legacy
`SCARLET_HOME`) overrides the folder for one run, and **Settings → System**
moves it permanently.

Everything below is editable in the desktop app under its category heading,
or from Discord with `/settings edit`.

## General

| Setting | Id | Type | What it does |
|---|---|---|---|
| Colour section headers with accent | `ui_accent_headers` | boolean (default `false`) | Tints section headings with your accent colour. |
| Confirmation dialog for group invites | `ui_confirm_group_invite` | boolean (default `false`) | Asks before sending a group invite. |
| Desktop UI language (restart to apply) | `ui_language` | one of a list discovered at runtime | Language for the desktop window. Takes effect after a restart. |
| Dim players who left (%) | `ui_left_player_dim_percent` | integer `0`–`80` (default `35`) | How far to fade players who have left, so they stay readable without competing with the people still present. |
| Show CLI tab | `ui_show_cli_tab` | boolean (default `true`) | Shows the CLI tab. |
| Show UI during load | `ui_show_during_load` | boolean (default `false`) | Opens the window while Celeste is still starting, rather than when it is ready. |

## Moderation

| Setting | Id | Type | What it does |
|---|---|---|---|
| Attempt avatar image match | `attempt_avatar_image_match` | boolean (default `false`) | Identifies a player's avatar by matching its image when the name alone is ambiguous. Costs extra API calls. |
| Instances: enforce 18+ | `enforce_instances_18_plus` | one of `DISABLED`, `ENABLED_ONLY_18_PLUS`, `ENABLED_NEVER_18_PLUS` (default `DISABLED`) | Closes group instances that are not age-gated to 18+. |
| Instances: enforce world list | `enforce_instances_world_list` | list of text, each matching `wr?ld_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}` | World ids group instances are allowed to use, one per line. |
| Instances: enforce worlds | `enforce_instances_worlds` | one of `DISABLED`, `ENABLED_WHITELIST`, `ENABLED_BLACKLIST` (default `DISABLED`) | Closes group instances opened in a world that is not on the list below. |
| Evidence: trigger capture hotkey on kick/ban | `evidence_capture_enabled` | boolean (default `false`) | Presses the capture hotkey automatically after a moderation action. |
| Evidence: capture hotkey (e.g. CTRL+SHIFT+F9) for OBS/Medal | `evidence_capture_hotkey` | text | The key combination to press. |
| Evidence submission | `evidence_enabled` | boolean (default `false`) | Enables evidence capture. |
| Evidence file path format | `evidence_file_path_format` | text matching `.+` | Where captures are saved, and how they are named. |
| Heuristic Kick Count | `heuristicKickCount` | integer `1`–`10` (default `3`) | How many instance kicks a player can collect within the period below before Celeste flags them for a ban. |
| Heuristic Period (days) | `heuristicPeriodDays` | integer `1`–`30` (default `3`) | How far back, in days, those kicks are counted. |
| Moderation log: include history from the past N days (0 = all time) | `moderation_log_lookback_days` | integer `0`–`3650` (default `0`) | How far back /moderation-log reads a user's history, in days. |
| Outstanding Period (days) | `outstandingPeriodDays` | integer `1`–`30` (default `3`) | How far back /outstanding-moderation looks for events still missing tags or a description. |
| Discord: show timed-ban buttons on moderation posts | `timed_bans_enabled` | boolean (default `true`) | Shows the timed-ban buttons on moderation threads, which ban for a fixed period and lift automatically. |
| Training mode (enable the event simulator) | `training_mode_enabled` | boolean (default `false`) | Routes simulated moderation events to the training channel instead of the real log. |

## Notifications

| Setting | Id | Type | What it does |
|---|---|---|---|
| Advisory: flag Nuisance trust rank | `advisory_flag_nuisance_rank` | boolean (default `true`) | Flags players VRChat has marked as a probable nuisance. |
| Advisory: flag Visitor trust rank | `advisory_flag_visitor_rank` | boolean (default `false`) | Flags players still on Visitor rank. |
| Advisory: mixed-character names | `advisory_show_mixed_character_names` | boolean (default `true`) | Flags display names that mix scripts, a common way to impersonate another member. |
| Advisory: new players | `advisory_show_new_players` | boolean (default `true`) | Flags accounts younger than the age set below. |
| Advisory: suspicious pronouns | `advisory_show_suspicious_pronouns` | boolean (default `true`) | Flags pronoun fields matching your bad-pronoun list. |
| Advisory: votes-to-kick | `advisory_show_votes_to_kick` | boolean (default `true`) | Flags players the instance has started a vote-kick against. |
| Advisory: watched avatars | `advisory_show_watched_avatars` | boolean (default `true`) | Flags players wearing a watched avatar. |
| Advisory: watched groups | `advisory_show_watched_groups` | boolean (default `true`) | Flags players who represent a watched group when they join. |
| Advisory: watched users | `advisory_show_watched_users` | boolean (default `true`) | Flags players on your watched-users list when they join. |
| Announce when a verified member is auto-invited (staff log channel + the user's ticket) | `announce_verify_complete` | boolean (default `true`) | Posts a message when someone finishes linking their account. |
| Mobile minimum severity | `mobile_min_severity` | one of `INFO`, `WATCH`, `WARNING`, `CRITICAL` (default `WATCH`) | Lowest severity worth sending to a phone. |
| Mobile: mixed-character names | `mobile_notify_mixed_character_names` | boolean (default `false`) | Pushes one for a mixed-script display name. |
| Mobile: moderation | `mobile_notify_moderation` | boolean (default `true`) | Pushes one when a moderation action is logged. |
| Mobile: new players | `mobile_notify_new_players` | boolean (default `false`) | Pushes one when a new account joins. |
| Mobile: staff joins/leaves | `mobile_notify_staff` | boolean (default `false`) | Pushes one when a staff member joins. |
| Mobile: suspicious pronouns | `mobile_notify_suspicious_pronouns` | boolean (default `true`) | Pushes one for a flagged pronoun field. |
| Mobile: votes-to-kick | `mobile_notify_votes_to_kick` | boolean (default `true`) | Pushes one when a vote-kick starts. |
| Mobile: watched avatars | `mobile_notify_watched_avatars` | boolean (default `true`) | Pushes one when someone wearing a watched avatar joins. |
| Mobile: watched groups | `mobile_notify_watched_groups` | boolean (default `true`) | Pushes one when someone representing a watched group joins. |
| Mobile: watched users | `mobile_notify_watched_users` | boolean (default `true`) | Pushes a notification when a watched user joins. |
| Desktop notifications | `notify_desktop_enabled` | boolean (default `false`) | Allows desktop notifications at all. The toggles below choose which events raise one. |
| Toast: mixed-character names | `toast_notify_mixed_character_names` | boolean (default `false`) | Raises one for a mixed-script display name. |
| Toast: moderation | `toast_notify_moderation` | boolean (default `true`) | Raises one when a moderation action is logged. |
| Toast: new players | `toast_notify_new_players` | boolean (default `false`) | Raises one when a new account joins. |
| Toast: staff joins/leaves | `toast_notify_staff` | boolean (default `false`) | Raises one when someone on your staff list joins. |
| Toast: suspicious pronouns | `toast_notify_suspicious_pronouns` | boolean (default `true`) | Raises one for a flagged pronoun field. |
| Toast: votes-to-kick | `toast_notify_votes_to_kick` | boolean (default `true`) | Raises one when a vote-kick starts. |
| Toast: watched avatars | `toast_notify_watched_avatars` | boolean (default `true`) | Raises one when someone wearing a watched avatar joins. |
| Toast: watched groups | `toast_notify_watched_groups` | boolean (default `true`) | Raises one when someone representing a watched group joins. |
| Toast: watched users | `toast_notify_watched_users` | boolean (default `true`) | Raises a desktop notification when a watched user joins. |
| TTS: Announce mixed-character names | `tts_announce_mixed_character_names` | boolean (default `true`) | Speaks a warning for display names that mix scripts. |
| TTS: Announce new players | `tts_announce_new_players` | boolean (default `true`) | Speaks a warning when an account younger than the age below joins. |
| TTS: announce Nuisance rank (with alarm chime) | `tts_announce_nuisance_rank` | boolean (default `true`) | Speaks a warning when a probable-nuisance player joins. |
| TTS: Announce players newer than (days) | `tts_announce_players_newer_than_days` | integer `1`–`365` (default `30`) | How new an account has to be, in days, to count as a new player. |
| TTS: Announce suspicious pronouns | `tts_announce_suspicious_pronouns` | boolean (default `true`) | Speaks a warning when a pronoun field matches your bad-pronoun list. |
| TTS: announce Visitor rank | `tts_announce_visitor_rank` | boolean (default `false`) | Speaks a warning when a Visitor-rank player joins. |
| TTS: Announce Votes-to-Kick | `tts_announce_votes_to_kick` | boolean (default `true`) | Speaks a warning when a vote-kick starts. |
| TTS: Announce watched avatars | `tts_announce_watched_avatars` | boolean (default `true`) | Speaks a warning when someone wearing a watched avatar joins. |
| TTS: Announce watched groups | `tts_announce_watched_groups` | boolean (default `true`) | Speaks a warning when someone representing a watched group joins. |
| TTS: Announce watched users | `tts_announce_watched_users` | boolean (default `true`) | Speaks a warning when a watched user joins. |
| TTS: Flag suspicious pronouns | `tts_flag_suspicious_pronouns` | boolean (default `true`) | Reads the pronoun itself aloud as part of the warning, rather than only saying one was flagged. |
| TTS: Use default system audio device | `tts_use_default_audio_device` | boolean (default `true`) | Speaks through this computer's audio output. Turn off to speak into the configured Discord voice channel instead. |
| TTS: Voice name | `tts_voice_name` | one of a list discovered at runtime | Which installed system voice to speak with. Leave empty for the default. |

## Discord

| Setting | Id | Type | What it does |
|---|---|---|---|
| Discord: Bundle Instance Kicks with causing User Ban | `discord_bundle_instance_kick_with_user_ban` | boolean (default `true`) | Posts an instance kick inside the ban thread it caused, rather than as its own entry. |
| Enable built-in Discord moderation commands | `discord_kick_ban_enabled` | boolean (default `false`) | Enables /discord-kick and /discord-ban for moderating the Discord server itself. |
| Discord moderation prompt shown | `discord_kick_ban_prompted` | boolean (default `false`) | Remembers that you have been asked about the Discord moderation commands. |
| Discord: Ping on Instance Kick | `discord_ping_instance_kick` | boolean (default `false`) | Mentions them when an instance kick is logged. |
| Discord: Ping on Instance Warn | `discord_ping_instance_warn` | boolean (default `false`) | Mentions the acting moderator when an instance warning is logged. |
| Discord: Ping on Member Remove | `discord_ping_member_remove` | boolean (default `true`) | Mentions them when a member is removed from the group. |
| Discord: Ping on outstanding Instance Kick | `discord_ping_outstanding_instance_kick` | boolean (default `true`) | Mentions them again if a kick is still untagged and undescribed. |
| Discord: Ping on outstanding Instance Warn | `discord_ping_outstanding_instance_warn` | boolean (default `false`) | Mentions them again if a warning is still untagged and undescribed. |
| Discord: Ping on outstanding Member Remove | `discord_ping_outstanding_member_remove` | boolean (default `false`) | Mentions them again if a removal is still untagged and undescribed. |
| Discord: Ping on outstanding User Ban | `discord_ping_outstanding_user_ban` | boolean (default `true`) | Mentions them again if a ban is still untagged and undescribed. |
| Discord: Ping on outstanding User Unban | `discord_ping_outstanding_user_unban` | boolean (default `false`) | Mentions them again if an unban is still untagged and undescribed. |
| Discord: Ping on User Ban | `discord_ping_user_ban` | boolean (default `true`) | Mentions them when a user is banned. |
| Discord: Ping on User Unban | `discord_ping_user_unban` | boolean (default `false`) | Mentions them when a user is unbanned. |
| Discord role snowflake that members must have before being prompted to link their VRChat account | `members_role_snowflake` | text matching `\d{1,20}` | Role given to someone confirmed as a group member. |
| Moderation summary: only list staff with activity | `moderation_summary_only_activity` | boolean (default `true`) | Limits the moderation summary to periods that actually had activity. |
| Discord role snowflake that triggers VRChat group auto-invite | `verified_role_snowflake` | text matching `\d{1,20}` | Role given to someone who links their VRChat account. |

## Integrations

| Setting | Id | Type | What it does |
|---|---|---|---|
| Translation API key, or a MyMemory email to raise the free limit (optional) | `advisory_translate_api_key` | text | API key for that translation server, if it needs one. |
| Advisory translation endpoint (optional; blank uses the free built-in service) | `advisory_translate_endpoint` | text | LibreTranslate server used to translate advisory text. Leave empty to skip translation. |
| After being offline, catch up on at most this many days of audit log (1-90) | `audit_catchup_max_days` | integer `1`–`90` (default `14`) | How much backlog to work through on startup, in days, after Celeste has been closed for a while. |
| Audit polling interval seconds (10-300 inclusive) | `audit_polling_interval` | integer `10`–`300` (default `60`) | How often to poll VRChat for new group audit events, in seconds. |
| Auto-close the bot's own instance this many minutes after everyone else leaves (0 = off) | `auto_close_bot_instance_minutes` | integer `0`–`1440` (default `0`) | Closes Celeste's own instance after this many minutes with nobody else in it. Zero leaves it open. |
| VRChat group ID to auto-invite verified members to | `auto_invite_group_id` | text matching `grp_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}` | Which group that invite is for. |
| Auto-invite to VRChat group when the verified role is added | `auto_invite_group_on_verify` | boolean (default `true`) | Sends a VRChat group invite once someone links their account. |
| VRCX-compatible avatar search providers | `custom_avatar_search_providers` | list of text, each matching `https?://.+` | Which avatar databases to search. |
| Use custom avatar search providers | `custom_avatar_search_providers_enabled` | boolean (default `false`) | Searches your chosen avatar databases instead of the built-in set. |
| Instance follow: fast detect interval (seconds, 0=off) | `instance_follow_fast_poll_seconds` | integer `0`–`300` (default `30`) | How often to check for newly created group instances, in seconds. |
| Message shown when a member can't be auto-verified and needs manual verification (blank to disable) | `link_vrchat_manual_verify_message` | text | Message shown to someone who has to verify manually. |
| Mobile companion | `mobile_enabled` | boolean (default `false`) | Enables the mobile companion. |
| Mobile FCM service account JSON | `mobile_fcm_service_account` | text | Service account used to send push notifications. |
| Mobile relay endpoint (optional, https) | `mobile_relay_endpoint` | text matching `https://.+` | Relay server the companion connects through. |
| VRChat Client: Launch on Instance Create | `vrchat_client_launch_on_instance_create` | boolean (default `false`) | Launches VRChat into a group instance as soon as one is created. |
| VRChat Help Desk report email | `vrchat_report_email` | text matching `.+@.+` | Address VRChat replies to when you file a report. |
| Append footer to template | `vrchat_report_template_footer` | boolean (default `true`) | Text appended to the bottom of every generated report. |

## System

| Setting | Id | Type | What it does |
|---|---|---|---|
| Delete files older than (days) | `cache_cleanup_days` | integer `1`–`30` (default `7`) | How old a file has to be before cleanup removes it, in days. |
| Auto-delete old cached files | `cache_cleanup_enabled` | boolean (default `true`) | Deletes old caches, logs and temporary audio on a schedule. |
| Discord setup wizard has run | `discord_setup_prompted` | boolean (default `false`) | Remembers that you have been offered Discord setup. |
| Acknowledged the "vibe-coded project" notice | `vibecoded_notice_acknowledged` | boolean (default `false`) | Remembers that you have read the first-run notice. |

---

100 settings in 6 categories.
