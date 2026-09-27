# Scarlet 0.4.18-b2

Full desktop localization in six languages, on-demand translation of shared advisory text, Discord moderation quality-of-life (join-request buttons and bulk watch-list cleanup), a safe moderator-training environment, an optional debug build for testing, and a major reliability pass for health monitoring and mobile notifications.

## Highlights

- **Use Scarlet in six languages** — English, Russian, German, Japanese, Spanish, and Indonesian — across the complete desktop experience, including spoken advisory callouts.
- **Translate shared watch-list advisories into your own language** on request (and restore the originals), so lists shared between communities aren't stuck in English.
- **Handle group join requests and prune watch lists straight from Discord** — Accept/Reject/Block buttons on join-request posts, and a dropdown to bulk-remove watched entries.
- **Train moderators safely** with simulated events that exercise the real presentation and Discord workflows without touching VRChat.
- **Stay ahead of silent failures** with session and log-tailer watchdogs, Discord operations alerts, crash-safe saves, and much more reliable mobile delivery.

## What's new

### Complete desktop localization — six languages

The desktop client is now fully localized in **English, Russian, German, Japanese, Spanish, and Indonesian**, verified 1:1 (no key missing in any language): the splash screen, menus, settings, instance wizard, moderation dialogs and results, avatar statistics, import/export and update dialogs, status bar, and spoken advisory/TTS messages.

- Scarlet follows the system language by default; `ui_language` can override it at startup.
- The language setting is a native-name dropdown (`System default`, `English`, `Deutsch`, `Español`, `Bahasa Indonesia`, `日本語`, `Русский`), and new `messages_<lang>.properties` files are discovered automatically.
- German, Japanese, Spanish, and Indonesian are community-seeded, with any remaining strings machine-drafted pending a native pass.
- A few strings that were still hard-coded in English (the "Copy"/"Paste" context actions, the "Alternate credentials" popup, and a bundle-read error) were pulled into the translation system too.
- Spoken callouts are localized as well. Names written in the active language's native script are spoken as-is; other scripts are romanized with a localized script label. Select a matching installed TTS voice for the best result.
- A startup validator and the `langlint` CLI command check community translation files for missing/unknown keys, MessageFormat errors, and broken placeholders.

### CLI commands accept your language

The desktop CLI still recognizes its English keywords (so scripts and the IPC pipe never break), but a command word is now resolved against the active UI language first — so a Spanish user can type `ayuda`/`salir`, a German user `hilfe`/`beenden`, a Japanese user `ヘルプ`/`終了`, and so on. `help` lists the localized aliases alongside the English ones.

### Translate shared advisory text (and restore it)

Watch lists shared between communities usually carry their advisory text in English. A new **Settings → Advisories** action translates that free-form prose — across watched groups, watched users, watched avatars, and moderation-tag descriptions — into your desktop language on request, and a companion action restores the English originals losslessly.

- **Group/user/avatar names are never translated** (they come from VRChat live), and ids, tags, enum values, config files, and the pronoun detection lists are deliberately left untouched so nothing breaks.
- It **works out of the box with no setup** via the free **MyMemory** service (no account or API key), and you can instead point it at a **LibreTranslate** endpoint — self-hosted for full privacy — for heavier use.
- Also available as the `translate-advisories` / `restore-advisories` CLI commands. A progress dialog shows while it runs and a second run can't start on top of the first.

### Discord moderation quality-of-life

- **Join-request actions.** When your group requires approval to join, the "Created Request" moderation post now carries **Accept / Reject / Block** buttons, so a moderator can action a request without opening VRChat. Each requires the group's invite-management permission (the same gate as `/list-join-requests`).
- **Bulk watch-list cleanup.** New `remove-menu` subcommands on `/watched-avatar`, `/watched-user`, and `/watched-group` post multi-select dropdowns of the watched entries by name, so you can tick the stale ones (deleted/banned avatars especially) and remove them in one action instead of one `remove` per id.

### Safer configuration migration

The import dialog includes **Carry over Discord channels**, enabled by default. Turn it off to retain the bot token, guild, roles, and other imported settings while clearing the old audit/log-channel routing — useful when moving a configuration to a different Discord server. Legacy bundles retain their previous import behavior.

### Moderator training mode

**Settings → Training** provides a safe simulator for onboarding and practice. Launch **Edit → Simulate event (training)…** or use the `simulate` CLI command to generate watched-group/user joins, new accounts, mixed-character names, suspicious pronouns, watched avatars, vote-to-kicks, and ordinary joins/leaves.

- Simulated events run through the genuine presentation and Discord pipelines: player-list advisories, TTS, desktop/mobile notifications, Discord posts, and ban/unban buttons.
- All Discord- and archive-bound training activity is marked `[TRAINING]`, and simulated users use the `usr_training-` ID namespace.
- While training is enabled, Scarlet presents a separate empty instance and parks the live player list in the background. Disabling training restores the live instance exactly as it is then; no live activity is lost.
- Training-player actions provide the normal success feedback but never make VRChat calls. Real actions on real users remain live.
- Set a dedicated training Discord channel with `/set-training-channel`. If none is set, training posts are dropped instead of reaching the live audit log.

