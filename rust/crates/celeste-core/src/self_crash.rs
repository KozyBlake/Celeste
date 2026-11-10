//! Noticing that Celeste's *own* previous run died, and writing down what
//! it had just seen.
//!
//! [`crash_watch`](crate::crash_watch) answers the neighbouring question —
//! did the *VRChat client* crash, and who was loading an avatar when it
//! did. It cannot answer this one. Its state lives in memory inside the
//! event listener, so when Celeste is the process that dies, the suspect
//! goes with it and nothing is ever written.
//!
//! A GPU reset is how that happens in practice. The graphics driver calls
//! `abort()`, the kernel ends the process, and no Celeste code runs again
//! — no panic hook, no unwind, no shutdown. So the only place this can be
//! noticed is the *next* run, and the only evidence that survives is what
//! was already on disk when the process stopped existing.
//!
//! Which is why this reads the previous run's log rather than keeping a
//! state file of its own. The log is written synchronously (see
//! [`log::init`](crate::log::init)), so every line Celeste had emitted is
//! on disk by the time it dies. A separate snapshot would be a second
//! thing to keep in sync, written on a hot path, to hold information the
//! log already holds.
//!
//! ## What this deliberately does not claim
//!
//! The report names the avatars that were loading when Celeste stopped. It
//! does not say they caused it. Avatar loads in a busy instance happen
//! continuously, so the last one before a crash is a coincidence at least
//! as often as a cause, and a genuine crasher may never appear — Celeste
//! only sees what VRChat's log and API told it, and an avatar that kills
//! the GPU mid-load can die before it is ever named. `crash_watch`'s
//! module docs make the same argument at greater length: a wrong
//! "so-and-so crashed it" is worse than silence, because it points a
//! moderator at a named person. The wording in [`render`] is written to
//! keep that distinction, and nothing here is wired to anything that
//! bans.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How far back from the last logged moment to collect avatar sightings.
///
/// Public because [`Findings::sightings`] is documented in terms of it, and
/// because a caller rendering its own summary needs to say what span it is
/// describing rather than guessing.
pub const SUSPECT_WINDOW_SECONDS: i64 = 60;

/// How many of the previous log's final lines to copy into the report.
///
/// Enough to show what the process was doing, short enough that the
/// report stays something a person will actually read. The full log is
/// named in the report for anyone who wants the rest.
const TAIL_LINES: usize = 40;

/// Errors worth surfacing above the raw tail, from this far back.
const ERROR_WINDOW_SECONDS: i64 = 120;

/// The file that marks a run as in progress.
///
/// Its presence at startup means a previous run never reached its
/// shutdown. Deliberately not inside `logs/`, which is pruned.
const MARKER_NAME: &str = "running-run.json";

/// What the running process records about itself, for the next run to read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMarker {
    /// The log this run is writing to.
    pub log_file: PathBuf,
    /// Celeste's version, so a report says which build died.
    pub version: String,
    pub started_at: DateTime<Utc>,
    pub pid: u32,
}

pub fn marker_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MARKER_NAME)
}

/// Records this run as in progress, replacing any marker already there.
///
/// Call after the log is open. Failure is returned rather than logged so
/// the caller decides, but it is never a reason not to start: a missing
/// marker costs a crash report, not a run.
pub fn mark_running(data_dir: &Path, marker: &RunMarker) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(marker)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    std::fs::write(marker_path(data_dir), json)
}

