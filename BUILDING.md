# Building Celeste from source

## Requirements

| | |
|---|---|
| Rust | stable, 2021 edition (install via [rustup](https://rustup.rs)) |
| C toolchain | a working `cc` and `pkg-config` |
| OpenSSL | development headers (`openssl` on Arch, `libssl-dev` on Debian/Ubuntu) |
| Network | required — dependencies come from crates.io |

Nothing else. There is no JDK, no Maven, and no vendored native library to
install first — that was Scarlet, and it is gone.

The desktop UI loads its GL and X11/Wayland libraries at runtime rather than
linking them, so the only shared objects the finished binary pulls in beyond
libc are `libssl` and `libcrypto`. Any normal desktop install already has
what it needs to run it.

## Build

```sh
git clone <your-fork-url> Celeste
cd Celeste/rust
cargo build --release
```

The binary is `rust/target/release/celeste`. Internal library crate names keep
the `celeste` spelling throughout the Rust workspace, and the executable uses the
user-visible project name so Linux process monitors and desktop launchers show
`celeste` consistently.

A cold build takes a few minutes; incremental builds are seconds.

## Tests

```sh
cargo test --release --workspace
```

506 tests, all expected to pass.

On a memory-constrained machine, building every test binary at once can get
the job OOM-killed. Run it per package instead:

```sh
cargo test --release -p celeste-core -p celeste-ui -- --test-threads=2
cargo test --release -p celeste-discord -p celeste-vrchat -p celeste-events \
                     -p celeste-gif -p celeste-dave -p celeste -- --test-threads=2
```

### The one test worth knowing about

```sh
cargo test -p celeste-discord command_surface_matches_java
```

This compares the Discord commands Celeste actually registers against
`crates/celeste-discord/java_commands.txt`, a committed snapshot of
Scarlet's full command inventory. It fails with a list naming exactly which
commands went missing. 130 are ported and 31 are deliberately absent; see
the README.

If you rename a command, add the pair to `RENAMED_FROM_JAVA` in
`crates/celeste-discord/src/commands/mod.rs` — do not edit the snapshot,
which is the record of what Scarlet had.

## Workspace layout

```
rust/
  crates/
    celeste-core/      settings, i18n, paths, data files, logging
    celeste-vrchat/    VRChat API client, entity cache, rate limiter
    celeste-discord/   bot, slash commands, components, voice/TTS
    celeste-ui/        egui desktop application
    celeste-events/    VRChat log tailer and event pipeline
    celeste-gif/       GIF encode/decode for animated emoji
    celeste-dave/      Discord end-to-end voice encryption
    celeste/           the binary
  PORTING.md           decisions, deliberate deviations, scope
  HANDOFF.md           working brief for the next session
```

## Running a development build

```sh
cargo run --release
```

`CELESTE_HOME` (or the legacy `SCARLET_HOME`) points the data folder
somewhere else for one run, which is the easy way to test against a scratch
configuration without touching a real install:

```sh
CELESTE_HOME=/tmp/celeste-scratch cargo run --release
```

## Logging

`RUST_LOG` controls verbosity through `tracing-subscriber`'s env filter:

```sh
RUST_LOG=debug cargo run --release
RUST_LOG=celeste_vrchat=trace,warn cargo run --release
```

Logs also go to the `logs/` folder inside the data directory.
