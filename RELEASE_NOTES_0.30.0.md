# Celeste 0.30.0

Mostly about Celeste saying when something is wrong. A copy that quietly
stops working is the failure nobody reports, so this release adds the
three things that make it visible: an ops-channel alert when audit
polling fails, a status-bar warning when it stalls, and a note at launch
when a newer release exists.

**If you are running 0.20.0, update.** Two thirds of Celeste's VRChat
requests bypassed the rate limiter, including the ones that poll on a
timer, so a rate-limited account kept making requests straight through
its own backoff. That is fixed here, along with two security fixes.

## Celeste says when moderation stops reaching Discord

Celeste going quiet looks exactly like a quiet night. Audit polling
could stop working — a token expired, VRChat refusing the query, the
group's permissions changed — and the window carried on looking healthy
while no kick, warn or ban reached Discord.

Two indicators now:

- **In the status bar**, a warning when no audit poll has completed for
  three polling intervals, with a three-minute floor so a short interval
  cannot make it jumpy. A poll runs on its timer whether or not anything
  happened, so a quiet group cannot trigger it, while a poller that has
  stopped, is failing, or never started does. It stays silent where
  polling is not expected to run at all — no VRChat sign-in, or no group
  configured — because telling a log-only install that its moderation
  has stalled would teach everyone to ignore the indicator.
- **In the ops alert channel**, three consecutive failed polls open one
  incident, and a successful poll closes it. One alert and one recovery
  rather than a retry log, and a single blip stays out of the channel
  entirely.

Both name the likely causes, and both say the audit cursor has not
advanced, so nothing is lost once the problem clears: Celeste resumes
from where it stopped.

## A note when a newer Celeste has been released

Celeste ships as a binary people download once and keep, so most
installs only learn about a release when somebody notices one. A copy
that quietly stops working on a version whose fix shipped three releases
ago is worse than one that just stops.

One request to the GitHub releases API at launch, on a ten-second
timeout, answered on a channel so a slow or unreachable GitHub can never
hold up the window. A newer tag appears as a clickable note in the
status bar that opens the release page.

Nothing is downloaded and nothing is installed. An auto-updater is a
code-execution channel into every install, and that is a decision to
take deliberately rather than to arrive at by accident.

Controlled by `update_check_on_launch`, on by default. It is the only
request Celeste makes to anywhere that is not VRChat or Discord, so
turning it off is worth being able to do.

## Every VRChat request goes through the rate limiter

Only 21 of the client's 55 request sites went through the limiter. The
other 34 called the HTTP client directly — the group audit log query,
the group-instances poll, sticker lookups and every search among them.

Those sites did not merely skip the token bucket. The limiter is also
where a global backoff is waited on and where a 429 is recorded, so a
rate-limited Celeste kept making requests through its own backoff, on
exactly the paths that poll on a timer. Every request site now goes
through it.

A hostile or mistaken `Retry-After` of `i64::MAX` also overflowed when
converted from seconds to milliseconds; that conversion now saturates.

## Opt-in automatic group bans

New under **Tools → Automatic ban groups**: a list of groups whose
members are banned from yours, using the VRChat account signed into
Celeste, when one of them joins.

Deliberately narrow, because this is the one place Celeste acts without
a human pressing the button:

- The list is its own file, `auto_ban_groups.json`. Watching a group
  stays advisory, so an existing watch list cannot suddenly become
  destructive.
- Only live joins to your configured group's own instance, and only
  memberships that account can see.
- The signed-in account needs `Manage Group Bans`, checked before
  anything is attempted. An install that is not configured for it still
  calls out a match and bans nobody.
- Anyone holding moderation authority in the matched group is called out
  and never banned. Invite or announcement permissions do not count as
  moderation authority.

VRChat's group API can apply the ban and prevent re-entry, but it does
not expose the in-client **Kick User** action, so Celeste reports the ban
and tells the moderator to kick if the person is still in the lobby.

## Text to speech

- **Festival** and **RHVoice** are detected on Linux, with the packages
  to install documented in the README.
- The voice pickers are searchable, and `tts_hidden_voices` hides whole
  engines or individual voices from them — eSpeak alone lists enough
  entries to make the picker hard to use. Hiding a voice does not
  uninstall it, and does not stop an already selected voice from
  speaking.
- A **TTS: Volume** setting, and a chime before spoken callouts.

## The interface is English only

The translation layer and the German, Spanish, Indonesian, Japanese and
Russian bundles are gone, along with the community `lang/` folder and
the `langlint` command that validated it. They were a port of the Java
app's localization and were drifting against a UI that changes every
release.

The operating system's language is still read, for one decision that is
not about wording: whether a display name is written in a script you
read natively, which decides whether TTS pronounces it as-is or
romanises it. A Russian speaker should hear a Cyrillic name read, not
transliterated, whatever language the buttons are in.

