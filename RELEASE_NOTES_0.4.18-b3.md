# Scarlet 0.4.18-b3

Cut ahead of schedule to track a VRChat API update, this release also brings **timed group bans**, **automatic evidence capture** for OBS/Medal, and a fix for a **recurring secure-store corruption** on Windows. Everything from 0.4.18-b2 (see `RELEASE_NOTES_0.4.18-b2.md`) is carried forward.

## Highlights

- **Consolidated moderation log** — one `/moderation-log` command collapses a user's whole moderation history into a single post with counts.
- **Timed group bans** — ban now, auto-unban later (6h / 24h / 3d / 7d, or a custom duration), straight from the moderation post.
- **Automatic evidence capture** — fire your OBS/Medal replay-buffer hotkey the moment a kick or ban happens, à la BanLogger.
- **Fixed the secure-store corruption** that periodically wiped saved credentials on Windows (`AEADBadTagException: Tag mismatch`).
- **VRChat API bump** to `1.20.9-nightly.5` — the reason this release exists.

## What's new

### Consolidated moderation log

When one user has racked up a wall of separate moderation posts, `/moderation-log user:<vrchat user>` pulls them together into a single **thread**.

- The thread opens with a summary header tallying the totals — **warns, mutes, instance kicks, group removals, bans, unbans** — followed by every action newest-first, each linking straight back to its original Discord post. (Run somewhere a thread can't be created, it falls back to an inline reply.)
- If the user is currently under a timed ban, the log notes the automatic-unban time.
- How far back it reaches is up to you: **Settings → Moderation → "Moderation log: include history from the past N days"** (0 = all recorded history).
- It reads Scarlet's own per-user audit index, not Discord's message history, so it isn't limited by how far back a bot can read — it goes back as far as Scarlet has logged. Redacted and already-bundled entries are skipped.

### Timed group bans

Every moderation post's action buttons now include a **Timed ban** row: one-click **6h / 24h / 3d / 7d**, plus **Timed ban…** for a custom duration (`30m`, `6h`, `3d`, `2w`, or a plain number of hours).

- Clicking one bans the user immediately and schedules the unban; if the user is *already* banned it just attaches a timer, turning a standing ban into a timed one.
- A background sweep lifts expired bans automatically and **attributes the unban to the moderator who set it**.
- Timers are saved to `pending_moderation_actions.json`, so a 7-day ban survives restarts and app closes.
- The sweep is fail-safe: a failed VRChat lookup or unban is retried on the next pass rather than dropping the timer or stranding the user, and a manually-lifted ban cleans up its own timer.
- Requires the same group ban-management permission as a normal ban. The button row can be hidden at **Settings → Moderation → "show timed-ban buttons on moderation posts"** (on by default); already-set timers still complete either way.

*(Based on a community contribution; it was reworked to fit the current codebase, which fixed a number of issues in the original — it would otherwise have overwritten the richer pending-actions store, never actually unbanned anyone, lost its timers on restart, treated a failed status lookup as "not banned," and shipped without the button handlers or a scheduler.)*

### Automatic evidence capture (OBS / Medal)

New **Settings → Evidence** options let Scarlet trigger your capture tool the instant a moderation event happens, the way BanLogger used to.

- Enable "trigger capture hotkey on kick/ban" and enter the hotkey you bound in OBS/Medal (e.g. `CTRL+SHIFT+F9`, or just `F9`).
- On a kick, group removal, or ban, Scarlet synthesizes that key press for 250 ms at the OS level, so a tool listening for a *global* replay-buffer/clip hotkey grabs the moment even when it isn't focused.
- Off by default, best-effort (it logs and moves on if the key can't be sent or the machine is headless), and it never blocks moderation handling. You can exercise it from the debug Event console.

### Fixed: recurring secure-store corruption

On Windows, saved credentials could periodically become unreadable — the `AEADBadTagException: Tag mismatch` error — because they lived only in the registry-backed Java Preferences store, which corrupts under antivirus locks, roaming/synced profiles, OneDrive-redirected AppData, or an unclean shutdown. Worse, one bad read of an internal key record used to silently discard *all* stored secrets at once.

- Scarlet now keeps a **redundant encrypted copy** of every secret in a file in the data folder. Writes go to both; a read only falls back to the file when the registry copy is missing or fails its integrity check, and then rebuilds the bad copy from the good one.
- It **never destroys data it can't verify**: the silent reset is gone, a full snapshot is taken before any unavoidable reseed, and unreadable bytes are quarantined to a timestamped file instead of being overwritten.
- New CLI: `repair-secure-store` reconciles the two stores and prints a report; `repair-secure-store cutover` (after a clean report) snapshots the registry and then empties it, making the data-folder file the primary store.
- Installs that were fine are unaffected — the file copy only ever activates when the registry has already failed.

### VRChat API bump

- Updated the bundled `vrchatapi-java` client to **1.20.9-nightly.5** (from 1.20.8-nightly.15). VRChat changed its API in a way the previous build no longer tracks cleanly; this release exists to pick that up. Updating from 0.4.18-b2 is recommended to stay in sync with VRChat.
