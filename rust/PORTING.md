# Scarlet → Rust porting status

Source of truth for where the Rust port stands. **Read this first** before
continuing the port in a new session — it exists specifically so progress
survives a session being cut off mid-work (the first porting attempt wasn't:
it staged a tarball of the Java source and committed nothing else).

The Java source (`src/main/java/...`) is untouched and remains the reference
implementation throughout the port. Total size: **70,679 lines** in
`src/main/java` alone (excludes `libdave-maven/` and `mobile/`). This is a
large, multi-session port — there is no shortcut to reading and faithfully
translating each subsystem.

Crate layout (`rust/crates/`), each mapped to a Java package/class group:

| Crate | Java equivalent | Status |
|---|---|---|
| `scarlet-core` | `net.sybyline.scarlet` root + `log` + `util` | **done for now** — log layer, app-identity constants, `Features`, `ScarletPermission`, `ScarletMeta`, `ScarletJsonCache`, `Maths`, `ChangeListener`, `FileBackups`, `EncryptedPrefs`, `ScarletSettings`, `ScarletData`, `UniqueStrings` all ported. Remaining `net.sybyline.scarlet.util` files not yet inventoried in detail (`MiscUtils`, `I18n`, `Platform`, `VrcIds`, `GithubApi`, TTS, etc.) — port on demand as scarlet-discord/scarlet-vrchat need them. |
| `scarlet-gif` | `com.madgag.gif.fmsware` | **done** — NeuQuant, LZWEncoder, AnimatedGifEncoder, GifDecoder all ported and cross-checked byte-for-byte against the real Java classes |
| `scarlet-discord` | `net.sybyline.scarlet.server.discord`, `ScarletDiscordJDA`, `ScarletDiscordCommands`, `ScarletEventListener` | real, working partial port: connects with a bot token and registers/dispatches seventeen top-level slash commands. `/vrchat-group` includes every currently identified Java subcommand, including the stateful `open-instance` component/modal picker (see Update 13), and all three watched-list families now include their full Java command surfaces (see Updates 15-16). Shared VRChat/data/settings/list state is available through serenity's `TypeMap`; unattended VRChat login and common linked-account/permission checks are wired. The remaining Java command and moderation-event surfaces are still being inventoried and ported. |
| `scarlet-vrchat` | `ScarletVRChat*`, `net.sybyline.scarlet.ext` (VRC-related) | real, working skeleton: VRChat's non-standard Basic Auth, login + TOTP 2FA, encrypted session/TOTP persistence, group audit log lookups, get-user, and the group moderation surface (get/ban/unban/member/roles/search — see `client.rs`, every endpoint confirmed against the community OpenAPI spec, not guessed) plus `user_has_group_permission` — all tested against a real mock HTTP server (`wiremock`), not just type-checked. No generated API client (see `client.rs` docs). Also: `log_tail` (port of `ScarletVRChatLogs`/`VrcAppData`) — locates and live-tails VRChat's own local `output_log_*.txt` (a different data source than the REST API above; this is what actually populates the Instance tab's player table from a running VRChat client), with Steam/Proton app-data-dir detection on Linux and `HOME`/`USERPROFILE`/`HOMEDRIVE`+`HOMEPATH` fallback for Windows/macOS. Line-parsing is unit-tested against every branch of the real log format; live growing-file tailing (preamble-catch-up vs. live) is tested end-to-end against a real appended-to file, not just the parser in isolation — but still **not yet confirmed against an actual running VRChat client** (no VRChat installed in this sandbox), so treat the live-tailing half as "should work" until the repo owner confirms it on their Linux machine |
| `scarlet-dave` | `libdave-maven/*`, `net.sybyline.scarlet.server.discord.dave`, `moe.kyokobot.libdave` | data types only; MLS crypto engine + native Opus/codec bindings not started |
| `scarlet-ui` | `net.sybyline.scarlet.ui`, `ScarletUI`, `ScarletDiscordUI`, `IScarletUI` (Swing/FlatLaf, ~8,500 lines total) | **visually confirmed working by the repo owner** on their own machine (screenshot review round-trip, see below). A first-run setup wizard (`setup.rs`: vibe-coded notice → Discord bot token/guild → real VRChat login with TOTP) runs before any of this on a fresh install — see item 11 below. Real toolbar, Instance/Settings/CLI tabs, and the full 17-column live player table are present. Table columns can be sorted, reordered, resized, hidden, and restored through Java's `ui_instance_columns` format; Unban and Invite remain hidden by default like Java. Ban/Unban/Invite call the shared VRChat client. The Settings tab is wired to the real `SettingsRegistry`, the status bar uses Java's wording, window geometry persists through `uiBounds`, and `TrustRank` is fully ported. Some toolbar actions and most of the CLI interpreter remain incomplete. |
| `scarlet` (bin) | `Main.java` / `Scarlet.java` entry point | wires settings + Discord + UI together; runs for real (`cargo run -p scarlet`). Spawns the VRChat log tailer and dispatches its events into real `UiState` player join/update/leave calls (`dispatch_vrchat_log_events`), so the Instance tab reflects a locally-running VRChat client without any REST polling. No longer spawns Discord eagerly at startup — that now happens once `scarlet-ui`'s setup wizard resolves a token (see below), or immediately for a headless run (no wizard to show) |

## Biggest files still to port (line counts, `src/main/java/net/sybyline/scarlet/`)

- `ScarletDiscordCommands.java` — 5,765 (all slash commands)
- `ScarletDiscordJDA.java` — 4,068 (JDA client wiring, → serenity)
- `Scarlet.java` — 3,655 (god-class: app wiring, settings fields, executors, data-folder migration, Swing bootstrap)
- `ScarletVRChat.java` — 3,367 (VRChat API client + caching)
- `ScarletEventListener.java` — 1,364
- `ScarletSettings.java` — 1,204 (typed settings/FileValued<T> system)
- `ScarletData.java` — 1,030
- Plus `ext/`, `util/`, `util/tts/`, `server/discord/dave/` packages not yet inventoried in detail.

## Scope decisions (agreed with the repo owner, 27 Sep 2026)

The goal is a **full 1:1 port** of the Java build, with these exceptions,
decided feature by feature rather than inferred:

**Keep and port:** desktop TTS, DAVE E2EE voice, the animated-emoji tools,
avatar-search providers, `/server-restart`, `/set-voice-channel`, and every
Discord command (all of them, whatever the exact count turns out to be —
Java has 50 top-level and 193 total `@SlashCmd` entries).

**Dropped, do not port:** the mobile companion (`ScarletMobile`, FCM relay,
pairing), evidence submission (`/submit-evidence`, `ScarletEvidence`), the
report-template subsystem, LibreTranslate advisory translation, `/schedule`
and calendar events, and training/simulation mode (`ScarletSimulation`, the
UI's `parkedPlayers`/training view). Where Java calls into one of these, the
Rust port omits the call and says so in a comment rather than stubbing it.

**Other standing decisions:**

- **Data compatibility is drop-in.** The Rust build reads and writes the
  same `settings.json`, `discord_bot.json`, `watched_*.json`, `staff_list.json`,
  per-entity data files and so on as the Java build, so either can run
  against the same data directory. Never change a file name, field name or
  enum spelling without checking the Java side.
- **UI is "1:1 plus fixes":** match Java's tabs, columns, wording and
  behaviour, but fix what was awkward in Swing where it doesn't change
  behaviour. Anything that *does* change behaviour gets a comment saying so.
- **Desktop first (Windows and Linux both), headless still supported.** The
  PowerPC/Wii target stays possible — keep the crate split clean and don't
  make the core depend on the GUI — but it isn't the focus.
- **Bugs are found by diffing against Java**, command by command, rather
  than waiting for them to be hit in testing.

## Design decisions made so far

- Discord library: **serenity** (chosen over twilight — closer in spirit to JDA's higher-level API, easier 1:1 command mapping).
- Async/HTTP: **tokio** + **reqwest**.
- Logging: **tracing** + **tracing-appender**, with `scarlet_core::log::normalize_target` preserving the old `KozyBlake/Scarlet/...` logger-name scheme from `ScarletLoggerFactory` for continuity in log greps, even though Rust doesn't need an SLF4J-style provider SPI.
- DAVE E2EE and the Swing UI are the two subsystems with **no drop-in Rust equivalent** of the underlying tech (JNI/MLS crypto native libs; Swing/FlatLaf). These need an explicit design decision before porting proper (not just translation) — flagged rather than guessed at.

## Next steps (in order)

1. ~~`scarlet-gif`~~ **done.** `AnimatedGifEncoder`/`LZWEncoder`/`NeuQuant`/`GifDecoder` all ported to `crates/scarlet-gif/src/{encoder,lzw,neuquant,decoder}.rs`. All four are pinned with byte-for-byte golden tests cross-checked against the real Java classes compiled with OpenJDK 21 (see the `matches_java_reference_output_byte_for_byte` tests) — do this same cross-check (`javac` + a small harness in the same package, diff the output) for any other numerically-tricky file before trusting a port of it. `GifDecoder` additionally has a real encode-with-our-encoder-then-decode-with-our-decoder round-trip test, and was manually verified frame-by-frame against the real Java decoder on a generated multi-frame GIF before the golden test was written (not just assumed correct because the round-trip test passed). `AnimatedGifEncoder`'s `BufferedImage`/`Graphics2D` resize-on-add (`scaleToFit`) was intentionally dropped, not silently approximated — see the doc comment on `encoder.rs` for the replacement API (raw RGB buffers) and the `FrameSizeMismatch` error it returns instead of resampling. `GifDecoder` represents frames as packed-ARGB `Vec<u32>` instead of `BufferedImage` (see `decoder.rs` docs for why that's exactly equivalent here, not an approximation).
2. ~~Port `ScarletSettings`~~ **done** (`scarlet_core::settings`: `store` = the JSON half, `registry` = the `Preferences`-replacement half, `file_valued` = the typed-settings registry with a dyn-safe `SettingDescriptor` trait standing in for Java's non-object-safe `FileValuedVisitor` generic-method interface, `backup_warning` + `prompt` for the two UI-adjacent pieces deferred to `scarlet-ui`). Two real Java quirks were caught by porting to working tests rather than guessing: `FileValued.set()` caches/persists the *raw* input value, not the validated/clamped/pattern-extracted one (only the change-listener notification sees the validated value) — confirmed against the actual Java logic and preserved, with tests documenting it so it isn't "fixed" by accident later.
3. ~~Port `ScarletData`~~ **done** (`scarlet_core::data`). Skipped the type-erased `Datum<?>`/`Data<?>` registry Java uses — grepping the codebase shows it's only ever called from inside `ScarletData` itself for five fixed (name/kind, type) pairs, so `ScarletData` now just has five concrete typed fields instead of reinventing Java's reflection-driven registry. `AuditEntryMetadata.entry` is a `serde_json::Value` placeholder for the not-yet-ported `GroupAuditLogEntry` VRChat model — swap it once that's ported (task 6), don't invent a partial model now. Also ported along the way: `UniqueStrings` (`scarlet_core::unique_strings`) and a `java_string_hash_code` port (`scarlet_core::java_hash`), the latter kept as an exact algorithmic port (not just "a hash") so a data directory carried over from a Java install stays filterable by custom-event type without rewriting every file.
4. The rest of `Scarlet.java`'s data-dir resolution/migration logic → `scarlet_core::paths` (the app-identity constants are already ported to `scarlet_core::identity`; this is the remainder of that god-class's static init block, still not started).
5. ~~`scarlet-discord`: serenity client bootstrap + event listener skeleton~~ **done** (`client.rs`/`handler.rs`/`token.rs`/`commands/`). Verified with an actual `cargo run`, not just a type-checked guess: with no token configured it logs and exits cleanly; `Settings::open` creates real `registry.json`/`secure-prefs.json`/log files on disk. Next: port `/scarlet-version`-shaped commands one at a time from `ScarletDiscordCommands.java`, and start on `ScarletEventListener.java` (moderation event handling) once a few commands exist to give it something to act on.
6. ~~`scarlet-vrchat`: auth/cookie-jar + a handful of high-traffic endpoints~~ **done for the skeleton** (`basic_auth.rs`/`client.rs`/`models.rs`). VRChat's non-standard "percent-encode before base64" Basic Auth quirk is ported exactly (confirmed against the Java version's own comment explaining why RFC-7617-correct clients fail VRChat logins) and covered by a test that would catch a regression to plain base64. Login, TOTP verification, persistent encrypted cookies/TOTP seeds, and group-audit-log pagination are all tested against a real local mock server (`wiremock`), not just compiled and trusted. Real gaps, tracked not hidden: no generated API client (hundreds of endpoints; hand-written incrementally) and no alt-account support.
7. ~~Revisit UI~~ **matches the real shape now, confirmed on a real display.** Toolkit: egui/eframe (immediate-mode, pure Rust, no native toolkit cross-compile concerns, good fit for a live-updating player list). `UiState` is real and unit-tested — the actual data model behind `IScarletUI`'s contract (player join/update/leave/sort, connection status, announcements, CLI log, TTS toggle), safely shareable across threads (no Swing-EDT equivalent needed for an immediate-mode renderer) — a fresh instance always starts empty, there is no seeded/sample data anywhere (guarded by `fresh_state_has_no_seeded_players`). `main.rs` was restructured off `#[tokio::main]` to a manually-built runtime specifically so the GUI can own the real OS main thread (`winit` requires this on some platforms) while Discord runs on background tokio tasks. The repo owner ran this on their own machine and confirmed the window opens and matches the target layout (toolbar/tabs/table/status bar) from a real screenshot of the Java app, which is how the exact column set and status-bar wording here were confirmed rather than guessed. Remaining: `ScarletUI.java`'s actual moderation-action wiring (Ban/Unban/Invite/Create Instance/Import Groups/Sync Discord), the real CLI command interpreter, and `ScarletDiscordUI.java`'s Discord config panel are still stubs/not started — build them incrementally against this now-confirmed-working foundation, and get another screenshot after any further UI change since this container still can't render one itself.
8. Revisit DAVE once the headless core round-trips (settings load → VRChat poll → Discord post) end to end.
9. ~~Port `ScarletVRChatLogs`/`VrcAppData` (local `output_log_*.txt` tailer)~~ **done** (`scarlet_vrchat::log_tail`). This is the piece that turns the Instance tab from "empty until Discord/REST wiring exists" into "shows who's actually in your current VRChat instance right now" — it reads VRChat's own log file instead of the REST API, the same trick VRCX uses. `resolve_vrc_app_data_dir` handles Linux (native + every common Steam library layout, including Flatpak, via `libraryfolders.vdf` parsing for the Proton `compatdata` prefix) and falls back to a plain `<home>/AppData/LocalLow/VRChat/VRChat` for Windows, with `HOME`/`USERPROFILE`/`HOMEDRIVE`+`HOMEPATH` tried in that order for the home directory itself so it isn't Linux-only. `parse_entry_text` is a faithful branch-for-branch port of `ScarletVRChatLogs.handleEntry`, using `strip_prefix`/`find`/`rfind` on the literal separators instead of transcribing Java's hardcoded substring offsets (deliberately, to rule out a whole class of off-by-one porting bugs), and has a unit test per branch. `tail_forever`/`spawn_tailer` are the actual live-tailing loop (not in the original write-up of this file) — polls for the newest `output_log_*.txt`, follows it by byte offset without re-reading or splitting a not-yet-fully-written line, and distinguishes "already there when we started watching" (`preamble: true`, silent catch-up) from "happened live" (`preamble: false`), matching `ScarletVRChatLogs`'s own distinction; this is tested end-to-end against a real growing file (append lines mid-test, assert the preamble→live flip happens at the right point), not just unit-tested parsing in isolation. Wired into `main.rs`'s `dispatch_vrchat_log_events`, which turns join/leave/avatar-switch events into real `UiState` calls and everything else (vote kicks, prop/pedestal/sticker spawns, API request logging, video loads) into CLI-tab log lines, so nothing from the log is silently dropped. **Not yet confirmed against a real running VRChat client** — no VRChat installed in this sandbox — so this is "should work, tested as thoroughly as possible without VRChat itself" until confirmed on the repo owner's Linux machine where VRChat actually runs.
10. ~~Expand `SettingsRegistry` to (most of) `Scarlet.java`'s real settings inventory~~ **done.** Went from 5 placeholder settings to ~90, inventoried by grepping every `settings.new FileValued*` call across `Scarlet.java`, `ScarletDiscordJDA.java`, `ScarletEventListener.java`, and `ScarletMobile.java` (119 total call sites) and porting every data-backed one (`FileValuedBoolean`/`FileValuedIntRange`/`FileValuedStringPattern`/`FileValuedStringArrayPattern`/`FileValuedStringChoice`/the new `FileValuedEnum`). Added `SettingCategory` (General/Moderation/Notifications/Discord/Integrations/System, matching the reference screenshots' sidebar) as a real field on every registered setting rather than an id-prefix guess the UI would have to maintain, and `SettingsRegistry::enum_setting` + `scarlet_core::settings::enums` (`EnforcementAgeState`/`EnforcementListState`/`NotifySeverity`, string reprs matching Java's `Enum.name()` exactly so a `settings.json` carried over from a Java install round-trips). `scarlet-ui`'s Settings tab now has a real category sidebar instead of one flat list, with a combo box for enum settings. Display names/defaults/validation patterns/ranges were all read from the Java source and `messages.properties` directly, not guessed. **Deliberately not ported**: the ~30 `FileValuedVoid` entries (Add/Remove/Reset alt credentials, UI scale, accent colour picker, cache-cleanup-now, pronoun-list editors, TTS voice installer, avatar-search-provider manager, mobile pairing QR/test notification/delivery-status, desktop-toast test) — these are action buttons wired to subsystems (TTS service, avatar search cache, mobile FCM relay, Discord token/guild reset) that don't exist yet, and stubbing a button that always says "not wired up" for ~30 more entries would add UI clutter without adding real functionality; port them alongside whatever subsystem backs each one. `tts_voice_name`'s `string_choice` valid-values list is empty until the real TTS service lands (see `scarlet_core::settings::mod.rs`'s comment on it) — any non-empty value is currently rejected, matching "no voices installed" rather than faking a voice list.
11. ~~Add the first-run setup wizard~~ **done** (`scarlet_ui::setup`). This was the last piece of "starts the app and it just works with no prior configuration" — previously `main.rs` skipped straight to the main window with whatever was (or wasn't) already in `settings.json`, and never called into `scarlet-vrchat`'s login API at all. Port of the interactive parts of `Scarlet.run()` (`showVibecodedNoticeIfNeeded`), `ScarletDiscordJDA`'s constructor (bot token + guild snowflake prompt), and `ScarletVRChat.login()` (username/password/TOTP/email-OTP), which in Java are blocking Swing dialogs shown before the main window exists. Since egui has no blocking-dialog equivalent, this is a small state machine (`Stage`: vibe-coded notice → Discord token/guild → VRChat login → optional 2FA → done) that `ScarletApp` renders in place of the whole toolbar/tabs/table until it finishes, the same way a modal sequence would look from the user's side. VRChat login is real — `scarlet_vrchat::client::VrChatClient::login_with_credentials` plus `verify_totp`/`verify_email_otp`, run on the shared tokio runtime via channels so the UI thread never blocks on the network. When VRChat offers both methods, Rust preserves Java's TOTP-first preference; email-only accounts use the email-OTP endpoint and receive matching UI instructions. Two intentional deviations from Java, both documented in `setup.rs`'s module docs: (1) the Discord step is gated on a new `discord_setup_prompted` flag rather than Java's own "is a token stored" check, because Java's gate re-prompts on *every* launch if you ever leave it blank (a `null`-after-normalize is never persisted) — that reads as a Java edge case, not a feature worth copying; (2) VRChat's step keeps Java's real gate (are credentials stored) since there's no meaningful permanent "skip VRChat forever" for a bot that needs it to function. `main.rs` no longer spawns Discord eagerly — `scarlet_discord::client::connect_and_serve` (pulled out of the old `run_discord` so both the wizard and the headless path can share it) is called once the wizard resolves a token, or immediately if setup already completed in an earlier session. Unit tests cover the pure stage-transition logic (gating, validation-before-network-call, method selection, skip/cancel flows), while both 2FA HTTP endpoints are covered by wiremock tests in `scarlet-vrchat`.

12. ~~Start on `ScarletDiscordCommands` (the biggest unported file)~~ **in progress, foundation done.** Before any command past `/scarlet-version` could be real (not a stub), Discord command handlers needed a way to reach a live VRChat session and `ScarletData` at all -- neither existed: the setup wizard's `VrChatClient` was dropped the moment setup finished, and `main.rs` never opened `ScarletData`. Built that foundation first (`scarlet-discord::data`'s `SharedVrChatClient`/`ScarletDataKey`/`SettingsKey` `TypeMap` entries, wired through `connect_and_serve`), then ported the first three commands achievable without a much larger VRChat API surface: `/config-info`, `/discord-user-info`, `/vrchat-user-info`. `scarlet_vrchat::client::login_with_stored_credentials` (unattended re-login using stored credentials, for whenever a fresh interactive wizard login isn't available this session) is the other real piece that came out of this. **Update**: `scarlet-vrchat`'s client has since been expanded with exactly that API surface -- `get_group`/`get_group_roles`/`get_group_bans`/`ban_from_group`/`unban_from_group`/`get_group_member`/`search_group_members`/`kick_group_member`/`add_group_member_role`/`remove_group_member_role`/`get_world`/`get_avatar`/`get_instance`/`close_instance`, plus the `Group`/`GroupRole`/`GroupMember`/`GroupMemberSearchResult`/`World`/`Avatar`/`Instance` models. Every endpoint's exact path/method/request/response shape was pulled from the community OpenAPI spec at github.com/vrchatapi/specification (not guessed -- VRChat's API has no official docs, and a wrong field name would silently deserialize to `None`/default instead of erroring). 36 tests in `scarlet-vrchat` cover this against `wiremock`. The setup wizard also gained a required VRChat-group-id step (`Stage::VrChatGroupId`, port of `getStringOrRequireInput("vrchat_group_id", ...)` in `ScarletVRChat`'s constructor -- previously missing entirely, another piece of "prior setup" the user flagged), stored plain (not encrypted) in `settings.json` under `vrchat_group_id`, validated as `grp_...`-shaped before accepting it, with a link to vrchat.com/home/groups matching Java's own fallback dialog.

**Update 2**: `/vrchat-user-ban`, `/vrchat-user-unban`, and `/vrchat-group add-role`/`remove-role` are now ported and real, using the expanded client above. Added `VrChatClient::user_has_group_permission` (port of `checkUserHasVRChatPermission`'s role-permission-list check plus the group-owner fallback from `checkSelfUserHasVRChatPermission` — the `allGroupPermissions` live-event cache Java checks first isn't ported, so this always does a fresh fetch) and `PERMISSION_GROUP_BANS_MANAGE`/`PERMISSION_ALL` constants with their real kebab-case wire values (confirmed against `components/schemas/GroupPermissions.yaml` — `group_all`'s actual value is the wildcard `"*"`, not `"group-all"`, which would have been an easy wrong guess). `scarlet-discord::commands::moderation_context`/`vrchat_context` centralize the shared preamble every moderation command needs (linked-account check, group-permission check, resolving the configured group id) — ban/unban use the permission-checked `moderation_context` (matching Java's real `checkMemberHasVRChatPermission` gate on those two commands); `add-role`/`remove-role` deliberately use the un-gated `vrchat_context`, because Java's own `VrchatGroups.addRole`/`removeRole` don't call any permission check either, relying solely on Discord's own per-command permissions — porting "no extra gate" faithfully means not inventing one. **Still not real**: `checkMemberHasScarletPermission` (a per-server permission-override fallback — the role-mapping config it needs isn't ported), `pendingModActions` (a dedup lock against two concurrent actions on the same target), `ScarletSimulation`'s training-mode branch, and `ScarletModerationTags`'s tag-selection UI that the real `tag-immediately` ban flow depends on.

**Also fixed this pass**: a real (if minor) porting bug in `Settings::check_has_version_changed_since_last_run` — Java's `File.lastModified()` returns `0` (epoch) when `settings.json` doesn't exist yet, but the Rust port defaulted to `Utc::now()` in that case, which flips "definitely not modified since last run" into "just modified this instant". Harmless most of the time, but reliably wrong on a slow first run (PBKDF2-HMAC-SHA256 key derivation for the encrypted secrets store — 100,000 iterations, matching Java's own parameters — takes over a second in an unoptimized/debug build), which is exactly what surfaced it: a test opening `Settings` twice in a row started failing once construction got slow enough to cross the 1-second window the check compares against. Fixed to default to the Unix epoch instead, matching Java's actual `0L`. Worth remembering for anyone building this for real use: **build with `cargo build --release`**, not the debug default — the PBKDF2 call alone is ~70ms in release vs. ~1.6s in debug, and every encrypted setting (Discord token, VRChat credentials) pays a first-access derivation cost.

**Update 3**: `/vrchat-group` gained `search-members`, `list-bans`, `view-announcement`, `create-announcement`, `delete-announcement` (7 of ~15 `VrchatGroups` subcommands now real). Added `get_group_announcement`/`create_group_announcement`/`delete_group_announcement` to `scarlet-vrchat::client` plus the `GroupAnnouncement` model (confirmed against the spec — `GET .../announcement` documents returning an **empty object**, not a 404, when no announcement exists, which `GroupAnnouncement::is_empty()` checks for) and three more `GroupPermissions` constants (`group-members-viewall`, `group-invites-manage`, `group-announcement-manage`, all confirmed against the spec's literal enum list). `search-members`/`list-bans` resolve each member's role ids to role names via a fresh `get_group_roles` lookup (best-effort — falls back to raw ids on failure) before rendering, matching Java's `roleNames`. `delete-announcement` keeps Java's exact safety gate: the `confirm-delete` option must be the literal string `DELETE`. `create-announcement` drops the optional image-attachment option (VRChat file uploads aren't ported).

**Update 4**: `/vrchat-group` gained `list-invites` and `list-join-requests` (9 of ~15 `VrchatGroups` subcommands now real). Added `get_group_invites`/`get_group_join_requests` to `scarlet-vrchat::client` (`GET /groups/{groupId}/invites` and `GET /groups/{groupId}/requests`, the latter taking a `blocked` query flag to distinguish pending vs. blocked join requests — both confirmed against the spec, both returning a bare `Vec<GroupMember>`). Both subcommands reuse the exact `list-bans` shape: `moderation_context` gated on `PERMISSION_GROUP_INVITES_MANAGE`, empty-result messaging, `role_names`/`member_line` rendering, paginated output.

**Update 5**: `/vrchat-group` gained `list-posts`, `create-post`, `delete-post`, `transfer-check`, `transfer-start`, `transfer-cancel`, `close-instance` (14 of ~15 `VrchatGroups` subcommands now real — every one except `open-instance`). Added to `scarlet-vrchat::client`: `get_group_posts`/`create_group_post`/`delete_group_post` (the list endpoint wraps results as `{posts, total}` rather than a bare array, confirmed against `GroupPostsResponse.yaml` — unlike bans/invites/join-requests), `get_group_transferability`/`initiate_or_accept_group_transfer`/`cancel_group_transfer`, and a `PERMISSION_GROUP_INSTANCE_MANAGE` constant (`group-instance-manage`, confirmed against the spec's enum list). `close_instance` gained its previously-unported `hardClose`/`closedAt` query params (also confirmed against the spec). New models: `GroupPost`, `GroupTransferable`/`GroupTransferableRequirements` (confirmed against their schemas). `transfer-start`/`transfer-cancel` check their `confirm-group-id` safety gate (must exactly equal the configured group id) via the ungated `vrchat_context` *before* running the real `group_all`-permission-gated transfer, matching the order Java's own `transferStart`/`transferCancel` do it in. `close-instance` splits its `vrchat-location` option on the first `:` the same way `Location.of` does, without porting the rest of `Location`'s structured field parsing (owner/type/region/nonce) since `closeInstance` never reads those fields either — only the raw world/instance halves.

**Update 13**: `/vrchat-group open-instance` and the component/modal dispatch prerequisite are now real. The Rust flow preserves Java's ephemeral picker and per-user draft state: access type, optional restricted roles, region, queue/hard-close/age/invite flags, all six content toggles, minimum avatar performance, Create/Cancel, and the additional display-name modal. Create re-checks the invoking user's linked VRChat account and the access/role/age-gate permissions selected by the draft, then posts the complete request to `/instances`. The shared dispatcher is intentionally prefix-routed so the watched-list note/removal UIs, account-link verification, and moderation confirmations can be added without changing gateway plumbing again. Unit tests cover request construction and the VRChat endpoint contract.

**Update 14**: desktop TTS is now functional rather than UI-only state. `scarlet-ui::tts::TtsService` provides Java-style serialized speech playback, pause/drop behavior, and interruption of the active announcement. It discovers eSpeak NG/eSpeak, Speech Dispatcher, or Flite on Linux, `say` on macOS, and PowerShell/SAPI on Windows without passing user text through a shell. The toolbar TTS toggle and Skip TTS button now control the service, and the CLI gained Java's `tts <text>` command. The Notifications setting now enumerates and validates installed eSpeak voices instead of presenting an empty choice list. Event-driven announcement generation (watched entities, new users, vote-kicks, nuisance/mixed-name alerts) still depends on the not-yet-ported `ScarletEventListener` processing pipeline.

**Update 15**: all three watched-list command families now include Java's component/modal flows: `/watched-group|user|avatar remove-menu` emits up to five select menus (125 entries), resolves display names when VRChat is available, and deletes selected entries immediately; `/watched-group|user|avatar set-notes` opens a pre-filled 1024-character modal and persists its result. These share the component/modal dispatcher introduced for instance creation.

**Update 16**: all three watched-list command families now have their complete Java slash-command surfaces. `/watched-group|user|avatar export` attaches a freshly serialized, pretty-printed JSON array, while `import` downloads a Discord attachment and accepts either the Java JSON schema or its six-column legacy Excel CSV format. The core importer preserves Java's legacy type conversion (`TOXIC`/`WATCH`/`SPECIAL`/`PARTNER`), lowercases split tags, rejects invalid ids, and overwrites matching entries. Imports save once after the complete batch; focused tests cover CSV conversion, id filtering, JSON overwrite behavior, and export deserialization.

**Update 17**: `/vrchat-group audit-types` is now real, completing the Java subcommand inventory for that family. Added the specification-backed `GET /groups/{groupId}/auditLogTypes` client call and `group-audit-view` permission gate, plus Java-equivalent display titles (with a readable fallback for future event types). The endpoint path and response are pinned by a mock-server test.

**Update 18**: `/vrchat-user-ban-multi` and `/vrchat-user-unban-multi` are now real end-to-end modal commands. Both re-check linked-account/group-ban permission at submission, extract every Java-compatible legacy or UUID user id from free-form input, verify account/status, execute changes sequentially, and report changed/skipped/failed counts. Rust returns the completed batch summary directly rather than feeding Java's background `queuedActions` poller, preserving serialized API calls while making individual failures visible. The shared moderation precondition now accepts command, modal, or component users without duplicating security checks.

**Update 19**: `/export-log` is now real. It selects the newest generated log by default or accepts an explicit filename, enforces Java's exact `scarlet_log_YYYY-MM-DD_HH-MM-SS.txt` filename shape as a path-traversal defense, reads only from the active Scarlet data directory's `logs/` folder, and sends the file as a Discord attachment. `Settings` now retains its data-directory path for consumers that need a safe filesystem root; tests cover filename rejection and newest-log ordering.

**Update 20**: the four Java audit-routing commands (`/set-audit-channel`, `/set-audit-ex-channel`, `/set-audit-secret-channel`, `/set-audit-secret-ex-channel`) and `/set-tts-voice` are now real. Audit routes validate the complete standard/extended event inventories, reject non-message Discord channels, preserve Java's event titles, and persist into Java's exact `discord_bot.json` map names. Added an unknown-field-preserving `DiscordConfigStore`, deliberately backed by the raw JSON object so changing one Rust-supported route cannot erase Java-only configuration during the incremental port. `/set-tts-voice` validates against voices actually installed on the host and updates the same settings-backed voice consumed by the desktop TTS service. Tests cover configuration preservation, inventories, titles, and the existing workspace behavior.

**Update 21**: `/aux-webhooks list|add|remove` and `/set-audit-aux-webhooks` are now real. Webhook URLs are restricted to Discord's webhook hosts and path shape, ids and URLs persist in Java's `scarletAuxWh2webhookUrl`, and the assignment command opens a default-aware select menu capped at Discord's 25-option limit. The component handler revalidates both the audit type and selected webhook ids before persisting Java-compatible compact string/array values in `auditType2scarletAuxWh`. Tests cover webhook URL validation and multi-value configuration round-tripping.

**Update 22**: five server-manager configuration commands are now real: `/set-training-channel`, `/set-ops-alert-channel`, `/set-discord-action-log-channel`, `/set-discord-account-age-alert`, and `/set-verification-auto-invite`. They carry Java's `MANAGE_SERVER` default permission, persist exact `discord_bot.json` field names or the existing settings fields, validate channel type/threshold/group id, and return the current effective auto-invite configuration. This establishes the channel ids later event-emission and Discord moderation batches consume rather than leaving those commands as disconnected UI.

**Update 23**: `/set-ticket-tool-auto-response` and `/set-ticket-age-verify-message` are now real, including the latter's pre-filled modal. The auto-response command validates category type and compiles a proposed channel-name regex before atomically persisting any changes, matching Java's all-or-nothing invalid-regex behavior. The modal trims custom text and restores Java's complete default age-verification instructions when blank. Both are `MANAGE_SERVER` commands and write the exact Java `discord_bot.json` fields for the later member/ticket event pipeline.

**Update 24**: `/discord-ban` and `/discord-kick` are now real confirmation-gated moderation flows. Commands verify the feature flag, Discord permission, server-owner protection, bot permission, and bot/target role hierarchy before creating a per-user opaque confirmation; confirm re-checks permissions, membership, ownership, and hierarchy, attempts the DM before acting, supplies a Discord audit-log reason, then bans/kicks through Serenity. Cancel clears the draft without acting. `/discord-warn` now also has its previously missing actor hierarchy/permission checks and all three commands emit success/failure embeds to the configured action-log channel. The draft token replaces Java's reason-in-button-id encoding, preserving behavior without exposing or truncating the reason in component metadata.

**Update 25**: four moderation-history commands are now real: `/moderation-log`, `/query-target-history`, `/query-actor-history`, and `/actor-moderation-summary`. The VRChat client now supports Java's date/actor/type/target audit filters, complete 100-entry pagination, oldest-first sorting, and count-only queries; a mock-server test pins the exact query parameter names and ordering. Actor history and summaries port Java's secret-staff redaction gate, target history redacts protected actors and their stored-message links, and stored metadata links are restored when visible. `/moderation-log` applies the configured lookback, filters redacted/child events, counts each moderation category, lists the newest 150 actions, creates a one-week Discord thread when permitted, and falls back to paginated channel replies when thread creation is unavailable. Active timed-ban display remains pending with the not-yet-ported pending-action scheduler.

**Update 26**: `/moderation-summary` and `/outstanding-moderation` are now real on-demand reports. The summary queries moderation and membership audit activity for the requested window, merges auxiliary actors, counts locally recorded staff/user instance joins, resolves inactive staff names, honors the activity-only setting, and emits Java-shaped header/detail embeds. Outstanding moderation builds its watched event-type filter from the five existing settings, excludes secret staff and redacted/child/already-tagged-or-described entries, groups unresolved items by moderator, mentions linked Discord accounts, and creates a public thread from the report header (falling back to the current channel if Discord rejects thread creation). This ports the command paths; automatic scheduled emission still belongs to the remaining scheduler/event pipeline.

**Update 27**: `/config-set` now ports the moderation-summary time, suggested-moderation kick-count/period, and outstanding-moderation period/time command groups. Values persist through the same settings objects consumed by the new reports. Time scheduling accepts both IANA names such as `America/New_York` and Java-compatible fixed offsets such as `-04:00`, validates local time, handles ambiguous/nonexistent daylight-saving times explicitly, and stores the next UTC occurrence. The feature-gated report-template subgroup remains pending with the report-template subsystem rather than registering nonfunctional leaves.

**Update 28**: `/settings edit` now exposes every non-action `SettingsRegistry` entry through Discord. The setting id has live autocomplete over both display names and ids, and the generated modal is type-aware: booleans, bounded integers, enum/string choices, regex-backed strings, and newline-separated string arrays all show their current value and validate through the exact descriptor used by the desktop Settings tab. Modal submission persists through `set_from_string` and reports invalid input without changing the setting. Void/action entries remain deliberately rejected until their backing subsystems exist, matching Java's own “currently can't be configured via Discord” branch.

**Update 29**: `/link-vrchat-account` is now a real ownership-verification flow rather than an unsafe direct link. It fetches the requested VRChat user, enforces Java's age-verification gate for members who do not yet hold the configured member role, creates a random 30-minute bio code, and stores the pending request per Discord user. The button handler re-fetches VRChat's current public-profile endpoint (where modern API versions expose `bio`), leaves retry available when the code is missing, expires stale requests, links both metadata directions only after proof, attempts the configured member-role grant, and sends a VRChat group invite through the newly ported endpoint. Mock-server coverage pins both the public-profile query and invite request body.

**Update 30**: the feature-gated `/vrchat-animated-emoji from-url|from-file` family and its message-context command are now real. `scarlet-gif` gained the Java `Gifs.VrcSpriteSheet` backend: bounded GIF validation, 1/2/4/8-cell selection, proportional frame fitting, >64-frame sampling, FPS calculation, and a 1024×1024 transparent PNG attachment. The URL path resolves Tenor's `twitter:image`, limits every response while streaming, caps redirects, and rejects credentials, non-HTTP schemes, private/loopback/link-local/special IPs, and redirects that resolve to them. Tests generate a real multi-frame GIF, decode it into a PNG sheet, verify its dimensions/signature, and cover the network-address gate. Both command types register only when `animated_emoji.enabled` is active.

**Update 6**: `/staff-list` and `/secret-staff-list` are now real (`list`/`add`/`delete` each) — the first command family ported outside `VrchatGroups`, and the one every `moderation_context` "you must have linked ids" error message actually points users at (`/staff-list add`). Added `scarlet_core::staff_list::StaffList`: a small JSON-file-backed `HashSet<String>` port of `ScarletStaffList`/`ScarletSecretStaffList` (`staff_list.json`/`secret_staff_list.json` in the data dir, saved via the existing `file_backups::write` crash-safe writer). Java hand-copies the same class twice for its two lists; this port gives them one generic type instead and two independent instances — same behavior, no duplicated logic. Wired into `scarlet-discord`'s shared `TypeMap` (`StaffListKey`/`SecretStaffListKey`, alongside the existing `VrChatClientKey`/`ScarletDataKey`/`SettingsKey`) and threaded through both `connect_and_serve` call sites (`scarlet`'s headless path and `scarlet-ui`'s wizard-driven path). `add`'s optional `discord-user` option is the actual Discord<->VRChat account-linking flow (`ScarletData::link_id_to_snowflake`, already ported) — previously reachable only by hand-editing data files. Both commands share one implementation (`staff_list::register_generic`/`run_generic`) parameterized by which list and label to use, matching Java's `_StaffList` base class shape without hand-copying the command body twice the way Java hand-copies the whole class. Like `add-role`/`remove-role`, there's no in-code permission gate (`vrchat_context` resolves the client/group id for display-name and role lookups only), matching Java's own `@DefaultPerms(Permission.USE_APPLICATION_COMMANDS)`-only gating on both commands.

**Update 7**: `/vrchat-search` gained `world`, `user`, `group` (real VRChat API search, not stubbed). Added `search_worlds`/`search_users`/`search_groups` to `scarlet-vrchat::client` (`GET /worlds`, `/users`, `/groups`, each confirmed against the spec — note the shared `number` parameter's real wire name is `n`, not `number`, unlike `getGroupPosts`'s own locally-defined `number` param; VRChat's API is genuinely inconsistent about this between endpoints) and a new `UserSearchResult` model for `/users`' `LimitedUserSearch` shape (distinct fields from `get_user`'s fuller `User`). `search_groups` reuses the existing `Group` model (`LimitedGroup`'s fields are a subset of the same names); `online_member_count`/`member_count` gained `#[serde(default)]` since search results omit them. Not gated (matches Java's `@DefaultPerms`-only `VrchatSearch`). **Not ported**: `avatar` search (Java's `AvatarSearch.vrcxSearchAllCached` is a separate multi-provider cached-search subsystem, not a plain endpoint call — faking it as one would be a real behavior change) and the "Report ___" link Java attaches to every result (`VRChatHelpDeskURLs` is a large VRChat-Zendesk-specific URL builder with its own per-category form-field ids and a display-name ASCII-transliteration pipeline — disproportionate to build for a decorative link; real search results either way).

**Update 8**: `/moderation-tags` is now real (`list`/`add`/`delete`). Added `scarlet_core::moderation_tags::ModerationTags`, a port of `ScarletModerationTags` (`moderation_tags.json`, same JSON-file-plus-`file_backups`-save shape as `StaffList`): `add_or_update_tag`/`remove_tag`/`get_tag`/`get_tags`/`get_tag_values`/`get_tag_label`, plus a faithful port of `searchTags`'s ranked, typo-tolerant search (exact match, then prefix, then substring, then description substring, then a Levenshtein-fuzzy fallback, with a Java-matching tie-break sort) — including a from-scratch Rust port of `MiscUtils.levenshtein` (char-based rather than UTF-16-code-unit-based, same Unicode-safety adjustment as `paginate_on_lines`). Wired into `scarlet-discord`'s shared `TypeMap` (`ModerationTagsKey`) and threaded through both `connect_and_serve` call sites. `Tag` keeps `description_original`/`translated_lang` fields (so a shared data directory round-trips them) but nothing writes them yet. **Not ported**: `translateAdvisories`/`restoreAdvisories` (a LibreTranslate-HTTP-endpoint integration, itself not ported — a disproportionate addition for this batch) and `/tag` (tagging the moderation event a Discord thread belongs to — depends on `ScarletDiscordUI`'s not-yet-ported thread-to-audit-entry tracking; the data-layer half it would eventually call, `ScarletData::audit_entry_metadata_edit_tags`, already exists).

**Update 9**: `/watched-group` is now real (`list`/`add`/`delete-watched-group`/`view`/`set-critical`/`set-silent`/`set-type`/`set-priority`/`set-message`/`set-tags`/`add-tag`/`remove-tag` — 12 of its 15 subcommands). Added `scarlet_core::watched_groups::WatchedGroups`, a port of `ScarletWatchedGroups` (`watched_groups.json`, same crash-safe-JSON-file shape as `StaffList`/`ModerationTags`) with a `WatchedGroupType` enum matching Java's exact `Enum.name()` wire values, reusing the already-ported `UniqueStrings` for tags. Wired into `scarlet-discord`'s shared `TypeMap` (`WatchedGroupsKey`). **Refactored `connect_and_serve`**: with this the shared-state parameter list hit 8 positional `Arc`s and tripped clippy's `too_many_arguments`; pulled every shared piece (`VrChatClient`/`ScarletData`/`Settings`/`StaffList`/`SecretStaffList`/`ModerationTags`/`WatchedGroups`) into one `scarlet_discord::data::SharedState` struct with an `insert_into` method, so `connect_and_serve(token, state)` stays a 2-argument function no matter how many more of `Scarlet`'s fields get ported and shared with commands later. **Not ported**: `remove-menu` (a select-menu component) and `set-notes` (a `Modal` text box) — no component/modal-interaction dispatch exists in this crate yet, same gap as `open-instance`; `export`/`import` (JSON/legacy-CSV file attachment I/O — a real, doable feature, just not built this round; `WatchedGroups` itself has no `import_json`/`export_json` yet either).

**Update 10**: `/watched-user` and `/watched-avatar` are now real (12 subcommands each: `list`/`add`/`remove`/`view`/`set-critical`/`set-silent`/`set-type`/`set-priority`/`set-message`/`set-tags`/`add-tag`/`remove-tag`). Added `scarlet_core::watched_entities::WatchedEntities`, a port of `ScarletWatchedEntities<E>` generalized over a plain `fn(&str) -> bool` id validator instead of Java's type parameter `E` (used there only to pick an embed-building callback this port doesn't need) — one struct serving both `watched_users.json` and `watched_avatars.json`, plus `is_valid_user_id`/`is_valid_avatar_id` ports of `VrcIds.id_user`/`id_avatar`'s regexes (the user pattern accepts either a legacy 10-character id or `usr_<uuid>`). Reuses `watched_groups::WatchedGroupType` for `type` rather than duplicating Java's second, textually-identical enum. `commands::watched_entity` holds one shared implementation (`register_generic`/`run_generic`, parameterized by a small `EntityKind::{User,Avatar}` enum for id-option-name/URL/VRChat-lookup differences) that `watched_user.rs`/`watched_avatar.rs` both call into, matching Java's `WatchedEntity_<E>` abstract-base shape without duplicating the command body per entity kind. **Not ported**: same gaps as `/watched-group` (`remove-menu`, `set-notes`, `export`/`import`), plus `@FeatureGate("watched.avatars.enabled")` on `/watched-avatar` — no command-registration-time feature gating exists yet, so it always registers (a minor parity gap, not a functional one).

**Update 11**: `/unlink-vrchat-account` (self-service) and `/unlink-vrchat-account-for` (staff, by Discord user and/or VRChat user, either or both) are now real, both built directly on `ScarletData::unlink_by_snowflake`/`unlink_by_id` (already ported). `unlink-vrchat-account-for` is the first command in this crate to register with `default_member_permissions` (`Permissions::MODERATE_MEMBERS`, matching Java's `@DefaultPerms(Permission.MODERATE_MEMBERS)`) rather than the no-restriction default every other command so far has used. **Not ported**: `/link-vrchat-account` (the other half of the pair) -- Java's version is a bio-code self-verification flow: reply with a one-time code, the user pastes it into their public VRChat bio, then presses a button that re-fetches their bio off-thread to confirm, gated by age-verification status and followed by auto-granting a Discord role and an auto-invite to the VRChat group. That's a button-interaction handler (same not-yet-built component-interaction dispatch gap as `open-instance`/`remove-menu`) plus several more not-yet-ported pieces (age-verification gating, auto-role-grant, auto-group-invite) -- tracked together rather than faked as a simpler, less-safe linking flow (which would be an actual security regression: anyone could claim any VRChat account without proving ownership).

**Update 12**: `/discord-warn` is now real — DMs a member a warning with an optional reason, gated via `default_member_permissions(MODERATE_MEMBERS)` and the existing `discord_kick_ban_enabled` setting. Checks the target isn't the guild owner (a cheap, self-contained `PartialGuild::owner_id` comparison) before sending. **Not ported**: the role-hierarchy check (`actorMember.canInteract`) — needs a shared "compare role positions between two members" helper this crate doesn't have yet, worth building once properly (it'll be needed again for `discord-kick`/`discord-ban`) rather than half-built here; and `emitDiscordActionLog` (the configured Discord action-log channel this would also write to isn't wired up — see `set-discord-action-log-channel` in the gap list below). **Not ported at all**: `discord-ban`/`discord-kick`, both of which Java gates behind a danger/cancel button confirmation before acting — the same not-yet-built component-interaction dispatch gap as `open-instance`/`remove-menu`/`link-vrchat-account`. Faking them as immediate, unconfirmed actions would remove a deliberate safety net, not just skip a nice-to-have, so they're tracked rather than attempted as a lesser (and less safe) substitute.

**Next**: continue the complete `ScarletDiscordCommands.java` inventory — event scheduling, remaining configuration, evidence/linking flows, and more — per the standing full-port goal. Important remaining component/modal work includes `link-vrchat-account`; other prerequisites include `VRChatHelpDeskURLs`, feature-gate-aware command registration, the moderation event-emission pipeline, and `ScarletDiscordUI` thread tracking for `/tag`.

**Bug fix (found via user testing)**: the setup wizard's VRChat login step was being silently skipped on a second launch even after the first launch's login attempt was actually rejected by VRChat -- the wizard would jump straight to asking for a group ID with no VRChat prompt at all, exactly as if login had already succeeded. Root cause: `Action::VrChatCredentialsSubmit` persists the entered username/password to the encrypted store *before* the login attempt resolves (matching Java's own `ScarletVRChat.login()`, which does the same up front) -- but Java's `catch (ApiException)` block clears them back out (`this.username.set(null); this.password.set(null);`) when VRChat rejects the credentials, and that clear step never got ported. `compute_stage_after_discord()` only checks "is *something* stored", not "did it actually work", so the stale rejected credentials made every subsequent launch believe VRChat was already configured. The exact same bug existed in `scarlet_vrchat::client::login_with_stored_credentials` (the unattended background re-login every launch attempts), which also never cleared on rejection -- meaning once this happened, *every* future launch would keep silently retrying the same known-bad credentials forever with no way to notice or recover short of manually editing the encrypted store. Both are fixed now (`crates/scarlet-ui/src/setup.rs`, `crates/scarlet-vrchat/src/client.rs`), gated specifically on `VrChatError::Api` (an actual credential rejection) rather than any error, so a transient network hiccup during setup doesn't wrongly wipe good credentials -- matching Java's own distinction (only `ApiException` triggers the clear, not other exception types). Caught a second bug while fixing the first: `RegistryStringValued::set(None)` is a *deliberate* no-op (see its own doc comment, and the module's "Java re-prompts... leaving the prompt blank never stores anything" note) -- my first fix attempt used `.set(None)` expecting it to clear the value, and the regression test written alongside the fix caught that it didn't; `.clear()` is the actual "remove this value" method. Both fixes have regression tests (`a_rejected_login_clears_stored_credentials_so_the_stage_reappears_next_launch` in `setup.rs`, `a_rejected_unattended_login_clears_the_stored_credentials` / `a_network_error_during_unattended_login_leaves_stored_credentials_alone` in `scarlet-vrchat`).


**Update 31 (event pipeline)**: the log tailer's `log_init`/`log_catchUp`
callbacks and `ScarletEventListener` are now ported, which is what makes the
Instance tab work at all. Before this, `main.rs` translated log events
straight into `UiState` calls: rows appeared for joins and leaves and
*nothing else in Java's pipeline ran*. There was no `clientLocation`, so
nothing knew an instance was open (the reported symptom), no group-instance
gate, no watched-list checks, no advisories, no announcements, no audit
events.

- `scarlet-vrchat::log_tail` now emits `TailerEvent::{Init, CatchUp, Entry}`
  with Java's exact sequencing, including `CatchUp { file: None }` plus the
  "The VRChat Client does not seem to be installed on this machine,
  disabling log tailer-parser!" path and the 10s wait-for-a-log-file loop.
  `VrcAppData` is ported properly (`resolve(relative)`, the override's
  prefix stripping, the `TEMP` directory).
- Java's `User Authenticated` branch is **unreachable** (nested
  `startsWith("[Behaviour] ")` then `startsWith("[User Authenticated: ")` on
  the same string), so Java never learns its own user id from the log. Fixed
  here and pinned against a real client log — `clientUserId` is what stops
  the local user being flagged by the mixed-name check.
- New `scarlet-events` crate holds the listener. It talks to the UI through
  `scarlet_core::ui::ScarletUi` (the ported `IScarletUI`, with a headless
  no-op) and to Discord through an `AuditEmitter` trait, so no cycles and no
  GUI dependency in a headless run.
- Ported alongside: `InstanceRoster`, `Pacer`, `PronounLists`,
  `PronounValidator`, `Names` (Unicode script analysis + the
  visual-confusable table), `I18n` (reading the real `messages.properties`
  catalogue via `include_str!` from the Java resources, so the two can't
  drift), and `TtsService.sanitizeName`.
- `ConnectedPlayer` moved to `scarlet-core` and gained the fields Java's
  rows carry: the base/avatar advisory pair with `rebuildAdvisoryAndStyle`,
  pronouns + flag, avatar info/note. `UiState` gained instance state, Java's
  real sort comparator, and `playerUpdate`'s pending-update queue.
- One `Entering Room:` line is now parsed that Java ignores, purely so the
  status bar can name the world instead of showing a `wrld_` id.

The initially recorded gaps in this area have since narrowed: avatar info
hydration is backed by the avatar-search providers and VRChat file-analysis
endpoints, and `main` wires `DiscordAuditEmitter` rather than the no-op
emitter. The bundled WAV alerts before nuisance-rank and mixed-character-name
announcements are now queued ahead of speech on both the local audio-player
route and Discord voice, matching Java's serial playback behavior. Alt-account
group lookups remain unported.

**New dependencies** added for the above, all pure Rust and
cross-compilation-friendly: `unicode-script`, `unicode-normalization`,
`unicode-general-category`, `deunicode` (Junidecode's equivalent),
`async-trait`.

**Update 32 (session persistence)**: the VRChat cookie jar is now persisted,
which was the real reason two-factor was being asked for on every launch. The
client's jar was in-memory only, so each start re-authenticated from
credentials and hit 2FA again; the unattended path then discarded the
half-authenticated client entirely, leaving the app permanently signed out
after first-run setup (the player table still filled from the log, so this
surfaced as "profile columns are blank" rather than "not logged in").

Java keeps the jar in the encrypted store under `<domain>.cookieStore`
(`global.cookieStore`), one `Set-Cookie` line per cookie. This port uses the
same key and the same format — so a session established by either build is
usable by the other — via `reqwest_cookie_store`: `VrChatClient::with_cookies`,
`export_cookies`, and the `client_from_saved_session`/`save_session`/
`clear_session` helpers. The unattended login now tries the saved session
first (no credentials, no code), falls back to credentials, and saves the jar
after a successful login *and* after a verified two-factor code — the latter
is the one that carries VRChat's `twoFactorAuth` cookie. An invalid saved
session is cleared and re-authenticated rather than retried.

The interactive prompt added just before this stays as the fallback for when
the cookie really has expired: `SetupWizard::for_two_factor_only` shows just
the code prompt, opened when a background login asks for one.
`vrchat_client_or_init` also starts from the saved jar, so even a fresh
credential login reuses an existing `twoFactorAuth` cookie.

Alt-account jars (`<domain>.cookieStoreAlt`) are not ported, consistent with
the rest of the alt-account support.

**Update 33 (group audit polling)**: Rust now runs Java's missing
`queryEmit` loop. It re-queries a five-minute overlap for VRChat's
out-of-order ingestion, advances long backlogs in bounded 24-hour windows,
honours `audit_catchup_max_days` and `audit_polling_interval`, persists the
same `lastAuditQuery` cursor, deduplicates by audit id, writes compatible
`gaud/<id>` metadata, and routes normal/secret-staff events through the
configured Discord audit maps. Moderation events create threads and retain
the audit id in their controls, which is the prerequisite for `/tag` and
the description/redaction interactions. Non-moderation audit events use a
generic complete embed for now; Java's event-specific decorations remain
to be ported incrementally. The Discord routing store is also now one shared
live instance: previously commands updated one in-memory `discord_bot.json`
copy while background emitters continued reading a stale second copy until
restart.

**Update 34 (`/tag`)**: `/tag add` and `/tag remove` are now registered and
functional inside moderation threads. The command recovers the audit id
from the thread's durable `edit-tags:<id>` component marker (so it works
after restart), searches all moderation tags with the existing ranked
typo-tolerant matcher, excludes values already selected in another option,
accepts either tag ids or labels, applies the same five-tag operation, and
updates both `gaud/<id>` metadata and the thread's auxiliary summary. The
permission path accepts members who can post in the thread, Discord
moderators/managers, or the linked VRChat actor who performed that audit
event.

**Update 35 (cached VRChat TOTP)**: Rust now mirrors Java's second layer of
2FA persistence in addition to the session cookie. The 32-character Base32
authenticator setup key is encrypted under the Java-compatible
`global:totpsecret` key; legacy plaintext `vrc_secret` is migrated and
removed. When a cookie has genuinely expired, unattended login fetches
VRChat's `/time`, generates a fresh RFC 6238 SHA-1 code, retries like Java,
and saves the newly issued session cookies. The interactive TOTP prompt has
an optional masked setup-key field and stores it only after VRChat accepts
the accompanying code. Six-digit one-time codes are never cached. Email OTP
has no reusable secret, so its verified session cookie remains the only
cacheable credential for that method.

**Update 36 (desktop parity pass)**: the disabled Create Instance toolbar
button is now a complete asynchronous picker. It supports Java's eight group
and personal access modes, four regions, avatar-performance gate, optional
display name, queue/age gate, and all six content switches; validates world
URLs/ids and names; checks the signed-in account's exact group permissions;
validates the world; creates the instance; records pending audit attribution;
and returns an openable/copyable launch link. The desktop CLI now implements
Java's user-facing operations (including localized aliases, logout, restart,
account linking, and guarded file/URL watched-group imports). Intentionally
excluded diagnostic/training/translation commands are recognized and explain
their status rather than falling through as unknown. Backed Java settings
actions were restored for pronoun-list editing, process/data-folder controls,
Discord setup reset, and complete VRChat credential/session/TOTP reset.

**Update 37 (DAVE status correction)**: Discord voice is already DAVE-capable.
Songbird 0.6's default `driver` feature includes the Rust `davey` MLS engine,
advertises the supported DAVE protocol version during voice connection, and
encrypts outgoing audio in the mixer. Celeste registers the same shared
Songbird manager used by Discord voice TTS. The old `scarlet-dave` data-only
crate remains historical scaffolding and is not the live engine; the earlier
“DAVE is unported” note predates the Songbird 0.6 upgrade.


## The Celeste rename (28 Sep 2026)

The fork is called **Celeste** now. Renamed: everything a user sees (window
title, status bar, CLI, Discord command text and embed footers, the whole
message catalogue in every shipped language) and the app identity that
follows from it — `NAME`, `APP_NAME`, `USER_AGENT_NAME`, the log-file prefix
(`celeste_log_*.txt`) and the logger root (`KozyBlake/Celeste`).

Deliberately **not** renamed, each for a reason:

- **Crate, module and type names** stay `scarlet-*` / `scarlet_*`. They are
  invisible to a user, and renaming them would touch every file in the
  workspace for no functional gain (and collide with anything in flight).
  The one exception is the `CelesteUi` trait, which was renamed along with
  the strings around it.
- **GitHub and meta URLs, and `FORK_REPOSITORY`** — the repository really is
  still named Scarlet, and a renamed URL would simply 404.
- **The Java source tree and its class names**, which remain the reference
  implementation this port is checked against.
- **CHANGELOG.md and the RELEASE_NOTES_* files**, which are a record of what
  shipped under the old name. Rewriting them would make past releases claim
  a name they never had.
- **Data file names** (`settings.json`, `discord_bot.json`, `watched_*.json`,
  ...) — drop-in compatibility with the Java build depends on them.

Nothing existing is stranded by the rename:

- The data folder is now `<platform>/KozyBlake/Celeste`, but discovery looks
  under the old `KozyBlake/Scarlet` (and the other legacy layouts) too, so an
  install from before the rename is found and adopted rather than starting
  blank.
- `CELESTE_HOME` is the current environment variable; `SCARLET_HOME` is still
  honoured, so existing launch scripts keep working.
- `/export-log` accepts both `celeste_log_*` and `scarlet_log_*` filenames.
- Logger names from before the rename normalise to the current root.

The logo is the window/taskbar icon and the Instance tab's empty state. A
boot splash was considered and **not** added: startup is well under a second,
so a splash would be a deliberate delay rather than the progress indicator it
was in Java (where the JVM and dependency loading genuinely took seconds).

## Behavioural diff against Java, pass 1 (28 Sep 2026)

Going command family by command family against the Java source rather than
just matching the command surface. Fixed so far:

- **Group-owner permission fallback.** Java only grants a group's owner an
  implicit permission in `checkSelfUserHasVRChatPermission`
  (`ScarletVRChat.java:2900`), not in `checkUserHasVRChatPermission`
  (`:2932`). Rust was applying it to every member check, so a group owner
  passed moderator gates they should not have. Split into
  `user_has_group_permission` (strict) and `self_has_group_permission`
  (owner fallback); only Celeste's own check uses the latter.
- **`staff-list` / `secret-staff-list` role changes.** Java short-circuits
  ("already has/lacks the role") only when the membership lookup returned a
  member; a null lookup still calls `add`/`removeGroupRole`. Rust's remove
  path treated a *failed* lookup as "already lacks it" and silently did
  nothing. Also: the role option is no longer dropped in silence when there
  is no VRChat session — it says so.
- **`staff-list add` / `secret-staff-list add` `discord-user`** is required,
  as Java registers it (`_discordUser`, `required = true`, no
  `@Required(false)` on those subcommands). It was optional here.
- **`/discord-ban` / `/discord-kick` with no reason.** Java omits the reason
  clause entirely from the audit-log reason and the DM; Rust was substituting
  "No reason provided" into both. Only the action-log embed's Reason field
  falls back to that string, which is what Java does.
- **The Discord action-log embed** now matches
  `ScarletDiscordJDA.emitDiscordActionLog`: a `Status` field
  (Completed/Failed, inline), `Moderator` with the mention and id (or
  "Unknown"), `Target` falling back to the bare id, a 1024-char-ellipsised
  `Reason`, an optional `Details` field, a UTC timestamp, and the colour the
  call site passes rather than one inferred from the title.
- **"Unclaimed" on a moderation thread's aux message.** Java clears it the
  moment a `group.user.ban` entry consumes a pending ban record — i.e. any
  ban issued through Celeste — even when the moderator supplied no tags. Rust
  was deciding purely from the metadata, so a tagless Celeste ban was
  labelled Unclaimed.
- **The tag/description access gate** (`ScarletDiscordUI.auditEntryTagDenial`)
  was missing three of Java's four grants: the `group-bans-manage` VRChat
  permission, the `groupex-bans-manage` and `groupex-tags-edit` Celeste
  overrides, and the `group-instance-moderate` re-check for a linked account.
  It also always tested `MESSAGE_SEND_IN_THREADS`; Java picks
  `MESSAGE_SEND` outside a thread.
- **Ban/unban permission denials** now carry Java's full wording, naming the
  exact `/scarlet-discord-permissions type:Other name:groupex-bans-manage
  value:Allow` an admin would run, instead of a generic sentence.

Ported along the way: `MiscUtils.maybeEllipsis` (as
`scarlet_core::misc_utils::maybe_ellipsis`) and the
`group-instance-moderate` permission constant.

Known, deliberate deviations confirmed in this pass:

- `entries-per-page` matches Java exactly (default 4, range 1-10), but list
  commands still render **text pages sent as messages** rather than Java's
  button-navigated embed paginator. This is the pre-existing presentation
  deviation, not a regression.
- `/fix-audit-thread` rebuilds an aux message from the entry's *current*
  tags and description. Java re-reads the (already-consumed) pending record,
  so it always prints a bare "Unclaimed" there and drops tags the entry has
  since gained.
- Discord **command names** keep their Java spelling
  (`/scarlet-discord-permissions`, ...) even after the Celeste rename, so the
  registered command surface still matches `java_commands.txt` and existing
  muscle memory and permission setups keep working. Only the text inside
  responses says Celeste.

Still to diff: `vrchat-group`, `watched-*`, `config`/`settings` and the
reports families.

## Entity caching (28 Sep 2026)

`ScarletJsonCache` was ported but **nothing was wired to it**: every VRChat
entity lookup in the Rust build was an unconditional HTTP request. Java runs
all of them through per-kind disk caches with a max-age per call site
(`ScarletVRChat.getUser(String, long)` and its identical siblings for
groups, worlds, avatars, user groups and public profiles).

This was not just slower. Watched-entity autocomplete resolves every watched
entity's name **per keystroke**; Java answers that from disk, the port was
answering it with up to 25 live API calls. Sustained, that is the kind of
request rate VRChat suspends accounts over — the reason this was fixed ahead
of the remaining command families.

What landed:

- `scarlet_vrchat::entity_cache` holds the per-kind caches (`usr`,
  `pubprof`, `wrld`, `grp`, `gmem`, `avtr` — Java's kind strings, which are
  the on-disk directory names and so part of data compatibility) plus the
  shared read/record helpers. The layout is Java's:
  `<data dir>/caches/<kind>/<id>.json` with a `known_404s.json` per kind, so
  a Java install's caches are picked up as-is.
- The caches are process-wide, installed from `main` right after the data
  directory resolves, matching Java's
  `static final File caches = new File(Scarlet.dir, "caches")`. A client
  built without them (every test, the whole wiremock suite) behaves exactly
  as before.
- `get_user`, `get_avatar`, `get_world`, `get_group`, `get_user_groups` and
  `get_public_profile` now follow Java's three steps: a new-enough cache
  entry wins; a known 404 returns without fetching; otherwise fetch, then
  record the value or the 404. Each gained a `*_cached(id, min_epoch)`
  sibling, and `get_user_fresh` ports `getUserFresh`.
- Call sites now pass the max-age Java passes them:
  `ALWAYS_PREFER_CACHED` for `vrchat-user`/`vrchat-world`/watched-entity
  autocomplete, `within_1_day()` for the staff-list rendering,
  `/vrchat-user-info`, the moderation embeds, and the avatar-match scan
  (which fans out over every avatar sharing a display name and was the
  largest single burst of calls Celeste made).

**One deliberate deviation.** Java's `ScarletJsonCache.getEntry` enforces
`minEpoch` when loading from disk but *not* on an in-memory LRU hit: with no
populator it returns the too-old entry anyway. So Java's `getUser(id)` —
which is `getUser(id, ALWAYS_FETCH)` — silently stops fetching for as long
as the entry stays in the LRU, which is presumably why `getUserFresh` had to
exist. This port honours `min_epoch` on that path too, so `ALWAYS_FETCH`
means what it is named. Reproducing the bug would have made every default
call site serve a stale display name or ban state for the process lifetime;
the caching benefit is unaffected, since it comes from the call sites that
pass a real max-age.

## Where the remaining differences are tracked

`AUDIT-2026-09-28.md` holds the full survey of the four families this file
listed as "still to diff" (`vrchat-group`, config/settings, reports, and the
desktop UI), with the Java and Rust sites for each finding. It is a work
queue, not a changelog: only the entry marked FIXED has been acted on.

The one fixed there was data loss — `ScarletData` serialised snake_case
where Java's Gson writes the field names verbatim (camelCase), so a Java
`global.json`/`live.json`/`usr/`/`gaud/` was read as all-defaults and
re-saved empty, taking the account-link table and every tag, description and
redaction with it. See that file's F1.

## Working rules for continuing this port

- **Commit after every subsystem chunk that compiles**, not just at the end. This is the #1 fix versus the first attempt.
- Prefer porting one Java file (or one cohesive method group) at a time to completion over touching many files shallowly.
- When a Java file is too large to read in one call, read it in sections (grep for method boundaries first) rather than skipping it.
- Keep this file updated as subsystems move from "not started" → "in progress" → "done", with the line-count table trimmed as things land.
