//! Port of `net.sybyline.scarlet.InstanceRoster`.
//!
//! Tracks who is present in the current VRChat instance and the order in
//! which per-player UI updates (join/leave) are applied.
//!
//! During log catch-up (preamble) UI updates are *deferred* and flushed in
//! log order once catch-up completes; live updates apply immediately. Both
//! joins and leaves go through the same [`InstanceRoster::run_or_defer`]
//! path, so a queued join can never flush *after* a leave and resurrect a
//! departed player — the inversion that inflated the "present" count in the
//! Java version before this class existed. Because the policy lives here and
//! applies identically to joins and leaves, a caller can't reintroduce it.
//!
//! Like the Java class this is deliberately dependency-free and UI-agnostic
//! (updates are opaque closures), so the join/leave/present state machine is
//! unit-testable without a running Scarlet.

use std::collections::HashMap;
use std::sync::Mutex;

/// A deferred UI update. Boxed rather than generic so one queue can hold the
/// join and leave closures together, in one order — the whole point of the
/// class.
type Update = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct RosterInner {
    /// userId -> displayName, in join order (Java uses a `LinkedHashMap`;
    /// this keeps a parallel `Vec` because Rust's `HashMap` is unordered).
    present: HashMap<String, String>,
    present_order: Vec<String>,
    pending: Vec<Update>,
}

#[derive(Default)]
pub struct InstanceRoster {
    inner: Mutex<RosterInner>,
}

impl std::fmt::Debug for InstanceRoster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("InstanceRoster")
            .field("present", &inner.present.len())
            .field("pending", &inner.pending.len())
            .finish()
    }
}

impl InstanceRoster {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the given user is currently in the instance.
    pub fn is_present(&self, user_id: &str) -> bool {
        if user_id.is_empty() {
            return false;
        }
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).present.contains_key(user_id)
    }

    /// Display name last seen for a present user.
    pub fn display_name(&self, user_id: &str) -> Option<String> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).present.get(user_id).cloned()
    }

    /// Display name for a present user, or `fallback` if they aren't present.
    pub fn display_name_or(&self, user_id: &str, fallback: &str) -> String {
        self.display_name(user_id).unwrap_or_else(|| fallback.to_string())
    }

    /// Number of users currently present.
    pub fn present_count(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).present.len()
    }

    /// Snapshot of the currently-present user ids, in join order.
    pub fn present_user_ids(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.present_order.iter().filter(|id| inner.present.contains_key(*id)).cloned().collect()
    }

    /// Records a join and schedules its UI update (deferred during preamble).
    pub fn join(&self, user_id: &str, display_name: &str, preamble: bool, ui_update: impl FnOnce() + Send + 'static) {
        if !user_id.is_empty() {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if inner.present.insert(user_id.to_string(), display_name.to_string()).is_none() {
                inner.present_order.retain(|id| id != user_id);
                inner.present_order.push(user_id.to_string());
            }
        }
        self.run_or_defer(preamble, ui_update);
    }

    /// Records a leave and schedules its UI update. During preamble this is
    /// deferred exactly like [`InstanceRoster::join`], so join/leave for the
    /// same player flush in log order rather than the leave running early and
    /// being undone by a late join.
    pub fn leave(&self, user_id: &str, preamble: bool, ui_update: impl FnOnce() + Send + 'static) {
        if !user_id.is_empty() {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.present.remove(user_id);
            inner.present_order.retain(|id| id != user_id);
        }
        self.run_or_defer(preamble, ui_update);
    }

    /// Runs a UI action now (live) or queues it to flush in order (preamble).
    pub fn run_or_defer(&self, preamble: bool, action: impl FnOnce() + Send + 'static) {
        if preamble {
            self.inner.lock().unwrap_or_else(|e| e.into_inner()).pending.push(Box::new(action));
        } else {
            action();
        }
    }

    /// Applies all queued updates in insertion (log) order, clearing the
    /// queue. Actions run outside the lock, so one of them may safely queue
    /// more work.
    pub fn flush(&self) {
        let queued: Vec<Update> = {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut inner.pending)
        };
        for action in queued {
            action();
        }
    }

    /// Drops queued updates without running them (e.g. leaving an instance
    /// mid-catch-up).
    pub fn clear_pending(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).pending.clear();
    }

    /// Number of currently-queued (not yet flushed) updates.
    pub fn pending_count(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).pending.len()
    }

    /// Clears all present players and queued updates.
    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.present.clear();
        inner.present_order.clear();
        inner.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn live_updates_run_immediately_and_preamble_updates_defer() {
        let roster = InstanceRoster::new();
        let ran = Arc::new(AtomicUsize::new(0));

        let counter = ran.clone();
        roster.join("usr_live", "Live", false, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(ran.load(Ordering::SeqCst), 1);

        let counter = ran.clone();
        roster.join("usr_pre", "Pre", true, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(ran.load(Ordering::SeqCst), 1, "preamble updates wait for the flush");
        assert_eq!(roster.pending_count(), 1);

        roster.flush();
        assert_eq!(ran.load(Ordering::SeqCst), 2);
        assert_eq!(roster.pending_count(), 0);
    }

    /// The bug this class exists to make impossible: a queued join flushing
    /// after a leave, marking a departed player present again.
    #[test]
    fn a_queued_join_cannot_flush_after_a_leave() {
        let roster = InstanceRoster::new();
        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));

        let log = order.clone();
        roster.join("usr_1", "One", true, move || log.lock().unwrap_or_else(|e| e.into_inner()).push("join"));
        let log = order.clone();
        roster.leave("usr_1", true, move || log.lock().unwrap_or_else(|e| e.into_inner()).push("leave"));

        // Presence is updated eagerly, even while the UI updates are queued.
        assert!(!roster.is_present("usr_1"));
        roster.flush();
        assert_eq!(*order.lock().unwrap_or_else(|e| e.into_inner()), vec!["join", "leave"]);
    }

    #[test]
    fn tracks_presence_names_and_order() {
        let roster = InstanceRoster::new();
        roster.join("usr_a", "Alpha", false, || {});
        roster.join("usr_b", "Beta", false, || {});
        assert!(roster.is_present("usr_a"));
        assert_eq!(roster.display_name("usr_b").as_deref(), Some("Beta"));
        assert_eq!(roster.display_name_or("usr_missing", "fallback"), "fallback");
        assert_eq!(roster.present_user_ids(), vec!["usr_a".to_string(), "usr_b".to_string()]);
        assert_eq!(roster.present_count(), 2);

        roster.leave("usr_a", false, || {});
        assert!(!roster.is_present("usr_a"));
        assert_eq!(roster.present_user_ids(), vec!["usr_b".to_string()]);

        roster.reset();
        assert_eq!(roster.present_count(), 0);
    }

    #[test]
    fn clear_pending_drops_queued_updates_without_running_them() {
        let roster = InstanceRoster::new();
        let ran = Arc::new(AtomicUsize::new(0));
        let counter = ran.clone();
        roster.run_or_defer(true, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        roster.clear_pending();
        roster.flush();
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn an_empty_user_id_is_never_present() {
        let roster = InstanceRoster::new();
        roster.join("", "Nameless", false, || {});
        assert!(!roster.is_present(""));
    }
}