## Security fixes

- **Archive imports reject inconsistent headers.** Five advisories were
  filed against zip 8, the most serious a parser differential between an
  archive's local and central-directory headers — "zip cloaking" —
  rated critical for arbitrary file overwrite. No 8.x fix exists, so
  Celeste moves to zip 9.3, and the legacy-import path now validates
  header consistency before reading or extracting anything. The 7Z
  reader also stops extracting into the temp directory just to list
  what an archive contains.
- **IPv4-mapped addresses count as the IPv4 they map to.** The guard on
  watched-group import URLs checked IPv6 addresses by their segments, so
  an address such as `::ffff:127.0.0.1` passed as public and an import
  URL could reach loopback or the private network.
- **rustls 0.23.46**, by way of a Dependabot update.

## Dead groups stop being reported as news

A watch list that has been running a while collects groups VRChat has
since deleted. They can never resolve, so every launch wrote a log line
for each one -- 132 of 370 entries on one install -- and the entries
still looked live in the file.

A group VRChat answers 404 for is now tagged `REMOVED` in
`watched_groups.json`. A tag rather than a new type, so the group keeps
its classification and the note saying why it was watched; a tag rather
than a second file, because a dead entry is still a watch-list entry.

The mark clears itself the moment VRChat answers for that group again,
and the entry is back to exactly what it was -- nothing else about it
ever changed. That re-check costs nothing day to day, and happens with
each cache clean-up, so a group that was only briefly missing is asked
about again rather than written off.

Only a real 404 marks anything. A network failure or an expired session
leaves the entry alone, because a request that did not reach VRChat says
nothing about whether the group exists.

`REMOVED` is Celeste's own bookkeeping and is not offered on Discord:
`add-tag` and `remove-tag` refuse it, `set-tags` ignores it while
keeping any real one underneath, and it is hidden from tag listings. A
mark applied by hand would be a claim nothing checked.

## The log says what Celeste is doing, in English

Celeste's log file was written for whoever was debugging that line at the
time. It now reads as sentences, because the person who needs it is
usually the person whose Celeste just stopped working.

Each VRChat request records what was asked, what came back and how long
it took -- `Asked VRChat: GET /groups/grp_… — 200 OK in 243ms` -- and
says so when it had to queue behind the rate limit first. A 429 names the
request that was refused, how long everything is being held, and whether
that figure came from VRChat or from Celeste's own default.

Every audit poll reports what it found and whether the cursor moved:
`Checked the group audit log for 23:03:41 to 23:04:41: 2 new entries —
took 214ms, caught up to 23:04:39`. A poll that stops appearing is what
moderation silently not reaching Discord looks like, and this is where
it can be seen happening. When the local log brings a check forward instead
of waiting out the timer, it says that too, which is the line that
explains how a kick reaches Discord in about a second.

An event that goes nowhere now says why -- no channel configured, or not
connected to Discord -- rather than vanishing.

Smaller things that were printing like developer output: cache clean-up
logged every deleted file, which on one install was 1,176 lines, and now
logs one line saying how many files and how much space; "Tailing",
"Tailer init" and "Steam library candidate" say what they mean; crash
detection explains its reasoning in a sentence; and losing the Discord
connection is reported in Celeste's own words rather than only as the
gateway library's raw debug output.

The startup line naming the data folder now says *why* that folder was
chosen, which is nearly always the explanation when Celeste opens with
settings somebody does not recognise.

## Also in this release

- Watched users and avatars learn their display names from the live
  events that already carry them, instead of hundreds of API requests
  made only to decorate a list.
- An offline probe — an ignored test — times the path from a mass of
  player joins and a local kick line to the request that wakes the audit
  poller, running the real listener and rate limiter against a local
  Wiremock server. It is for investigating a delay, never touches a
  VRChat account, and cannot measure VRChat's own audit-entry
  availability or Discord delivery.
- 106 settings in 6 categories, with `SETTINGS.md` regenerated from the
  registry.
- The README now describes the Rust port rather than the Java app: its
  localization and training-mode sections documented features the port
  does not have.
- The licence and contact details describe this project rather than the
  one it was forked from. The manifest said MIT, and **Help -> About ->
  License** opened a 404 that also claimed MIT, while Celeste has been
  PolyForm Shield 1.0.0 since 0.20.0. The repository URL, which goes out
  in the VRChat User-Agent on every request, named a repository that no
  longer exists, and the contact in that header named Scarlet's author
  rather than whoever runs this fork. Credit for Scarlet is unchanged in
  `CREDITS.md`, which correctly still records that Scarlet itself is MIT.

Celeste is still a vibe-coded project, and it is still looking for Rust
developers willing to improve it and for testers willing to run it.
