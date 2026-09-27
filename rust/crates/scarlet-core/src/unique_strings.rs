//! Port of `net.sybyline.scarlet.util.UniqueStrings`: an insertion-ordered
//! set of strings with a compact JSON representation used for tag lists
//! (`AuditEntryMetadata.entryTags`): empty serializes as `null`, a single
//! element as a bare JSON string, and more than one as a JSON array. This
//! keeps `moderation_tags.json`-style files readable by hand for the common
//! single-tag case instead of always wrapping in `["tag"]`.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UniqueStrings {
    // Insertion-ordered, matching Java's LinkedHashSet. Small collections
    // in practice (tag lists), so linear contains/dedup is fine.
    strings: Vec<String>,
}

impl UniqueStrings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn add(&mut self, value: impl Into<String>) -> bool {
        let value = value.into();
        if self.strings.contains(&value) {
            false
        } else {
            self.strings.push(value);
            true
        }
    }

    pub fn add_all<I: IntoIterator<Item = String>>(&mut self, values: I) -> bool {
        let mut changed = false;
        for v in values {
            changed |= self.add(v);
        }
        changed
    }

    pub fn remove(&mut self, value: &str) -> bool {
        let before = self.strings.len();
        self.strings.retain(|s| s != value);
        self.strings.len() != before
    }

    pub fn remove_all<'a, I: IntoIterator<Item = &'a str>>(&mut self, values: I) -> bool {
        let mut changed = false;
        for v in values {
            changed |= self.remove(v);
        }
        changed
    }

    pub fn clear(&mut self) -> &mut Self {
        self.strings.clear();
        self
    }

    pub fn contains(&self, value: &str) -> bool {
        self.strings.iter().any(|s| s == value)
    }

    pub fn as_slice(&self) -> &[String] {
        &self.strings
    }

    pub fn iter(&self) -> std::slice::Iter<'_, String> {
        self.strings.iter()
    }
}

impl<'a> IntoIterator for &'a UniqueStrings {
    type Item = &'a String;
    type IntoIter = std::slice::Iter<'a, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.strings.iter()
    }
}

impl FromIterator<String> for UniqueStrings {
    fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        let mut u = UniqueStrings::new();
        u.add_all(iter);
        u
    }
}

impl Serialize for UniqueStrings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.strings.as_slice() {
            [] => serializer.serialize_none(),
            [one] => serializer.serialize_str(one),
            many => many.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for UniqueStrings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueStringsVisitor)
    }
}

struct UniqueStringsVisitor;

impl<'de> Visitor<'de> for UniqueStringsVisitor {
    type Value = UniqueStrings;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("null, a string, a bool, a number, or an array of these")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueStrings::new())
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueStrings::new())
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        let mut u = UniqueStrings::new();
        u.add(v);
        Ok(u)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        let mut u = UniqueStrings::new();
        u.add(v.to_string());
        Ok(u)
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        let mut u = UniqueStrings::new();
        u.add(v.to_string());
        Ok(u)
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        let mut u = UniqueStrings::new();
        u.add(v.to_string());
        Ok(u)
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        let mut u = UniqueStrings::new();
        u.add(v.to_string());
        Ok(u)
    }
    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut u = UniqueStrings::new();
        // Mirrors the Java adapter's recursive read: a nested array flattens
        // into the same set (defensive against malformed hand-edited files).
        while let Some(nested) = seq.next_element::<UniqueStrings>()? {
            u.add_all(nested.strings);
        }
        Ok(u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_serializes_as_null() {
        let u = UniqueStrings::new();
        assert_eq!(serde_json::to_string(&u).unwrap(), "null");
    }

    #[test]
    fn single_serializes_as_bare_string() {
        let mut u = UniqueStrings::new();
        u.add("nsfw");
        assert_eq!(serde_json::to_string(&u).unwrap(), "\"nsfw\"");
    }

    #[test]
    fn multiple_serializes_as_array() {
        let mut u = UniqueStrings::new();
        u.add("nsfw");
        u.add("griefing");
        assert_eq!(serde_json::to_string(&u).unwrap(), "[\"nsfw\",\"griefing\"]");
    }

    #[test]
    fn round_trips_through_all_three_shapes() {
        for json in ["null", "\"tag\"", "[\"a\",\"b\",\"c\"]"] {
            let u: UniqueStrings = serde_json::from_str(json).unwrap();
            let back = serde_json::to_string(&u).unwrap();
            let u2: UniqueStrings = serde_json::from_str(&back).unwrap();
            assert_eq!(u, u2);
        }
    }

    #[test]
    fn add_deduplicates() {
        let mut u = UniqueStrings::new();
        assert!(u.add("a"));
        assert!(!u.add("a"));
        assert_eq!(u.len(), 1);
    }

    #[test]
    fn deserializes_a_bool_and_a_number_as_strings() {
        let u: UniqueStrings = serde_json::from_str("true").unwrap();
        assert!(u.contains("true"));
        let u2: UniqueStrings = serde_json::from_str("42").unwrap();
        assert!(u2.contains("42"));
    }
}