/// Clears the marker, declaring this run to have ended on purpose.
///
/// A missing marker is success: this runs on the orderly-exit path, which
/// can be reached more than once, and "already gone" is the desired state
/// either way.
pub fn mark_exited(data_dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(marker_path(data_dir)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Reads a marker left behind by a run that did not clear it.
///
/// `None` covers both "nothing there" (the normal case) and a marker too
/// damaged to read — a crash during the write that created it, say. An
/// unreadable marker is not worth a diagnostic of its own; it just means
/// there is no report to write.
pub fn take_stale_marker(data_dir: &Path) -> Option<RunMarker> {
    let path = marker_path(data_dir);
    let raw = std::fs::read(&path).ok()?;
    let marker = serde_json::from_slice::<RunMarker>(&raw).ok();
    // Remove it either way. A marker that cannot be parsed would otherwise
    // be retried on every launch forever.
    let _ = std::fs::remove_file(&path);
    marker
}

/// One `<name>'s chosen avatar <avatar> is <rating>` line from the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarSighting {
    pub at: DateTime<Utc>,
    pub display_name: String,
    pub avatar_name: String,
    pub rating: String,
}

/// A log line split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LogLine<'a> {
    at: DateTime<Utc>,
    level: &'a str,
    /// Everything after the level: the target and the message.
    rest: &'a str,
}

/// Splits `<rfc3339> <LEVEL> <target>: <message>`.
///
/// `None` for a line that does not start with a timestamp, which includes
/// every continuation line of a multi-line message and anything the
/// graphics driver wrote straight to the terminal.
fn parse_line(raw: &str) -> Option<LogLine<'_>> {
    let (stamp, after) = raw.split_once(' ')?;
    let at = DateTime::parse_from_rfc3339(stamp).ok()?.with_timezone(&Utc);
    // `tracing` pads the level to five characters, so the gap between the
    // timestamp and the level is one space for `ERROR` and two for ` INFO`.
    let after = after.trim_start();
    let (level, rest) = after.split_once(' ')?;
    Some(LogLine { at, level, rest: rest.trim_start() })
}

/// The opening of the line [`crash_signal`](crate::crash_signal) writes.
/// Matched as a literal because that handler builds it from constants and
/// cannot be asked to do anything cleverer while the process is dying.
const FATAL_PREFIX: &str = "*** FATAL: Celeste was killed by signal ";
const FATAL_MIDDLE: &str = " at unix time ";

/// Reads the handler's line back. It carries no timestamp of its own in
/// the log's format, so [`parse_line`] skips it and this runs separately.
fn parse_killed_by(raw: &str) -> Option<KilledBy> {
    let rest = raw.trim_start().strip_prefix(FATAL_PREFIX)?;
    let (signal, rest) = rest.split_once(FATAL_MIDDLE)?;
    let at_unix = rest.split_whitespace().next()?;
    Some(KilledBy {
        signal: signal.trim().parse().ok()?,
        at_unix: at_unix.trim().parse().ok()?,
    })
}

const SIGHTING_TARGET: &str = "celeste_events::listener: ";
const SIGHTING_MIDDLE: &str = "'s chosen avatar ";

/// Pulls a sighting out of a listener line, if that is what it is.
///
/// The display name is taken up to the *first* `'s chosen avatar `, and the
/// rating from the *last* ` is `: a display name can contain almost
/// anything and an avatar name can contain the word "is", so each end is
/// anchored at the end a VRChat name cannot reach past.
fn parse_sighting(line: &LogLine<'_>) -> Option<AvatarSighting> {
    let message = line.rest.strip_prefix(SIGHTING_TARGET)?;
    let (display_name, remainder) = message.split_once(SIGHTING_MIDDLE)?;
    let (avatar_name, rating) = remainder.rsplit_once(" is ")?;
    if display_name.is_empty() || avatar_name.is_empty() || rating.is_empty() {
        return None;
    }
    Some(AvatarSighting {
        at: line.at,
        display_name: display_name.to_string(),
        avatar_name: avatar_name.to_string(),
        rating: rating.to_string(),
    })
}

/// What the crash handler managed to record on its way out.
///
/// Present only when the process was killed by a signal *and* the handler
/// was armed. Absent covers a clean kill the handler never saw, a build
/// without it, and a machine that lost power -- all of which mean the same
/// thing to a reader: no account of the ending beyond where the log stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KilledBy {
    pub signal: i64,
    /// Unix seconds. The handler may not call a date formatter, so it
    /// writes the raw value and this side turns it into a date.
    pub at_unix: i64,
}

