# Scarlet 0.4.19

The largest maintenance release in a while. It keeps Scarlet working against VRChat's current API, **restores the avatar images on moderation messages** (the ones that had degraded to the robot placeholder), makes Scarlet **much harder to lose data from** — settings and moderation state are now written crash-safely — adds a **verified-user notification** so staff and the user both know when someone passes age verification, and lands a large batch of reliability, correctness, and security fixes from a full pass over the codebase.


## VRChat API 1.21.0 — compatibility and images

- **Scarlet tracks VRChat API `1.21.0`** (the current stable community client). Upstream removed and relocated a number of fields, so a build pinned to the older client can no longer read VRChat correctly.
- **Moderation avatar images are back.** VRChat moved a user's image fields (`currentAvatarImageUrl`, `iconUrl`, `userIcon`) off the `User` object and onto the separate **`PublicProfile`** response. Scarlet was still reading them off `User`, getting blanks, and falling back to the hardcoded robot — which is why warns, kicks, bans, and other moderation embeds were showing a robot instead of the person. Scarlet now fetches the public profile (cached, so it isn't an extra API call per message) and resolves the image in order — full current-avatar image → its thumbnail → icon URL → user icon → robot only if every one is blank. The same resolver feeds the spawn-pedestal report thumbnail and the user-info embeds. Real avatars again.
- **Bios** are read from the public-profile endpoint (they moved too), sharing the same cached fetch as the image lookup.
- **Group lookups, group calls, and the group gallery** were moved to the new call signatures (the gallery response is now a wrapped one-of type that Scarlet unwraps).
- **The API-update checker no longer cries wolf over nightlies.** Scarlet's "a newer VRChat API is available" notice now only fires for **stable** releases — it ignores the community client's frequent `-nightly.N` pre-releases, which you don't want to run on a live moderation bot. It'll speak up when an actual stable (e.g. `1.21.1`) ships, and stay quiet otherwise.

## New — verified-user notifications

When a member passes age verification and Scarlet auto-invites them to the VRChat group, it now announces it on both sides:

- **Staff** get an "Age Verification Complete" entry in the Discord action-log channel — the member, their VRChat account (name + ID + profile link), their **actual VRChat age-verification status** (18+/verified), and the group.
- **The user** gets a confirmation in their open ticket ("you're verified — a group invite has been sent, accept it to get your role"). This is especially useful on the path where a member earns the verified role directly, where they previously got no acknowledgement at all.

It fires once, reliably, for both the verified-role path and the self-link path, and is controlled by a new setting (**on by default**) that you can turn off without a rebuild. It never interferes with the invite itself.

## New — idle-instance auto-close

An opt-in setting (`auto_close_bot_instance_minutes`, default off) closes the bot's *own* VRChat instance once everyone else has left and a timer elapses — for hosts who forget to close up or step away. It uses Scarlet's live in-instance presence rather than the group-instance API, so an active instance is never mistaken for empty: the timer resets while anyone else is present, re-checks the roster at expiry before acting, stops on its own if a host or moderator closes the instance first, and only ever touches the instance the bot is in.

## Data durability — stop losing settings and moderation state on a crash

`settings.json` already had crash-safe writes (fsync + recover-from-backup). **That protection now covers almost every other data file:**

- timed bans and pending moderation actions
- the staff list and the secret staff list
- the Discord↔VRChat link map and per-user / per-audit-entry metadata
- the recurring-event calendar and its created-event IDs
- the moderation report template
- Discord permissions

Each of these now writes through the fsync + temp-then-atomic-rename path instead of truncating the file the moment it opens, and a file left truncated by a hard crash or power loss is **recovered from a backup on next load** rather than failing to parse and silently resetting. High-cardinality per-entity files use the atomic write without spawning a dated backup per entry; the singleton configs keep dated backups. In practice: a blue-screen or power loss mid-write no longer wipes a 7-day ban timer, your staff roster, or your moderation links. (Tested by actually hard-crashing the machine mid-edit — settings survived.)

## Moderation & Discord reliability

