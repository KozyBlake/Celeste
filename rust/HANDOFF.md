# Celeste port — working brief

For whoever picks this up next (Codex, a future session, or Blake). Read
this, then `AUDIT-2026-09-28.md` for the queue and `PORTING.md` for the
decisions already made.

## What this project is

A 1:1 port of Scarlet (Java VRChat moderation logger) to Rust, renamed
**Celeste**. The Java source is the reference implementation and lives in
this same repo at `src/main/java/net/sybyline/scarlet/`. Upstream Scarlet is
archived and will not be updated; this fork is the continuation. The Java
tree stays until the port is settled, then gets deleted.

**Private repo. Nothing is pushed** — every commit lives on the local branch
`claude/magical-brahmagupta-semov4`, 60+ ahead of origin, deliberately. Don't
push without asking.

## The one rule that matters

**Every behavioural claim is checked against the Java source, with a
file:line on both sides.** Not against intuition, not against what seems
sensible. The port is "done" when the Rust does what the Java does, and the
only way to know that is to read both.

This is why the audit is worth trusting: each entry names `Java file:line`
and `Rust file:line`. When you fix one, quote the Java in the comment so the
next reader doesn't have to re-derive it.

Corollary: when the Rust deviates *on purpose*, say so in a comment at the
site **and** in PORTING.md. Silent deviations are how a port rots.

## Conventions this codebase already follows

- **Comments explain the Java, not the Rust.** `// Java: getUser(id,
  within1day)` beats `// fetch the user`. The mechanical part is readable;
  the reason it's shaped that way is not.
- **Extract a pure function to make a fix testable.** Several bugs here were
  invisible until the decision was pulled out of an async handler into a
  function taking plain values. See `redacted_description`,
  `member_already_lacks_role`, `permission_denial`, `row_foreground`.
- **A test per fix, and the test explains the bug.** Not "asserts correct
  behaviour" — states what went wrong. Grep for "The bug this guards" and
  "The leak this guards".
- **Data files are drop-in compatible with the Java build.** Key names,
  directory names, file names, enum wire strings. Renaming one silently
  loses the user's data. `data.rs` has a test pinning the JSON keys after
  exactly that happened; add to it rather than trusting review.
- **Crate/module/type names stay `scarlet_*`.** Only user-visible strings say
  Celeste. Discord command names keep their Java spelling
  (`/scarlet-discord-permissions`) so the registered surface still matches
  `java_commands.txt` and existing permission setups keep working.
- Commit after every chunk that compiles, with a message that says what was
  wrong, not what changed.

## How to know what's still missing

```
cargo test -p scarlet-discord command_surface_matches_java
```

Fails with a list when a Java command has no Rust counterpart and isn't on
the deliberately-absent list. All 161 Java leaves are accounted for: 130 are
ported and 31 are deliberately excluded (the whole `schedule` family, report
templates, `submit-evidence`, and `server-restart update-now`). The updater is
an explicit owner decision: this private native build does not need one.

`cargo test --workspace` should pass all 482 tests with no failures.

## Build and ship

The workspace builds with plain `cargo build --release`; the binary is
`target/release/scarlet`, shipped to Blake as `celeste-testing`.

There is no cargo on the machine this has been developed against — builds
happen elsewhere and the binary is copied over. If you *can* build locally,
do; just don't assume the toolchain is there.

## Queue, in the order I'd do it

Everything below is a measured divergence from Java, not a preference. What
differs between them is consequence, not whether they're real.

**1. Correctness you can't see until it bites — completed 2026-09-28.** B4,
B13, B3, and A6 are fixed in Rust with regression coverage; see the audit for
the exact compatibility changes.

**2. Things that make the app usable — completed 2026-09-28.** Table sorting
(B5), complete and persistent columns (B6), window geometry (B15), intentional
Settings section cards, and the per-event Voice/Desktop notification matrix are
implemented. The owner explicitly prefers a modern Rust UI rather than a
visual Swing clone; use Java as the behavioral and information-architecture
reference while continuing the Rust design language.

**3. Discoverability regressions — completed.** Audit event
choices/autocomplete (A5), `/aux-webhooks remove`, `/export-log`, and the full
typed `/settings edit` selection flow (B2) are restored.

**4. The measured desktop tail — completed 2026-09-30.** The Help menu,
confirmation, timestamp, account-age, diagnostics, scale, accent, and
maintenance differences are closed; see the updates below.

The `watched-*` families have since been audited and their slash-command,
component, modal, import, and export surfaces are ported. Remaining validation
is live integration testing, not an unaccounted command inventory.

