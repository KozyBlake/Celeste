//! Port of `net.sybyline.scarlet.ScarletModerationTags`: a small JSON-file
//! list of custom moderation tags (`moderation_tags.json`) that moderators
//! can attach to a ban/kick/warn audit entry.
//!
//! **Not ported**: `translateAdvisories`/`restoreAdvisories`, which call
//! out to a LibreTranslate-compatible HTTP endpoint to machine-translate
//! each tag's `description` (keeping `descriptionOriginal` for restore).
//! That's a whole separate external-API integration (`Translator.translate`,
//! itself not ported), out of proportion to add just to round out this
//! module — `description_original`/`translated_lang` are kept on [`Tag`]
//! (so a data directory shared with a Java install round-trips those
//! fields unchanged) but nothing in this port ever sets them.
//!
//! Java's `addOrUpdateTag`/`removeTag` return an `int` status code with a
//! `default: return Integer.MIN_VALUE` branch for "the in-memory list is
//! null" and a `-1`/list-mutation-failed branch for `List::add`/`List::remove`
//! returning `false` on a `CopyOnWriteArrayList` -- both defensive against
//! failure modes that can't happen here (this port always has a loaded
//! `Vec`, and `Vec::push`/`Vec::remove` never "fail" the way a concurrent
//! collection's `add`/`remove` theoretically could), so [`TagUpdateResult`]/
//! [`TagRemoveResult`] only have the outcomes that are actually reachable.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Java allows at most 125 tags (`addOrUpdateTag`'s `tags.size() >= 125`
/// check) -- Discord's own autocomplete choice list caps at 25 entries per
/// dropdown, but `searchTags`'s point is to search across all of them.
pub const MAX_TAGS: usize = 125;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tag {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "descriptionOriginal", default)]
    pub description_original: Option<String>,
    #[serde(rename = "translatedLang", default)]
    pub translated_lang: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagUpdateResult {
    Created,
    Updated,
    MaxTagsReached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagRemoveResult {
    Removed,
    NotFound,
    Empty,
}

pub struct ModerationTags {
    file: PathBuf,
    tags: Mutex<Vec<Tag>>,
}

impl ModerationTags {
    pub fn open(dir: &Path, file_name: &str) -> Self {
        let file = dir.join(file_name);
        let tags = match std::fs::read_to_string(&file) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|err| {
                tracing::error!("Exception loading moderation tags {}: {}", file.display(), err);
                Vec::new()
            }),
            Err(_) => Vec::new(),
        };
        ModerationTags { file, tags: Mutex::new(tags) }
    }

    fn save(&self, tags: &[Tag]) {
        match serde_json::to_string_pretty(tags) {
            Ok(json) => {
                if let Err(err) = crate::file_backups::write(&self.file, &json) {
                    tracing::error!("Exception saving moderation tags {}: {}", self.file.display(), err);
                }
            }
            Err(err) => tracing::error!("Exception serializing moderation tags {}: {}", self.file.display(), err),
        }
    }

    pub fn get_tags(&self) -> Vec<Tag> {
        self.tags.lock().unwrap().clone()
    }

    pub fn get_tag(&self, value: &str) -> Option<Tag> {
        self.tags.lock().unwrap().iter().find(|t| t.value == value).cloned()
    }

    /// Creates a new tag, or updates `label`/`description` on an existing
    /// one (only the fields given as `Some` are changed on an update,
    /// matching Java's `if (label != null) old.label = label;`).
    pub fn add_or_update_tag(&self, value: &str, label: Option<&str>, description: Option<&str>) -> TagUpdateResult {
        let mut tags = self.tags.lock().unwrap();
        if let Some(existing) = tags.iter_mut().find(|t| t.value == value) {
            if let Some(label) = label {
                existing.label = label.to_string();
            }
            if let Some(description) = description {
                existing.description = description.to_string();
            }
            self.save(&tags);
            return TagUpdateResult::Updated;
        }
        if tags.len() >= MAX_TAGS {
            return TagUpdateResult::MaxTagsReached;
        }
        tags.push(Tag {
            value: value.to_string(),
            label: label.unwrap_or_default().to_string(),
            description: description.unwrap_or_default().to_string(),
            description_original: None,
            translated_lang: None,
        });
        self.save(&tags);
        TagUpdateResult::Created
    }

    pub fn remove_tag(&self, value: &str) -> TagRemoveResult {
        let mut tags = self.tags.lock().unwrap();
        if tags.is_empty() {
            return TagRemoveResult::Empty;
        }
        let Some(index) = tags.iter().position(|t| t.value == value) else {
            return TagRemoveResult::NotFound;
        };
        tags.remove(index);
        self.save(&tags);
        TagRemoveResult::Removed
    }

    pub fn get_tag_values(&self) -> Vec<String> {
        self.tags.lock().unwrap().iter().map(|t| t.value.clone()).collect()
    }

    /// Falls back to `value` itself when the tag doesn't exist, matching
    /// Java's `getTagLabel`.
    pub fn get_tag_label(&self, value: &str) -> String {
        self.get_tag(value).map(|t| t.label).unwrap_or_else(|| value.to_string())
    }

    /// Port of `searchTags`: ranked, typo-tolerant search across
    /// label/value/description, case-insensitive. A blank query returns a
    /// plain browse of the first `limit` tags (in stored order); never
    /// returns more than `limit` results.
    pub fn search_tags(&self, query: &str, limit: usize) -> Vec<Tag> {
        let all = self.tags.lock().unwrap();
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return all.iter().take(limit).cloned().collect();
        }
        let q_chars: Vec<char> = q.chars().collect();
        let fuzz = (q_chars.len() / 3).max(1);

        let mut scored: Vec<(i32, usize, String, Tag)> = Vec::new();
        for tag in all.iter() {
            let label = tag.label.to_lowercase();
            let value = tag.value.to_lowercase();
            let desc = tag.description.to_lowercase();
            let score = if label == q || value == q {
                0
            } else if label.starts_with(&q) || value.starts_with(&q) {
                1
            } else if label.contains(&q) || value.contains(&q) {
                2
            } else if desc.contains(&q) {
                3
            } else if levenshtein(&q, &label) <= fuzz {
                4
            } else {
                continue;
            };
            let label_distance = levenshtein(&q, &label);
            // Java's tie-break sorts by `label` unconditionally (its `Tag`
            // constructor guarantees `label` is never null, only possibly
            // empty, and the ternary there only tests nullity) -- matched
            // exactly rather than falling back to `value` on an empty label.
            scored.push((score, label_distance, tag.label.clone(), tag.clone()));
        }
        scored.sort_by(|a, b| {
            a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then_with(|| a.2.to_lowercase().cmp(&b.2.to_lowercase()))
        });
        scored.into_iter().take(limit).map(|(_, _, _, tag)| tag).collect()
    }
}

