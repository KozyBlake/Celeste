# Scarlet 0.4.18-b4

A second VRChat API update in quick succession. Unlike the last one this could not be absorbed by a dependency bump alone — VRChat renamed a field Scarlet reads, so this build carries an actual source change and is a **required update**. Everything from 0.4.18-b3 (see `RELEASE_NOTES_0.4.18-b3.md`) is carried forward.

## Highlights

- **VRChat API bump** to `1.20.9` — the reason this release exists. Earlier builds can no longer display group post roles correctly.
- **Group announcements now show visibility, targeted roles, and editor** — the same detail group posts already displayed.

## What's new

### VRChat API 1.20.9

Updated the bundled `vrchatapi-java` client to **1.20.9** (from `1.20.9-nightly.5`).

VRChat renamed the `roleId` field on group posts to `roleIds`. A build pinned to the older client can no longer read group post roles correctly, which is why this is a required update rather than an optional one.

*(For anyone checking the upstream tags: `1.20.9` really is the newest. The `1.20.9-nightly.N` tags that sort after it were all cut **before** it — the nightlies lead up to the stable, they don't follow it.)*

### Group announcements: visibility, roles, and editor

`GroupAnnouncement` gained three fields in this API version, and Scarlet now surfaces all three — bringing announcement embeds in line with what group post embeds already showed.

- **Visibility** — who the announcement is shown to.
- **Roles** — which roles the announcement targets. Role IDs resolve to role names where Scarlet has them cached, and fall back to the raw ID where it doesn't.
- **Edited by** — shown only when the editor differs from the original author, so an unedited announcement doesn't carry a redundant duplicate field.

No new settings and nothing to configure; the fields simply appear when VRChat provides them.

## Notes for anyone building from source

- Build instructions now live in `BUILDING.md`. The short version: `libdave-maven/` must be `mvn install`ed **before** the root project, or Maven reports a misleading `401 Unauthorized` from JitPack for an artifact JitPack never hosted.
- Upstream also removed `User.username` in this API version. Scarlet already used `displayName` throughout and is unaffected — noted only in case you maintain a fork that still reads it.
- Nothing else in Scarlet's API surface was affected. Of the 74 VRChat API types Scarlet references, only `GroupPost` and `User` lost members; every other change upstream was additive.
