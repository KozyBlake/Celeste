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

/// How deep into the watched lists a user sits.
///
/// Moderators do not judge a suspect on one watched group; they look at
/// how *many* they are in and of what kind. Someone in a single community
/// group is nobody; someone in five malicious ones is the sort of account
/// that turns up to break instances. That judgement is the fallback now
/// that VRChat no longer lets anyone pull an avatar and test it, so the
/// crash report should carry it rather than making someone go and look it
/// up at the moment they are least able to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WatchedGroupProfile {
    pub total: usize,
    /// Counts per watched-group type, largest first; the type strings are
    /// the Java enum names (MALICIOUS, NUISANCE, ...).
    pub by_type: Vec<(String, usize)>,
}

impl WatchedGroupProfile {
    /// Builds the profile from the type of each watched group the user is
    /// in. Ties break alphabetically so the summary is stable between
    /// runs rather than reshuffling on every lookup.
    pub fn from_types<'a>(types: impl IntoIterator<Item = &'a str>) -> Self {
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        let mut total = 0;
        for kind in types {
            *counts.entry(kind.to_string()).or_default() += 1;
            total += 1;
        }
        let mut by_type: Vec<(String, usize)> = counts.into_iter().collect();
        by_type.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        WatchedGroupProfile { total, by_type }
    }

    /// One line for a report: "5 watched groups (3 MALICIOUS, 2 NUISANCE)".
    pub fn describe(&self) -> String {
        if self.total == 0 {
            return "none".to_string();
        }
        let breakdown = self
            .by_type
            .iter()
            .map(|(kind, count)| format!("{count} {kind}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{} watched group{} ({breakdown})", self.total, if self.total == 1 { "" } else { "s" })
    }
}

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
    /// The full `avtr_` id, when VRChat revealed it.
    pub avatar_id: Option<String>,
    /// The wearer's standing in the watched lists, when Celeste checked
    /// their groups on join. `None` means it never got to look.
    pub watched_groups: Option<WatchedGroupProfile>,
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

    /// Attach the full `avtr_` id once VRChat reveals it.
    ///
    /// Worth more than everything else in the report put together: it is
    /// the identifier VRChat stopped exposing, and the one a moderator
    /// used to be able to act on directly.
    pub fn record_avatar_id(&mut self, display_name: &str, avatar_id: &str) {
        let Some(load) = self.last_avatar_load.as_mut() else { return };
        if load.display_name == display_name {
            load.avatar_id = Some(avatar_id.to_string());
        }
    }

    /// Attach the wearer's watched-group standing.
    pub fn record_watched_groups(&mut self, display_name: &str, profile: WatchedGroupProfile) {
        let Some(load) = self.last_avatar_load.as_mut() else { return };
        if load.display_name == display_name {
            load.watched_groups = Some(profile);
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
            avatar_id: None,
            watched_groups: None,
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
                avatar_id: None,
                watched_groups: None,
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

#[cfg(test)]
mod watched_profile_tests {
    use super::*;

    #[test]
    fn the_breakdown_leads_with_the_commonest_type() {
        let profile = WatchedGroupProfile::from_types(["NUISANCE", "MALICIOUS", "MALICIOUS", "MALICIOUS", "NUISANCE"]);
        assert_eq!(profile.total, 5);
        assert_eq!(profile.describe(), "5 watched groups (3 MALICIOUS, 2 NUISANCE)");
    }

    /// Equal counts must not reorder between runs, or the same account
    /// reads differently each time it is looked at.
    #[test]
    fn ties_are_ordered_stably() {
        let a = WatchedGroupProfile::from_types(["NUISANCE", "MALICIOUS"]);
        let b = WatchedGroupProfile::from_types(["MALICIOUS", "NUISANCE"]);
        assert_eq!(a, b);
        assert_eq!(a.describe(), "2 watched groups (1 MALICIOUS, 1 NUISANCE)");
    }

    #[test]
    fn one_group_reads_as_singular_and_none_says_none() {
        assert_eq!(WatchedGroupProfile::from_types(["COMMUNITY"]).describe(), "1 watched group (1 COMMUNITY)");
        assert_eq!(WatchedGroupProfile::default().describe(), "none");
    }

    /// Being in nothing is itself worth reporting: it is the evidence
    /// *against* the lead, and a moderator weighing an accusation needs
    /// that as much as the counts.
    #[test]
    fn a_clean_user_is_recorded_rather_than_left_blank() {
        let mut watch = CrashWatch::new();
        watch.went_live();
        watch.record_avatar_load(false, AvatarLoad {
            display_name: "Suspect".into(),
            user_id: None,
            avatar_name: None,
            avatar_author: None,
            bundle_file_id: None,
            avatar_id: None,
            watched_groups: None,
            at: chrono::NaiveDate::from_ymd_opt(2026, 10, 4).unwrap().and_hms_opt(21, 48, 13).unwrap(),
        });
        watch.record_watched_groups("Suspect", WatchedGroupProfile::default());
        let load = watch.finish().unwrap().last_avatar_load.unwrap();
        assert_eq!(load.watched_groups.unwrap().describe(), "none");
    }
}
