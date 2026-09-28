//! Port of `ScarletDiscordCommands.VrchatSearch`
//! (`@SlashCmd("vrchat-search")`): world, user, group, and the cached,
//! multi-provider VRCX-compatible avatar search.
//!
//! **Simplified**: Java fetches a fixed batch of 50 results per search and
//! renders them as paginated per-result embeds (author/thumbnail/"Report"
//! link via `VRChatHelpDeskURLs`) using `discord.interactions.Pagination`
//! with an `entries-per-page` option. This drops `entries-per-page` (the
//! same established simplification as every other list-rendering command
//! in this crate) and renders one text line per result via
//! [`super::paginate_on_lines`] instead of embeds. The "Report ___" link
//! on each result is also dropped: it's built by `VRChatHelpDeskURLs`, a
//! large, VRChat-Zendesk-specific URL-builder (per-category form field ids,
//! display-name ASCII transliteration, a UUID short-id encoder) that isn't
//! ported and isn't proportionate to build just for a decorative link on
//! search results -- a real, tracked gap, not something worth faking.
//! `search-query` still hits the real VRChat search endpoints and returns
//! real results either way.

use serenity::builder::{CreateCommand, CreateCommandOption};
use serenity::client::Context;
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};

use scarlet_vrchat::models::{Group, UserSearchResult, World};

use crate::avatar_search::{VrcxAvatar, BUILTIN_PROVIDERS};
use crate::data::{AvatarSearchKey, SettingsKey};

pub const NAME: &str = "vrchat-search";

/// Java fetches exactly this many results per search
/// (`searchWorlds(..., 50, ...)`/`searchUsers(query, null, 50, 0, null)`/
/// `searchGroups(query, 0, 50)`) -- not user-configurable there either, so
/// this isn't exposed as an option here.
const SEARCH_LIMIT: u32 = 50;

pub fn register() -> CreateCommand {
    let query_option = || {
        CreateCommandOption::new(
            CommandOptionType::String,
            "search-query",
            "The search query",
        )
        .required(true)
    };
    let command = CreateCommand::new(NAME)
        .description("Searches for VRChat things")
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "world",
                "Searches for worlds",
            )
            .add_sub_option(query_option()),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::SubCommand, "user", "Searches for users")
                .add_sub_option(query_option()),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "group",
                "Searches for groups",
            )
            .add_sub_option(query_option()),
        );
    if scarlet_core::features::avatar_search_enabled() {
        command.add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "avatar",
                "Searches for avatars",
            )
            .add_sub_option(query_option()),
        )
    } else {
        command
    }
}

fn find_query<'a>(sub_options: &'a [ResolvedOption<'a>]) -> Option<&'a str> {
    sub_options
        .iter()
        .find(|o| o.name == "search-query")
        .and_then(|o| match &o.value {
            ResolvedValue::String(s) => Some(*s),
            _ => None,
        })
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;

    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A subcommand is required.").await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "Expected a subcommand.").await;
    };
    let Some(query) = find_query(sub_options) else {
        return super::reply_ephemeral(ctx, command, "`search-query` is required.").await;
    };

    match subcommand.name {
        "world" => {
            let context = match super::vrchat_context(ctx).await {
                Ok(context) => context,
                Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
            };
            let results = match context.client.search_worlds(query, SEARCH_LIMIT, 0).await {
                Ok(results) => results,
                Err(err) => {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        &format!("Failed to search worlds: {err}"),
                    )
                    .await
                }
            };
            if results.is_empty() {
                return super::reply_ephemeral(ctx, command, "No worlds matched that search.")
                    .await;
            }
            let lines: Vec<String> = results.iter().map(world_line).collect();
            super::send_pages(
                ctx,
                command,
                super::paginate_on_lines(&lines.join("\n"), 2000),
            )
            .await
        }
        "user" => {
            let context = match super::vrchat_context(ctx).await {
                Ok(context) => context,
                Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
            };
            let results = match context.client.search_users(query, SEARCH_LIMIT, 0).await {
                Ok(results) => results,
                Err(err) => {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        &format!("Failed to search users: {err}"),
                    )
                    .await
                }
            };
            if results.is_empty() {
                return super::reply_ephemeral(ctx, command, "No users matched that search.").await;
            }
            let lines: Vec<String> = results.iter().map(user_line).collect();
            super::send_pages(
                ctx,
                command,
                super::paginate_on_lines(&lines.join("\n"), 2000),
            )
            .await
        }
        "group" => {
            let context = match super::vrchat_context(ctx).await {
                Ok(context) => context,
                Err(message) => return super::reply_ephemeral(ctx, command, &message).await,
            };
            let results = match context.client.search_groups(query, SEARCH_LIMIT, 0).await {
                Ok(results) => results,
                Err(err) => {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        &format!("Failed to search groups: {err}"),
                    )
                    .await
                }
            };
            if results.is_empty() {
                return super::reply_ephemeral(ctx, command, "No groups matched that search.")
                    .await;
            }
            let lines: Vec<String> = results.iter().map(group_line).collect();
            super::send_pages(
                ctx,
                command,
                super::paginate_on_lines(&lines.join("\n"), 2000),
            )
            .await
        }
        "avatar" => {
            if !scarlet_core::features::avatar_search_enabled() {
                return super::reply_ephemeral(ctx, command, "Avatar search is disabled.").await;
            }
            let (search, settings) = {
                let data = ctx.data.read().await;
                (
                    data.get::<AvatarSearchKey>().cloned(),
                    data.get::<SettingsKey>().cloned(),
                )
            };
            let Some(search) = search else {
                return super::reply_ephemeral(ctx, command, "Avatar search is unavailable.").await;
            };
            let providers: Vec<String> = match settings {
                Some(settings) if settings.avatar_search_providers_enabled.get() => settings
                    .avatar_search_providers
                    .get()
                    .into_iter()
                    .filter(|url| !url.to_ascii_lowercase().contains("api.avatarrecovery.com"))
                    .collect(),
                _ => BUILTIN_PROVIDERS
                    .iter()
                    .map(|url| (*url).to_string())
                    .collect(),
            };
            let results = search.search_all(&providers, query).await;
            if results.is_empty() {
                return super::reply_ephemeral(ctx, command, "No avatars matched that search.")
                    .await;
            }
            let lines: Vec<String> = results.iter().map(avatar_line).collect();
            super::send_pages(
                ctx,
                command,
                super::paginate_on_lines(&lines.join("\n"), 2000),
            )
            .await
        }
        other => {
            super::reply_ephemeral(ctx, command, &format!("Unknown subcommand: {other}")).await
        }
    }
}

