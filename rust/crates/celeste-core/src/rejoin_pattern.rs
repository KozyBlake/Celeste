//! Noticing an account that joins, leaves, and comes back, over and over.
//!
//! The automation services that harvest instances work by cycling: a bot
//! enters, records who is present and what they are wearing, and leaves
//! again, on the order of a minute. Several people reporting these have
//! independently described the same shape.
//!
//! This reads that shape out of VRChat's own log, which means it costs
//! nothing — no API request, no profile lookup, no rate-limit budget. It
//! also survives the thing a bio marker cannot: the day a vendor stops
//! advertising itself, the cycling is still there, because the cycling is
//! what the bot is *for*.
//!
//! ## Why it takes more than one short visit
//!
//! People leave instantly all the time — wrong instance, a friend moved,
//! a crash on load. One short visit is not evidence of anything, and a
//! call-out for every brief visitor is a call-out everybody learns to
//! ignore. So this reports only a *repeated* pattern inside a window, and
//! the thresholds are deliberately dull: three short visits in fifteen
//! minutes, where short means under ninety seconds.
//!
//! ## What it is not
//!
//! It is not proof either. Someone whose game is crashing on load looks
//! very similar from the outside, and that person deserves help rather
//! than suspicion. Like the bio markers, this raises an advisory and says
//! what it counted; it takes no action and nothing is wired to one.

use std::collections::HashMap;

use chrono::NaiveDateTime;

/// A visit shorter than this counts toward the pattern.
pub const SHORT_VISIT_SECONDS: i64 = 90;

/// How far back short visits are counted.
pub const WINDOW_SECONDS: i64 = 15 * 60;

/// How many short visits inside the window before it is worth saying.
pub const THRESHOLD: usize = 3;

/// What the tracker counted, for an advisory to quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejoinPattern {
    pub display_name: String,
    pub user_id: String,
    /// Short visits counted inside the window, including the one that
    /// crossed the threshold.
    pub short_visits: usize,
    /// Minutes between the first counted visit and the latest.
    pub span_minutes: i64,
}

impl RejoinPattern {
    /// `joined and left 3 times in 4 minutes`, for a call-out or advisory.
    ///
    /// States the count and the span rather than a conclusion, so whoever
    /// reads it can decide whether it looks like a harvester or like
    /// somebody whose game keeps crashing.
    pub fn describe(&self) -> String {
        let minutes = if self.span_minutes <= 1 {
            "under a minute".to_string()
        } else {
            format!("{} minutes", self.span_minutes)
        };
        format!("joined and left {} times in {minutes}", self.short_visits)
    }
}

#[derive(Debug, Default)]
struct Visitor {
    /// When the current visit began, if they are in the instance now.
    joined_at: Option<NaiveDateTime>,
    /// End times of recent short visits, oldest first.
    short_visits: Vec<NaiveDateTime>,
    /// Already called out for this run of the pattern, so a bot cycling
    /// for an hour produces one advisory rather than twenty.
    reported: bool,
}

/// Per-instance tracking. Reset when the client changes instance: a
/// pattern is about one room, and carrying counts across instances would
/// accumulate them for somebody who simply follows their friends around.
#[derive(Debug, Default)]
pub struct RejoinTracker {
    visitors: HashMap<String, Visitor>,
}

