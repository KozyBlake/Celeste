//! Finding one person in a live roster.
//!
//! The Instance tab lists everyone the log has seen in the current
//! instance, and in a full world that list is longer than the window. Before
//! this, locating a single player meant scrolling the table — or worse,
//! reading the VRChat log beside it — while joins and leaves kept arriving
//! and moving the rows around.
//!
//! The matching lives here rather than in the UI crate for two reasons: it
//! is ordinary logic over [`ConnectedPlayer`] that deserves tests without a
//! window attached, and it belongs next to [`crate::names`], which already
//! knows how to read VRChat display names.
//!
//! ## Why a plain `contains` is not enough
//!
//! VRChat display names are free-form Unicode, so the name a moderator can
//! see on a nameplate is frequently not text they can type:
//!
//! * decorative fonts — `ＳＡＫＵＲＡ`, `𝓢𝓪𝓴𝓾𝓻𝓪`, `🅂🄰🄺🅄🅁🄰` — are
//!   mathematical, fullwidth or enclosed codepoints, not the ASCII letters
//!   they draw;
//! * names written in another script are reachable by their transliteration
//!   (`さくら` found by typing `sakura`);
//! * an impersonator's name mixes scripts deliberately (`Вlake` with a
//!   Cyrillic `В`), and the moderator will type the Latin letters they
//!   appear to see.
//!
//! So both sides of the comparison are folded with the same two functions
//! the moderation checks already use: [`crate::names::to_ascii`] (what the
//! name *says*) and [`crate::names::to_visual_ascii`] (what the name
//! *looks like*). Keeping both means a confusable name is found whichever
//! spelling is tried — `Вlake` answers to `blake` through the visual fold
//! and to `vlake` through the transliteration.
//!
//! Folding is only attempted for text that is not already ASCII, so the
//! common case — a plain query against a plain name — stays a lowercase
//! substring test, cheap enough to run for every row of every frame.

use crate::names;
use crate::ui::ConnectedPlayer;

/// A prepared roster query: the typed text, in every form worth matching on.
///
/// Built from the search box each frame and thrown away again, so it holds
/// no reference to the roster and cannot go stale as players join and leave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerQuery {
    /// Lowercased, in search order: the text as typed, then its folds.
    needles: Vec<String>,
}

impl PlayerQuery {
    /// Prepares `raw` for matching, or `None` when it is blank — an empty
    /// search box is not a filter that matches nothing, it is no filter.
    pub fn new(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(PlayerQuery {
            needles: fold_forms(trimmed),
        })
    }

    /// The fields a moderator would search by: who they are (display name,
    /// user id) and what they are wearing (avatar name).
    ///
    /// Deliberately not the advisory or watched-group columns: those are
    /// read to decide what to do about a player, and matching on them would
    /// make a search for a name quietly return people whose names do not
    /// contain it.
    pub fn matches(&self, player: &ConnectedPlayer) -> bool {
        self.matches_text(&player.name)
            || self.matches_text(&player.id)
            || player
                .avatar_name
                .as_deref()
                .is_some_and(|avatar| self.matches_text(avatar))
    }

    /// Whether one piece of text answers to this query, by its own
    /// characters or by either reading of them (see the module docs).
    pub fn matches_text(&self, value: &str) -> bool {
        if self.contained_in(&value.to_lowercase()) {
            return true;
        }
        // An ASCII name has no other reading: lowercasing was the whole of
        // the work, and the folds would return the same string.
        if value.is_ascii() {
            return false;
        }
        names::to_ascii(value).is_some_and(|ascii| self.contained_in(&ascii.to_lowercase()))
            || names::to_visual_ascii(value)
                .is_some_and(|visual| self.contained_in(&visual.to_lowercase()))
    }

    fn contained_in(&self, haystack: &str) -> bool {
        self.needles
            .iter()
            .any(|needle| haystack.contains(needle.as_str()))
    }
}

