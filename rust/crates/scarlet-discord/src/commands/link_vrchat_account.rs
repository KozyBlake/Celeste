//! Self-service Discord↔VRChat linking with the Java bio-code proof.

use chrono::{Duration, Utc};
use serenity::builder::{
    CreateButton, CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateInteractionResponseMessage, EditInteractionResponse,
};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ComponentInteraction, ResolvedValue,
};
use serenity::model::id::RoleId;

use crate::data::{PendingVrChatLinksKey, ScarletDataKey, SettingsKey};

pub const NAME: &str = "link-vrchat-account";
pub const VERIFY_PREFIX: &str = "linkvrc-verify:";

#[derive(Debug, Clone)]
pub struct PendingVrChatLink {
    pub vrc_id: String,
    pub display_name: String,
    pub code: String,
    pub expires_at: chrono::DateTime<Utc>,
    pub had_member_role: bool,
}

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Link your Discord account to your VRChat account")
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "vrchat-user",
                "The VRChat user id (usr_...)",
            )
            .required(true),
        )
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    command
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(
                CreateInteractionResponseMessage::new().ephemeral(true),
            ),
        )
        .await?;
    let vrc_id = command
        .data
        .options()
        .iter()
        .find(|option| option.name == "vrchat-user")
        .and_then(|option| match option.value {
            ResolvedValue::String(value) => Some(value.to_string()),
            _ => None,
        });
    let Some(vrc_id) = vrc_id else {
        return super::reply_ephemeral(ctx, command, "A `vrchat-user` option is required.").await;
    };
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
    };
    let user = match context.client.get_user(&vrc_id).await {
        Ok(user) => user,
        Err(_) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("No VRChat user found with id {vrc_id}"),
            )
            .await
        }
    };
    let Some(member) = command.member.as_ref() else {
        return super::reply_ephemeral(ctx, command, "Could not resolve your Discord membership.")
            .await;
    };
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let Some(settings) = settings else {
        return super::reply_ephemeral(ctx, command, "Settings are unavailable.").await;
    };
    let member_role = configured_role(&settings.members_role_sf.get());
    let had_member_role = member_role.is_some_and(|role| member.roles.contains(&role));
    if !had_member_role
        && !matches!(
            user.age_verification_status.as_deref(),
            Some("18+") | Some("verified")
        )
    {
        let manual = settings.not_verified_link_message.get();
        let body = if manual.trim().is_empty() {
            "I couldn't verify you automatically. Please contact staff for manual verification."
                .to_string()
        } else {
            format!("I couldn't verify you automatically.\n\n{manual}")
        };
        return super::reply_ephemeral(ctx, command, &body).await;
    }
    let display_name = user.display_name.unwrap_or_else(|| vrc_id.clone());
    let code = format!(
        "VRC-VERIFY-{}",
        uuid::Uuid::new_v4().simple().to_string()[..8].to_uppercase()
    );
    let pending = PendingVrChatLink {
        vrc_id: vrc_id.clone(),
        display_name: display_name.clone(),
        code: code.clone(),
        expires_at: Utc::now() + Duration::minutes(30),
        had_member_role,
    };
    let links = {
        ctx.data
            .read()
            .await
            .get::<PendingVrChatLinksKey>()
            .cloned()
    };
    let Some(links) = links else {
        return super::reply_ephemeral(ctx, command, "VRChat link state is unavailable.").await;
    };
    links
        .write()
        .await
        .insert(command.user.id.to_string(), pending);
    let intro = if had_member_role {
        format!("To link [{display_name}](https://vrchat.com/home/user/{vrc_id}) to your Discord account, confirm you own it.")
    } else {
        format!("You're not a verified member yet. To verify and join the VRChat group, confirm you own [{display_name}](https://vrchat.com/home/user/{vrc_id}).")
    };
    command
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content(format!(
                    "{intro}\n\nAdd this code anywhere in your VRChat bio:\n```\n{code}\n```\nEdit your bio at <https://vrchat.com/home/profile>, **save** it, then press the button below. You can remove the code afterward.\nThis request expires in 30 minutes."
                ))
                .button(CreateButton::new(format!("{VERIFY_PREFIX}{vrc_id}")).label("I've added the code — verify"))
                .ephemeral(true),
        )
        .await
        .map(|_| ())
}