fn avatar_line(avatar: &VrcxAvatar) -> String {
    let name = avatar.name.as_deref().unwrap_or(&avatar.id);
    let author = avatar.author_name.as_deref().unwrap_or("unknown author");
    let author_link = avatar.author_id.as_ref().map_or_else(
        || author.to_string(),
        |id| format!("[{author}](<https://vrchat.com/home/user/{id}>)"),
    );
    let performance = avatar
        .performance
        .as_ref()
        .map(|performance| {
            let mut ratings = Vec::new();
            if let Some(pc) = &performance.pc_rating {
                ratings.push(format!("PC {pc}"));
            }
            if let Some(android) = &performance.android_rating {
                ratings.push(format!("Quest {android}"));
            }
            if let Some(ios) = &performance.ios_rating {
                ratings.push(format!("iOS {ios}"));
            }
            if ratings.is_empty() {
                String::new()
            } else {
                format!(" — {}", ratings.join(", "))
            }
        })
        .unwrap_or_default();
    format!(
        "[{name}](<https://vrchat.com/home/avatar/{}>) `{}` — by {author_link}{performance}",
        avatar.id, avatar.id,
    )
}

fn world_line(world: &World) -> String {
    format!(
        "[{}](<https://vrchat.com/home/world/{}>) `{}` — by {}",
        world.name,
        world.id,
        world.id,
        world.author_name.as_deref().unwrap_or("unknown")
    )
}

fn user_line(user: &UserSearchResult) -> String {
    let name = user.display_name.as_deref().unwrap_or(&user.id);
    format!(
        "[{name}](<https://vrchat.com/home/user/{}>) `{}`",
        user.id, user.id
    )
}

fn group_line(group: &Group) -> String {
    let code = match (&group.short_code, &group.discriminator) {
        (Some(short_code), Some(discriminator)) => format!(" ({short_code}.{discriminator})"),
        _ => String::new(),
    };
    format!(
        "[{}](<https://vrchat.com/home/group/{}>){code} `{}` — {} members",
        group.name, group.id, group.id, group.member_count
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_line_includes_name_id_and_author() {
        let world = World {
            id: "wrld_1".to_string(),
            name: "Test World".to_string(),
            author_name: Some("Bob".to_string()),
            ..Default::default()
        };
        let line = world_line(&world);
        assert!(line.contains("Test World"));
        assert!(line.contains("wrld_1"));
        assert!(line.contains("by Bob"));
    }

    #[test]
    fn user_line_falls_back_to_id_when_display_name_missing() {
        let user = UserSearchResult {
            id: "usr_1".to_string(),
            display_name: None,
            ..Default::default()
        };
        let line = user_line(&user);
        assert!(line.contains("usr_1"));
    }

    #[test]
    fn group_line_includes_short_code_and_discriminator_when_present() {
        let group = Group {
            id: "grp_1".to_string(),
            name: "Test Group".to_string(),
            short_code: Some("TG".to_string()),
            discriminator: Some("1234".to_string()),
            member_count: 42,
            ..Default::default()
        };
        let line = group_line(&group);
        assert!(line.contains("TG.1234"));
        assert!(line.contains("42 members"));
    }

    #[test]
    fn group_line_omits_code_when_missing() {
        let group = Group {
            id: "grp_1".to_string(),
            name: "Test Group".to_string(),
            ..Default::default()
        };
        let line = group_line(&group);
        assert_eq!(
            line,
            "[Test Group](<https://vrchat.com/home/group/grp_1>) `grp_1` — 0 members"
        );
    }

    #[test]
    fn avatar_line_links_avatar_and_author_and_shows_performance() {
        let avatar = VrcxAvatar {
            id: "avtr_1".to_string(),
            name: Some("Test Avatar".to_string()),
            author_id: Some("usr_1".to_string()),
            author_name: Some("Alice".to_string()),
            performance: Some(crate::avatar_search::AvatarPerformance {
                pc_rating: Some("Good".to_string()),
                android_rating: Some("Poor".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let line = avatar_line(&avatar);
        assert!(line.contains("home/avatar/avtr_1"));
        assert!(line.contains("home/user/usr_1"));
        assert!(line.contains("PC Good, Quest Poor"));
    }
}
