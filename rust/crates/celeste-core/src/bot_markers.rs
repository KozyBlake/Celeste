//! A user-editable list of strings that, found in a VRChat profile bio,
//! mean the account is advertising an automation service.
//!
//! The bots this is for join an instance, record who is present and what
//! avatar each person is wearing, and leave again, feeding avatar
//! databases and re-upload tooling. Several of them carry the vendor's
//! domain in their bio, which is the cheapest signal there is: no
//! behavioural analysis, just a string.
//!
//! ## What this is not
//!
//! It is not proof, and nothing here acts on a match. A bio is text the
//! account owner types, so it can be wrong in both directions: somebody
//! can put a vendor's name in their bio as a joke, or to get a rival
//! flagged in every group running Celeste, and a real bot can simply stop
//! advertising. A match raises the same advisory and call-out a watched
//! group does, and a person decides what to do about it.
//!
//! That cuts both ways in a useful direction: because the consequence of a
//! false positive is a conversation rather than a ban, the list can afford
//! to be broad.
//!
//! ## Matching
//!
//! Case-insensitive, whitespace removed from both sides, then a substring
//! test -- so `hexed.cc` is found in `check out HEXED . CC`. It will not
//! survive other obfuscation: `h-e-x-e-d.cc` or a lookalike character
//! passes. Widening it further would start matching ordinary bios, and a
//! deny-list that cries wolf is one people stop reading.
//!
//! Modelled on [`pronoun_lists`](crate::pronoun_lists), which is the same
//! shape: a curated JSON file the user edits without a rebuild.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Written on first run so the file exists with something real in it.
///
/// Only domains that have actually been observed in the wild and reported
/// by more than one person. Guesses do not belong here: every entry is a
/// standing instruction to interrupt a moderator.
pub const DEFAULT_MARKERS: &[&str] = &["hexed.cc", "hexxed.cc"];

const FILE_NAME: &str = "bot_bio_markers.json";

#[derive(Debug)]
pub struct BotMarkers {
    pub file: PathBuf,
    markers: RwLock<BTreeSet<String>>,
}

impl BotMarkers {
    /// Opens the list in `dir`, creating it with [`DEFAULT_MARKERS`] when
    /// it does not exist yet.
    pub fn open(dir: &Path) -> Self {
        let markers = BotMarkers {
            file: dir.join(FILE_NAME),
            markers: RwLock::new(BTreeSet::new()),
        };
        markers.load();
        markers
    }

    /// Rereads the file. Safe to call at any time; the in-memory set is
    /// swapped in atomically.
    pub fn load(&self) {
        *self.markers.write().unwrap() = Self::load_file(&self.file);
    }

    fn load_file(file: &Path) -> BTreeSet<String> {
        if !file.is_file() {
            let values: Vec<&str> = DEFAULT_MARKERS.to_vec();
            match serde_json::to_string_pretty(&values) {
                Ok(json) => {
                    if let Err(err) = crate::file_backups::write(file, &json) {
                        tracing::error!("Exception saving {FILE_NAME}: {err}");
                    }
                }
                Err(err) => tracing::error!("Exception serializing {FILE_NAME}: {err}"),
            }
            return DEFAULT_MARKERS.iter().map(|s| s.to_ascii_lowercase()).collect();
        }
        match std::fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        {
            Some(values) => values
                .into_iter()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
            None => {
                tracing::error!("Exception loading {FILE_NAME} — using defaults");
                DEFAULT_MARKERS.iter().map(|s| s.to_ascii_lowercase()).collect()
            }
        }
    }

    /// The first marker found in `bio`, or `None`.
    ///
    /// Returns the marker rather than a bare `bool` so the advisory can say
    /// *which* string it matched. "Bot indicator: hexed.cc in bio" is a
    /// fact a moderator can check for themselves; "bot detected" is a
    /// claim they have to take on trust.
    pub fn matched(&self, bio: &str) -> Option<String> {
        if bio.trim().is_empty() {
            return None;
        }
        let haystack = despace(&bio.to_ascii_lowercase());
        self.markers
            .read()
            .unwrap()
            .iter()
            .filter(|marker| !marker.is_empty())
            .find(|marker| haystack.contains(&despace(marker)))
            .cloned()
    }

    /// How many markers are active, for the settings screen.
    pub fn count(&self) -> usize {
        self.markers.read().unwrap().len()
    }
}

fn despace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers(dir: &Path) -> BotMarkers {
        BotMarkers::open(dir)
    }

    #[test]
    fn creates_the_file_with_defaults_on_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let list = markers(dir.path());
        assert!(list.file.is_file(), "the file should be written so it can be edited");
        assert_eq!(list.count(), DEFAULT_MARKERS.len());
    }

    #[test]
    fn finds_a_marker_however_it_is_cased_or_spaced() {
        let dir = tempfile::tempdir().unwrap();
        let list = markers(dir.path());
        for bio in [
            "hexed.cc",
            "check out HEXED.CC for cheap avatars",
            "hexed . cc",
            "\u{2764} hexed.cc \u{2764}",
        ] {
            assert_eq!(list.matched(bio).as_deref(), Some("hexed.cc"), "for {bio:?}");
        }
    }

    #[test]
    fn an_ordinary_bio_matches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let list = markers(dir.path());
        for bio in ["", "   ", "she/her, ask me about my cat", "I like hex codes #ff00cc"] {
            assert_eq!(list.matched(bio), None, "for {bio:?}");
        }
    }

    /// The honest limit, written down as a test so nobody assumes more of
    /// it than it does.
    #[test]
    fn obfuscation_beyond_spacing_is_not_caught() {
        let dir = tempfile::tempdir().unwrap();
        let list = markers(dir.path());
        assert_eq!(list.matched("h-e-x-e-d.cc"), None);
        assert_eq!(list.matched("hexed[.]cc"), None);
    }

    #[test]
    fn an_edited_file_is_what_gets_used() {
        let dir = tempfile::tempdir().unwrap();
        let list = markers(dir.path());
        std::fs::write(&list.file, br#"["example.test", "  SPACED.TEST  "]"#).unwrap();
        list.load();
        assert_eq!(list.count(), 2);
        assert_eq!(list.matched("see example.test").as_deref(), Some("example.test"));
        assert_eq!(list.matched("see spaced.test").as_deref(), Some("spaced.test"));
        assert_eq!(list.matched("hexed.cc"), None, "the defaults are gone once edited");
    }

    #[test]
    fn an_unreadable_file_falls_back_to_defaults_rather_than_matching_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let list = markers(dir.path());
        std::fs::write(&list.file, b"{ not an array").unwrap();
        list.load();
        assert_eq!(list.matched("hexed.cc").as_deref(), Some("hexed.cc"));
    }

    #[test]
    fn an_empty_list_disables_it_rather_than_erroring() {
        let dir = tempfile::tempdir().unwrap();
        let list = markers(dir.path());
        std::fs::write(&list.file, b"[]").unwrap();
        list.load();
        assert_eq!(list.count(), 0);
        assert_eq!(list.matched("hexed.cc"), None);
    }
}
