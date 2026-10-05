//! A player's watched-group memberships, as the desktop shows them.
//!
//! The advisory column joins every watched group's message into one
//! string and truncates it to fit a table cell. For someone in one group
//! that is fine. For the accounts that matter — the ones in five, ten,
//! twenty watched groups — everything past the first is invisible, which
//! is exactly backwards: the more a person is worth looking at, the less
//! the table shows.
//!
//! This carries the whole set, per group, so a panel can lay them out by
//! type instead of flattening them into prose.

/// One watched group a player is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedMembership {
    pub group_id: String,
    /// Java's enum name: MALICIOUS, NUISANCE, COMMUNITY, AFFILIATED, ...
    pub kind: String,
    /// What VRChat currently calls the group, when known. Watched entries
    /// store no name of their own, so this is the only accurate label.
    pub current_name: Option<String>,
    /// The moderator's own note from `watched_groups.json`.
    pub message: Option<String>,
    /// This user hid the membership on their profile.
    pub hidden: bool,
    pub critical: bool,
    pub silent: bool,
}

impl WatchedMembership {
    /// What to show as the group's name: VRChat's current name, else the
    /// raw id. Never the moderator's message — that is a note about the
    /// group, not its name, and showing it as one is how a renamed group
    /// stays invisible.
    pub fn label(&self) -> &str {
        self.current_name.as_deref().filter(|name| !name.trim().is_empty()).unwrap_or(&self.group_id)
    }
}

/// Groups memberships by type, worst first, for display.
///
/// Order is fixed rather than alphabetical: a moderator scanning this
/// panel wants MALICIOUS at the top every time, in the same place,
/// regardless of what else the account happens to be in.
pub fn group_by_kind(memberships: &[WatchedMembership]) -> Vec<(String, Vec<WatchedMembership>)> {
    const ORDER: [&str; 6] = ["MALICIOUS", "NUISANCE", "OTHER", "UNKNOWN", "COMMUNITY", "AFFILIATED"];
    let mut out: Vec<(String, Vec<WatchedMembership>)> = Vec::new();
    for kind in ORDER {
        let mut matching: Vec<WatchedMembership> =
            memberships.iter().filter(|m| m.kind == kind).cloned().collect();
        if matching.is_empty() {
            continue;
        }
        // Within a type: critical first, then by name, so the panel is
        // stable between openings.
        matching.sort_by(|a, b| {
            b.critical.cmp(&a.critical).then_with(|| a.label().to_lowercase().cmp(&b.label().to_lowercase()))
        });
        out.push((kind.to_string(), matching));
    }
    // Anything with a type not in ORDER still has to appear; dropping a
    // membership because its type is unrecognised would hide exactly the
    // entry someone added by hand.
    let mut leftovers: Vec<WatchedMembership> =
        memberships.iter().filter(|m| !ORDER.contains(&m.kind.as_str())).cloned().collect();
    if !leftovers.is_empty() {
        leftovers.sort_by(|a, b| a.label().to_lowercase().cmp(&b.label().to_lowercase()));
        out.push(("OTHER TYPES".to_string(), leftovers));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(kind: &str, name: &str, critical: bool) -> WatchedMembership {
        WatchedMembership {
            group_id: format!("grp_{name}"),
            kind: kind.to_string(),
            current_name: Some(name.to_string()),
            message: None,
            hidden: false,
            critical,
            silent: false,
        }
    }

    #[test]
    fn malicious_comes_first_and_affiliated_last() {
        let all = vec![m("AFFILIATED", "Friends", false), m("COMMUNITY", "Chill", false), m("MALICIOUS", "Crashers", false)];
        let kinds: Vec<String> = group_by_kind(&all).into_iter().map(|(k, _)| k).collect();
        assert_eq!(kinds, vec!["MALICIOUS", "COMMUNITY", "AFFILIATED"]);
    }

    #[test]
    fn critical_entries_lead_their_section() {
        let all = vec![m("MALICIOUS", "Zebra", false), m("MALICIOUS", "Alpha", false), m("MALICIOUS", "Yankee", true)];
        let (_, section) = group_by_kind(&all).remove(0);
        let names: Vec<&str> = section.iter().map(|m| m.label()).collect();
        assert_eq!(names, vec!["Yankee", "Alpha", "Zebra"]);
    }

    /// A type nobody anticipated must still be shown. Dropping it would
    /// hide precisely the entry someone added by hand.
    #[test]
    fn an_unrecognised_type_is_still_listed() {
        let all = vec![m("MALICIOUS", "Crashers", false), m("WEIRD_CUSTOM", "Odd", false)];
        let sections = group_by_kind(&all);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[1].0, "OTHER TYPES");
        assert_eq!(sections[1].1[0].label(), "Odd");
    }

    /// A renamed group must show its current name; falling back to the
    /// moderator's note would re-hide the rename.
    #[test]
    fn the_label_is_the_current_name_or_the_id_never_the_note() {
        let mut entry = m("MALICIOUS", "masmavia", false);
        entry.message = Some("Kick On Site. Black Squad is a Crasher Group".into());
        assert_eq!(entry.label(), "masmavia");

        entry.current_name = None;
        assert_eq!(entry.label(), "grp_masmavia", "falls back to the id, not the note");
    }

    #[test]
    fn nothing_in_produces_nothing_out() {
        assert!(group_by_kind(&[]).is_empty());
    }
}
