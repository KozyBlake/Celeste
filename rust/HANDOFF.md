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
the deliberately-absent list. 161 Java leaves; 130 ported, 30 deliberately
out of scope (the whole `schedule` family, report templates,
`submit-evidence`), 1 real gap (`server-restart update-now` — needs a design
decision, not a port: Java restarts into a freshly-fetched jar and a native
binary has no equivalent).

`cargo test --workspace` should be 15 green groups with no failures.

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

**2. Things that make the app usable.** Table sorting (B5), complete and
persistent columns (B6), and window geometry (B15) were fixed on 2026-09-28.
The remaining UI structure gaps are:
- Settings tab guesses section grouping by id prefix; Java has 19
  explicitly ordered named sections.
- The notification matrix is flattened into ~27 loose toggles instead
  of Java's per-event grid.

**3. Discoverability regressions.** Audit event choices/autocomplete (A5),
`/aux-webhooks remove`, and `/export-log` were restored on 2026-09-28. B2
remains: `/settings edit` hides valid values behind a 45-char truncated label.

**4. The cosmetic tail.** Wording, the menu bar, timestamp seconds. Log
them; don't spend a session on them.

**Not yet diffed at all:** the `watched-*` command family. Four families
were audited on 28 Sep; that one wasn't. Expect it to yield findings at a
similar rate.

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
