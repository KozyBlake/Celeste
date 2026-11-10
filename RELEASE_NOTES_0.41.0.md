# Celeste 0.41.0

A call-out when an account advertising an automation service joins, and two
release-pipeline faults that had been quietly wrong for several versions —
one of which meant **the in-app update check had never worked at all**.

**If you are running 0.40.0 or earlier, Celeste will not tell you about
this release.** See below; you will need to check the releases page
yourself this once.

## A call-out for accounts advertising a bot service

There is a class of automated account that joins a public instance,
records who is present and what avatar each person is wearing, and leaves
again — feeding avatar databases and re-upload tooling. Some of them
carry the vendor's domain in their VRChat bio, which is the cheapest
signal available: nothing to observe over time, just a string.

Celeste now reads a joining player's bio and, when it matches an entry in
`bot_bio_markers.json`, raises the same advisory and spoken call-out a
watched group does:

> Vex joined the lobby. Bot indicator, hexed.cc in bio.

The call-out names the string it matched rather than announcing a verdict.
"Bot indicator: hexed.cc in bio" is a fact a moderator can go and check;
"bot detected" is a claim they would have to take on trust.

**Celeste takes no action on this, and nothing is wired so that it could.**
A bio is text its owner typed, which makes it wrong in both directions: a
person can put a vendor's name in theirs as a joke, or to get somebody
else flagged in every group running Celeste, and a real bot can simply
stop advertising. A match is a reason for a moderator to look, and that is
all it is. Because the consequence of a false positive is a conversation
rather than a ban, the list can afford to be broad.

`bot_bio_markers.json` sits in the data folder beside the pronoun lists,
and is created on first run with the two domains reported by more than one
person independently. Edit it without rebuilding; an empty list turns the
check off.

Matching is case-insensitive and ignores spacing, so `HEXED . CC` is
found. It will not survive other obfuscation — `h-e-x-e-d.cc` passes, and
there is a test saying so, because widening it further starts matching
ordinary bios and a deny-list that cries wolf is one people stop reading.

Two settings, both on by default, under **Notifications**:

- `advisory_flag_bot_indicator` — flag it in the advisory
- `tts_announce_bot_indicator` — say it aloud

Reading a bio costs one extra API request per join, because VRChat moved
`bio` to the public-profile endpoint in API 1.21.0. The request is made
only while one of those two settings is on, and it shares the five-minute
join cache with the existing user lookup, on the grounds that a bio does
not change minute to minute. For scale: the join path already made two
requests per player, so this is a third rather than a new kind of traffic.

## The update check had never once worked

Releases here are tagged `Celeste-0.40.0`. The version parser stripped a
leading `v`, then split on the first `-` and treated what followed as a
pre-release label — so the tag became the number `Celeste` with a
pre-release of `0.40.0`, `Celeste` is not a number, and parsing failed.
The comparison then fell through to its catch-all and answered "no update".

Every release this project has ever published was invisible to it. Nobody
was ever told a new version existed, and because the failure was silent
there was nothing in any log to say so.

The parser now strips a `Celeste-` or `Scarlet-` prefix first, taking both
names from the project identity so a future rename carries the update
check with it instead of breaking it the same way twice.

**This fix only helps from the build that contains it.** A copy of 0.40.0
is still running the old parser, so it will not announce 0.41.0. This is
the last release that has to be found by hand.

## Releases stopped carrying every previous AppImage

The Linux job writes its AppImage into `rust/target/`, which is restored
from the build cache — so every run brought the previous releases'
AppImages back, and the upload matched all of them. The 0.40.0 download
carried 0.20.0 and 0.30.0 alongside it.

The version bump is what spread it: a new `Cargo.lock` makes a new cache
key, and the cache saved under that key kept the old AppImage beside the
new one. The packaging script does delete its own output first, but only
for the version it is building, so an older filename was never its
business.

The job now clears that directory before packaging and uploads the one
file built by that run, named for its version rather than matched by a
wildcard. The Windows build was never affected: it copies the executable
to a directory that is not cached.

Related, and the reason this had gone unnoticed: the release workflow only
triggered on tags matching `v*`, while every tag here is `Celeste-<version>`.
It had therefore never run, and releases were assembled by hand from the
build artifacts — which is exactly where the stale AppImages were. It now
triggers on `Celeste-*` too, and checks the tag against the version in
`rust/Cargo.toml` either way.

## Also in this release

- `SETTINGS.md` regenerated: 108 settings in 6 categories.

Celeste is still a vibe-coded project, and it is still looking for Rust
developers willing to improve it and for testers willing to run it.
