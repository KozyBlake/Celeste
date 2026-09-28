//! Port of Java's `/export-log`, including its strict generated-filename
//! check so the command cannot be used for path traversal.

use std::path::{Path, PathBuf};

use serenity::builder::{
    CreateAttachment, CreateCommand, CreateCommandOption, CreateInteractionResponseFollowup,
};
use serenity::client::Context;
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};

use crate::data::SettingsKey;

pub const NAME: &str = "export-log";

pub fn register() -> CreateCommand {
    CreateCommand::new(NAME)
        .description("Attaches a log file")
        .add_option(CreateCommandOption::new(
            CommandOptionType::String,
            "file-name",
            "The name of the log file",
        ))
}

fn valid_log_filename(name: &str) -> bool {
    regex::Regex::new(r"^(celeste|scarlet)_log_\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}\.txt$")
        .expect("static log filename regex")
        .is_match(name)
}

fn available_logs(logs_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut logs = std::fs::read_dir(logs_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(valid_log_filename)
        })
        .collect::<Vec<_>>();
    logs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    logs.truncate(25);
    Ok(logs)
}

pub async fn run(ctx: &Context, command: &CommandInteraction) -> serenity::Result<()> {
    super::defer(ctx, command).await?;
    let settings = { ctx.data.read().await.get::<SettingsKey>().cloned() };
    let Some(settings) = settings else {
        return super::reply_ephemeral(ctx, command, "Settings are not available.").await;
    };
    let requested = command
        .data
        .options()
        .iter()
        .find(|option| option.name == "file-name")
        .and_then(|option| match &option.value {
            ResolvedValue::String(value) => Some((*value).to_string()),
            _ => None,
        });
    let logs_dir = settings.data_dir.join("logs");
    let filename = match requested {
        Some(filename) => filename,
        None => match available_logs(&logs_dir)
            .ok()
            .and_then(|logs| logs.into_iter().next())
            .and_then(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            }) {
            Some(filename) => filename,
            None => {
                return super::reply_ephemeral(ctx, command, "No log files detected, strange...")
                    .await
            }
        },
    };
    if !valid_log_filename(&filename) {
        return super::reply_ephemeral(ctx, command, "Invalid log file name").await;
    }
    let target = logs_dir.join(&filename);
    if !target.is_file() {
        return super::reply_ephemeral(ctx, command, "That log file does not exist").await;
    }
    let contents = match tokio::fs::read(&target).await {
        Ok(contents) => contents,
        Err(err) => {
            return super::reply_ephemeral(
                ctx,
                command,
                &format!("Could not read `{filename}`: {err}"),
            )
            .await
        }
    };
    command
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new()
                .content(format!("`{filename}`"))
                .add_file(CreateAttachment::bytes(contents, filename)),
        )
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_scarlet_generated_log_names() {
        assert!(valid_log_filename("celeste_log_2026-09-27_16-30-00.txt"));
        // Logs written before the rename are still exportable.
        assert!(valid_log_filename("scarlet_log_2026-09-27_16-30-00.txt"));
        assert!(!valid_log_filename(
            "../scarlet_log_2026-09-27_16-30-00.txt"
        ));
        assert!(!valid_log_filename("settings.json"));
    }

    #[test]
    fn newest_log_is_first_and_only_logs_are_returned() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("scarlet_log_2026-09-26_10-00-00.txt"),
            "old",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("scarlet_log_2026-09-27_10-00-00.txt"),
            "new",
        )
        .unwrap();
        std::fs::write(dir.path().join("other.txt"), "ignore").unwrap();
        let logs = available_logs(dir.path()).unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(
            logs[0].file_name().unwrap(),
            "scarlet_log_2026-09-27_10-00-00.txt"
        );
    }
}
