//! `/vrchat-animated-emoji` GIF-to-spritesheet commands.


use serenity::builder::{
    CreateAttachment, CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup,
};
use serenity::client::Context;
use serenity::model::application::CommandType;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

pub const NAME: &str = "vrchat-animated-emoji";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Generates a VRChat animated emoji spritesheet from a gif")
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "from-url",
                "Generates a spritesheet from the gif at this URL",
            )
            .add_sub_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "gif-url",
                    "The URL of the gif",
                )
                .required(true),
            ),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "from-file",
                "Generates a spritesheet from the uploaded gif",
            )
            .add_sub_option(
                CreateCommandOption::new(CommandOptionType::Attachment, "gif-file", "The gif file")
                    .required(true),
            ),
        )
}

pub fn register_message() -> CreateCommand {
    CreateCommand::new(NAME).kind(CommandType::Message)
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    if command.data.kind == CommandType::Message {
        return run_message(ctx, command).await;
    }
    super::defer(ctx, command).await?;
    let options = command.data.options();
    let Some(subcommand) = options.first() else {
        return super::reply_ephemeral(ctx, command, "A source subcommand is required.").await;
    };
    let ResolvedValue::SubCommand(sub_options) = &subcommand.value else {
        return super::reply_ephemeral(ctx, command, "Expected a source subcommand.").await;
    };
    let source = match subcommand.name {
        "from-url" => {
            let Some(url) = sub_options
                .iter()
                .find(|option| option.name == "gif-url")
                .and_then(|option| match option.value {
                    ResolvedValue::String(value) => Some(value),
                    _ => None,
                })
            else {
                return super::reply_ephemeral(ctx, command, "A `gif-url` option is required.")
                    .await;
            };
            let parsed = match reqwest::Url::parse(url) {
                Ok(url) => url,
                Err(err) => {
                    return super::reply_ephemeral(ctx, command, &format!("Invalid GIF URL: {err}"))
                        .await
                }
            };
            let name = parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|name| !name.is_empty())
                .unwrap_or("animated-emoji.gif")
                .to_string();
            match download_gif(parsed).await {
                Ok(bytes) => (name, bytes),
                Err(err) => {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        &format!("Could not download GIF: {err}"),
                    )
                    .await
                }
            }
        }
        "from-file" => {
            let Some(attachment) = sub_options
                .iter()
                .find(|option| option.name == "gif-file")
                .and_then(|option| match option.value {
                    ResolvedValue::Attachment(value) => Some(value),
                    _ => None,
                })
            else {
                return super::reply_ephemeral(ctx, command, "A `gif-file` option is required.")
                    .await;
            };
            let parsed = match reqwest::Url::parse(&attachment.url) {
                Ok(url) => url,
                Err(err) => {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        &format!("Invalid attachment URL: {err}"),
                    )
                    .await
                }
            };
            match crate::url_guard::download_public(parsed, scarlet_gif::sprite_sheet::MAX_GIF_BYTES).await {
                Ok(bytes) => (attachment.filename.clone(), bytes),
                Err(err) => {
                    return super::reply_ephemeral(
                        ctx,
                        command,
                        &format!("Could not download attachment: {err}"),
                    )
                    .await
                }
            }
        }
        _ => return super::reply_ephemeral(ctx, command, "Unknown source subcommand.").await,
    };
    send_sheet(ctx, command, source.0, source.1).await
}

async fn run_message(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let Some(message) = command.data.resolved.messages.values().next() else {
        return super::reply_ephemeral(ctx, command, "Could not resolve the selected message.")
            .await;
    };
    let raw = message.content.trim();
    if !raw.starts_with("https://tenor.com/view/") {
        return super::reply_ephemeral(ctx, command, "Message doesn't contain a Tenor gif.").await;
    }
    let url = match reqwest::Url::parse(raw) {
        Ok(url) => url,
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("Invalid Tenor URL: {err}")).await
        }
    };
    let bytes = match download_gif(url).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return super::reply_ephemeral(ctx, command, &format!("Could not download GIF: {err}"))
                .await
        }
    };
    send_sheet(ctx, command, "tenor.gif".into(), bytes).await
}

async fn send_sheet(
    ctx: &Context,
    command: &CommandInteraction,
    name: String,
    bytes: Vec<u8>,
) -> serenity::Result<()> {
    let sheet = match scarlet_gif::VrcSpriteSheet::from_gif(&name, &bytes) {
        Ok(sheet) => sheet,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Exception generating spritesheet: {err}"),
            )
            .await
        }
    };
    let info = sheet.info();
    let file_name = sheet.file_name();
    command
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content(info)
                .add_file(CreateAttachment::bytes(sheet.png, file_name)),
        )
        .await
        .map(|_| ())
}

async fn download_gif(mut url: reqwest::Url) -> Result<Vec<u8>, String> {
    if url.host_str() == Some("tenor.com") && url.path().starts_with("/view/") {
        let html_bytes = crate::url_guard::download_public(url.clone(), 2 * 1024 * 1024).await?;
        let html = String::from_utf8_lossy(&html_bytes);
        let pattern = regex::Regex::new(
            r#"(?is)<meta[^>]+(?:name|property)=[\"']twitter:image[\"'][^>]+content=[\"']([^\"']+)[\"']|<meta[^>]+content=[\"']([^\"']+)[\"'][^>]+(?:name|property)=[\"']twitter:image[\"']"#,
        )
        .expect("static regex");
        let captures = pattern
            .captures(&html)
            .ok_or_else(|| "Tenor page did not expose a GIF image URL".to_string())?;
        let image_url = captures
            .get(1)
            .or_else(|| captures.get(2))
            .unwrap()
            .as_str();
        url = url.join(image_url).map_err(|err| err.to_string())?;
    }
    crate::url_guard::download_public(url, scarlet_gif::sprite_sheet::MAX_GIF_BYTES).await
}

