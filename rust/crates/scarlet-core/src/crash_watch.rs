//! Decides whether a finished VRChat session crashed, and remembers who
//! was last loading an avatar when it did.
//!
//! ## Why absence of a quit line is the signal
//!
//! VRChat writes `VRCApplication: HandleApplicationQuit at <seconds>` when
//! it shuts down in an orderly way. A crash never reaches that line: the
//! log simply stops. The Unity crash dialog is *not* reliable — it does
//! not always appear — so the only dependable evidence is the missing
//! quit line, not the presence of a crash marker.
//!
//! This is also why force-terminating a frozen client still counts as a
//! crash, which matches what actually happened: the game hung, and killing
//! the hung process is how the person got out of it.
//!
//! ## The false positives this is built to avoid
//!
//! A wrong "so-and-so crashed the instance" is worse than silence; it
//! points a moderator at a named person. Three guards:
//!
//! 1. **A clean quit is never a crash.** Closing the game writes the quit
//!    line, so [`CrashWatch::saw_clean_quit`] disarms the whole thing.
//! 2. **Only sessions Celeste actually watched live.** Starting Celeste
//!    after the fact, and reading an old log that happens to end
//!    mid-stream, reports nothing — it never saw that session run.
//! 3. **Only when there is something to name.** No avatar load observed
//!    means no accusation; the crash is still reported, with no suspect.

use chrono::NaiveDateTime;

/// The avatar load Celeste saw most recently before a session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvatarLoad {
    pub display_name: String,
    pub user_id: Option<String>,
    pub avatar_name: Option<String>,
    pub at: NaiveDateTime,
}

/// What a crash report can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    /// The last avatar load before the log stopped, if any was seen.
    pub last_avatar_load: Option<AvatarLoad>,
    /// Instance the client was in when it died, if known.
    pub location: Option<String>,
}

/// Per-session state behind the decision.
#[derive(Debug, Default, Clone)]
pub struct CrashWatch {
    saw_clean_quit: bool,
    watched_live: bool,
    last_avatar_load: Option<AvatarLoad>,
    location: Option<String>,
}

impl CrashWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// The tailer caught up with this log, so anything after this point is
    /// a session Celeste is watching as it happens.
    pub fn went_live(&mut self) {
        self.watched_live = true;
    }

    /// `VRCApplication: HandleApplicationQuit` — an orderly shutdown.
    pub fn saw_clean_quit(&mut self) {
        self.saw_clean_quit = true;
    }

    pub fn set_location(&mut self, location: Option<String>) {
        self.location = location;
    }

    /// Record an avatar finishing its load. Only live events count: a
    /// replay of an old log must not supply the suspect for a crash.
    pub fn record_avatar_load(&mut self, preamble: bool, load: AvatarLoad) {
        if preamble {
            return;
        }
        self.last_avatar_load = Some(load);
    }

    /// Called when this session's log is replaced by a new one, i.e. the
    /// session is over. `Some` means it ended badly.
    pub fn finish(&self) -> Option<CrashReport> {
        if self.saw_clean_quit || !self.watched_live {
            return None;
        }
        Some(CrashReport {
            last_avatar_load: self.last_avatar_load.clone(),
            location: self.location.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 4).unwrap().and_hms_opt(12, 48, 0).unwrap()
    }

    fn load(name: &str) -> AvatarLoad {
        AvatarLoad {
            display_name: name.to_string(),
            user_id: Some("usr_1".into()),
            avatar_name: Some("Some Avatar".into()),
            at: at(),
        }
    }

    /// The rule the owner set: closing the game is not a crash.
    #[test]
    fn a_clean_quit_is_never_reported() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        watch.record_avatar_load(false, load("Someone"));
        watch.saw_clean_quit();
        assert_eq!(watch.finish(), None);
    }

    /// A freeze reaches no quit line, so the session ends without one.
    /// Force-killing a hung client lands here too, correctly: the game had
    /// already stopped working.
    #[test]
    fn a_session_that_never_quit_cleanly_is_a_crash() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        watch.set_location(Some("wrld_1:42".into()));
        watch.record_avatar_load(false, load("Last Loader"));
        let report = watch.finish().expect("no quit line means a crash");
        assert_eq!(report.last_avatar_load.unwrap().display_name, "Last Loader");
        assert_eq!(report.location.as_deref(), Some("wrld_1:42"));
    }

    /// Starting Celeste and finding an old half-written log must not
    /// produce an accusation about a session it never saw.
    #[test]
    fn a_session_never_watched_live_is_not_reported() {
        let mut watch = CrashWatch::new();
        watch.record_avatar_load(true, load("Replayed"));
        assert_eq!(watch.finish(), None);
    }

    /// Replayed history must not supply the suspect for a later crash.
    #[test]
    fn preamble_avatar_loads_are_not_remembered() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        watch.record_avatar_load(true, load("From The Replay"));
        let report = watch.finish().expect("still a crash");
        assert!(report.last_avatar_load.is_none(), "nobody should be named from replayed history");
    }

    /// A crash with nothing observed still reports the crash, naming
    /// no one, rather than staying silent or inventing a suspect.
    #[test]
    fn a_crash_with_no_avatar_load_names_nobody() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        let report = watch.finish().expect("a crash is still a crash");
        assert!(report.last_avatar_load.is_none());
    }

    #[test]
    fn the_most_recent_load_wins() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        watch.record_avatar_load(false, load("First"));
        watch.record_avatar_load(false, load("Second"));
        assert_eq!(watch.finish().unwrap().last_avatar_load.unwrap().display_name, "Second");
    }
}
