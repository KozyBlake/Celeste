//! Formats what Celeste knows about a player's avatar into a block of text
//! a moderator can paste into a VRChat Help Desk report.
//!
//! ## Why this exists in this shape
//!
//! VRChat no longer exposes another player's `avtr_` id. For an avatar you
//! merely stood next to — the crasher case — there is no avatar id to
//! report, and the avatar-database lookup that would normally bridge
//! name → id cannot help either: a crasher avatar is private and
//! uncatalogued, so no database has an entry to find.
//!
//! What does survive is the asset bundle. VRChat's own client logs the
//! bundle it downloaded (`[API] ... /analysis/file_...`), and the file
//! record for that bundle carries an `ownerId`. That comes from the local
//! log plus a file lookup Celeste already performs, so it works for a brand
//! new private avatar — the client had to fetch the bundle in order to
//! render (or crash on) it.
//!
//! ## The rule this module follows
//!
//! **Never print a field Celeste does not actually know.** A report is read
//! by a human deciding whether to action an account; a plausible-looking
//! guess is worse than an absence. Unknown fields are omitted, and the
//! footer says plainly why an avatar id may be missing so the reader does
//! not assume it was withheld.

use crate::ui::ConnectedPlayer;

/// One line of the evidence block, as `label: value`.
fn line(out: &mut String, label: &str, value: &str) {
    out.push_str(label);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

/// Renders the evidence block. `location` is the instance the player was
/// seen in, when known.
pub fn render(player: &ConnectedPlayer, location: Option<&str>) -> String {
    let mut out = String::new();

    line(&mut out, "Player", &player.name);
    line(&mut out, "User ID", &player.id);
    if let Some(location) = location {
        line(&mut out, "Instance", location);
    }
    line(&mut out, "Joined (local)", &player.joined.format("%Y-%m-%d %H:%M:%S").to_string());
    if let Some(left) = player.left {
        line(&mut out, "Left (local)", &left.format("%Y-%m-%d %H:%M:%S").to_string());
    }

    if let Some(avatar_name) = &player.avatar_name {
        line(&mut out, "Avatar name", avatar_name);
    }

    let mut avatar_id_known = false;
    if let Some(info) = &player.avatar_info {
        if let Some(avatar_id) = &info.avatar_id {
            line(&mut out, "Avatar ID", avatar_id);
            avatar_id_known = true;
        }
        if let Some(file_id) = &info.file_id {
            line(&mut out, "Asset bundle file ID", file_id);
        }
        if let Some(owner_id) = &info.owner_id {
            line(&mut out, "Bundle uploaded by", owner_id);
        }
        if let Some(file_name) = &info.file_name {
            line(&mut out, "Bundle file name", file_name);
        }
        if let Some(rating) = &info.performance_rating {
            line(&mut out, "Performance rating", rating);
        }
    }

    // Say why the field a reporter expects to see is absent, rather than
    // leaving them to wonder whether it was omitted on purpose.
    if !avatar_id_known {
        out.push('\n');
        out.push_str(
            "No avatar ID: VRChat does not expose another player's avatar ID, and this avatar is not \
             in the public avatar databases. The asset bundle file ID above identifies the exact \
             bundle this client downloaded, and its uploader is the account that published it.\n",
        );
    }

    out
}

/// Whether there is anything worth copying. A row with no avatar
/// information at all would produce a block naming only the player, which
/// the moderator already has.
pub fn has_avatar_evidence(player: &ConnectedPlayer) -> bool {
    player
        .avatar_info
        .as_ref()
        .is_some_and(|info| info.file_id.is_some() || info.avatar_id.is_some() || info.owner_id.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::AvatarBundleInfo;
    use chrono::NaiveDate;

    fn player() -> ConnectedPlayer {
        ConnectedPlayer::new(
            "usr_1111",
            "Crasher Person",
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap().and_hms_opt(20, 15, 0).unwrap(),
        )
    }

    #[test]
    fn exact_path_reports_the_bundle_and_its_uploader() {
        let mut p = player();
        p.avatar_name = Some("x".into());
        p.avatar_info = Some(AvatarBundleInfo {
            file_id: Some("file_abc".into()),
            file_name: Some("Avatar - x - Asset bundle - ...".into()),
            performance_rating: Some("VeryPoor".into()),
            owner_id: Some("usr_uploader".into()),
            avatar_id: None,
        });
        let text = render(&p, Some("wrld_1:42~group(grp_1)"));
        assert!(text.contains("User ID: usr_1111"));
        assert!(text.contains("Asset bundle file ID: file_abc"));
        assert!(text.contains("Bundle uploaded by: usr_uploader"));
        assert!(text.contains("Instance: wrld_1:42~group(grp_1)"));
        // The explanation has to be there, or the reader assumes the avatar
        // id was simply left out.
        assert!(text.contains("No avatar ID"));
    }

    /// The bug this guards: printing an "Avatar ID" line for an avatar
    /// Celeste never identified. A wrong id on a moderation report is worse
    /// than no id, because someone may act on it.
    #[test]
    fn never_invents_an_avatar_id() {
        let mut p = player();
        p.avatar_name = Some("Totally Normal Avatar".into());
        p.avatar_info = Some(AvatarBundleInfo {
            file_id: Some("file_abc".into()),
            file_name: None,
            performance_rating: None,
            owner_id: None,
            avatar_id: None,
        });
        let text = render(&p, None);
        assert!(!text.contains("Avatar ID:"));
        assert!(!text.contains("avtr_"));
    }

    #[test]
    fn database_match_reports_the_real_avatar_id_and_no_disclaimer() {
        let mut p = player();
        p.avatar_info = Some(AvatarBundleInfo {
            file_id: Some("file_abc".into()),
            file_name: None,
            performance_rating: None,
            owner_id: Some("usr_author".into()),
            avatar_id: Some("avtr_real".into()),
        });
        let text = render(&p, None);
        assert!(text.contains("Avatar ID: avtr_real"));
        assert!(!text.contains("No avatar ID"));
    }

    /// Omitting the instance must not shift the remaining labels.
    #[test]
    fn omits_unknown_fields_rather_than_printing_blanks() {
        let p = player();
        let text = render(&p, None);
        assert!(!text.contains("Instance:"));
        assert!(!text.contains("Avatar name:"));
        assert!(!text.contains(": \n"));
    }

    #[test]
    fn a_row_with_no_avatar_data_has_nothing_to_copy() {
        let mut p = player();
        assert!(!has_avatar_evidence(&p));
        p.avatar_info = Some(AvatarBundleInfo {
            file_id: Some("file_abc".into()),
            file_name: None,
            performance_rating: None,
            owner_id: None,
            avatar_id: None,
        });
        assert!(has_avatar_evidence(&p));
    }
}
