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
    /// Who uploaded the avatar, from the asset-bundle unpack line. The
    /// wearer and the author are usually different people, and for a
    /// report it is the author that matters.
    pub avatar_author: Option<String>,
    /// The asset bundle VRChat was unpacking, when it is known. This is
    /// the identifier that survives VRChat no longer exposing avatar ids.
    pub bundle_file_id: Option<String>,
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
    /// Already reported for this session, so the log rolling over later
    /// does not announce the same crash a second time.
    reported: bool,
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

    /// Attach the author from an unpack line to the load already
    /// recorded, when they describe the same avatar.
    ///
    /// The unpack arrives a beat after the switch, so this is a late
    /// enrichment rather than a separate event. Matching on the avatar
    /// name keeps an unrelated avatar unpacking in the background from
    /// attributing itself to the wrong person.
    pub fn record_avatar_author(&mut self, preamble: bool, avatar_name: &str, author: Option<&str>) {
        if preamble {
            return;
        }
        let Some(load) = self.last_avatar_load.as_mut() else { return };
        if load.avatar_name.as_deref() == Some(avatar_name) {
            load.avatar_author = author.map(str::to_string);
        }
    }

    /// Attach the asset bundle id once the analysis lookup identifies it.
    pub fn record_bundle_file_id(&mut self, display_name: &str, file_id: &str) {
        let Some(load) = self.last_avatar_load.as_mut() else { return };
        if load.display_name == display_name {
            load.bundle_file_id = Some(file_id.to_string());
        }
    }

    /// The client started writing again after a stall, so it was hung
    /// rather than dead and the report must be withdrawn.
    pub fn resumed(&mut self) -> bool {
        std::mem::take(&mut self.reported)
    }

    /// Why [`finish`] will decide the way it does, in words.
    ///
    /// Crash detection is a judgement about a named person, so the
    /// reasoning has to be inspectable in the log rather than inferred
    /// from whether a window appeared.
    ///
    /// [`finish`]: CrashWatch::finish
    pub fn summary(&self) -> String {
        if self.reported {
            return "not reported again: already reported for this session".to_string();
        }
        if !self.watched_live {
            return "not reported: Celeste never watched this session live".to_string();
        }
        if self.saw_clean_quit {
            return "not reported: VRChat wrote its clean-exit marker".to_string();
        }
        match &self.last_avatar_load {
            Some(load) => format!(
                "crash: no clean-exit marker; last avatar load was {} at {}",
                load.display_name, load.at
            ),
            None => "crash: no clean-exit marker, and no avatar load was seen".to_string(),
        }
    }

    /// The log stopped growing, or was replaced by a new one: either way
    /// this session is over. `Some` means it ended badly.
    ///
    /// Reports at most once per session. The live stall fires first and
    /// the log rotating when the person relaunches would otherwise
    /// announce the same crash again.
    pub fn finish(&mut self) -> Option<CrashReport> {
        if self.saw_clean_quit || !self.watched_live || self.reported {
            return None;
        }
        self.reported = true;
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
            avatar_author: None,
            bundle_file_id: None,
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

#[cfg(test)]
mod enrichment_tests {
    use super::*;
    use chrono::NaiveDate;

    fn watch_with_load() -> CrashWatch {
        let mut watch = CrashWatch::new();
        watch.went_live();
        watch.record_avatar_load(
            false,
            AvatarLoad {
                display_name: "Wearer".into(),
                user_id: Some("usr_1".into()),
                avatar_name: Some("Crasher Avatar".into()),
                avatar_author: None,
                bundle_file_id: None,
                at: NaiveDate::from_ymd_opt(2026, 10, 4).unwrap().and_hms_opt(12, 47, 56).unwrap(),
            },
        );
        watch
    }

    #[test]
    fn the_author_is_attached_when_the_avatar_matches() {
        let mut watch = watch_with_load();
        watch.record_avatar_author(false, "Crasher Avatar", Some("Uploader"));
        let load = watch.finish().unwrap().last_avatar_load.unwrap();
        assert_eq!(load.avatar_author.as_deref(), Some("Uploader"));
    }

    /// Another avatar finishing its download in the background must not
    /// rewrite the suspect's author.
    #[test]
    fn an_unrelated_unpack_does_not_attach_itself() {
        let mut watch = watch_with_load();
        watch.record_avatar_author(false, "Someone Elses Avatar", Some("Wrong Person"));
        let load = watch.finish().unwrap().last_avatar_load.unwrap();
        assert_eq!(load.avatar_author, None);
    }

    #[test]
    fn the_bundle_id_is_attached_for_the_matching_wearer() {
        let mut watch = watch_with_load();
        watch.record_bundle_file_id("Someone Else", "file_wrong");
        watch.record_bundle_file_id("Wearer", "file_right");
        let load = watch.finish().unwrap().last_avatar_load.unwrap();
        assert_eq!(load.bundle_file_id.as_deref(), Some("file_right"));
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;

    #[test]
    fn each_outcome_explains_itself() {
        let mut fresh = CrashWatch::new();
        assert!(fresh.summary().contains("never watched this session live"));

        let mut clean = CrashWatch::new();
        clean.went_live();
        clean.saw_clean_quit();
        assert!(clean.summary().contains("clean-exit marker"));

        let mut crashed = CrashWatch::new();
        crashed.went_live();
        assert!(crashed.summary().starts_with("crash:"));
    }

    /// The summary must agree with the decision, or the log explains one
    /// thing while the window shows another.
    #[test]
    fn the_summary_never_contradicts_the_verdict() {
        for (watched, clean) in [(false, false), (false, true), (true, false), (true, true)] {
            let mut watch = CrashWatch::new();
            if watched {
                watch.went_live();
            }
            if clean {
                watch.saw_clean_quit();
            }
            let says_crash = watch.summary().starts_with("crash:");
            assert_eq!(says_crash, watch.finish().is_some(), "watched={watched} clean={clean}");
        }
    }
}

#[cfg(test)]
mod stall_tests {
    use super::*;

    /// The stall fires first and the log rotates when the person
    /// relaunches. Without the once-per-session guard that is two popups
    /// for one crash, and the second arrives minutes later looking like a
    /// second incident.
    #[test]
    fn a_crash_is_reported_once_even_though_both_paths_fire() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        assert!(watch.finish().is_some(), "the stall reports it");
        assert!(watch.finish().is_none(), "the rotation must not report it again");
        assert!(watch.summary().contains("already reported"));
    }

    /// "Wait Longer" rather than "Terminate": the client was hung, not
    /// dead. A report left standing would accuse someone over a hiccup.
    #[test]
    fn recovering_withdraws_the_report_and_rearms() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        assert!(watch.finish().is_some());
        assert!(watch.resumed(), "there was a report to withdraw");

        // Armed again: if it really dies later, that is a fresh crash.
        assert!(watch.finish().is_some());
    }

    #[test]
    fn recovering_without_a_report_withdraws_nothing() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        assert!(!watch.resumed(), "nothing was reported, so nothing to take back");
    }

    /// A stall after a clean quit is VRChat being closed, not a crash.
    #[test]
    fn a_stall_after_a_clean_quit_is_still_not_a_crash() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        watch.saw_clean_quit();
        assert!(watch.finish().is_none());
    }
}
