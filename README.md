<div align="center">

# Celeste

A self-hostable VRChat group management utility with Discord integration.

</div>

> **About this project.** Celeste is maintained by **KozyBlake** and developed largely with AI assistance — a "vibe-coded" project. Features and fixes come from KozyBlake and community ideas, implemented and reasoned through with an AI pair, then reviewed and built by KozyBlake. On first launch Celeste shows this notice and lets you close it if running AI-assisted software isn't for you. (KozyBlake maintains and is accountable for the project; the AI is a tool, not the maintainer.)

> **Celeste is the Rust rewrite of [Scarlet](https://github.com/KozyBlake/Scarlet).** Scarlet was a Java/Swing application; it is archived and will not be updated. Celeste is a native binary with the same feature set, the same data files, and the same Discord command surface (minus the parts listed under [Deliberately absent](#deliberately-absent)). Versioning restarts at **0.10.0**. An existing Scarlet data folder is found and adopted automatically — see [Data folder](#data-folder).

- [Settings reference](SETTINGS.md)
- [Building from source](BUILDING.md)
- [Installation](#installation)
- [VRChat Group](https://vrchat.com/home/group/grp_f12667c7-df5f-454f-9a34-5ed8c33112a1)

---

## Why self-host?

Celeste is self-hosted, meaning you have complete control of your group's data.
Your VRChat and Discord credentials are held only on your own machine — they are never sent to a third party, and no Celeste-operated service ever receives them.
Since there is no automatic synchronization of data between groups running Celeste, you don't have to worry about other groups seeing who your group has moderated or what groups you are tracking.

Celeste talks to VRChat and Discord because that is the job. Beyond those, the only outbound connections come from **optional** features that are off by default: avatar-database search (to whichever VRCX-compatible providers you enable) and the mobile companion. The companion's off-network relay ships pointing at a maintainer-run host and is not contacted until you have paired a phone and accepted an on-screen notice naming that host and what it receives; you can decline and keep using the companion over your own network, or point it at a server you run.

**There is no update check and no telemetry.** Celeste never phones home. If a build behaves differently from that claim, treat the claim as the thing to check the build against.

---

## Highlights

- **Native, no runtime to install.** A single binary. No JVM, no `.jar`, no `run.bat`. Built for Linux and Windows; developed and tested on Arch Linux.
- **Near-instant moderation posts.** Moderation actions (Discord command, UI, or bulk-ban queue) and in-game warns/kicks/bans post to Discord within seconds via burst polling, then settle back to the idle interval.
- **The full group audit log, rendered.** All 34 VRChat group audit event types get their own title, colour and structured fields rather than a raw JSON dump.
- **Live instance monitoring.** When VRChat runs on the same machine, Celeste reads the client log and keeps a live player list — joins, leaves, avatar changes, staff and watched-entity badges — in its own Discord thread.
- **Watched users, groups and avatars**, each with priority, tags, notes, a custom advisory message, and silent/critical modes.
- **Text-to-speech callouts**, locally or into a Discord voice channel.
- **Crash-safe data files** with dated backups beside each one.

---

## Features

### Discord commands

Celeste registers **130 slash commands**. They are guild commands, registered against the one server you configure.

<details>
<summary><strong>Moderation</strong></summary>

```
/discord-warn              /vrchat-user-ban            /vrchat-user-info
/discord-kick              /vrchat-user-ban-multi      /discord-user-info
/discord-ban               /vrchat-user-unban
                           /vrchat-user-unban-multi

/tag add                   /moderation-tags add
/tag remove                /moderation-tags delete
                           /moderation-tags list
```

`/tag add` and `/tag remove` are used inside a moderation thread and search every tag as you type. Tagging is allowed for anyone whose role can post in that thread.
</details>

<details>
<summary><strong>Watched entities</strong></summary>

The three families are symmetrical — `watched-user`, `watched-group` and `watched-avatar` each support:

```
add            list           set-priority     set-tags        export
remove *       view           set-critical     add-tag         import
remove-menu    set-message    set-silent       remove-tag
               set-notes      set-type
```

\* `watched-group` spells its removal `delete-watched-group`; `watched-avatar` and `watched-user` use `remove`.
</details>

<details>
<summary><strong>VRChat group management</strong></summary>

```
/vrchat-group open-instance        /vrchat-group list-bans
/vrchat-group close-instance       /vrchat-group list-invites
/vrchat-group add-role             /vrchat-group list-join-requests
/vrchat-group remove-role          /vrchat-group search-members
/vrchat-group audit-types

/vrchat-group create-post          /vrchat-group create-announcement
/vrchat-group list-posts           /vrchat-group view-announcement
/vrchat-group delete-post          /vrchat-group delete-announcement

/vrchat-group transfer-start       /vrchat-animated-emoji from-file
/vrchat-group transfer-check       /vrchat-animated-emoji from-url
/vrchat-group transfer-cancel

/vrchat-search user    /vrchat-search group
/vrchat-search world   /vrchat-search avatar
```

`/vrchat-group delete-announcement` keeps its safety gate: the `confirm-delete` option must be the literal string `DELETE`.
</details>

<details>
<summary><strong>Audit and logging</strong></summary>

```
/moderation-log                    /set-audit-channel
/export-log                        /set-audit-ex-channel
/moderation-summary                /set-audit-secret-channel
/actor-moderation-summary          /set-audit-secret-ex-channel
/outstanding-moderation            /set-discord-action-log-channel
/query-actor-history
/query-target-history              /aux-webhooks add
                                   /aux-webhooks list
                                   /aux-webhooks remove
                                   /set-audit-aux-webhooks
```
</details>

<details>
<summary><strong>Configuration</strong></summary>

```
/settings edit                     /set-tts-voice
/config-info                       /set-voice-channel
/celeste-version                   /set-training-channel
/server-restart restart-now        /set-ops-alert-channel
                                   /set-discord-account-age-alert
/celeste-discord-permissions list  /set-verification-auto-invite
/celeste-discord-permissions set   /set-ticket-age-verify-message
                                   /set-ticket-tool-auto-response

/config-set moderation-summary time-of-day
/config-set outstanding-moderation period-days
/config-set outstanding-moderation time-of-day
/config-set suggested-moderation kick-count
/config-set suggested-moderation period-days
```

`/settings edit` is the typed editor for everything in [SETTINGS.md](SETTINGS.md).
</details>

<details>
<summary><strong>Staff lists and account linking</strong></summary>

```
/staff-list add            /secret-staff-list add
/staff-list delete         /secret-staff-list delete
/staff-list list           /secret-staff-list list

/link-vrchat-account       /unlink-vrchat-account
                           /unlink-vrchat-account-for
```
</details>

#### Renamed from Scarlet

Two commands changed name with the project. Nothing else did.

| Scarlet | Celeste |
|---|---|
| `/scarlet-discord-permissions` | `/celeste-discord-permissions` |
| `/scarlet-version` | `/celeste-version` |

If you had per-role permission overrides configured against the old names, set them again against the new ones.

#### Deliberately absent

31 of Scarlet's commands are not in Celeste, by decision rather than oversight:

- **The whole `/schedule` family** (25 commands) — VRChat calendar events.
- **Report templates** (5 `/config-set report-template` commands).
- **`/submit-evidence`.**
- **`/server-restart update-now`** — Celeste ships as a native binary with no self-updater. `/server-restart restart-now` still works.

A test (`command_surface_matches_java`) compares the registered surface against the Scarlet inventory on every build and fails with a list if anything else goes missing.

---

### CLI commands

The CLI tab (View → CLI, or `Settings → Appearance`) accepts:

| | |
|---|---|
| `help` | list commands |
| `info` | version and environment |
| `tts <text>` | speak text through the configured route |
| `link` | link a VRChat account |
| `logout` | end the VRChat session |
| `explore` | open the data folder |
| `importgroups`, `importgroupsjson` | bulk-import watched groups |
| `simulate` | fire a training event (requires training mode) |
| `restore-advisories`, `translate-advisories` | advisory maintenance |
| `langlint` | validate community translation files |
| `popup-test`, `vrchatapi-test` | diagnostics |
| `repair-secure-store` | rebuild the encrypted credential store |
| `clear` | clear the console |
| `restart`, `exit` | restart or quit |

---

### Interoperability

Celeste links to the official VRChat website wherever possible — user profiles, group posts, instances and more.
It can also generate a link that pre-fills the VRChat Help Desk report form from the custom tags on a moderation event, which cuts out a step and a class of transcription mistake.

---

### Localization

- The desktop UI and the spoken TTS callouts are translated into **German, English, Spanish, Indonesian, Japanese and Russian**. All five non-English bundles are substantially complete; anything untranslated falls back to English rather than showing a raw key.
- Pick a language in **Settings → Appearance** (a dropdown of native names). It defaults to your operating system language and applies on restart. The Discord bot renders each reply in the invoking user's own Discord language.
- **Community translations without a rebuild:** drop a `messages_<lang>.properties` file into the `lang/` folder in Celeste's data directory and restart — new languages are auto-discovered, and Celeste writes an up-to-date English `messages.template.properties` there as a starting point. The `langlint` CLI command (and a startup check) validates community files, flagging missing keys and broken `{0}` placeholders.

---

### Training mode

- Enable **Settings → Training** to unlock an event simulator (Edit → *Simulate event (training)…*, or the `simulate` CLI command) that fires realistic events on demand — so new moderators can learn on a screenshare instead of a live incident, and nobody has to actually join a bad group.
- The simulator can fire **moderator warn / kick / ban** events. Each produces a genuine moderation post and thread with the real **Edit tags**, **Edit description**, **Ban / Unban** and **Timed ban** buttons (and `/tag` works in its thread), so a trainer can exercise the whole tagging workflow from a moderator's account without banning anyone. These posts go **only** to the channel set with `/set-training-channel` — nothing is posted if none is set. Ban / Unban / Timed ban on a training user run the real permission checks and then reply with a simulated result without calling VRChat, and drills never appear in moderation summaries, which are counted from VRChat's own audit log.
- While training, Celeste behaves as a **separate client**: the real instance is parked and kept updated in the background, actions on training players show real success feedback but make no VRChat call, and turning training off restores the live instance instantly. Everything training-related is marked `[TRAINING]`, so a drill can never be mistaken for a real record.

---

### Reliability and health monitoring

- A background watchdog turns silent failure modes into loud, recoverable ones: if the VRChat log goes quiet while the game is running, or the VRChat session expires mid-run, Celeste raises a popup, attempts unattended re-login, and (with `/set-ops-alert-channel`) posts a Discord health alert. A healthy session is never torn down just because it is old.
- Data files (watched lists, settings, Discord config, moderation tags, …) are written crash-safely and keep the last several dated backups in a `backups/` folder beside each file, so a crash mid-write cannot lose a blocklist.
- After downtime, Celeste catches up on the missed audit log quickly and at most `audit_catchup_max_days` back (default 14, see [SETTINGS.md](SETTINGS.md)), so live moderation posts resume within minutes rather than replaying months of old events.
- The **Diagnostics** view (Help → *Celeste: Diagnostics…*) shows connectivity, rate-limit and avatar-provider status from cached state, without making extra API calls.

---

### Desktop and mobile notifications

- Advisories can raise native desktop notifications (Windows Action Center, Linux `notify-send`, macOS `osascript` — PowerShell is never invoked), configured per category in **Settings → Desktop Notifications**.
- A mobile companion can push the same alerts to a phone; pair it from **Settings → Mobile Companion**.

---

### About extended events

Extended audit events (Instance Inactive, Staff Join, Staff Leave, Vote-to-Kick Initiated) are routed with `/set-audit-ex-channel`.

Most of them require a VRChat client to be running **on the same machine** and in a group instance, because Celeste derives them from the client log file rather than a server-side audit record. They are therefore not of the same evidentiary weight as the canonical group audit events — but similar functionality may one day exist in VRChat's first-party API.

---

### About actors and targets

**Actors** are the users who *initiated or performed* an event, like a group staff member.
**Targets** are the users an event was *performed on*, like the user a staff member kicks from an instance.
For some events the target is not a user at all — group instance creation, for example.

---

## Installation

### Requirements

- A VRChat group you (or a dedicated bot account) have permissions for.
- A Discord server (guild) in which you have permissions.
- A Discord bot account (application).
- A Windows or Linux PC. **No Java, and no other runtime, is needed.**

The Discord and VRChat-API features work anywhere. The VRChat-log features — instance monitoring, extended events, avatar statuses, the live player list — require the VRChat client to be running on the same machine, natively on Windows or under Proton/Wine on Linux.

---

### Setting up the Discord application

It is recommended that you create a bot account dedicated specifically to running Celeste.
Create a new application via the Discord developer dashboard: https://discord.com/developers/applications

Celeste requires some permissions above the bare defaults:

1. In the `Installation` tab for your app:
    - Celeste only supports installation for servers.
      Ensure that only the `Guild Install` box is checked in the `Installation Contexts` area.
      ![setup installation contexts](images/setup_installation_contexts.png?raw=true)
    - In the `Guild Install` part of the `Default Install Settings` area:
      - Add the `bot` scope.
      - Add the `Attach Files`, `Create Polls`, `Create Public Threads`, `Embed Links`, `Manage Webhooks`, `Read Message History`, `Send Messages`, `Send Messages in Threads`, `Speak`, and `View Channels` permissions.
        ![setup default install settings](images/setup_default_install_settings.png?raw=true)
2. In the `Bot` tab:
    - Enable the `Server Members` and `Message Content` intents in the `Privileged Gateway Intents` area.
      ![setup privileged gateway intents](images/setup_privileged_gateway_intents.png?raw=true)
3. Invite the bot to your server.
4. In the `Installation` tab for your app, select the `None` option for the `Install Link` area.
   ![setup install link](images/setup_install_link.png?raw=true)

---

### Setting up the VRChat group

The VRChat account Celeste uses must have at least these group permissions:

- `Manage Group Member Data`
- `View Audit log`
- `View All Members`

Celeste does not enforce moderation actions against users on its own. Kicks, bans and the rest are performed by a human — Celeste provides the Discord commands, buttons and records around them.

---

### Running Celeste

Celeste is a single executable. There is nothing to install.

```sh
chmod +x celeste        # Linux, first run only
./celeste
```

First launch opens a setup wizard for the Discord bot token, the guild, and the VRChat account.

To build it yourself, see [BUILDING.md](BUILDING.md).

---

### Data folder

By default Celeste stores everything under:

| | |
|---|---|
| Linux | `$XDG_DATA_HOME/KozyBlake/Celeste`, or `~/.local/share/KozyBlake/Celeste` |
| Windows | `%LOCALAPPDATA%\KozyBlake\Celeste` |

**Upgrading from Scarlet:** discovery also looks under the old `KozyBlake/Scarlet` locations and the other legacy layouts, so an existing install is found and adopted rather than starting empty. Data files are byte-compatible in both directions — the same `settings.json`, `watched_*.json`, `discord_bot.json` and so on.

Credentials are the exception. The encrypted credential store is not readable by Celeste, so you will sign in to VRChat and re-enter the Discord bot token once. **If VRChat two-factor is enabled on the account, confirm you still have your authenticator before you remove Scarlet.**

To move the data folder, use **Settings → System → data folder**; `CELESTE_HOME` (or the legacy `SCARLET_HOME`) overrides it for one run.

---

### Optional: text-to-speech

TTS is optional and off until you choose a voice. Celeste does not install anything for you — it detects what is present and tells you if nothing is.

On Linux it enumerates every engine it finds, in this order, and offers all of their voices together:

| Engine | Arch package | Debian/Ubuntu package |
|---|---|---|
| Flite | `flite` | `flite` |
| Pico | `svox-pico-bin` (AUR) | `libttspico-utils` |
| eSpeak NG | `espeak-ng` | `espeak-ng` |
| eSpeak | `espeak` | `espeak` |

Package names differ between distributions; the table covers the two most common. Playback goes through `pw-play`, `paplay` or `aplay`, whichever is present — set `CELESTE_TTS_PLAYER` to override.

macOS uses the built-in `say`; Windows uses SAPI voices via PowerShell.

If no engine is installed, Celeste opens an explanatory warning instead of a voice list.

---

## Credits and licence

Celeste is MIT licensed, and the licence it inherits is Scarlet's. Scarlet was
written by **Vinyarion** of the **Sybyline Network**, who holds the copyright
in `LICENSE`; that notice stays, and so does the credits list the app shows
under Help. This is a rewrite of their work, not a replacement of their
authorship.

Scarlet's logo is **not** included here and is not Celeste's. It belonged to
that project and was never mine to carry over. Celeste uses its own crescent
logo, including dedicated sizes for application windows, taskbars and small
system icons.

The Discord setup screenshots in `images/` are crops of Discord's own
developer dashboard.
