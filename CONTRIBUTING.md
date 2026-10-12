# Contributing to Celeste

Celeste is maintained by KozyBlake and developed largely with AI
assistance. Both of the things below would help, and neither requires
knowing VRChat.

## Reporting a problem

The most useful reports say three things: which build you were running,
what you did, and what you expected to happen instead. "This felt wrong"
is worth sending — a description of something confusing is often more
useful than a clean reproduction, because it points at a design problem
rather than a bug.

If Celeste misbehaved, attach the log. It lives in `logs/` inside the
data folder:

| | |
|---|---|
| Linux | `$XDG_DATA_HOME/KozyBlake/Celeste`, or `~/.local/share/KozyBlake/Celeste` |
| Windows | `%LOCALAPPDATA%\KozyBlake\Celeste` |

Logs are small by default. If you are chasing something specific, run
Celeste with `CELESTE_LOG=debug` to record everything, including the
libraries Celeste depends on — that produces a much larger file, so turn
it off afterwards.

**Please redact before sending.** A log can contain VRChat display
names, user IDs and group IDs from the instances you were in. It does
not contain your Discord token or VRChat password.

## Sending code

Rust developers are the thing this project most needs. Much of what
wants doing is ordinary Rust — saying what is idiomatic and what is not,
and taking work on directly.

Before opening a pull request:

- `cargo test --workspace` passes.
- `cargo check --workspace --all-targets` reports no warnings. The
  project keeps a zero-warning build, and dead code gets deleted rather
  than silenced.
- New behaviour comes with a test that fails without the change. If a
  test cannot fail, it is not holding anything down.

See [BUILDING.md](BUILDING.md) for how to build it.

## Licensing of contributions

By opening a pull request you offer your contribution under the same
licence as the rest of Celeste — see [LICENSE](LICENSE). You keep the
copyright in what you wrote; this is permission for the project to use
and ship it, not a transfer of ownership.

This matters practically: without it, merged code could not be included
in a release. It also means the project's licence cannot later be
changed in a way that covers your work without your agreement.

Everyone whose code is merged is listed in [CREDITS.md](CREDITS.md). If
you would rather be credited under a different name, or not at all, say
so in the pull request.