- **Clicking Invite no longer unbans the target.** A stray duplicated code path ran after the real invite and issued an unban (and blocked the UI on two REST calls). Removed — Invite now only invites.
- **`actor-moderation-summary` now redacts.** It gained the same `shouldRedact` gate the query-history commands use, so it no longer exposes another protected actor's kick/warn/ban counts to someone who shouldn't see them.
- **Instance blacklist/whitelist enforcement was inverted** — it force-closed the wrong instances. Fixed.
- **Configured audit-type colors were wiped on every save** (an inverted empty-check). Fixed — your colors persist.
- **Two-level (subgroup) Discord config now loads** instead of logging "Missing value" — it was reading the wrong map key and value.
- **Deferred moderation commands no longer hang** on "This interaction failed" when VRChat returns no user (not-found or a transient API blip): add-role, remove-role, transfer-check, transfer-start and actor-summary now null-check the target and reply cleanly.
- **The immediate-ban tag menu** no longer throws when a group has more than 25 moderation tags (Discord's select-menu cap) — it shows the first 25 and says so; the full set is still reachable through the tag editor's search.
- **Editing a setting whose value is an enum with more than 25 options** no longer throws — the paged select-menu adds the current page's slice instead of the whole list, and only marks a default on the page that contains it.
- **The multi-ban / multi-unban modals open again** (their text input had an invalid negative length bound).
- **The busy-instance monitor embed updates again** — the ">4096 chars, see players.txt" branch was building its edit but never sending it.
- **Missing `return`s fixed:** the age-gated-instance permission check no longer creates the instance *after* denying it, and immediate-ban with no tags no longer falls through into a broken menu build.
- **Boolean settings checkboxes** now reflect saved/programmatic state (`setSelected`, not `setEnabled`), and a wrongly-typed value in a saved settings file is skipped instead of crashing the whole settings load.
- **Spawn-pedestal reports** are classified correctly again (a `switch` was missing its `break`s, so every content type resolved to "Other" and got the wrong VRChat report link).

## Stability — crash guards & null-safety

- **Empty or corrupt JSON data files load cleanly** instead of throwing: the staff list, secret staff list, watched entities, watched groups, and Discord permissions all tolerate an empty/missing file (and the staff lists no longer choke on Java's fixed-size list view).
- **The group-audit poll** tolerates VRChat omitting `hasNext`/`results` — it no longer throws and kills the poll loop.
- **Unknown/new audit-event titles render as text** instead of `[C@…` (a `char[]` was being `toString()`'d).
- Assorted NPE guards: permission checks when the group isn't resolved yet, the new-player advisory when a join date is missing, the calendar's close-after-end handling, and the history-redaction path when a description/actor/self name is null.
- **The data-directory discovery** now looks for a location that actually works (existing data, then a writable spot) rather than nulling out — fixing a class of "it won't start / it lost my data" reports on fresh or unusual setups.
- **Closing a URL-input dialog without entering anything no longer freezes the UI** on Linux/X11 (and the dialog-sizing sweep keeps off-screen dialogs reachable/scrollable).

## Security & hardening

- **SSRF hardening:** NAT64 (`64:ff9b::/96`) addresses are now rejected by the public-URL guard, closing an edge where an internal IPv4 could be reached through an IPv6 mapping.
- **Log export** is hardened against path traversal — an anchored filename match plus a canonical-path containment check, so a crafted filename can't read files outside the logs directory.
- **Untrusted URL reads** (e.g. animated-emoji GIFs) are size-capped *while streaming*, so a hostile or oversized response can't exhaust memory.

## Platform & environment

- **The IPC command socket** stops at end-of-stream instead of filling its buffer with `0xFF` garbage on a Unix domain socket — CLI / second-instance commands now match on Linux.
- **Locating VRChat from the Windows registry** no longer throws `StringIndexOutOfBounds` on an empty value; it reports a clean "couldn't locate VRChat" instead.
- **An unset `PATH`** no longer NPEs a static initializer (which cascaded into installer / toast / pkexec detection).
- **Linux TTS-package handling:** the terminal fallback uses `xterm -e sh -c …` (the bare form silently failed); Solus and Clear Linux are detected correctly (the checks were using `test -f` on directories); and the package-search / command-check subprocesses run with a timeout and are cleaned up, so a locked package database can't hang the thread.

- **Multiple-data-folder picker** — when more than one data folder holds a saved config, Scarlet now asks which to load at startup instead of silently adopting one (with headless/CLI fallback); the scan also covers a bare `~/Scarlet` from older layouts. `SCARLET_HOME` still overrides everything.
- **Linux TTS** — removed the unreliable **Mimic** and **Festival** engines (Festival mis-parsed an error message into ~22 bogus voices; a Mimic-3 binary rejected the flags Scarlet sends), leaving Flite / Pico / eSpeak(-NG). Each Flite-family engine is now probed once at startup, so no listed Linux voice can fail to speak.

## Smaller fixes

- The avatar-search "author" provider no longer prepends a stray literal to every query, URL-encodes the search term, and skips a malformed result row instead of discarding the entire result set on one bad line.
- The extended User-Agent statics are `volatile` (no stale group/operator tag).
- A copy-paste that stored the iOS avatar rating into the PC field is fixed.
- Renaming a moderation tag refreshes its autocomplete label.
- Report-tag lists get their separators right ("A, B, and C", not "ABand C").
- A GitHub release missing both `tag_name` and `name` no longer NPEs the update check.
- A location string with no `:` no longer throws.
- A JSON stream is read as UTF-8 explicitly.
- An option-builder helper honors its `required` argument.

## Notes & deferred

- **Left as-is by design:** the Discord ban/kick hierarchy check (that's how Discord works).
- **Reviewed and deferred** for proper testing before touching: the JSON cache holding its map lock across a network fetch (a throughput issue whose fix is a concurrency refactor of the cache core); a few EDT/threading hot-spots (a blocking lookup before a modal, PID probes on the UI thread); the logging shutdown hook not draining its queue; and a mangled image-asset filename regex whose intended format wasn't worth guessing at.
- Scarlet stays on stable **1.21.0** deliberately — the newest nightly (`1.21.1-nightly.6`) adds nothing Scarlet uses and only removes economy/prop-publish calls it doesn't touch, so there's no reason to move a live bot onto a pre-release.
