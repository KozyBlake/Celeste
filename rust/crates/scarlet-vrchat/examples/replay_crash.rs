//! Replays a real VRChat log through the parser and applies the same
//! crash rules the listener does, to check the verdict against a log
//! whose outcome is already known.
use scarlet_core::crash_watch::{AvatarLoad, CrashWatch};
use scarlet_vrchat::log_tail::{parse_log_line, parse_entry_text, LogEventKind};

fn main() {
    let path = std::env::args().nth(1).expect("usage: replay_crash <log>");
    let text = std::fs::read_to_string(&path).expect("read log");
    let mut watch = CrashWatch::new();
    watch.went_live();
    let (mut lines, mut events) = (0u64, 0u64);
    for line in text.lines() {
        lines += 1;
        let Some((timestamp, _level, body)) = parse_log_line(line) else { continue };
        let Some(kind) = parse_entry_text(body) else { continue };
        events += 1;
        match kind {
            LogEventKind::PlayerSwitchAvatar { display_name, avatar_display_name } => {
                watch.record_avatar_load(false, AvatarLoad {
                    display_name,
                    user_id: None,
                    avatar_name: Some(avatar_display_name),
                    avatar_author: None,
                    bundle_file_id: None,
                    at: timestamp,
                });
            }
            LogEventKind::AvatarUnpacked { avatar_name, author_name } => {
                watch.record_avatar_author(false, &avatar_name, author_name.as_deref());
            }
            LogEventKind::UserQuit { .. } => watch.saw_clean_quit(),
            _ => {}
        }
    }
    println!("{lines} lines, {events} recognised events");
    match watch.finish() {
        None => println!("VERDICT: clean shutdown, no crash reported"),
        Some(report) => {
            println!("VERDICT: crash");
            match report.last_avatar_load {
                Some(l) => println!(
                    "  last avatar load: {} wearing {:?} (uploaded by {:?}) at {}",
                    l.display_name, l.avatar_name, l.avatar_author, l.at
                ),
                None => println!("  no avatar load observed; nobody named"),
            }
        }
    }
}