impl KilledBy {
    /// `signal 6 (SIGABRT)`, or just `signal 64` for one with no name.
    pub fn describe(&self) -> String {
        match crate::crash_signal::signal_name(self.signal) {
            Some(name) => format!("signal {} ({name})", self.signal),
            None => format!("signal {}", self.signal),
        }
    }
}

/// Everything the previous log could tell us about how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Findings {
    /// The last line that carried a timestamp.
    pub last_logged: Option<DateTime<Utc>>,
    /// Avatar sightings within [`SUSPECT_WINDOW_SECONDS`] of the end,
    /// oldest first.
    pub sightings: Vec<AvatarSighting>,
    /// `WARN`/`ERROR` lines near the end, oldest first, as written.
    pub errors: Vec<String>,
    /// The final lines of the log, as written.
    pub tail: Vec<String>,
    /// What the crash handler recorded, if it ran.
    pub killed_by: Option<KilledBy>,
}

/// Reads a previous run's log into [`Findings`].
///
/// Takes the whole text because a Celeste log is a couple of megabytes at
/// most — the writer prunes to ten files — and seeking backwards through a
/// UTF-8 file to find a line boundary costs more than it saves at that
/// size.
pub fn examine(log: &str) -> Findings {
    let lines: Vec<&str> = log.lines().collect();
    let last_logged = lines.iter().rev().filter_map(|raw| parse_line(raw)).map(|line| line.at).next();

    let mut findings = Findings {
        last_logged,
        tail: lines
            .iter()
            .rev()
            .take(TAIL_LINES)
            .rev()
            .map(|raw| (*raw).to_string())
            .collect(),
        ..Default::default()
    };

    let Some(end) = last_logged else {
        return findings;
    };

    for raw in &lines {
        if let Some(killed_by) = parse_killed_by(raw) {
            findings.killed_by = Some(killed_by);
        }
        let Some(line) = parse_line(raw) else { continue };
        let age = (end - line.at).num_seconds();
        if age <= SUSPECT_WINDOW_SECONDS {
            if let Some(sighting) = parse_sighting(&line) {
                findings.sightings.push(sighting);
            }
        }
        if age <= ERROR_WINDOW_SECONDS && matches!(line.level, "ERROR" | "WARN") {
            findings.errors.push((*raw).to_string());
        }
    }
    findings
}