### Debug build for testing (`scarlet-<ver>-debug.jar`)

`mvnd clean package` now also produces a debug edition (manifest marker `Scarlet-Debug=true`); only that jar — or launching any build with `-Dscarlet.debug=true` — shows an in-app **Debug** menu, so debug tooling can never appear in a release build. It's built every time but doesn't have to be shipped.

- Starts **offline by default** (skips VRChat sign-in, no Discord bot needed) so the UI can be exercised with no setup; pass `-Dscarlet.debug.offline=false` to sign in for real.
- An **Event console** can fire **any** event — the player/instance simulations and every group-audit type (bans, kicks, role changes, join requests, posts, updates, …) — through Scarlet's real processing pipeline, and mirrors what each would post to Discord into the console window, so you can test with **no bot required**.
- **Inject sample watched data** adds fake groups/users/avatars (with unmistakable `*_debug-` ids) to exercise the lists and the new remove-menu.

### Safer data and operational monitoring

- Small JSON data files use atomic saves and retain the latest 10 dated backups in a sibling `backups/` folder. This covers watch lists, moderation tags, settings, Discord configuration, mobile devices, and pronoun lists; several files are now explicitly UTF-8 as well.
- A watchdog warns when VRChat is running but no log line arrives for five minutes, and reports recovery when log traffic resumes.
- Scarlet actively verifies the VRChat session about every five minutes. A genuine `401` triggers an alert and unattended credential/TOTP re-login attempts every 10 minutes; ordinary network failures are not treated as lost authentication.
- Configure `/set-ops-alert-channel` to send session, log-tailer, hard rate-limit, update-available, and VRChat API-version alerts to Discord.

### Mobile companion visibility and reachability

- **Settings → Mobile Companion → Show mobile delivery status** shows direct-LAN listener status and port, live event streams and backlog, Firebase configuration, and device-level transport, enabled state, and recent delivery/failure information.
- Set `mobile_fcm_service_account` to a Firebase service-account JSON path to enable FCM push delivery outside the LAN/relay. Leaving it blank preserves Firebase-free behavior, and unusable transports now record why they cannot be reached.

### Other additions

- **Settings → Appearance → Dim players who left (%)** controls departed-row dimming live; `0` disables dimming and the default is `35`.

## Improvements

- The VRChat API update check now compares both JitPack metadata and GitHub tags, taking the newest valid version from either source. It handles common version-format differences and longer JitPack cold-build response times.
- The credits list now displays contributors as `Name — Role` and includes the new translation contributors.

## Fixes

### Desktop and startup

- **Japanese and other CJK text no longer renders as empty boxes.** When the UI language needs glyphs the theme font can't draw, Scarlet now also falls back to Java's logical composite fonts, so any machine with a CJK font installed renders correctly; if nothing can draw the language, a clear log line says to install one (e.g. `fonts-noto-cjk`).
- Departed players no longer become progressively darker down the player list; each row is dimmed from the base text color by the configured amount.
- The VRChat API version check runs off the startup path, so a slow JitPack response can no longer delay Scarlet launching.

### Mobile delivery reliability

- Long-lived phone event streams no longer consume Scarlet's shared moderation worker pool. The mobile listener and notification delivery now use dedicated pools, preventing paired devices from delaying audit polling, VRChat calls, moderation posts, or other alerts.
- A phone that silently leaves Wi-Fi can no longer block alert delivery for every device behind it. Each connection now has an independent send queue; stalled connections are dropped and reconnect on their own.
- Idle connections are kept alive with a 15-second heartbeat. The companion app reconnects after 45 seconds of silence instead of waiting indefinitely on a dead connection.
- Transient notification-send failures are retried up to five times with increasing backoff (2s, 5s, 15s, and 45s); genuine rejections still fail immediately.
- Firebase push is now correctly enabled when `mobile_fcm_service_account` is configured, and the missing Mobile Companion setting reference has been corrected.
- Devices are contacted independently, so one unreachable endpoint cannot hold up notifications to the rest.
- The companion app now uses capped reconnect backoff (1–60 seconds), resets after a stable connection, and reconnects immediately when the network returns.

## Upgrade notes

- Advisory translation is off until you use it, and works with no setup via the free MyMemory service; set a LibreTranslate endpoint in **Settings → Advisories** for privacy or heavier use.
- For training Discord posts, configure `/set-training-channel`; without one, training events are deliberately not sent to Discord.
- FCM delivery requires a valid Firebase service-account JSON path in `mobile_fcm_service_account`.
- The debug build is a separate `scarlet-<ver>-debug.jar` artifact; the normal jar never includes debug tooling.
