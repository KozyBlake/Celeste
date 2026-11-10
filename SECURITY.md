# Security and contact

Celeste is a beta project maintained by one person. This page says how to
reach that person, which is the part that matters most if you operate a
service Celeste talks to.

## Reporting a vulnerability

Please do not open a public issue for a vulnerability.

- **Email:** chieriontop@chierion.top
- **GitHub:** use *Report a vulnerability* on the
  [Security tab](https://github.com/KozyBlake/Celeste/security), which
  opens a private advisory
- **Discord:** `@chieriontop` (user ID `656328060701900836`, which
  resolves as `<@656328060701900836>` in any server)

Email is the route that needs no account anywhere. Expect a reply within a
few days; this is not a staffed project and there is no SLA.

Useful in a report: the Celeste version, the platform, what an attacker
would need in order to reach the issue, and what they would get. Working
proof-of-concept code is welcome but not required.

Only the most recent release receives fixes. Celeste is pre-1.0 and there
are no maintained release branches.

## If you run a service Celeste queries

Celeste makes requests to the VRChat API, to Discord, to GitHub for its
update check, and to community avatar databases. If you operate any of
those and want Celeste throttled, identified differently, or stopped, the
contacts above reach the maintainer directly. **You do not need to open a
public issue, and you do not need to join a Discord server.** A note by
email is enough, and a request to back off will be honoured rather than
argued with.

Every request Celeste makes carries a `User-Agent` of this shape:

```
KozyBlake-Celeste/<version> (+https://github.com/KozyBlake/Celeste; mailto:chieriontop@chierion.top; Discord:@chieriontop/656328060701900836; https://discord.gg/shXBVdppW)
```

If you match on it for an allowlist or a rate-limit rule, match the
product name `KozyBlake-Celeste` and not the whole string: the version
changes with every release, and a rule built on the full string will break
at the next one.

Older builds in the wild send earlier strings — `Celeste/<version>` to
avatar databases, and Scarlet-era strings from before this project was
renamed. Celeste does not auto-update, so those taper off as people
install newer builds rather than stopping on a release date.

Avatar-database requests specifically are name lookups only — there is no
reverse-image or by-picture search. They are made when a moderator runs a
command, are cached for the life of the process, and a provider that fails
or rate-limits is backed off with escalating delays before any retry.
Celeste does not crawl or enumerate.

## Still to be written

The Java project this was forked from carried a fuller *Security and Data
Transparency* document: a table of every outbound destination, what it is
for, when it is contacted and how to turn it off, plus sections on
credential storage, process execution and SSRF protection. That has not
been ported to Celeste yet, and copying it across unchanged would describe
features the Rust port does not have. Until it is written, the honest
summary is above and the code is the reference.
