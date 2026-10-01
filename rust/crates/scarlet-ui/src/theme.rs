//! The desktop's visual tokens.
//!
//! Why this exists: the UI had grown 32 distinct hard-coded colours across
//! 47 call sites, four corner radii and five inner margins, because every
//! panel was styled where it was written. Five near-identical greys were
//! doing the job of one surface ramp (`#0C0F15`, `#101319`, `#141B23`,
//! `#161A22`, `#181D26` all appeared as panel fills), and seven more were
//! doing the job of one border. Nested rounded boxes with mismatched radii
//! are what made the result read as separate widgets stacked on each other
//! rather than one application.
//!
//! The rules, which are the point:
//!
//! 1. **Colour is state.** Grey is structure. If a row is coloured,
//!    something about that player needs attention — which is the entire job
//!    of a moderation console. The accent is reserved for selection and
//!    focus; it is never decoration.
//! 2. **Hierarchy comes from surface value and hairline rules**, not from
//!    radius and shadow. Three surface levels, one rule weight.
//! 3. **One radius decision.** Structural panels are square; only things
//!    you can press are rounded, and only slightly, so "pressable" stays
//!    legible without the stacked-card look.
//! 4. **Four type sizes.** Anything outside the scale is a mistake.
//!
//! Everything here is a plain value rather than a setting: the one thing a
//! user configures is the accent (`ui_accent_color`), which is passed in.

use egui::Color32;

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

// ── Surfaces ──────────────────────────────────────────────────────────────
// Three levels, each with one job. A fourth would start the drift again.

/// Application chrome: the window background, toolbar and status bar.
pub const INK: Color32 = rgb(11, 14, 19);
/// A panel sitting on the chrome: the table's container, a settings card.
pub const PANEL: Color32 = rgb(20, 25, 33);
/// Something raised off a panel: an input, a button, an alternating row.
pub const RAISED: Color32 = rgb(28, 34, 44);
/// Recessed, for text entry and the inside of a scroll area.
pub const WELL: Color32 = rgb(8, 11, 15);

// ── Rules ─────────────────────────────────────────────────────────────────
// One hairline weight everywhere. Structure is drawn, not implied by gaps.

/// The default 1px divider and control border.
pub const RULE: Color32 = rgb(40, 48, 61);
/// A border that needs to read as active or hovered without using colour.
pub const RULE_STRONG: Color32 = rgb(58, 69, 86);

// ── Text ──────────────────────────────────────────────────────────────────

/// Body text and table values.
pub const TEXT: Color32 = rgb(220, 227, 237);
/// Column headings, labels, anything supporting.
pub const TEXT_DIM: Color32 = rgb(151, 161, 178);
/// Timestamps, counts, hints — present but not competing.
pub const TEXT_MUTED: Color32 = rgb(107, 117, 133);

// ── State ─────────────────────────────────────────────────────────────────
// The entire colour budget. Each one means something specific; none is
// available for decoration.

/// Something is wrong and needs acting on.
pub const DANGER: Color32 = rgb(214, 73, 80);
/// Something is worth noticing but is not yet a problem.
pub const WARN: Color32 = rgb(222, 158, 58);
/// Confirmation: connected, completed, present.
pub const OK: Color32 = rgb(86, 178, 120);
/// Informational highlight, e.g. a link or an id worth clicking.
pub const INFO: Color32 = rgb(98, 160, 221);

// ── Ported state colours ──────────────────────────────────────────────────
// These are Java's exact values from `ScarletUI.getOverrideForegroundColor`
// / `getOverrideBackgroundColor` (`ScarletUI.java:762-813`). They are part
// of the port's parity surface, not this palette's to restyle, so they live
// here named rather than as literals in the renderer.

/// Java's amber foreground for a player with flagged pronouns.
pub const PRONOUNS_FLAGGED_TEXT: Color32 = rgb(235, 190, 80);
/// Java's amber background for the same.
pub const PRONOUNS_FLAGGED_FILL: Color32 = rgb(80, 55, 10);

/// Java's `Swing.DEFAULT_ACCENT`. The port had defaulted to a blue of its
/// own, so an install that never picked an accent did not look like Java's.
pub const DEFAULT_ACCENT: Color32 = rgb(200, 55, 65);

// ── Geometry ──────────────────────────────────────────────────────────────

/// Structural panels are square. This is the single biggest reason the old
/// UI read as stacked cards: four different radii nesting inside each other.
pub const RADIUS_PANEL: f32 = 0.0;
/// Only things you can press. Small enough to read as a control rather than
/// a card.
pub const RADIUS_CONTROL: f32 = 3.0;

/// The spacing scale. Every margin and gap is one of these.
pub const GAP_TIGHT: f32 = 4.0;
pub const GAP: f32 = 8.0;
pub const GAP_WIDE: f32 = 16.0;

/// Padding inside a panel. One value, so panels line up with each other.
pub const PANEL_PAD: f32 = 12.0;