/// `26h 19m`, or `4m 12s` for a short run. Empty when the end precedes the
/// start, which a clock change can produce and which is not worth a word.
fn describe_duration(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let seconds = (end - start).num_seconds();
    if seconds < 0 {
        return String::new();
    }
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// The report, as the text file a person opens.
pub fn render(marker: &RunMarker, findings: &Findings) -> String {
    let mut out = String::new();
    out.push_str("Celeste crash report\n====================\n\n");
    out.push_str(&format!(
        "Celeste {} did not shut down cleanly.\n\n",
        marker.version
    ));

    out.push_str(&format!(
        "Run started   {}\n",
        marker.started_at.format("%Y-%m-%d %H:%M:%S UTC")
    ));
    match findings.last_logged {
        Some(end) => {
            let ran_for = describe_duration(marker.started_at, end);
            let ran_for = if ran_for.is_empty() {
                String::new()
            } else {
                format!("   (ran for {ran_for})")
            };
            out.push_str(&format!(
                "Last logged   {}{ran_for}\n",
                end.format("%Y-%m-%d %H:%M:%S UTC")
            ));
        }
        None => out.push_str("Last logged   nothing with a timestamp\n"),
    }
    match findings.killed_by {
        Some(killed) => {
            let at = DateTime::from_timestamp(killed.at_unix, 0)
                .map(|at| at.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| format!("unix time {}", killed.at_unix));
            out.push_str(&format!("Ended         killed by {} at {at}\n", killed.describe()));
        }
        None => out.push_str(
            "Ended         no record; it did not reach its shutdown\n",
        ),
    }
    out.push_str(&format!("Process       pid {}\n", marker.pid));
    out.push_str(&format!("Log file      {}\n\n", marker.log_file.display()));

    out.push_str(
        "This was written by the following run. The previous one ended without\n\
         reaching its shutdown, so it could not finish reporting on itself. A\n\
         graphics driver aborting after a GPU reset is the usual reason, but\n\
         any outright kill looks the same from here -- the line above says\n\
         which signal it was, when the crash handler caught it.\n\n",
    );

    let heading = format!("Avatars loading in the last {SUSPECT_WINDOW_SECONDS} seconds");
    out.push_str(&format!("{heading}\n{}\n", "-".repeat(heading.len())));
    if findings.sightings.is_empty() {
        out.push_str("  Celeste had not seen an avatar load in that time.\n");
    } else {
        for sighting in &findings.sightings {
            out.push_str(&format!(
                "  {}  {}  wearing \"{}\" ({})\n",
                sighting.at.format("%H:%M:%S"),
                sighting.display_name,
                sighting.avatar_name,
                sighting.rating,
            ));
        }
    }
    out.push_str(
        "\n  These are the avatars Celeste happened to be looking at when it\n\
         \x20 stopped. They are not a cause. In a busy instance avatar loads are\n\
         \x20 continuous, so the last one before a crash is a coincidence at\n\
         \x20 least as often as a culprit, and a real crasher may not be listed\n\
         \x20 here at all -- Celeste only sees what VRChat told it, and an avatar\n\
         \x20 that kills the GPU can do it before it is ever named. Somewhere to\n\
         \x20 start looking; not grounds for a ban.\n\n",
    );

    if !findings.errors.is_empty() {
        out.push_str("Warnings and errors beforehand\n------------------------------\n");
        for error in &findings.errors {
            out.push_str(&format!("  {error}\n"));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "Final {} lines of the log\n-------------------------\n",
        findings.tail.len()
    ));
    for line in &findings.tail {
        out.push_str(&format!("  {line}\n"));
    }
    if findings.tail.is_empty() {
        out.push_str("  The log was empty.\n");
    }
    out.push_str(
        "\nThe graphics driver writes to the terminal rather than to this log, so\n\
         a message like \"amdgpu: The CS has cancelled because the context is\n\
         lost\" will not appear above even when it is the reason. Check the\n\
         terminal Celeste was started from, and `coredumpctl list` on Linux.\n",
    );
    out
}

/// Where a report for a run that started at `started_at` is written.
pub fn report_path(data_dir: &Path, started_at: DateTime<Utc>) -> PathBuf {
    data_dir
        .join("logs")
        .join(format!("celeste_crash_{}.txt", started_at.format("%Y-%m-%d_%H-%M-%S")))
}

/// Writes a report for a stale marker, returning where it went.
///
/// The previous log being unreadable is not fatal: the marker alone still
/// establishes that a run died and when it started, which is worth
/// recording even with nothing to say about the end.
pub fn write_report(data_dir: &Path, marker: &RunMarker) -> std::io::Result<PathBuf> {
    let log = std::fs::read_to_string(&marker.log_file).unwrap_or_default();
    let findings = examine(&log);
    let path = report_path(data_dir, marker.started_at);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, render(marker, &findings))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    fn marker() -> RunMarker {
        RunMarker {
            log_file: PathBuf::from("logs/celeste_log_2026-11-08_22-10-37.txt"),
            version: "0.30.0".to_string(),
            started_at: stamp("2026-11-08T22:10:37Z"),
            pid: 848384,
        }
    }

    /// The real shape of the lines this has to read, copied from the log of
    /// the crash that prompted it: an Xwayland abort taking Celeste with it.
    const REAL_TAIL: &str = "\
2026-11-10T00:28:59.122587Z  INFO celeste_events::listener: VØłĐɆ₭Ɇ₭'s chosen avatar Yasuo is VeryPoor
2026-11-10T00:29:01.756121Z  INFO celeste_events::listener: joazera17's chosen avatar Robot is Excellent
2026-11-10T00:29:11.768235Z  INFO celeste_events::listener: Dαρԋɳҽ's chosen avatar Asphyxia is Unknown
2026-11-10T00:29:37.807454Z DEBUG celeste_vrchat::rate_limit: Asked VRChat: GET /api/1/analysis/file_bc08 — 200 OK in 180ms
2026-11-10T00:29:45.621524Z ERROR arboard::platform::linux::x11: Worker thread errored with: Unknown error while interacting with the clipboard: The X11 server closed the connection";

    #[test]
    fn reads_the_real_lines_the_crash_left_behind() {
        let findings = examine(REAL_TAIL);
        assert_eq!(findings.last_logged, Some(stamp("2026-11-10T00:29:45.621524Z")));
        let names: Vec<&str> =
            findings.sightings.iter().map(|s| s.display_name.as_str()).collect();
        assert_eq!(names, ["VØłĐɆ₭Ɇ₭", "joazera17", "Dαρԋɳҽ"]);
        assert_eq!(findings.sightings[2].avatar_name, "Asphyxia");
        assert_eq!(findings.sightings[2].rating, "Unknown");
        assert_eq!(findings.errors.len(), 1, "the X11 error should be surfaced");
        assert!(findings.errors[0].contains("X11 server closed the connection"));
    }

    /// Non-ASCII display names are the norm in VRChat, and byte-slicing a
    /// name like `Dαρԋɳҽ` would panic rather than mis-parse.
    #[test]
    fn handles_multibyte_display_names() {
        let findings = examine(
            "2026-11-10T00:29:11.768235Z  INFO celeste_events::listener: Nani娜妮なん's chosen avatar ふわふわ is Good",
        );
        assert_eq!(findings.sightings.len(), 1);
        assert_eq!(findings.sightings[0].display_name, "Nani娜妮なん");
        assert_eq!(findings.sightings[0].avatar_name, "ふわふわ");
    }

    /// An avatar called "this is fine" must not have its name cut at the
    /// first " is ", and a name with an apostrophe must not confuse the
    /// front anchor.
    #[test]
    fn splits_at_the_outer_anchors() {
        let findings = examine(
            "2026-11-10T00:29:11.000000Z  INFO celeste_events::listener: O'Brien's chosen avatar this is fine is Poor",
        );
        assert_eq!(findings.sightings.len(), 1);
        assert_eq!(findings.sightings[0].display_name, "O'Brien");
        assert_eq!(findings.sightings[0].avatar_name, "this is fine");
        assert_eq!(findings.sightings[0].rating, "Poor");
    }

    /// Sightings from earlier in a 26-hour run are not suspects.
    #[test]
    fn only_the_last_minute_counts() {
        let log = "\
2026-11-09T04:00:00.000000Z  INFO celeste_events::listener: Ancient's chosen avatar Old is Good
2026-11-10T00:29:11.000000Z  INFO celeste_events::listener: Recent's chosen avatar New is Good
2026-11-10T00:29:45.000000Z ERROR arboard: the display went away";
        let findings = examine(log);
        let names: Vec<&str> =
            findings.sightings.iter().map(|s| s.display_name.as_str()).collect();
        assert_eq!(names, ["Recent"]);
    }

    /// Lines the graphics driver wrote, and continuation lines, carry no
    /// timestamp and must be skipped rather than derailing the parse.
    #[test]
    fn untimestamped_lines_are_ignored() {
        let log = "\
amdgpu: The CS has cancelled because the context is lost. This context is innocent.
2026-11-10T00:29:11.000000Z  INFO celeste_events::listener: Someone's chosen avatar A is Good
not a log line at all";
        let findings = examine(log);
        assert_eq!(findings.sightings.len(), 1);
        assert_eq!(findings.last_logged, Some(stamp("2026-11-10T00:29:11Z")));
    }

    #[test]
    fn reads_back_what_the_crash_handler_wrote() {
        let log = format!(
            "2026-11-10T01:41:58.675445Z DEBUG celeste: last ordinary line\n{}6{}1794275743 -- written by the crash handler; no further Celeste output follows.\n",
            FATAL_PREFIX, FATAL_MIDDLE
        );
        let findings = examine(&log);
        let killed = findings.killed_by.expect("the handler's line should be read back");
        assert_eq!(killed.signal, 6);
        assert_eq!(killed.at_unix, 1_794_275_743);
        assert_eq!(killed.describe(), "signal 6 (SIGABRT)");

        let text = render(&marker(), &findings);
        assert!(text.contains("killed by signal 6 (SIGABRT)"), "{text}");
        assert!(text.contains("2026-11-10 01:55:43 UTC"), "{text}");
    }

    /// Without the handler's line the report must not invent a cause --
    /// power loss and a `kill -9` reach here too.
    #[test]
    fn says_nothing_about_a_cause_it_does_not_know() {
        let findings = examine("2026-11-10T01:41:58.675445Z DEBUG celeste: and then nothing");
        assert_eq!(findings.killed_by, None);
        let text = render(&marker(), &findings);
        assert!(text.contains("no record; it did not reach its shutdown"), "{text}");
        assert!(!text.contains("killed by signal"), "{text}");
    }

    #[test]
    fn an_empty_log_still_reports() {
        let findings = examine("");
        assert_eq!(findings.last_logged, None);
        assert!(findings.sightings.is_empty());
        let text = render(&marker(), &findings);
        assert!(text.contains("did not shut down cleanly"));
        assert!(text.contains("The log was empty."));
    }

    #[test]
    fn the_report_names_the_suspects_and_refuses_to_accuse_them() {
        let text = render(&marker(), &examine(REAL_TAIL));
        assert!(text.contains("Dαρԋɳҽ"), "{text}");
        assert!(text.contains("Asphyxia"), "{text}");
        assert!(text.contains("0.30.0"));
        assert!(text.contains("ran for 26h 19m"), "{text}");
        assert!(text.contains("not grounds for a ban"), "{text}");
        assert!(text.contains("They are not a cause."), "{text}");
    }

    #[test]
    fn marker_round_trips_and_is_taken_once() {
        let dir = tempfile::tempdir().unwrap();
        assert!(take_stale_marker(dir.path()).is_none(), "nothing there yet");

        mark_running(dir.path(), &marker()).unwrap();
        assert_eq!(take_stale_marker(dir.path()), Some(marker()));
        assert!(
            take_stale_marker(dir.path()).is_none(),
            "taking it must consume it, or every later launch reports the same crash"
        );
    }

    #[test]
    fn a_clean_exit_leaves_nothing_to_report() {
        let dir = tempfile::tempdir().unwrap();
        mark_running(dir.path(), &marker()).unwrap();
        mark_exited(dir.path()).unwrap();
        assert!(take_stale_marker(dir.path()).is_none());
        // The orderly-exit path can be reached twice; the second must not error.
        mark_exited(dir.path()).unwrap();
    }

    #[test]
    fn an_unreadable_marker_is_consumed_rather_than_retried_forever() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(marker_path(dir.path()), b"{ truncated").unwrap();
        assert!(take_stale_marker(dir.path()).is_none());
        assert!(!marker_path(dir.path()).exists());
    }

    #[test]
    fn write_report_handles_a_log_that_is_no_longer_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_report(dir.path(), &marker()).unwrap();
        assert!(path.exists());
        assert!(std::fs::read_to_string(&path).unwrap().contains("did not shut down cleanly"));
    }

    /// The report must not be named like a log, or the log pruner would
    /// count it towards the ten it keeps and eventually delete it.
    #[test]
    fn the_report_is_not_named_like_a_log() {
        let path = report_path(Path::new("/data"), stamp("2026-11-08T22:10:37Z"));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("celeste_crash_"), "{name}");
        assert!(!name.starts_with("celeste_log_"), "{name}");
        assert!(!name.starts_with("scarlet_log_"), "{name}");
    }
}
