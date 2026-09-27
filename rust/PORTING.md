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
| `scarlet-gif` | `com.madgag.gif.fmsware` | encoder side done (NeuQuant, LZWEncoder, AnimatedGifEncoder); GifDecoder not started |
| `scarlet-discord` | `net.sybyline.scarlet.server.discord`, `ScarletDiscordJDA`, `ScarletDiscordCommands`, `ScarletEventListener` | real, working skeleton: connects with a bot token, registers and dispatches one real slash command (`/scarlet-version`) end to end; the other dozens of commands and the moderation event-handling in `ScarletEventListener` (1,364 lines) are not ported |
| `scarlet-vrchat` | `ScarletVRChat*`, `net.sybyline.scarlet.ext` (VRC-related) | real, working skeleton: VRChat's non-standard Basic Auth, login + TOTP 2FA, group audit log lookups, get-user — all tested against a real mock HTTP server (`wiremock`), not just type-checked. No generated API client (see `client.rs` docs) and no persistent cookie storage yet (in-memory only via reqwest's cookie jar) |
| `scarlet-dave` | `libdave-maven/*`, `net.sybyline.scarlet.server.discord.dave`, `moe.kyokobot.libdave` | data types only; MLS crypto engine + native Opus/codec bindings not started |
| `scarlet-ui` | `net.sybyline.scarlet.ui`, `ScarletUI`, `ScarletDiscordUI`, `IScarletUI` (Swing/FlatLaf, ~8,500 lines total) | real skeleton: toolkit chosen (egui/eframe), a fully-ported and unit-tested `UiState` data model (the actual `IScarletUI` player-list/status/announcement contract), and a minimal working window rendering it. **Not visually verified** — this container has no display server, so the window only compiles here, it has never actually been seen rendering (see `app.rs` docs) |
| `scarlet` (bin) | `Main.java` / `Scarlet.java` entry point | stub only |

## Biggest files still to port (line counts, `src/main/java/net/sybyline/scarlet/`)

- `ScarletDiscordCommands.java` — 5,765 (all slash commands)
- `ScarletDiscordJDA.java` — 4,068 (JDA client wiring, → serenity)
- `Scarlet.java` — 3,655 (god-class: app wiring, settings fields, executors, data-folder migration, Swing bootstrap)
- `ScarletVRChat.java` — 3,367 (VRChat API client + caching)
- `ScarletEventListener.java` — 1,364
- `ScarletSettings.java` — 1,204 (typed settings/FileValued<T> system)
- `ScarletData.java` — 1,030
- Plus `ext/`, `util/`, `util/tts/`, `server/discord/dave/` packages not yet inventoried in detail.

## Design decisions made so far

