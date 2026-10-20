# Celeste 0.20.0

The first release since the public beta. The headline is that Celeste now
starts on machines it simply refused to open on before, and that group
audit events reach Discord again.

**If you are running 0.10.0, update.** A field-name mismatch meant every
group audit entry was discarded before it reached Discord, so kicks,
warns and bans never arrived. That is fixed here.

## Celeste starts on machines with no GPU

Celeste could not open a window at all on a machine with no usable
graphics driver — a Proxmox or QEMU guest on a display-only virtual GPU,
a VPS, or any system running on basic display drivers. It failed with
"no suitable adapter found" and the automatic recovery could not save it.

Three causes, stacked:

- The wgpu version Celeste used discarded the "Microsoft Basic Render
  Driver" adapter. That adapter is WARP, the software rasterizer every
  Windows 10 and 11 ships, and on a machine with no GPU driver it is the
  only Direct3D 12 adapter there is. Celeste now runs on wgpu 24, where
  that adapter is kept.
- Windows was pinned to Direct3D 12 alone, so a machine with Vulkan or
  OpenGL but no D3D12 — an older machine — had nothing to fall back to.
  Automatic now tries DirectX 12, Vulkan and OpenGL.
- The single recovery attempt went to OpenGL, which on a driverless
  Windows box is version 1.1, far below what Celeste needs, so the
  safety net never caught anything.

Celeste now chooses its own graphics adapter: real hardware first through
any API, a CPU adapter accepted as an answer rather than treated as a
failure, and adapters that cannot draw to a window rejected with a message
naming them. Every adapter found is written to the log, so a startup log
is enough to tell what a machine offered.

There is a new **Software rendering** option under Settings → General →
Graphics renderer, and `--renderer software` on the command line.
`--safe-mode` now means software rendering on Windows and OpenGL
elsewhere. If a renderer fails, Celeste steps down a ladder — automatic,
then software, then OpenGL — and now recovers from an explicitly saved
renderer too, so a stale setting cannot be the thing that stops Celeste
opening.

Software rendering needs no graphics driver whatsoever. It draws on the
processor, so it is slower than a GPU, but Celeste's interface is light
enough that it stays usable.

## Group audit events reach Discord again

VRChat serves the group audit log's timestamp as `created_at`, while
every other field in the same object is camelCase. Celeste required
`createdAt`, and because that field is mandatory the whole entry failed
to parse — so **every** audit entry was silently dropped. No kicks, no
warns, no bans, no role changes, no moderation threads, no tags.

Celeste now accepts either spelling, on this and the two other places a
timestamp is read.

## Instance warns show up within a second

The audit poller runs on a timer, so an action could be up to a minute
old before Discord heard about it. Celeste now reads instance warns out
of the local VRChat log the moment they happen and brings the next audit
poll forward, instead of waiting for the timer.

This is not a second notification path: the embed, the moderation thread,
the tags and the actor attribution still come from the one audit entry.
The local line only says "there is something to fetch".

It covers the instance Celeste is actually in. Moderation in a group's
other instances produces no local log line, so those still arrive on the
polling interval, which is configurable from 10 to 300 seconds.

## A searchable instance roster

Ctrl+F narrows a packed instance to the person you are looking for,
matching display name, user ID or worn avatar. Decorative fonts, other
writing systems and look-alike letters are all searched, so the letters
on the nameplate are the letters you type.

## New logo

Celeste has its own mark, drawn by
[CeleryDinoSauR](https://vrchat.com/home/user/usr_12af8628-8dbe-4396-89de-0c92cea53782).
The Linux desktop launcher now installs a full icon set, so Celeste looks
right in application menus and docks.

## Licence change

Celeste moved from PolyForm Noncommercial to **PolyForm Shield 1.0.0**.
Noncommercial pointed the wrong way: it forbade a company moderating its
own group, which was never the concern, while freely granting the
redistribution and forking that were.

Shield permits any purpose except providing a product that competes with
Celeste. Running it inside a business is fine. Distribution and changes
stay granted, so distro packaging and the AUR remain open.

0.10.0 shipped under Noncommercial and stays under it. Those terms cannot
be withdrawn from anyone who already has that release.

## Also in this release

- `CONTRIBUTING.md` states the inbound terms: contributions are offered
  under the project's licence, you keep your copyright, and the project
  gets permission to ship your work. No copyright assignment.
- `CREDITS.md` records Vinyarion and the Sybyline Network for Scarlet,
  and names contributors and testers.
- The whole workspace is clean under `cargo clippy -- -D warnings` on
  Windows and Linux, with no lints silenced by attribute. Several
  functions that took long lists of interchangeable arguments now take
  named structs instead.

## Thanks

- [VRC Kmart](https://vrchat.com/home/group/grp_8c7c423b-cfab-40f5-a53a-a8431655826b)
  reported that Celeste would not start on a virtual machine with no
  usable GPU, which uncovered the missing software-rendering fallback.
- [Fran560](https://vrchat.com/home/user/usr_60257903-b52a-463a-a48e-e188eb5b6947)
  reported the Intel Arc/Vulkan VRAM issue.

Celeste is still a vibe-coded project, and it is still looking for Rust
developers willing to improve it and for testers willing to run it.