## Things that are already right — don't re-audit

- The settings store: all 99 `FileValued*` ids, 73 boolean defaults, 11
  int-range triples, every `discord_bot.json` map name, both audit-type
  tables (34 + 21), `UniqueStrings` encoding, `discord_perms.json` casing.
  Verified key by key.
- `vrchat-group` and `vrchat-search` subcommand coverage, each mapped to the
  same `GroupPermissions` constant Java uses.
- `moderation-log` cutoffs, sorting, counts and caps; the redaction gates'
  *semantics* (the leak that was there was in the substitution, now fixed).
- Departed players sinking and fading; the TTS local-vs-Discord route
  switch.
- There is no system tray in Java, so nothing is missing there.

## Two traps worth knowing about

**Java has bugs, and porting them faithfully is sometimes wrong.** Found so
far: a cache whose `ALWAYS_FETCH` stops fetching once the entry is in the
LRU; `discord-user-info` reading the target's roles where it means the
invoker's; a `moderation-summary` count that is structurally always zero; a
removal message that prints `null`. Each one is a judgement call — document
which way you went and why. The default is to match Java; deviate when
matching would mean being confidently wrong about moderation data.

**The VRChat API will get the account banned if you hammer it.** Entity
lookups go through `scarlet_vrchat::entity_cache`, which ports Java's
`ScarletJsonCache`. Before adding a lookup to any loop or autocomplete path,
check what max-age Java passes at the equivalent site — `ALWAYS_PREFER_CACHED`
for anything per-keystroke, `within_1_day()` for rendering, `ALWAYS_FETCH`
only when the answer must be current. This caching existed in the port,
fully written, and was wired to nothing for weeks; autocomplete was firing
up to 25 live API calls per keystroke.

## Update 2026-09-29: Discord event-specific parity

Alt-account support is deliberately out of scope at the owner's request; do
not carry it as an unfinished parity item.

The group-audit poller now uses Java's exact titles and colours for all 34
`GroupAuditType` values and renders structured event fields instead of one raw
JSON dump. Announcement, post, membership/request, role, and update events
have their Java-specific titles, links, descriptions, images, and controls.
Moderation embeds now include the available target profile image, account age,
age-verification state, and pronouns. `group.instance.create` also creates the
configured `groupex.instance.monitor` message/thread, writes `### Instance log`,
and persists the live-instance routing metadata used by extended join/leave
events; `group.instance.close` records its close timestamp.

The Java-style live-player-list message is also created in that thread when
its route is enabled. Join/leave/avatar changes update it through a two-second
debounce, including staff/watched badges and advisory text, and instance close
marks the session ended and removes its Join button.

Still worth testing with a live Discord server: instance-monitor creation,
thread routing, live-list edits, and close finalisation, because the unit suite
can validate embed/data behavior but cannot exercise Discord's thread
permissions. The owner has explicitly deferred Java's instance-close
reply/backlink to the opening audit post; revisit it only if live Discord
testing shows that omission causes an operational problem.

## Update 2026-09-30: runtime reliability

`ScarletData`'s dirty-tracked `global.json` and `live.json` are now flushed
every 30 seconds and saved on GUI/headless shutdown and before self-restart;
previously no Rust caller invoked `save_dirty`/`save_all`, so links and live
instance routing could remain memory-only. A five-minute VRChat session probe
now treats only HTTP 401 as session loss, retries unattended recovery every ten
minutes, refreshes auth after 72 hours, saves rotated cookies, and can reopen
the existing UI 2FA prompt during a running session.

Avatar-switch events now use the already-ported provider adapter (exact-name
matches, deduplicated; World Balancer remains excluded), restoring watched-
avatar advisories. Automatic six-hour cleanup now applies the configured
retention to TTS files, entity caches, Celeste/legacy Scarlet logs, and—by
explicit owner request—old VRChat output logs, always protecting the newest app
and VRChat log files.

## Update 2026-09-30: bot-instance auto-close

`auto_close_bot_instance_minutes` is now functional in the Rust instance
lifecycle worker. It reads the current location and active player ids from the
same log-derived `UiState` that drives the Instance table, resolves Celeste's
own VRChat user id when it starts tracking a location, resets the full idle
timer whenever another user is present or the bot moves, rechecks the roster
immediately before acting, hard-closes only the concrete current location, and
emits the configured instance-enforcement notice on success. Tests pin the
uninterrupted bot-only interval and location-reset behavior.

## Update 2026-09-30: fast instance follow

