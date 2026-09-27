//! Port of `net.sybyline.scarlet.ScarletPermission`.
//!
//! The Java version also implements `DEnum.DEnumString`, a generic
//! enum-to-Discord-slash-command-choice framework (`net.sybyline.scarlet.
//! server.discord.DEnum`) built entirely around JDA types (`Command.Choice`,
//! `SelectOption`, `OptionMapping`, `CommandAutoCompleteInteractionEvent`,
//! reflection over `Class<DE>`) plus Levenshtein-sorted autocomplete. That
//! framework doesn't have a line-for-line Rust translation — serenity's
//! command-choice API is shaped differently — so it isn't ported here; it
//! belongs with the `scarlet-discord` command layer (task 5 in PORTING.md),
//! once there's a concrete serenity-based design for "enum as a slash
//! command option" to port it *onto* rather than transliterating JDA
//! generics that don't map over. Only the plain data (id/title lookup) is
//! ported here, since every other Scarlet subsystem needs that regardless of
//! how Discord command options end up wired.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScarletPermission {
    GroupexBansManage,
    GroupexTagsEdit,
}

impl ScarletPermission {
    pub const ALL: [ScarletPermission; 2] =
        [ScarletPermission::GroupexBansManage, ScarletPermission::GroupexTagsEdit];

    pub const fn id(self) -> &'static str {
        match self {
            ScarletPermission::GroupexBansManage => "groupex-bans-manage",
            ScarletPermission::GroupexTagsEdit => "groupex-tags-edit",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            ScarletPermission::GroupexBansManage => "Override: Manage Bans",
            ScarletPermission::GroupexTagsEdit => "Override: Tag and describe moderation events",
        }
    }

    /// Port of `ScarletPermission.of(String)`. The Java version stores ids
    /// as-given but looks them up lowercased; since every id here is already
    /// lowercase that's unobservable, but the lookup is kept
    /// case-insensitive to match.
    pub fn of(id: &str) -> Option<ScarletPermission> {
        let lower = id.to_lowercase();
        Self::ALL.into_iter().find(|p| p.id() == lower)
    }
}

impl std::fmt::Display for ScarletPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn of_round_trips_ids() {
        for p in ScarletPermission::ALL {
            assert_eq!(ScarletPermission::of(p.id()), Some(p));
        }
        assert_eq!(ScarletPermission::of("GROUPEX-BANS-MANAGE"), Some(ScarletPermission::GroupexBansManage));
        assert_eq!(ScarletPermission::of("nonexistent"), None);
    }
}