/// Every reading of `value` worth matching, lowercased and deduplicated.
///
/// Always at least one entry, so a query is never prepared with nothing to
/// match on.
fn fold_forms(value: &str) -> Vec<String> {
    let mut forms = Vec::with_capacity(3);
    let mut push = |form: String| {
        if !form.is_empty() && !forms.contains(&form) {
            forms.push(form);
        }
    };
    push(value.to_lowercase());
    if !value.is_ascii() {
        if let Some(ascii) = names::to_ascii(value) {
            push(ascii.to_lowercase());
        }
        if let Some(visual) = names::to_visual_ascii(value) {
            push(visual.to_lowercase());
        }
    }
    // A query of nothing but decorative codepoints folds away to nothing;
    // keep the text as typed so the caller still has a usable query.
    if forms.is_empty() {
        forms.push(value.to_lowercase());
    }
    forms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> chrono::NaiveDateTime {
        chrono::DateTime::from_timestamp(secs, 0).unwrap().naive_utc()
    }

    fn player(id: &str, name: &str) -> ConnectedPlayer {
        ConnectedPlayer::new(id, name, at(1_000))
    }

    #[test]
    fn a_blank_query_is_not_a_filter() {
        assert!(PlayerQuery::new("").is_none());
        assert!(PlayerQuery::new("   ").is_none());
    }

    #[test]
    fn a_partial_name_matches_regardless_of_case() {
        let query = PlayerQuery::new("BELLA").unwrap();
        assert!(query.matches(&player("usr_1", "Blake Belladonna")));
        assert!(!query.matches(&player("usr_2", "Someone Else")));
    }

    #[test]
    fn surrounding_whitespace_in_the_box_is_ignored() {
        let query = PlayerQuery::new("  blake  ").unwrap();
        assert!(query.matches(&player("usr_1", "Blake")));
    }

    /// Pasting a user id out of a report or a Discord message has to find
    /// the row, which is the whole point of having the id column.
    #[test]
    fn a_pasted_user_id_finds_its_row() {
        let query = PlayerQuery::new("usr_8f9a1b2c").unwrap();
        assert!(query.matches(&player("usr_8f9a1b2c", "Anyone")));
        assert!(!query.matches(&player("usr_0000", "Anyone")));
    }

    #[test]
    fn the_worn_avatar_is_searchable_too() {
        let mut wearing = player("usr_1", "Someone");
        wearing.avatar_name = Some("Crimson Protogen".to_string());
        assert!(PlayerQuery::new("protogen").unwrap().matches(&wearing));
        assert!(!PlayerQuery::new("protogen")
            .unwrap()
            .matches(&player("usr_2", "Someone")));
    }

    /// A decorative display name is the common case this exists for: the
    /// moderator types the letters they can see.
    #[test]
    fn a_decorative_font_name_answers_to_plain_letters() {
        for fancy in [
            "ＳＡＫＵＲＡ",            // fullwidth
            "𝓢𝓪𝓴𝓾𝓻𝓪",     // mathematical script
            "Ｓａｋｕｒａ",            // fullwidth lowercase
        ] {
            let found = PlayerQuery::new("sakura")
                .unwrap()
                .matches(&player("usr_1", fancy));
            assert!(found, "{fancy} should be findable by typing sakura");
        }
    }

    #[test]
    fn a_name_in_another_script_answers_to_its_transliteration() {
        let query = PlayerQuery::new("sakura").unwrap();
        assert!(query.matches(&player("usr_1", "さくら")));
    }

    /// The impersonation case. `Вlake` opens with a Cyrillic В, so the
    /// typed Latin `blake` is not in the name at all.
    #[test]
    fn a_confusable_name_answers_to_the_letters_it_imitates() {
        let query = PlayerQuery::new("blake").unwrap();
        assert!(query.matches(&player("usr_1", "Вlake")));
    }

    /// And to the transliteration, for a moderator who types what the
    /// codepoints actually are rather than what they look like.
    #[test]
    fn a_confusable_name_also_answers_to_its_transliteration() {
        let query = PlayerQuery::new("vlake").unwrap();
        assert!(query.matches(&player("usr_1", "Вlake")));
    }

    /// The reverse direction: the name is plain, the query was pasted from
    /// somewhere that styled it.
    #[test]
    fn a_decorative_query_finds_a_plain_name() {
        let query = PlayerQuery::new("Ｓａｋｕｒａ").unwrap();
        assert!(query.matches(&player("usr_1", "sakura")));
    }

    #[test]
    fn a_query_of_pure_decoration_still_matches_itself() {
        let query = PlayerQuery::new("✦✦").unwrap();
        assert!(query.matches(&player("usr_1", "✦✦ Star ✦✦")));
        assert!(!query.matches(&player("usr_2", "Star")));
    }

    #[test]
    fn nothing_matches_a_query_nobody_answers_to() {
        let query = PlayerQuery::new("zzzz").unwrap();
        assert!(!query.matches(&player("usr_1", "Blake")));
    }
}
