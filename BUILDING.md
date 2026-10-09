# Building Celeste from source

## Requirements

| | |
|---|---|
| Rust | stable, 2021 edition (install via [rustup](https://rustup.rs)) |
| C toolchain | a working `cc` and `pkg-config` |
| OpenSSL | development headers (`openssl` on Arch, `libssl-dev` on Debian/Ubuntu) |
| Network | required — dependencies come from crates.io |

The desktop UI loads its GL and X11/Wayland libraries at runtime rather than
linking them, so the finished binary pulls in nothing beyond libc except
`libssl` and `libcrypto`. Any normal desktop install already has what it needs
to run it.

## Build

```sh
git clone <your-fork-url> Celeste
cd Celeste/rust
cargo build --release
```

The binary is `rust/target/release/celeste`.

A cold build takes a few minutes; incremental builds are seconds.

## Tests

```sh
cargo test --release --workspace
```

On a memory-constrained machine, building every test binary at once can get
the job OOM-killed. Run it per package instead:

```sh
cargo test --release -p celeste-core -p celeste-ui -- --test-threads=2
cargo test --release -p celeste-discord -p celeste-vrchat -p celeste-events \
                     -p celeste-gif -p celeste-dave -p celeste -- --test-threads=2
```

### Checks worth knowing about

A test compares the Discord commands Celeste registers against a committed
inventory, so a command cannot disappear unnoticed — it fails naming exactly
which ones went missing. If you rename a command deliberately, record the pair
rather than editing the inventory.

`SETTINGS.md` is generated from the settings registry and a test fails when it
falls out of date. Regenerate it after adding or changing a setting:

```sh
cd rust && UPDATE_SETTINGS_MD=1 cargo test -p celeste-core settings_md_is_current
```

Run that on Linux or macOS. Regenerating on Windows writes CRLF line endings,
which produces a spurious diff.

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
```

## Running a development build

```sh
cargo run --release
```

`CELESTE_HOME` points the data folder somewhere else for one run, which is the
easy way to test against a scratch configuration without touching a real
install:

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

## Platform notes

Windows-only code paths are behind `#[cfg(target_os = "windows")]` and are not
compiled by a Linux build, so a Linux `cargo check` proves nothing about them.
The Windows CI job is what compiles them.