/// Port of `MiscUtils.levenshtein`: classic single-row DP edit distance,
/// O(min(m,n)) space. Operates on `char`s rather than Java's UTF-16 code
/// units -- the right Unicode-safe equivalent, same as this codebase's
/// other `MiscUtils` ports (see `paginate_on_lines`'s docs).
fn levenshtein(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        if a.len() < b.len() { (b, a) } else { (a, b) }
    };
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut curr = vec![0usize; b.len() + 1];
        curr[0] = i;
        for j in 1..=b.len() {
            curr[j] =
                if a[i - 1] == b[j - 1] { prev[j - 1] } else { 1 + prev[j - 1].min(prev[j]).min(curr[j - 1]) };
        }
        prev = curr;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_matches_known_distances() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("same", "same"), 0);
    }

    #[test]
    fn add_creates_then_update_changes_label_and_description() {
        let dir = tempfile::tempdir().unwrap();
        let tags = ModerationTags::open(dir.path(), "moderation_tags.json");
        assert_eq!(tags.add_or_update_tag("harassment", Some("Harassment"), Some("desc")), TagUpdateResult::Created);
        assert_eq!(tags.add_or_update_tag("harassment", Some("Harassment 2"), None), TagUpdateResult::Updated);
        let tag = tags.get_tag("harassment").unwrap();
        assert_eq!(tag.label, "Harassment 2");
        assert_eq!(tag.description, "desc"); // untouched since `description` was None
    }

    #[test]
    fn add_rejects_past_the_max_tag_count() {
        let dir = tempfile::tempdir().unwrap();
        let tags = ModerationTags::open(dir.path(), "moderation_tags.json");
        for i in 0..MAX_TAGS {
            assert_eq!(tags.add_or_update_tag(&format!("tag{i}"), None, None), TagUpdateResult::Created);
        }
        assert_eq!(tags.add_or_update_tag("one_too_many", None, None), TagUpdateResult::MaxTagsReached);
    }

    #[test]
    fn remove_reports_empty_not_found_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let tags = ModerationTags::open(dir.path(), "moderation_tags.json");
        assert_eq!(tags.remove_tag("nope"), TagRemoveResult::Empty);
        tags.add_or_update_tag("harassment", None, None);
        assert_eq!(tags.remove_tag("nope"), TagRemoveResult::NotFound);
        assert_eq!(tags.remove_tag("harassment"), TagRemoveResult::Removed);
        assert!(tags.get_tag("harassment").is_none());
    }

    #[test]
    fn search_exact_match_ranks_above_substring_match() {
        let dir = tempfile::tempdir().unwrap();
        let tags = ModerationTags::open(dir.path(), "moderation_tags.json");
        tags.add_or_update_tag("harassment-mild", Some("Mild harassment"), None);
        tags.add_or_update_tag("harassment", Some("harassment"), None);
        let results = tags.search_tags("harassment", 10);
        assert_eq!(results[0].value, "harassment");
    }

    #[test]
    fn search_blank_query_returns_a_plain_browse() {
        let dir = tempfile::tempdir().unwrap();
        let tags = ModerationTags::open(dir.path(), "moderation_tags.json");
        tags.add_or_update_tag("a", None, None);
        tags.add_or_update_tag("b", None, None);
        assert_eq!(tags.search_tags("  ", 10).len(), 2);
    }

    #[test]
    fn get_tag_label_falls_back_to_value() {
        let dir = tempfile::tempdir().unwrap();
        let tags = ModerationTags::open(dir.path(), "moderation_tags.json");
        assert_eq!(tags.get_tag_label("unknown"), "unknown");
        tags.add_or_update_tag("harassment", Some("Harassment"), None);
        assert_eq!(tags.get_tag_label("harassment"), "Harassment");
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let tags = ModerationTags::open(dir.path(), "moderation_tags.json");
            tags.add_or_update_tag("harassment", Some("Harassment"), Some("desc"));
        }
        let reopened = ModerationTags::open(dir.path(), "moderation_tags.json");
        let tag = reopened.get_tag("harassment").unwrap();
        assert_eq!(tag.label, "Harassment");
        assert_eq!(tag.description, "desc");
    }
}
