//! What is wrong with this install, and what to do about it.
//!
//! This is deliberately not a state dump. A dump tells an operator what
//! Celeste is configured with and leaves them to work out which part is
//! the problem; a check states the problem and the fix. Every finding
//! carries a remedy the person reading it can act on without knowing how
//! Celeste is built.
//!
//! The types live in core so the same report can be rendered by the
//! desktop Diagnostics panel and printed by the CLI, rather than each
//! growing its own idea of what "healthy" means.

/// How badly a finding affects moderating.
///
/// The distinction that matters is whether the operator has lost a
/// capability or merely some polish, because that decides whether they
/// should stop and fix it before a patrol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Working.
    Ok,
    /// Works, but something is missing or will behave unexpectedly.
    Degraded,
    /// A capability the operator expects is unavailable.
    Blocked,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Ok => "OK",
            Severity::Degraded => "CHECK",
            Severity::Blocked => "BLOCKED",
        }
    }
}

/// One checked thing.
#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    /// The subsystem, in the operator's words ("Text to speech"), not the
    /// crate's ("celeste-ui tts").
    pub area: &'static str,
    /// What is true right now. A complete sentence.
    pub problem: String,
    /// What to do about it. `None` only when there is nothing to do,
    /// which in practice means the finding is `Ok`.
    pub remedy: Option<String>,
}

impl Finding {
    pub fn ok(area: &'static str, problem: impl Into<String>) -> Self {
        Finding { severity: Severity::Ok, area, problem: problem.into(), remedy: None }
    }

    pub fn degraded(
        area: &'static str,
        problem: impl Into<String>,
        remedy: impl Into<String>,
    ) -> Self {
        Finding {
            severity: Severity::Degraded,
            area,
            problem: problem.into(),
            remedy: Some(remedy.into()),
        }
    }

    pub fn blocked(
        area: &'static str,
        problem: impl Into<String>,
        remedy: impl Into<String>,
    ) -> Self {
        Finding {
            severity: Severity::Blocked,
            area,
            problem: problem.into(),
            remedy: Some(remedy.into()),
        }
    }
}

/// Every finding from one run of the checks.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    pub fn push(&mut self, finding: Finding) {
        self.findings.push(finding);
    }

    /// The worst thing found, which is what a status line should show.
    pub fn worst(&self) -> Severity {
        self.findings.iter().map(|finding| finding.severity).max().unwrap_or(Severity::Ok)
    }

    /// Only the findings worth acting on, worst first. Stable within a
    /// severity so the order does not shuffle between runs.
    pub fn problems(&self) -> Vec<&Finding> {
        let mut problems: Vec<&Finding> =
            self.findings.iter().filter(|finding| finding.severity != Severity::Ok).collect();
        problems.sort_by(|left, right| right.severity.cmp(&left.severity));
        problems
    }

    /// One line summarising the whole report.
    pub fn headline(&self) -> String {
        let blocked =
            self.findings.iter().filter(|f| f.severity == Severity::Blocked).count();
        let degraded =
            self.findings.iter().filter(|f| f.severity == Severity::Degraded).count();
        match (blocked, degraded) {
            (0, 0) => "Everything Celeste checks is working.".to_string(),
            (0, n) => format!("{n} thing{} to look at.", plural(n)),
            (n, 0) => format!("{n} thing{} not working.", plural(n)),
            (n, m) => {
                format!("{n} thing{} not working, {m} to look at.", plural(n))
            }
        }
    }

    /// The whole report as text, for the CLI and for pasting into a bug
    /// report. Problems first: someone reading this already suspects
    /// something is wrong, and should not have to scroll past what works.
    pub fn to_text(&self) -> String {
        let mut out = String::from("Celeste health check\n");
        out.push_str(&format!("{}\n", self.headline()));
        let problems = self.problems();
        if !problems.is_empty() {
            out.push('\n');
            for finding in problems {
                out.push_str(&render(finding));
            }
        }
        let working: Vec<&Finding> =
            self.findings.iter().filter(|f| f.severity == Severity::Ok).collect();
        if !working.is_empty() {
            out.push_str("\nWorking:\n");
            for finding in working {
                out.push_str(&format!("  {} - {}\n", finding.area, finding.problem));
            }
        }
        out
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn render(finding: &Finding) -> String {
    let mut out = format!("[{}] {}\n  {}\n", finding.severity.label(), finding.area, finding.problem);
    if let Some(remedy) = &finding.remedy {
        out.push_str(&format!("  Fix: {remedy}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A status line shows the worst thing, not the newest.
    #[test]
    fn the_worst_finding_decides_the_overall_state() {
        let mut report = Report::default();
        assert_eq!(report.worst(), Severity::Ok, "nothing checked is not a failure");
        report.push(Finding::ok("VRChat", "Signed in."));
        assert_eq!(report.worst(), Severity::Ok);
        report.push(Finding::degraded("Text to speech", "No player.", "Install one."));
        assert_eq!(report.worst(), Severity::Degraded);
        report.push(Finding::blocked("Discord", "Not connected.", "Check the token."));
        assert_eq!(report.worst(), Severity::Blocked);
        // Adding a working thing afterwards must not improve the verdict.
        report.push(Finding::ok("Logs", "Writing."));
        assert_eq!(report.worst(), Severity::Blocked);
    }

    /// Someone opening this already suspects a problem, so the blocked
    /// items come before the merely degraded ones, and working things
    /// never push a problem off the top.
    #[test]
    fn problems_come_worst_first_and_exclude_what_works() {
        let mut report = Report::default();
        report.push(Finding::ok("Logs", "Writing."));
        report.push(Finding::degraded("Text to speech", "No player.", "Install one."));
        report.push(Finding::blocked("Discord", "Not connected.", "Check the token."));

        let problems = report.problems();
        assert_eq!(problems.len(), 2, "working findings are not problems");
        assert_eq!(problems[0].area, "Discord");
        assert_eq!(problems[1].area, "Text to speech");
    }

    /// Every problem must say what to do. A finding that only states the
    /// bad news is the state dump this replaces.
    #[test]
    fn every_problem_carries_a_remedy() {
        let mut report = Report::default();
        report.push(Finding::degraded("Text to speech", "No player.", "Install pw-play."));
        report.push(Finding::blocked("Discord", "Not connected.", "Check the token."));
        for finding in report.problems() {
            assert!(
                finding.remedy.is_some(),
                "{} states a problem with no remedy",
                finding.area
            );
        }
        let text = report.to_text();
        assert!(text.contains("Fix: Install pw-play."));
        assert!(text.contains("Fix: Check the token."));
    }

    #[test]
    fn the_headline_counts_both_kinds() {
        let mut report = Report::default();
        assert_eq!(report.headline(), "Everything Celeste checks is working.");
        report.push(Finding::degraded("A", "p", "r"));
        assert_eq!(report.headline(), "1 thing to look at.");
        report.push(Finding::blocked("B", "p", "r"));
        assert_eq!(report.headline(), "1 thing not working, 1 to look at.");
        report.push(Finding::blocked("C", "p", "r"));
        assert_eq!(report.headline(), "2 things not working, 1 to look at.");
    }
}