pub async fn verify(ctx: &Context, interaction: &ComponentInteraction) -> serenity::Result<()> {
    interaction
        .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
        .await?;
    let requested_id = interaction
        .data
        .custom_id
        .strip_prefix(VERIFY_PREFIX)
        .unwrap_or_default();
    let links = {
        ctx.data
            .read()
            .await
            .get::<PendingVrChatLinksKey>()
            .cloned()
    };
    let Some(links) = links else {
        return edit(ctx, interaction, "VRChat link state is unavailable.", false).await;
    };
    let pending = {
        links
            .read()
            .await
            .get(&interaction.user.id.to_string())
            .cloned()
    };
    let Some(pending) = pending else {
        return edit(
            ctx,
            interaction,
            "This link request is no longer active. Please run `/link-vrchat-account` again.",
            false,
        )
        .await;
    };
    if requested_id != pending.vrc_id {
        return edit(
            ctx,
            interaction,
            "This link request is no longer active. Please run `/link-vrchat-account` again.",
            false,
        )
        .await;
    }
    if Utc::now() > pending.expires_at {
        links.write().await.remove(&interaction.user.id.to_string());
        return edit(
            ctx,
            interaction,
            "This verification code has expired. Please run `/link-vrchat-account` again.",
            false,
        )
        .await;
    }
    let context = match super::vrchat_context(ctx).await {
        Ok(context) => context,
        Err(message) => return edit(ctx, interaction, &message, true).await,
    };
    let bio = context
        .client
        .get_public_profile(&pending.vrc_id)
        .await
        .ok()
        .and_then(|profile| profile.bio);
    if !bio
        .as_deref()
        .is_some_and(|bio| bio.contains(&pending.code))
    {
        let manual = ctx
            .data
            .read()
            .await
            .get::<SettingsKey>()
            .map(|settings| settings.not_verified_link_message.get())
            .unwrap_or_default();
        let suffix = if manual.trim().is_empty() {
            String::new()
        } else {
            format!("\n\n{manual}")
        };
        return edit(
            ctx,
            interaction,
            &format!("I couldn't find the code `{}` in your VRChat bio. Double-check you **saved** your bio, then press the button again.{suffix}", pending.code),
            true,
        )
        .await;
    }
    links.write().await.remove(&interaction.user.id.to_string());
    let scarlet_data = { ctx.data.read().await.get::<ScarletDataKey>().cloned() };
    if let Some(scarlet_data) = scarlet_data {
        scarlet_data.link_id_to_snowflake(&pending.vrc_id, &interaction.user.id.to_string());
    }
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let mut result = format!(
        "Verified! Linked your Discord account to VRChat user [{}](https://vrchat.com/home/user/{}).",
        pending.display_name, pending.vrc_id
    );
    if let (Some(settings), Some(member)) = (settings.as_ref(), interaction.member.as_ref()) {
        if !pending.had_member_role {
            let role_ok = match configured_role(&settings.members_role_sf.get()) {
                Some(role) if member.roles.contains(&role) => true,
                Some(role) => member.add_role(&ctx.http, role).await.is_ok(),
                None => false,
            };
            result.push_str(if role_ok {
                "\nYou've been given the member role."
            } else {
                "\nI couldn't assign your member role automatically. Please contact staff."
            });
        }
        let configured_group = settings.auto_invite_group_id.get();
        let group_id = if configured_group.is_empty()
            || configured_group == "grp_abc12345-6789-0abc-def0-123456789abc"
        {
            context.group_id.as_str()
        } else {
            configured_group.as_str()
        };
        match context.client.create_group_invite(group_id, &pending.vrc_id, false).await {
            Ok(()) => result.push_str("\nYou've been sent an invite to our VRChat group - check your VRChat notifications!"),
            Err(err) => result.push_str(&format!("\nI couldn't send the VRChat group invite: {err}")),
        }
    }
    edit(ctx, interaction, &result, false).await
}

async fn edit(
    ctx: &Context,
    interaction: &ComponentInteraction,
    content: &str,
    keep_button: bool,
) -> serenity::Result<()> {
    let mut response = EditInteractionResponse::new().content(content);
    if !keep_button {
        response = response.components(Vec::new());
    }
    interaction
        .edit_response(&ctx.http, response)
        .await
        .map(|_| ())
}

fn configured_role(value: &str) -> Option<RoleId> {
    if value.is_empty() || value == "1234567890" {
        None
    } else {
        value.parse::<u64>().ok().map(RoleId::new)
    }
}
