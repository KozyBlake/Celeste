//! What a person must set up in Discord before Celeste works, on both
//! sides of a distinction that is the usual source of confusion.
//!
//! There are two separate permission systems and Celeste needs both:
//!
//! 1. **Server permissions** - what the *bot* is allowed to do in the
//!    Discord server. Granted by the invite and by channel settings.
//! 2. **Celeste permissions** - which *people* may run which commands.
//!    Granted inside Celeste, and unrelated to anyone's Discord role.
//!
//! A bot invited with every permission still refuses commands if nobody
//! has been granted them, and a fully-granted moderator still gets
//! nothing from a bot that cannot see the channel. People hit one, fix
//! it, and are surprised to be stopped by the other.
//!
//! Celeste cannot create an application, flip a portal switch, or invite
//! itself, so the most it can do is state the requirements exactly and
//! hand over a link that cannot be assembled wrongly.

use serenity::model::permissions::Permissions;

/// Server permissions the bot needs in any channel Celeste will use.
///
/// Built from the same constants the bot acts with rather than written
/// out as a number, so an invite granting too little cannot drift away
/// from what the code actually does.
pub fn required_permissions() -> Permissions {
    // Reading and posting: every audit and advisory post is an embed.
    Permissions::VIEW_CHANNEL
        | Permissions::SEND_MESSAGES
        | Permissions::EMBED_LINKS
        | Permissions::ATTACH_FILES
        | Permissions::READ_MESSAGE_HISTORY
        // Moderation history opens a thread per subject.
        | Permissions::CREATE_PUBLIC_THREADS
        | Permissions::SEND_MESSAGES_IN_THREADS
        // Spoken announcements in a voice channel.
        | Permissions::CONNECT
        | Permissions::SPEAK
}

/// Privileged intents that must be switched on in the Developer Portal.
///
/// These are the usual reason a new bot never connects: Discord refuses
/// the gateway session outright when a bot requests an intent its
/// application has not been granted, which reads as "my token is wrong"
/// to someone setting up for the first time.
pub const PRIVILEGED_INTENTS: [&str; 2] = ["Server Members Intent", "Message Content Intent"];

/// The OAuth2 URL that invites the bot with exactly the right scopes and
/// permissions. `applications.commands` is what registers slash
/// commands; a bot invited without it connects and then appears to have
/// no commands at all.
pub fn invite_url(application_id: u64) -> String {
    format!(
        "https://discord.com/oauth2/authorize?client_id={application_id}\
         &scope=bot%20applications.commands&permissions={}",
        required_permissions().bits()
    )
}

/// Creating the bot and getting it into the server.
pub fn bot_steps() -> Vec<&'static str> {
    vec![
        "Open https://discord.com/developers/applications and create a New Application.",
        "In Bot, press Reset Token and copy it. Discord shows it once; Celeste stores it encrypted.",
        "Still in Bot, turn on both Server Members Intent and Message Content Intent. A bot without these cannot connect at all, which looks like a bad token.",
        "Invite the bot with the link below, which already carries the right scopes and permissions.",
        "Check the bot can see and post in the channels you plan to use. Channel overrides win over the invite.",
        "Paste the token here.",
    ]
}

/// Letting people actually use Celeste once the bot is in.
///
/// Deliberately separate from [`bot_steps`]: this is the half people do
/// not know exists, and it is where "the bot is in my server but nothing
/// works" comes from.
pub fn access_steps() -> Vec<&'static str> {
    vec![
        "A Discord administrator, and the server owner, can already run every Celeste command. Nobody else can until you say so.",
        "Grant a person or a role with /celeste-discord-permissions. Celeste's permissions are its own; a Discord moderator role grants nothing here by itself.",
        "Link each moderator's Discord account to their VRChat account. Unlinked moderators are refused even when they have been granted the command.",
        "Group actions also need the permission in VRChat itself: a linked account must hold the matching group permission, or be the group owner.",
        "Celeste's own VRChat account needs that permission too. Without it a correctly-granted moderator is still refused, because Celeste cannot perform the action.",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The invite must grant everything the bot actually does. Each of
    /// these matches a real call in the codebase, so a permission
    /// dropped from the set would produce an invite that looks fine and
    /// then fails at the moment it matters.
    #[test]
    fn the_invite_covers_what_the_bot_does() {
        let permissions = required_permissions();
        for (permission, why) in [
            (Permissions::VIEW_CHANNEL, "read the channels it posts to"),
            (Permissions::SEND_MESSAGES, "post audit and advisory messages"),
            (Permissions::EMBED_LINKS, "every event post is an embed"),
            (Permissions::CREATE_PUBLIC_THREADS, "moderation history opens a thread"),
            (Permissions::SEND_MESSAGES_IN_THREADS, "and then writes into it"),
            (Permissions::CONNECT, "join a voice channel for spoken alerts"),
            (Permissions::SPEAK, "and be audible in it"),
        ] {
            assert!(permissions.contains(permission), "the bot needs to {why}");
        }
    }

    /// Administrator is what people reach for when an invite does not
    /// work. Celeste asks for what it uses instead: a moderation tool
    /// that demands full control of a server is a bad trade for the
    /// person installing it, and an invite is the moment they decide
    /// whether to trust it.
    #[test]
    fn the_invite_does_not_ask_for_administrator() {
        let permissions = required_permissions();
        assert!(!permissions.contains(Permissions::ADMINISTRATOR));
        assert!(!permissions.contains(Permissions::MANAGE_GUILD));
        assert!(!permissions.contains(Permissions::MANAGE_CHANNELS));
        assert!(
            !permissions.contains(Permissions::BAN_MEMBERS),
            "Celeste bans in VRChat, not in Discord"
        );
    }

    /// Without `applications.commands` the bot joins and then appears to
    /// have no commands, which is a confusing way to fail.
    #[test]
    fn the_invite_url_carries_both_scopes_and_the_permission_bits() {
        let url = invite_url(123456789);
        assert!(url.contains("client_id=123456789"));
        assert!(url.contains("scope=bot%20applications.commands"));
        assert!(url.contains(&format!("permissions={}", required_permissions().bits())));
    }

    /// Both intents are named exactly as the portal labels them: a step
    /// saying "enable the members intent" sends people hunting for a
    /// switch with a different name.
    #[test]
    fn the_steps_name_both_privileged_intents() {
        let steps = bot_steps().join(" ");
        for intent in PRIVILEGED_INTENTS {
            assert!(steps.contains(intent), "the steps never mention {intent}");
        }
    }

    /// The point of splitting the guidance is that the second half
    /// exists at all. It must cover each gate a command passes through,
    /// because being stopped by an unmentioned one is the complaint this
    /// is answering.
    #[test]
    fn the_access_steps_cover_every_gate_a_command_passes() {
        let steps = access_steps().join(" ").to_lowercase();
        assert!(steps.contains("/celeste-discord-permissions"), "how to grant a command");
        assert!(steps.contains("link"), "the linked-account requirement");
        assert!(steps.contains("group permission"), "the VRChat group side");
        assert!(steps.contains("celeste's own vrchat account"), "the bot's own permission");
        assert!(steps.contains("administrator"), "who already has access");
    }
}
