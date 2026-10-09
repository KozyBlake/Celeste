//! The one rendering of how Celeste is set up.
//!
//! This existed in three places that had drifted apart: the setup
//! wizard explained it in detail and then was never reachable again, the
//! guided tour explained it again more vaguely, and Settings and the
//! health check each pointed somewhere different. Two explanations of
//! one subject is worse than either alone, because the person who reads
//! the weaker one comes away believing they understood it.
//!
//! So there is one renderer, over one source of words
//! ([`celeste_discord::setup_guide`]), shown in a page that stays
//! reachable. Everywhere else points here rather than re-explaining.
//!
//! Nothing here collapses. The whole point is that a person can see what
//! is required without first guessing which heading hides it.

/// What the reader asked to do while reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideAction {
    None,
    CopyInvite,
    OpenFullSetup,
}

/// Render the guide into `ui`.
///
/// `show_setup_button` is false inside the setup wizard, where offering
/// to open the wizard would be circular.
pub fn render(ui: &mut egui::Ui, show_setup_button: bool) -> GuideAction {
    let mut action = GuideAction::None;

    ui.label(
        egui::RichText::new(
            "Celeste needs two separate things to be right, and they are easy to mistake for \
             one. The server decides what the bot may do. Celeste decides who may ask it to. \
             Either one alone leaves a bot that is present and useless.",
        )
        .color(crate::theme::TEXT_DIM),
    );
    ui.add_space(crate::theme::GAP_WIDE);

    section(ui, "1. The server: what the bot may do");
    for (number, step) in celeste_discord::setup_guide::bot_steps().iter().enumerate() {
        numbered(ui, number + 1, step);
    }
    ui.add_space(crate::theme::GAP_TIGHT);
    if ui.button("Copy invite link").clicked() {
        action = GuideAction::CopyInvite;
    }
    ui.label(
        egui::RichText::new("Grants only what Celeste uses - not Administrator.")
            .small()
            .color(crate::theme::TEXT_MUTED),
    );

    ui.add_space(crate::theme::GAP_WIDE);
    section(ui, "2. Celeste: who may use it");
    ui.label(
        egui::RichText::new(
            "Inviting the bot grants nobody anything. These are Celeste's own permissions; a \
             Discord moderator role does not carry over.",
        )
        .small()
        .color(crate::theme::TEXT_DIM),
    );
    ui.add_space(crate::theme::GAP_TIGHT);
    for (number, step) in celeste_discord::setup_guide::access_steps().iter().enumerate() {
        numbered(ui, number + 1, step);
    }

    if show_setup_button {
        ui.add_space(crate::theme::GAP_WIDE);
        ui.separator();
        ui.add_space(crate::theme::GAP_TIGHT);
        if ui.button("Set up Discord and VRChat").clicked() {
            action = GuideAction::OpenFullSetup;
        }
        ui.label(
            egui::RichText::new("Adds an integration you skipped. Existing settings are left alone.")
                .small()
                .color(crate::theme::TEXT_MUTED),
        );
    }

    action
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).strong());
    ui.add_space(crate::theme::GAP_TIGHT);
}

fn numbered(ui: &mut egui::Ui, number: usize, text: &str) {
    // One wrapping label rather than a number beside a label. Inside a
    // horizontal layout egui gives a label unbounded width, so the text
    // ran off the panel and was clipped instead of reflowing. The
    // hanging indent is not worth losing half of every sentence.
    ui.label(format!("{number}.  {text}"));
}