- Discord library: **serenity** (chosen over twilight — closer in spirit to JDA's higher-level API, easier 1:1 command mapping).
- Async/HTTP: **tokio** + **reqwest**.
- Logging: **tracing** + **tracing-appender**, with `scarlet_core::log::normalize_target` preserving the old `KozyBlake/Scarlet/...` logger-name scheme from `ScarletLoggerFactory` for continuity in log greps, even though Rust doesn't need an SLF4J-style provider SPI.
- DAVE E2EE and the Swing UI are the two subsystems with **no drop-in Rust equivalent** of the underlying tech (JNI/MLS crypto native libs; Swing/FlatLaf). These need an explicit design decision before porting proper (not just translation) — flagged rather than guessed at.

## Next steps (in order)

1. `scarlet-gif` **partially done.** `AnimatedGifEncoder`/`LZWEncoder`/`NeuQuant` ported to `crates/scarlet-gif/src/{encoder,lzw,neuquant}.rs`. `GifDecoder.java` (783 lines, GIF *decoding*) was **not yet ported** — it's a real gap, not a no-op: `net.sybyline.scarlet.util.Gifs.java` uses it to download and decode a GIF URL into a `VrcSpriteSheet` (frame-by-frame), so `scarlet-vrchat`/`scarlet-discord` work that touches sprite sheets or GIF avatars will need it. Tracked as follow-up work, not silently dropped. NeuQuant and LZWEncoder are pinned with byte-for-byte golden tests cross-checked against the real Java classes compiled with OpenJDK 21 (see the `matches_java_reference_output_byte_for_byte` tests) — do this same cross-check (`javac` + a small harness in the same package, diff the output) for any other numerically-tricky file before trusting a port of it. `AnimatedGifEncoder`'s `BufferedImage`/`Graphics2D` resize-on-add (`scaleToFit`) was intentionally dropped, not silently approximated — see the doc comment on `encoder.rs` for the replacement API (raw RGB buffers) and the `FrameSizeMismatch` error it returns instead of resampling.
2. ~~Port `ScarletSettings`~~ **done** (`scarlet_core::settings`: `store` = the JSON half, `registry` = the `Preferences`-replacement half, `file_valued` = the typed-settings registry with a dyn-safe `SettingDescriptor` trait standing in for Java's non-object-safe `FileValuedVisitor` generic-method interface, `backup_warning` + `prompt` for the two UI-adjacent pieces deferred to `scarlet-ui`). Two real Java quirks were caught by porting to working tests rather than guessing: `FileValued.set()` caches/persists the *raw* input value, not the validated/clamped/pattern-extracted one (only the change-listener notification sees the validated value) — confirmed against the actual Java logic and preserved, with tests documenting it so it isn't "fixed" by accident later.
3. ~~Port `ScarletData`~~ **done** (`scarlet_core::data`). Skipped the type-erased `Datum<?>`/`Data<?>` registry Java uses — grepping the codebase shows it's only ever called from inside `ScarletData` itself for five fixed (name/kind, type) pairs, so `ScarletData` now just has five concrete typed fields instead of reinventing Java's reflection-driven registry. `AuditEntryMetadata.entry` is a `serde_json::Value` placeholder for the not-yet-ported `GroupAuditLogEntry` VRChat model — swap it once that's ported (task 6), don't invent a partial model now. Also ported along the way: `UniqueStrings` (`scarlet_core::unique_strings`) and a `java_string_hash_code` port (`scarlet_core::java_hash`), the latter kept as an exact algorithmic port (not just "a hash") so a data directory carried over from a Java install stays filterable by custom-event type without rewriting every file.
4. The rest of `Scarlet.java`'s data-dir resolution/migration logic → `scarlet_core::paths` (the app-identity constants are already ported to `scarlet_core::identity`; this is the remainder of that god-class's static init block, still not started).
5. ~~`scarlet-discord`: serenity client bootstrap + event listener skeleton~~ **done** (`client.rs`/`handler.rs`/`token.rs`/`commands/`). Verified with an actual `cargo run`, not just a type-checked guess: with no token configured it logs and exits cleanly; `Settings::open` creates real `registry.json`/`secure-prefs.json`/log files on disk. Next: port `/scarlet-version`-shaped commands one at a time from `ScarletDiscordCommands.java`, and start on `ScarletEventListener.java` (moderation event handling) once a few commands exist to give it something to act on.
6. ~~`scarlet-vrchat`: auth/cookie-jar + a handful of high-traffic endpoints~~ **done for the skeleton** (`basic_auth.rs`/`client.rs`/`models.rs`). VRChat's non-standard "percent-encode before base64" Basic Auth quirk is ported exactly (confirmed against the Java version's own comment explaining why RFC-7617-correct clients fail VRChat logins) and covered by a test that would catch a regression to plain base64. Login, TOTP verification, and group-audit-log pagination are all tested against a real local mock server (`wiremock`), not just compiled and trusted. Real gaps, tracked not hidden: no generated API client (hundreds of endpoints; hand-written incrementally), no persistent encrypted cookie storage across restarts (in-memory only for now), no alt-account support.
7. ~~Revisit UI~~ **started** (`scarlet-ui`: `state.rs`/`platform.rs`/`app.rs`). Toolkit: egui/eframe (immediate-mode, pure Rust, no native toolkit cross-compile concerns, good fit for a live-updating player list). `UiState` is real and unit-tested — the actual data model behind `IScarletUI`'s contract (player join/update/leave/sort, connection status, announcements), safely shareable across threads (no Swing-EDT equivalent needed for an immediate-mode renderer). `main.rs` was restructured off `#[tokio::main]` to a manually-built runtime specifically so the GUI can own the real OS main thread (`winit` requires this on some platforms) while Discord runs on background tokio tasks — verified with `cargo run` that the headless path (no display in this container) logs correctly and blocks as intended; **the graphical path has never been seen rendering, only compiled** — verify on a machine with a display before trusting it. Remaining: the actual ~8,500 lines of Swing panels (settings UI, moderation actions, Discord config panel, CLI tab) are not ported; build them incrementally against the now-real `UiState`/`ScarletApp` foundation.
8. Revisit DAVE once the headless core round-trips (settings load → VRChat poll → Discord post) end to end.

## Working rules for continuing this port

- **Commit after every subsystem chunk that compiles**, not just at the end. This is the #1 fix versus the first attempt.
- Prefer porting one Java file (or one cohesive method group) at a time to completion over touching many files shallowly.
- When a Java file is too large to read in one call, read it in sections (grep for method boundaries first) rather than skipping it.
- Keep this file updated as subsystems move from "not started" → "in progress" → "done", with the line-count table trimmed as things land.
