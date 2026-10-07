//! Port of `net.sybyline.scarlet.util.TrustRank`.
//!
//! VRChat trust rank, inferred from the `system_trust_*`/troll entries in a
//! user's `tags` — VRChat doesn't expose trust rank as a first-class API
//! field. Ordinal order is low-to-high urgency for a moderator's sake:
//! [`TrustRank::Nuisance`] sorts first so it's easy to surface, then Visitor
//! upward, with [`TrustRank::Unknown`] last.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrustRank {
    Nuisance,
    Visitor,
    NewUser,
    User,
    Known,
    Trusted,
    Unknown,
}

impl TrustRank {
    pub const fn label(self) -> &'static str {
        match self {
            TrustRank::Nuisance => "Nuisance",
            TrustRank::Visitor => "Visitor",
            TrustRank::NewUser => "New User",
            TrustRank::User => "User",
            TrustRank::Known => "Known",
            TrustRank::Trusted => "Trusted",
            TrustRank::Unknown => "Unknown",
        }
    }

    /// Indicative color for this rank (VRChat-like, as surfaced by VRCX),
    /// `None` for [`TrustRank::Unknown`]. Matches the Java version's colors
    /// exactly (lightened from VRChat's own `#782F2F` for Nuisance, for
    /// readability on a dark table).
    pub const fn color_rgb(self) -> Option<(u8, u8, u8)> {
        match self {
            TrustRank::Nuisance => Some((0xB8, 0x4A, 0x4A)),
            TrustRank::Visitor => Some((0xCC, 0xCC, 0xCC)),
            TrustRank::NewUser => Some((0x17, 0x78, 0xFF)),
            TrustRank::User => Some((0x2B, 0xCF, 0x5C)),
            TrustRank::Known => Some((0xFF, 0x7B, 0x42)),
            TrustRank::Trusted => Some((0x81, 0x43, 0xE6)),
            TrustRank::Unknown => None,
        }
    }

    pub const fn is_nuisance(self) -> bool {
        matches!(self, TrustRank::Nuisance)
    }
    pub const fn is_visitor(self) -> bool {
        matches!(self, TrustRank::Visitor)
    }

    /// Classifies a trust rank from a user's tag list. `None` yields
    /// [`TrustRank::Unknown`] (the lookup gave us nothing — not the same as
    /// a Visitor); `Some` with no trust tag yields
    /// [`TrustRank::Visitor`]. A troll tag overrides any trust tag.
    pub fn of(tags: Option<&[String]>) -> TrustRank {
        let Some(tags) = tags else {
            return TrustRank::Unknown;
        };
        let has = |tag: &str| tags.iter().any(|t| t == tag);
        if has("system_troll") || has("system_probable_troll") {
            TrustRank::Nuisance
        } else if has("system_trust_veteran") {
            TrustRank::Trusted
        } else if has("system_trust_trusted") {
            TrustRank::Known
        } else if has("system_trust_known") {
            TrustRank::User
        } else if has("system_trust_basic") {
            TrustRank::NewUser
        } else {
            TrustRank::Visitor
        }
    }
}

impl std::fmt::Display for TrustRank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_tags_list_is_unknown_not_visitor() {
        assert_eq!(TrustRank::of(None), TrustRank::Unknown);
    }

    #[test]
    fn empty_tags_is_visitor() {
        assert_eq!(TrustRank::of(Some(&[])), TrustRank::Visitor);
    }

    #[test]
    fn troll_tag_overrides_trust_tag() {
        let t = tags(&["system_trust_veteran", "system_troll"]);
        assert_eq!(TrustRank::of(Some(&t)), TrustRank::Nuisance);
    }

    #[test]
    fn highest_trust_tag_wins() {
        assert_eq!(TrustRank::of(Some(&tags(&["system_trust_veteran"]))), TrustRank::Trusted);
        assert_eq!(TrustRank::of(Some(&tags(&["system_trust_trusted"]))), TrustRank::Known);
        assert_eq!(TrustRank::of(Some(&tags(&["system_trust_known"]))), TrustRank::User);
        assert_eq!(TrustRank::of(Some(&tags(&["system_trust_basic"]))), TrustRank::NewUser);
    }
}
