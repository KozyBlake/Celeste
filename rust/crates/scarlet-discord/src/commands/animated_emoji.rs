//! `/vrchat-animated-emoji` GIF-to-spritesheet commands.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
            match download_public(parsed, scarlet_gif::sprite_sheet::MAX_GIF_BYTES).await {
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
        let html = download_public(url.clone(), 2 * 1024 * 1024).await?;
        let html = String::from_utf8_lossy(&html);
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
    download_public(url, scarlet_gif::sprite_sheet::MAX_GIF_BYTES).await
}

async fn download_public(mut url: reqwest::Url, limit: usize) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(scarlet_core::identity::user_agent())
        .build()
        .map_err(|err| err.to_string())?;
    for _ in 0..=5 {
        validate_public_url(&url).await?;
        let mut response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "image/gif,text/html;q=0.5")
            .send()
            .await
            .map_err(|err| err.to_string())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "redirect response omitted Location".to_string())?;
            url = url.join(location).map_err(|err| err.to_string())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err("GIF exceeds maximum allowed size".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|err| err.to_string())? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err("GIF exceeds maximum allowed size".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
    Err("too many redirects".into())
}

async fn validate_public_url(url: &reqwest::Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only HTTP(S) URLs are accepted".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs containing credentials are not accepted".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "URL has no usable port".to_string())?;
    let addresses: Vec<IpAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|err| format!("could not resolve host: {err}"))?
        .map(|address| address.ip())
        .collect();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(*address)) {
        return Err("URL resolves to a non-public address".into());
    }
    Ok(())
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
        || octets[0] >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_and_special_addresses() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }
}