impl RejoinTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget everything. Call on an instance change.
    pub fn reset(&mut self) {
        self.visitors.clear();
    }

    pub fn record_join(&mut self, user_id: &str, at: NaiveDateTime) {
        let visitor = self.visitors.entry(user_id.to_string()).or_default();
        visitor.joined_at = Some(at);
        Self::prune(visitor, at);
    }

    /// Records a departure and returns a pattern when this visit is the
    /// one that crosses the threshold.
    ///
    /// `None` covers every ordinary case: a long visit, a short one that
    /// is the first or second, a leave with no matching join (Celeste
    /// started mid-instance and never saw them arrive), and a pattern
    /// already reported.
    pub fn record_leave(
        &mut self,
        user_id: &str,
        display_name: &str,
        at: NaiveDateTime,
    ) -> Option<RejoinPattern> {
        let visitor = self.visitors.get_mut(user_id)?;
        let joined_at = visitor.joined_at.take()?;
        let dwell = (at - joined_at).num_seconds();
        // A negative dwell means the clock moved backwards between two log
        // lines. Not a visit worth counting, and not worth a diagnostic.
        if !(0..=SHORT_VISIT_SECONDS).contains(&dwell) {
            return None;
        }
        visitor.short_visits.push(at);
        Self::prune(visitor, at);

        if visitor.reported || visitor.short_visits.len() < THRESHOLD {
            return None;
        }
        visitor.reported = true;
        let first = *visitor.short_visits.first()?;
        Some(RejoinPattern {
            display_name: display_name.to_string(),
            user_id: user_id.to_string(),
            short_visits: visitor.short_visits.len(),
            span_minutes: (at - first).num_minutes(),
        })
    }

    /// Short visits counted for this user inside the window, for a join
    /// advisory to mention when they come back again.
    pub fn short_visits(&self, user_id: &str, now: NaiveDateTime) -> usize {
        self.visitors
            .get(user_id)
            .map(|visitor| {
                visitor
                    .short_visits
                    .iter()
                    .filter(|at| (now - **at).num_seconds() <= WINDOW_SECONDS)
                    .count()
            })
            .unwrap_or(0)
    }

    /// Whether this user has already been called out for cycling.
    pub fn already_reported(&self, user_id: &str) -> bool {
        self.visitors.get(user_id).is_some_and(|v| v.reported)
    }

    /// Drops visits that have fallen out of the window, and lets a
    /// reported visitor be reported again once they have genuinely
    /// stopped — otherwise a bot that returns an hour later is silent.
    fn prune(visitor: &mut Visitor, now: NaiveDateTime) {
        visitor
            .short_visits
            .retain(|at| (now - *at).num_seconds() <= WINDOW_SECONDS);
        if visitor.short_visits.is_empty() {
            visitor.reported = false;
        }
    }

    /// How many users are being tracked, for diagnostics.
    pub fn tracked(&self) -> usize {
        self.visitors.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(minute: i64, second: i64) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 10)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            + chrono::Duration::minutes(minute)
            + chrono::Duration::seconds(second)
    }

    /// One short visit, then a second, must say nothing at all.
    #[test]
    fn one_or_two_short_visits_are_not_a_pattern() {
        let mut t = RejoinTracker::new();
        t.record_join("usr_a", at(0, 0));
        assert_eq!(t.record_leave("usr_a", "Alice", at(0, 30)), None);
        t.record_join("usr_a", at(1, 0));
        assert_eq!(t.record_leave("usr_a", "Alice", at(1, 30)), None);
    }

    #[test]
    fn the_third_short_visit_inside_the_window_reports() {
        let mut t = RejoinTracker::new();
        for minute in 0..3 {
            t.record_join("usr_a", at(minute, 0));
            let hit = t.record_leave("usr_a", "Alice", at(minute, 40));
            if minute < 2 {
                assert_eq!(hit, None, "minute {minute}");
            } else {
                let hit = hit.expect("the third should report");
                assert_eq!(hit.short_visits, 3);
                assert_eq!(hit.display_name, "Alice");
                assert_eq!(hit.describe(), "joined and left 3 times in 2 minutes");
            }
        }
    }

    /// A bot cycling for an hour should interrupt a moderator once.
    #[test]
    fn a_continuing_pattern_is_reported_only_once() {
        let mut t = RejoinTracker::new();
        let mut reports = 0;
        for minute in 0..8 {
            t.record_join("usr_a", at(minute, 0));
            if t.record_leave("usr_a", "Alice", at(minute, 40)).is_some() {
                reports += 1;
            }
        }
        assert_eq!(reports, 1, "eight cycles, one call-out");
    }

    /// Somebody who stays is not cycling, however often they come back.
    #[test]
    fn long_visits_never_count() {
        let mut t = RejoinTracker::new();
        for minute in [0, 10, 20] {
            t.record_join("usr_a", at(minute, 0));
            assert_eq!(t.record_leave("usr_a", "Alice", at(minute + 5, 0)), None);
        }
        assert_eq!(t.short_visits("usr_a", at(20, 0)), 0);
    }

    /// Short visits spread thinly fall out of the window instead of
    /// accumulating forever.
    #[test]
    fn visits_outside_the_window_are_forgotten() {
        let mut t = RejoinTracker::new();
        for minute in [0, 20, 40] {
            t.record_join("usr_a", at(minute, 0));
            assert_eq!(
                t.record_leave("usr_a", "Alice", at(minute, 30)),
                None,
                "each is alone in its own window"
            );
        }
        assert_eq!(t.short_visits("usr_a", at(40, 30)), 1);
    }

    /// Celeste starting mid-instance sees a leave it never saw the join
    /// for; that is not a short visit.
    #[test]
    fn a_leave_without_a_join_is_not_counted() {
        let mut t = RejoinTracker::new();
        assert_eq!(t.record_leave("usr_a", "Alice", at(0, 10)), None);
        assert_eq!(t.short_visits("usr_a", at(0, 10)), 0);
    }

    /// Two people each cycling twice must not add up to one pattern.
    #[test]
    fn visitors_are_counted_separately() {
        let mut t = RejoinTracker::new();
        for minute in 0..2 {
            for user in ["usr_a", "usr_b"] {
                t.record_join(user, at(minute, 0));
                assert_eq!(t.record_leave(user, "Someone", at(minute, 30)), None);
            }
        }
        assert_eq!(t.tracked(), 2);
    }

    /// A pattern belongs to one room; following friends between instances
    /// must not accumulate.
    #[test]
    fn an_instance_change_clears_the_counts() {
        let mut t = RejoinTracker::new();
        for minute in 0..2 {
            t.record_join("usr_a", at(minute, 0));
            t.record_leave("usr_a", "Alice", at(minute, 30));
        }
        t.reset();
        assert_eq!(t.tracked(), 0);
        t.record_join("usr_a", at(3, 0));
        assert_eq!(t.record_leave("usr_a", "Alice", at(3, 30)), None, "counting starts again");
    }

    /// A clock that steps backwards between log lines must not produce a
    /// negative dwell that reads as a very short visit.
    #[test]
    fn a_backwards_clock_is_ignored() {
        let mut t = RejoinTracker::new();
        t.record_join("usr_a", at(5, 0));
        assert_eq!(t.record_leave("usr_a", "Alice", at(4, 0)), None);
        assert_eq!(t.short_visits("usr_a", at(5, 0)), 0);
    }

    #[test]
    fn the_description_reads_as_a_count_not_a_verdict() {
        let pattern = RejoinPattern {
            display_name: "Alice".into(),
            user_id: "usr_a".into(),
            short_visits: 4,
            span_minutes: 0,
        };
        assert_eq!(pattern.describe(), "joined and left 4 times in under a minute");
        assert!(!pattern.describe().contains("bot"), "it states what it counted");
    }
}
