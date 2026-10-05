//! Replays a VRChat log and reports which users Celeste could pin a full
//! avatar id to, using the same QuickMenu-then-fetch pairing the listener
//! applies.
use scarlet_vrchat::log_tail::{parse_entry_text, parse_log_line, LogEventKind};
use std::collections::HashMap;

fn main() {
    let path = std::env::args().nth(1).expect("usage: replay_avatar_ids <log>");
    let text = std::fs::read_to_string(&path).expect("read log");
    let mut names: HashMap<String, String> = HashMap::new();
    let mut pending: Option<(String, chrono::NaiveDateTime)> = None;
    let mut pinned: Vec<(String, String, String)> = Vec::new();
    let (mut fetches, mut orphans) = (0u32, 0u32);

    for line in text.lines() {
        let Some((timestamp, _l, body)) = parse_log_line(line) else { continue };
        let Some(kind) = parse_entry_text(body) else { continue };
        match kind {
            LogEventKind::PlayerJoined { display_name, user_id } => {
                names.insert(user_id, display_name);
            }
            LogEventKind::QuickMenuUserSelected { user_id } => pending = Some((user_id, timestamp)),
            LogEventKind::AvatarFetched { avatar_id } => {
                fetches += 1;
                match pending.take() {
                    Some((user_id, at)) if (timestamp - at).num_seconds().abs() <= 5 => {
                        let name = names.get(&user_id).cloned().unwrap_or_else(|| "(not in roster)".into());
                        pinned.push((name, user_id, avatar_id));
                    }
                    _ => orphans += 1,
                }
            }
            _ => {}
        }
    }
    println!("{fetches} avatar fetches; {} attributed, {orphans} unattributed", pinned.len());
    let mut uniq: Vec<_> = pinned.into_iter().collect();
    uniq.sort();
    uniq.dedup();
    println!("{} distinct user->avatar pairs:", uniq.len());
    for (name, user, avatar) in uniq.iter().take(8) {
        println!("  {name:<22} {user}  ->  {avatar}");
    }
}