// ── Type ──────────────────────────────────────────────────────────────────
// Four steps. egui ships one family, so weight and size carry the hierarchy.

/// Timestamps, counts, secondary meta.
pub const TEXT_SM: f32 = 11.0;
/// Table rows and body copy — the size most of the app is read at.
pub const TEXT_BODY: f32 = 13.0;
/// A section or panel heading.
pub const TEXT_HEADING: f32 = 15.0;
/// The one page-level title.
pub const TEXT_TITLE: f32 = 19.0;

/// A hairline in [`RULE`], for drawing structure.
pub fn hairline() -> egui::Stroke {
    egui::Stroke::new(1.0, RULE)
}

/// A square panel: filled, ruled, and padded consistently. Using this
/// instead of a bespoke `Frame` per call site is what keeps panels from
/// drifting apart again.
pub fn panel_frame(fill: Color32) -> egui::Frame {
    egui::Frame::none()
        .fill(fill)
        .stroke(hairline())
        .rounding(egui::Rounding::same(RADIUS_PANEL))
        .inner_margin(egui::Margin::same(PANEL_PAD))
}

/// Whether white or black text is readable on `colour`.
///
/// The accent is user-chosen, so the label on top of it cannot be a fixed
/// near-white the way it can for a fixed palette: pick a bright accent and
/// white-on-accent becomes unreadable. 0.179 is where white-on-colour and
/// black-on-colour give equal contrast — both 4.5:1, the WCAG AA floor.
pub fn readable_text_on(colour: Color32) -> Color32 {
    let channel = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let luminance = 0.2126 * channel(colour.r())
        + 0.7152 * channel(colour.g())
        + 0.0722 * channel(colour.b());
    if luminance > 0.179 { rgb(10, 13, 18) } else { rgb(244, 248, 255) }
}

/// A darker shade of the accent, for a pressed control.
pub fn shade(colour: Color32, factor: f32) -> Color32 {
    let apply = |v: u8| (v as f32 * factor).round().clamp(0.0, 255.0) as u8;
    rgb(apply(colour.r()), apply(colour.g()), apply(colour.b()))
}

/// A lighter shade of the accent, for the border of an active control.
pub fn tint(colour: Color32, amount: f32) -> Color32 {
    let lift =
        |v: u8| (v as f32 + (255.0 - v as f32) * amount).round().clamp(0.0, 255.0) as u8;
    rgb(lift(colour.r()), lift(colour.g()), lift(colour.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The surface ramp has to be monotonic and actually distinguishable:
    /// the old palette's problem was greys two or three points apart, which
    /// read as inconsistency rather than hierarchy.
    #[test]
    fn the_surface_ramp_is_ordered_and_separable() {
        let value = |c: Color32| c.r() as i32 + c.g() as i32 + c.b() as i32;
        assert!(value(WELL) < value(INK), "well sits under the chrome");
        assert!(value(INK) < value(PANEL), "a panel lifts off the chrome");
        assert!(value(PANEL) < value(RAISED), "a control lifts off its panel");
        for (a, b) in [(WELL, INK), (INK, PANEL), (PANEL, RAISED)] {
            assert!(
                value(b) - value(a) >= 9,
                "steps closer than this read as a mistake, not a level"
            );
        }
    }

    /// A fixed light label would be unreadable on a bright accent, which is
    /// a colour the user picks.
    #[test]
    fn label_colour_follows_the_accents_brightness() {
        // Crimson, the default accent: light text.
        assert_eq!(readable_text_on(rgb(200, 55, 65)).r(), 244);
        // Amber: dark text. This is the case a fixed near-white gets wrong.
        assert_eq!(readable_text_on(rgb(212, 130, 26)).r(), 10);
        assert_eq!(readable_text_on(Color32::WHITE).r(), 10);
        assert_eq!(readable_text_on(Color32::BLACK).r(), 244);
    }

    #[test]
    fn body_text_is_legible_against_every_surface() {
        // Rough contrast check: body text must stay well clear of each
        // surface it is ever drawn on.
        let value = |c: Color32| c.r() as i32 + c.g() as i32 + c.b() as i32;
        for surface in [WELL, INK, PANEL, RAISED] {
            assert!(
                value(TEXT) - value(surface) > 400,
                "body text is too close to a surface it sits on"
            );
        }
    }

    #[test]
    fn structural_panels_are_square_and_only_controls_are_rounded() {
        assert_eq!(RADIUS_PANEL, 0.0);
        assert!(RADIUS_CONTROL > 0.0 && RADIUS_CONTROL <= 4.0);
    }

    #[test]
    fn shade_and_tint_move_in_the_directions_they_claim() {
        let base = rgb(200, 55, 65);
        assert!(shade(base, 0.72).r() < base.r());
        assert!(tint(base, 0.35).r() > base.r());
        // Saturating rather than wrapping at the ends.
        assert_eq!(tint(Color32::WHITE, 0.5), Color32::WHITE);
        assert_eq!(shade(Color32::BLACK, 0.5), Color32::BLACK);
    }
}
