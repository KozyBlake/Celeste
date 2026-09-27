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
| `scarlet-core` | `net.sybyline.scarlet` root + `log` + `util` | log layer + app-identity constants ported; config/data model not started |
| `scarlet-gif` | `com.madgag.gif.fmsware` | encoder side done (NeuQuant, LZWEncoder, AnimatedGifEncoder); GifDecoder not started |
| `scarlet-discord` | `net.sybyline.scarlet.server.discord`, `ScarletDiscordJDA`, `ScarletDiscordCommands`, `ScarletEventListener` | skeleton only |
| `scarlet-vrchat` | `ScarletVRChat*`, `net.sybyline.scarlet.ext` (VRC-related) | skeleton only |
| `scarlet-dave` | `libdave-maven/*`, `net.sybyline.scarlet.server.discord.dave`, `moe.kyokobot.libdave` | data types only; MLS crypto engine + native Opus/codec bindings not started |
| `scarlet-ui` | `net.sybyline.scarlet.ui` (Swing/FlatLaf) | not started beyond a stub; needs a toolkit decision (egui/iced/tauri) since Rust has no Swing equivalent |
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
2. Port `ScarletSettings`'s `FileValued<T>` system and `ScarletData`/`Features`/`ScarletPermission`/`ScarletMeta`/`ScarletJsonCache` into `scarlet-core` — everything else depends on this.
3. The rest of `Scarlet.java`'s data-dir resolution/migration logic → `scarlet_core::paths` (the app-identity constants are already ported to `scarlet_core::identity`; this is the remainder of that god-class's static init block, still not started).
4. `scarlet-discord`: serenity client bootstrap + event listener skeleton, then commands incrementally (there are dozens; port a handful at a time, verifying compile each time).
5. `scarlet-vrchat`: auth/cookie-jar + a handful of high-traffic endpoints first (group audit log, instance/user lookup), not the whole 3,367-line file at once.
6. Revisit DAVE and UI once the headless core round-trips (settings load → VRChat poll → Discord post) end to end.

## Working rules for continuing this port

- **Commit after every subsystem chunk that compiles**, not just at the end. This is the #1 fix versus the first attempt.
- Prefer porting one Java file (or one cohesive method group) at a time to completion over touching many files shallowly.
- When a Java file is too large to read in one call, read it in sections (grep for method boundaries first) rather than skipping it.
- Keep this file updated as subsystems move from "not started" → "in progress" → "done", with the line-count table trimmed as things land.
