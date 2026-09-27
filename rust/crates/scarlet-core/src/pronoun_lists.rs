//! Port of `net.sybyline.scarlet.ScarletPronounLists`.
//!
//! Two user-editable JSON files in the data directory alongside
//! `settings.json`:
//!
//! - `good_pronoun.json` — strings always accepted as legitimate pronouns
//!   (a manual false-positive suppression list).
//! - `bad_pronoun.json` — strings always flagged (a deny-list).
//!
//! Both are created with Java's exact default content on first run so a user
//! has an example to build on, and both round-trip with a Java install's
//! files unchanged (same names, same JSON array shape, same defaults).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Default content written on first run (verbatim from Java's `DEFAULT_GOOD`).
pub const DEFAULT_GOOD: &[&str] = &[
    "she/her", "she/her/hers",
    "he/him", "he/him/his",
    "they/them", "they/them/theirs", "they/them/themself",
    "it/its", "one/one's",
    "she/they", "he/they", "they/she", "they/he", "she/he", "he/she",
    "he/him or she/her", "she/her or they/them", "he/him or they/them",
    "any", "any/all", "all/any", "any pronouns", "any normative",
    "ask", "ask/me", "ask me", "no pronouns", "none", "use my name",
    "ae/aer", "ae/aer/aers",
    "ey/em", "ey/em/eir", "ey/em/eirs",
    "e/em/eir", "e/em/es",
    "fae/faer", "fae/faer/faers",
    "xe/xem", "xe/xem/xyr", "xe/xem/xyrs", "xe/xir", "xe/xyr",
    "ze/hir", "ze/hir/hirs", "ze/zir", "ze/zir/zirs", "zie/zir",
    "co/cos", "hu/hum", "ne/nem", "ne/nir",
    "per/per", "per/pers", "thon/thons",
    "ve/ver", "ve/ver/vers", "vi/vir", "vi/vim",
    "zhe/zher", "ki/kin",
];

/// Default content written on first run (verbatim from Java's `DEFAULT_BAD`).
///
/// Slurs are in this user-editable file, not in code, exactly as Java has
/// it — the point is that the list can be curated without a rebuild.
/// Self-harm and violence directives are detected in
/// [`crate::pronoun_validator`] instead and need no entry here.
pub const DEFAULT_BAD: &[&str] = &[
    "nick/her",
    "nick/him",
    "call/me/daddy",
    "not/specified",
    "i/dont/care",
    "who/cares",
    "tranny",
    "faggot",
    "nigger",
    "retard",
    "kike",
    "chink",
    "spic",
];

#[derive(Debug)]
pub struct PronounLists {
    pub good_file: PathBuf,
    pub bad_file: PathBuf,
    good: RwLock<BTreeSet<String>>,
    bad: RwLock<BTreeSet<String>>,
}

impl PronounLists {
    /// Opens (and creates, with defaults) both files in `dir`, matching
    /// Java's `new ScarletPronounLists(new File(dir, "good_pronoun.json"),
    /// new File(dir, "bad_pronoun.json"))` followed by `load()`.
    pub fn open(dir: &Path) -> Self {
        let lists = PronounLists {
            good_file: dir.join("good_pronoun.json"),
            bad_file: dir.join("bad_pronoun.json"),
            good: RwLock::new(BTreeSet::new()),
            bad: RwLock::new(BTreeSet::new()),
        };
        lists.load();
        lists
    }

    /// Reloads both files, creating them with defaults when missing. Safe to
    /// call at any time; the in-memory sets are swapped in atomically.
    pub fn load(&self) {
        let good = Self::load_file(&self.good_file, DEFAULT_GOOD, "good_pronoun");
        let bad = Self::load_file(&self.bad_file, DEFAULT_BAD, "bad_pronoun");
        *self.good.write().unwrap() = good;
        *self.bad.write().unwrap() = bad;
    }

    fn load_file(file: &Path, defaults: &[&str], label: &str) -> BTreeSet<String> {
        if !file.is_file() {
            let values: Vec<&str> = defaults.to_vec();
            match serde_json::to_string_pretty(&values) {
                Ok(json) => {
                    if let Err(err) = crate::file_backups::write(file, &json) {
                        tracing::error!("Exception saving {}.json: {}", label, err);
                    }
                }
                Err(err) => tracing::error!("Exception serializing {}.json: {}", label, err),
            }
            return defaults.iter().map(|s| s.to_ascii_lowercase()).collect();
        }
        match std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok()) {
            Some(values) => values.into_iter().map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect(),
            None => {
                tracing::error!("Exception loading {}.json — using defaults", label);
                defaults.iter().map(|s| s.to_ascii_lowercase()).collect()
            }
        }
    }

    /// Exact lowercase match against the allow-list.
    pub fn is_known_good(&self, normalised: &str) -> bool {
        self.good.read().unwrap().contains(normalised)
    }

    /// Exact lowercase match against the deny-list.
    pub fn is_known_bad(&self, normalised: &str) -> bool {
        self.bad.read().unwrap().contains(normalised)
    }

    /// True if any deny-list entry appears anywhere in the field, compared
    /// with whitespace stripped from both sides of the comparison, so
    /// embedded entries are caught: the deny-list entry `call/me/daddy`
    /// flags the field `she/her call/me/daddy`.
    ///
    /// Note that Java's own doc comment on this method claims the example
    /// `she/her call me daddy` (spaces, no slashes) is caught. It isn't —
    /// `despace` only removes whitespace, so the entry's slashes still have
    /// to be present in the field. This port reproduces the code's real
    /// behaviour rather than the comment's claim; "fixing" it to match the
    /// comment would silently widen a moderation deny-list, which is not a
    /// change to make on a doc comment's say-so.
    pub fn contains_known_bad(&self, normalised: &str) -> bool {
        if normalised.is_empty() {
            return false;
        }
        let haystack = despace(normalised);
        self.bad.read().unwrap().iter().filter(|bad| !bad.is_empty()).any(|bad| haystack.contains(&despace(bad)))
    }
}

fn despace(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_both_files_with_defaults_on_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let lists = PronounLists::open(dir.path());
        assert!(lists.good_file.is_file());
        assert!(lists.bad_file.is_file());
        assert!(lists.is_known_good("she/her"));
        assert!(lists.is_known_bad("who/cares"));
        // A second open reads the written files rather than the defaults.
        let reopened = PronounLists::open(dir.path());
        assert!(reopened.is_known_good("they/them"));
    }

    #[test]
    fn deny_list_matches_ignoring_whitespace_anywhere_in_the_field() {
        let dir = tempfile::tempdir().unwrap();
        let lists = PronounLists::open(dir.path());
        assert!(lists.contains_known_bad("she/her call/me/daddy"));
        // Whitespace inside the field is ignored on both sides...
        assert!(lists.contains_known_bad("she/her call / me / daddy"));
        // ...but the entry's own non-space characters still have to match,
        // which is why Java's doc-comment example does not actually trip.
        assert!(!lists.contains_known_bad("she/her call me daddy"));
        assert!(!lists.contains_known_bad("she/her"));
        assert!(!lists.contains_known_bad(""));
    }

    #[test]
    fn a_corrupt_file_falls_back_to_defaults_instead_of_emptying_the_list() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad_pronoun.json"), "{ not an array").unwrap();
        let lists = PronounLists::open(dir.path());
        assert!(lists.is_known_bad("who/cares"));
    }
}