`vrchat_client_launch_on_instance_create` and
`instance_follow_fast_poll_seconds` now drive a dedicated low-latency worker.
The first successful group-instance response becomes a baseline; later polls
select at most one genuinely new location and exclude the bot's current
location. API failures back off for a minute and the configured period is
clamped to Java's 15-second minimum. Both this path and the eventual
`group.instance.create` audit event share a five-minute launch-dedup guard.
Celeste requests the dedicated secure-name token (falling back to the general
instance response), closes an already-running VRChat client, and cold-launches
desktop mode with the deep link, profile and debug-log flags. Linux prefers
`steam -applaunch` and falls back to `xdg-open`; Windows tries Steam then the
registered URI handler. Fast detection also wakes the audit poller immediately
so the Discord monitor is not delayed by its normal interval.

## Update 2026-09-30: maintenance and live settings actions

The Settings/System `Run cache cleanup now` action is complete: it previews
the eligible file count and size, asks for confirmation, deletes off the UI
thread, and reports deleted/failed counts and bytes freed. Runtime entity
caches are ignored by Git (`rust/caches/`); commit `c533d3c` accidentally
added 122 generated cache files and follow-up `e5ff613` removed only their Git
tracking while preserving the local files on disk.

The remaining Java appearance/maintenance actions now have real Rust UI
implementations. UI scale (50–400%) and accent colour (presets plus custom
RGB) apply live and persist under Java's `ui_scale` and `ui_accent_color`
keys. The shared `PronounLists` object is now passed into the window, so
`Reload lists` atomically updates the same allow/deny sets the live event
listener reads and reports the loaded counts. The avatar-provider manager
can enable, disable, add, and restore VRCX-compatible sources; saving clears
the process cache/back-off state and future lookups read the new selection
without restart. World Balancer is excluded both in the manager and at the
network boundary. If local TTS has no supported engine, the desktop now opens
a cross-platform explanatory warning rather than offering an installer.

Verification at this point: `cargo test --workspace` passes 482 tests. Two
old unused moderation-context helper warnings remain; the selected-widget
theme warnings introduced in `c533d3c` were fixed with explicit `f32` stroke
widths.

## Update 2026-09-30: desktop parity tail

The remaining measured desktop divergences from `AUDIT-2026-09-28.md` are
closed without reverting to Swing's layout. Manual `Clear current instance`
now has Java's confirmation step. Joined/Left timestamps include seconds
(`yyyy-MM-dd HH:mm:ss`), and account ages use Java's `stringify_ymd` behavior
(zero components omitted, all-zero rendered as `0D`). A Help menu makes the
previously unreachable credits/about information, project and policy links,
and a read-only diagnostics report available. Diagnostics includes the data
folder, VRChat/Discord connection state, local TTS engine, audit cursor and
polling interval, and configured avatar-provider back-offs; it uses cached
state only and never performs or delays an API request. Manual updater actions
remain deliberately omitted because the owner does not need an updater for
the private repository.

## Update 2026-09-30: Rust-owned bundled resources

Rust no longer compiles assets out of Java's `src/main/resources` tree. The
six translation bundles now live under `crates/scarlet-core/resources/i18n`,
while credits and both bundled TTS alert sounds live under
`crates/scarlet-ui/resources`. They were copied byte-for-byte so behavior and
attribution do not change. The Java copies remain only because the Java source
is still being kept as a parity reference; deleting that reference tree is a
separate final-cleanup decision.

The remaining release gate is hands-on testing: Discord permissions, command
responses, threads/components/modals, VRChat authentication and live logs, TTS,
and desktop behavior on the owner's real configuration. Unit tests cannot
fully exercise those external services.

## Update 2026-09-30: Discord guild persistence and permissions

The setup wizard stored the guild id under encrypted `discord.guild_id`, while
the global interaction gate, voice reconnect/TTS, and audit metadata still read
Java's `discord_bot.json` field `guildSf`. With no legacy field, every command,
component, and modal was rejected before the owner/Administrator bypass ran.
Celeste now resolves one authoritative guild id with Java fallback, synchronizes
both stores on startup/connect, and clears both on Discord reset. Owner checking
also falls back to Discord's API when the guild cache is cold. This restores the
save handlers that the broken gate prevented from running and keeps Discord
configuration available after restart. The workspace now passes 484 tests.

Settings sidebar navigation now exits the global-search mode when a category
is clicked. Previously the selected category and heading changed while stale
cross-category search results remained visible, making the navigation appear
nonfunctional after searching/editing a setting. Regression coverage brings
the workspace total to 485 tests.
