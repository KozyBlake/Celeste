<div align="center">

<img src="rust/assets/celeste.png" alt="Celeste logo" width="240">

# Celeste

A self-hostable VRChat group management utility with Discord integration.

</div>

> **About this project.** Celeste is maintained by **KozyBlake** and developed largely with AI assistance — a "vibe-coded" project. Features and fixes come from KozyBlake and community ideas, implemented and reasoned through with an AI pair, then reviewed and built by KozyBlake. On first launch Celeste shows this notice and lets you close it if running AI-assisted software isn't for you. (KozyBlake maintains and is accountable for the project; the AI is a tool, not the maintainer.)

> **Upgrading from an earlier install?** Your existing data folder is found and adopted automatically, and the data files are compatible in both directions. See [Data folder](#data-folder) for what carries over and what you will re-enter.

- [Settings reference](SETTINGS.md)
- [Building from source](BUILDING.md)
- [Contributing](CONTRIBUTING.md)
- [Credits](CREDITS.md)
- [Installation](#installation)
- [License](https://github.com/KozyBlake/Celeste?tab=License-1-ov-file)

---

## Why self-host?

Celeste is self-hosted, meaning you have complete control of your group's data.
Your VRChat and Discord credentials are held only on your own machine — they are never sent to a third party, and no Celeste-operated service ever receives them.
Since there is no automatic synchronization of data between groups running Celeste, you don't have to worry about other groups seeing who your group has moderated or what groups you are tracking.

Celeste talks to VRChat and Discord because that is the job. Beyond those, the only outbound connections come from **optional** features that are off by default: avatar-database search (to whichever VRCX-compatible providers you enable).

**There is no update check and no telemetry.** Celeste never phones home. If a build behaves differently from that claim, treat the claim as the thing to check the build against.

---

## Highlights

- **A single binary, no runtime to install.** Built for Linux and Windows; developed and tested on Arch Linux.
- **Near-instant moderation posts.** Moderation actions (Discord command, UI, or bulk-ban queue) and in-game warns/kicks/bans post to Discord within seconds via burst polling, then settle back to the idle interval.
- **The full group audit log, rendered.** All 34 VRChat group audit event types get their own title, colour and structured fields rather than a raw JSON dump.
- **Live instance monitoring.** When VRChat runs on the same machine, Celeste reads the client log and keeps a live player list — joins, leaves, avatar changes, staff and watched-entity badges — in its own Discord thread.
- **A searchable roster.** Ctrl+F narrows a packed instance to the person you are looking for, matching display name, user ID or worn avatar. Decorative fonts, other writing systems and look-alike letters are all searched, so the letters on the nameplate are the letters you type.
- **Watched users, groups and avatars**, each with priority, tags, notes, a custom advisory message, and silent/critical modes.
- **Opt-in automatic group bans.** A separate `auto_ban_groups.json` list can ban a live join whose visible memberships match, using the VRChat account signed into Celeste.
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
/celeste-version                   /set-ops-alert-channel
/server-restart restart-now        /set-discord-account-age-alert
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

#### Renamed commands

Two commands were renamed when the project was. Nothing else changed.

| Previously | Now |
|---|---|
| `/scarlet-discord-permissions` | `/celeste-discord-permissions` |
| `/scarlet-version` | `/celeste-version` |

If you had per-role permission overrides set against the old names, set them again against the new ones.

#### Deliberately absent

Some command families are deliberately not included:

- **The whole `/schedule` family** (25 commands) — VRChat calendar events.
- **Report templates** (5 `/config-set report-template` commands).
- **`/submit-evidence`.**
- **`/server-restart update-now`** — Celeste ships as a native binary with no self-updater. `/server-restart restart-now` still works.

A test compares the registered commands against a committed inventory on every build, so nothing else can go missing unnoticed.

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
| `restore-advisories`, `translate-advisories` | advisory maintenance |
| `popup-test`, `vrchatapi-test` | diagnostics |
| `repair-secure-store` | rebuild the encrypted credential store |
| `clear` | clear the console |
| `restart`, `exit` | restart or quit |

---

### Interoperability

Celeste links to the official VRChat website wherever possible — user profiles, group posts, instances and more.
It can also generate a link that pre-fills the VRChat Help Desk report form from the custom tags on a moderation event, which cuts out a step and a class of transcription mistake.

---

### Reliability and health monitoring

- A background watchdog turns silent failure modes into loud, recoverable ones: if the VRChat log goes quiet while the game is running, or the VRChat session expires mid-run, Celeste raises a popup, attempts unattended re-login, and (with `/set-ops-alert-channel`) posts a Discord health alert. A healthy session is never torn down just because it is old.
- Data files (watched lists, settings, Discord config, moderation tags, …) are written crash-safely and keep the last several dated backups in a `backups/` folder beside each file, so a crash mid-write cannot lose a blocklist.
- After downtime, Celeste catches up on the missed audit log quickly and at most `audit_catchup_max_days` back (default 14, see [SETTINGS.md](SETTINGS.md)), so live moderation posts resume within minutes rather than replaying months of old events.
- The **Diagnostics** view (Help → *Celeste: Diagnostics…*) shows connectivity, rate-limit and avatar-provider status from cached state, without making extra API calls.

---

### Desktop notifications

- Advisories can raise native desktop notifications (Windows Action Center, Linux `notify-send`, macOS `osascript` — PowerShell is never invoked), configured per category in **Settings → Desktop Notifications**.

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
- A Windows or Linux PC. Nothing else to install.

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

Most moderation remains human-directed through Celeste's Discord commands and buttons. The one explicit exception is the optional automatic-ban list under **Tools → Automatic ban groups**. If enabled, the signed-in VRChat account also needs `Manage Group Bans`. Celeste checks only live joins to the configured group's own instance, only memberships visible to that account, and writes the opt-in list separately to `auto_ban_groups.json`.

VRChat's group API can apply the group ban and prevent re-entry, but it does not expose the in-client **Kick User** action. Celeste therefore reports a successful automatic ban and tells the moderator to use that action if the user remains in the current lobby.

---

### Running Celeste

Celeste is a single executable. There is nothing to install.

```sh
chmod +x celeste        # Linux, first run only
./celeste
```

First launch opens a setup wizard for the Discord bot token, the guild, and the VRChat account.

#### Graphics compatibility and safe mode

Celeste is meant to open on anything, including a machine with no GPU at all.
On Windows it tries DirectX 12, then Vulkan, then OpenGL, and if the machine
has no usable graphics driver it falls back to **software rendering** on the
processor. Linux uses OpenGL, where Mesa's llvmpipe already provides a complete
software implementation. The renderer can be fixed under **Settings → General
→ Graphics renderer**; the change takes effect after a restart.

Software rendering needs no graphics driver whatsoever, so it is the right
choice on a virtual machine (Proxmox, QEMU, Hyper-V, a VPS) or on a system
running on basic display drivers. It is slower than a GPU but entirely
correct, and Celeste's interface is light enough that it stays comfortable.

If a renderer cannot start, Celeste relaunches with the next one down and shows
the graphics error that caused the recovery. The fallback is temporary and does
not overwrite the saved setting. To go straight to the renderer with the
fewest requirements:

```powershell
.\celeste.exe --safe-mode
```

Every platform also accepts `--renderer auto`, `--renderer software`,
`--renderer vulkan`, `--renderer opengl`, and `--renderer directx12` on
Windows. For launchers and desktop shortcuts, the same override can be
supplied through `CELESTE_RENDERER`. Celeste logs every graphics adapter it
finds and the one it picks, so a startup log is enough to tell what a machine
offered.

#### Unsigned builds

Celeste's releases are **not code-signed**. A signing certificate is a recurring
cost that an unfunded project does not carry, so Windows will stop you
the first time you run it:

> Windows protected your PC

Choose **More info**, then **Run anyway**. SmartScreen is reporting that the
publisher is unverified, which is true and will stay true. It is not telling you
anything about the file itself.

Check the file instead. Every release lists a SHA-256 for each download on its
release page:

```powershell
Get-FileHash .\celeste.exe -Algorithm SHA256
```

```sh
sha256sum celeste
```

If what you get does not match what the release page says, do not run it — open
an issue.

To build it yourself, see [BUILDING.md](BUILDING.md).

---

### Data folder

By default Celeste stores everything under:

| | |
|---|---|
| Linux | `$XDG_DATA_HOME/KozyBlake/Celeste`, or `~/.local/share/KozyBlake/Celeste` |
| Windows | `%LOCALAPPDATA%\KozyBlake\Celeste` |

**Upgrading from Scarlet:** discovery also looks under the old `KozyBlake/Scarlet` locations and the other legacy layouts, so an existing install is found and adopted rather than starting empty. Data files are byte-compatible in both directions — the same `settings.json`, `watched_*.json`, `discord_bot.json` and so on.

`auto_ban_groups.json` is deliberately separate from `watched_groups.json`: watching is informational, while placing a group on the automatic-ban list authorizes a real external moderation action.

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

## Help wanted

Celeste is built largely with AI assistance, as the note at the top of this page
says. That is what got it this far, and it is also why the two things below
would help most.

**Rust developers.** The project would benefit from people who write Rust
properly — reviewing what is here, saying what is idiomatic and what is not, and
taking work on directly. You do not need to know VRChat or Scarlet to be useful;
much of what needs doing is ordinary Rust.

**Testers.** Celeste is in beta and the rough edges are still being found. The
most useful reports name the build, say what you did, and say what you expected
to happen instead. Group-permission edge cases, unusual instance transitions,
high-DPI and multi-monitor setups, and running under Proton are all
under-tested.

Issues and pull requests: https://github.com/KozyBlake/Celeste.
[CONTRIBUTING.md](CONTRIBUTING.md) covers what makes a report useful, what a
pull request needs to pass, and the terms contributions are offered under.

---

## Credits and licence

Celeste is licensed under the **[PolyForm Shield License 1.0.0](https://github.com/KozyBlake/Celeste?tab=License-1-ov-file)**.
It is source-available, not open source. You may use it for any purpose,
including inside a business, and you may change it, build on it and pass it on
— the one exception is providing a product that competes with Celeste. Read
[the licence](LICENSE) for what counts as competing; it is broader than
selling.

Releases before this change, including 0.10.0, were published under the
PolyForm Noncommercial License 1.0.0 and remain available under those terms.

Celeste is a rewrite of Scarlet. Scarlet was written by **Vinyarion** of the
**Sybyline Network** and released under the MIT License. The applicable Scarlet
copyright and licence notices remain in place, and so does the credits list the
app shows under Help. This is a rewrite of their work, not a replacement of
their authorship.

Scarlet's logo is **not** included here and is not Celeste's. It belonged to
that project and was never mine to carry over. Celeste uses its own crescent
logo, created by
[CeleryDinoSauR](https://vrchat.com/home/user/usr_12af8628-8dbe-4396-89de-0c92cea53782),
including dedicated sizes for application windows, taskbars and small system
icons.

The Discord setup screenshots in `images/` are crops of Discord's own
developer dashboard.
