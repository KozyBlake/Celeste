# Celeste 0.40.0

Mostly about Celeste being able to account for its own death, and being
reachable by the people who run the services it queries.

A GPU reset — an avatar hanging the graphics card, which is a thing that
happens in VRChat — takes down every program holding a graphics context.
The driver aborts them. Celeste went with the rest, and the log it left
behind ended mid-sentence with nothing in it about the ending. That is
the worst shape a crash can take, because the record of what happened is
the part that was thrown away.

Three changes fix that, and a fourth fixes something the investigation
turned up on the way: the community avatar databases Celeste queries had
no way of telling who was querying them.

**Nothing here is a vulnerability fix**, so there is no urgency to this
one. It is worth having if you want a crash to leave evidence, and worth
having if you run a service Celeste talks to.

## The log no longer loses the moments before a crash

Celeste wrote its log through a background writer: a line was formatted,
pushed onto an in-memory queue, and written to the file a moment later by
a worker thread. Between those two steps the line existed only in memory.
The buffer was flushed when its guard was dropped — which happens on an
orderly exit, and never when the process is killed outright.

So every log ended at whatever the worker had got around to writing, and
the lines describing the last seconds of a run were the ones most likely
to be lost. A crash threw away its own explanation.

The log is now written synchronously. Each line goes straight to the file
with one `write`, and once that call returns the bytes belong to the
kernel rather than to Celeste — so killing the process cannot take them
back. Writes are serialised through a mutex, because the formatter emits
several writes per event and Celeste logs from the interface thread and
every async worker; without it, lines from different threads would
interleave into nonsense.

The cost is one system call per line instead of a handoff to another
thread. At the volume Celeste logs — a couple of megabytes across a day
with debug logging on — this is not measurable.

This protects against the process dying, not against the machine dying.
There is no `fsync` per line, so a power cut can still lose whatever the
kernel has not yet written out. Data files are a different matter: those
are fsynced and renamed into place, because they are written rarely
enough to afford it.

## A line is written from inside the crash

A Rust panic has always been recorded, by the panic hook. Being killed
outright was not: when the graphics driver calls `abort()`, no Celeste
code runs afterwards — no hook, no unwinding, no shutdown.

Celeste now installs a handler for the fatal signals (`SIGABRT`,
`SIGSEGV`, `SIGBUS`, `SIGILL`, `SIGFPE`) which writes one line to the log
on the way out:

```
*** FATAL: Celeste was killed by signal 6 at unix time 1794276293 -- written by the crash handler; no further Celeste output follows.
```

A signal handler may only call a short list of functions that are safe to
run at an arbitrary point in a program — it may not allocate, format a
string, or take the logger's lock, because doing any of those in a
process that is already dying risks a deadlock, which is worse than a
missing line. So everything that can be prepared is prepared when Celeste
starts: the file is already open, and every byte except the two numbers
is already laid out. The handler writes those bytes, converts two
integers to digits on the stack, and re-raises the signal.

Re-raising matters: the process still dies with the same exit status and
still writes a core dump, so nothing a debugger or `coredumpctl` would
have given you is lost.

This is Unix only. Windows has no signal disposition to install into, and
its fatal errors are structured exceptions rather than signals, so a
Windows build still relies on the crash report below.

## The next start writes a crash report

A run that is killed cannot summarise itself, so the following one does
it. Celeste marks a run as in progress while it is running and clears the
mark on an orderly exit; a mark still present at startup means the last
run did not finish. Celeste then reads that run's log and writes
`logs/celeste_crash_<timestamp>.txt`, naming:

- **how it ended** — the signal and the time, when the handler caught it,
  and plainly nothing when it did not, rather than guessing a cause;
- **the avatars loading in the last minute before it stopped**, with the
  wearer and the performance rating;
- **warnings and errors** from the final two minutes;
- **the last forty lines** of the log, and where the full log is.

The report is not named like a log file, so the log pruner will not count
it among the ten it keeps and delete it.

On the avatar list: **it is not an accusation.** It records what Celeste
happened to be looking at, which in a busy instance means whatever was
loading at the time. The last avatar before a crash is a coincidence at
least as often as a cause, and a real crasher may not be in the list at
all — Celeste only sees what VRChat told it, and an avatar that kills the
graphics card can do it before it is ever named. The report says so, in
those words, where anyone reading it will see them. It is somewhere to
start looking. It is not grounds for a ban, and it is not wired to
anything that bans.

## Celeste identifies itself to the services it queries

Celeste sends a `User-Agent` naming the project, its version and how to
reach whoever runs it. The avatar databases were not getting it. Those
requests used a separate client that sent only `Celeste/<version>` — no
repository, no contact, no way to reach anybody — which is the one place
it mattered most, because those services are small and often run by one
person, and an unidentified client is one you block rather than ask
about. Worse, the name differed from the one every other request sent, so
nothing an operator saw there could be matched to anything else.

Every outbound request now carries the same identity:

```
KozyBlake-Celeste/0.40.0 (+https://github.com/KozyBlake/Celeste; mailto:chieriontop@chierion.top; Discord:@chieriontop/656328060701900836; https://discord.gg/shXBVdppW)
```

The shape changed as well as the contents. The contacts used to sit as
bare text after the product name, where the characters they contain are
not legal — a strict parser or a filtering proxy may keep only the text
before the first `;`, which is the product and nothing anyone could act
on. They now sit inside a parenthesised comment, which is the form the
specification describes and the form crawler operators already recognise.

**If you operate a service Celeste queries and you match on this header,
match the product name `KozyBlake-Celeste` and not the whole string.**
The version changes every release, so a rule built on the full string
will break at the next one. Older installs still send the earlier
strings, including `Celeste/<version>` to avatar databases and Scarlet-era
strings from before the project was renamed; Celeste does not update
itself, so those taper off as people install newer builds rather than
stopping on a release date.

## Reaching the maintainer without Discord

Every route into this project went through Discord or a public GitHub
issue. Someone who wants to say "your client is hitting my API too hard,
throttle it or I block it" will often do neither.

There is now a `SECURITY.md`: how to report a vulnerability privately, and
a section for anyone running a service Celeste queries, saying what the
avatar-database requests actually do — name lookups only, made when a
moderator runs a command, cached, and backed off when a provider fails or
rate-limits — and that a request to back off will be honoured rather than
argued with. The README carries the same contacts.

An email address is in all three places, and in the `User-Agent`. It needs
no account anywhere.

## Also in this release

- The log now opens with what Celeste is and which data folder it
  resolved, before anything else it has to say. A crash report for the
  previous run used to be the first line in a fresh log, ahead of the
  identity banner.
- `tracing-appender` is gone from the dependency tree. It existed only
  for the background writer that the first change above removed.
- The in-progress marker is in `.gitignore`, for anyone whose data folder
  lives inside a checkout.
- The `SECURITY.md` section on outbound connections is deliberately
  incomplete. The Java project this was forked from carried a full data
  transparency document — every destination, what it is for, when it is
  contacted, how to turn it off — and copying that across unchanged would
  describe features the Rust port does not have. Until it is rewritten,
  the code is the reference.

Celeste is still a vibe-coded project, and it is still looking for Rust
developers willing to improve it and for testers willing to run it.
