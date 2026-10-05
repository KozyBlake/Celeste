//! Port of `net.sybyline.scarlet.util.ChangeListener` (and its
//! `ListenerList` inner class): a registry of named, priority-ordered
//! change callbacks used by [`crate::settings::FileValued`] so UI widgets
//! (and other subsystems) can react when a setting changes.
//!
//! Java stores listeners in a `ConcurrentSkipListSet` ordered by
//! `(priority, source)` and re-registering under an existing `source`
//! replaces the previous entry (so rebuilding a settings panel after a
//! relogin/theme-change doesn't accumulate stale listeners bound to
//! discarded widgets). This port uses a `Mutex<Vec<..>>` kept sorted the
//! same way — the data volumes here (a handful of UI widgets per setting)
//! don't justify a skip-list, a `Vec` re-sorted on each (rare) registration
//! is simpler and behaves identically.

use std::sync::Mutex;

type Callback<T> = Box<dyn Fn(&T, &T, bool, &str) + Send + Sync>;

struct Holder<T> {
    source: String,
    priority: i32,
    listens_self: bool,
    callback: Callback<T>,
}

pub struct ListenerList<T> {
    listeners: Mutex<Vec<Holder<T>>>,
}

impl<T> Default for ListenerList<T> {
    fn default() -> Self {
        ListenerList { listeners: Mutex::new(Vec::new()) }
    }
}

impl<T: PartialEq + Clone> ListenerList<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `callback` under `source`, replacing any existing
    /// registration with the same source. Returns `true` (mirroring the
    /// Java `Set::add` return value; always true here since the replace
    /// makes the slot available).
    pub fn register(
        &self,
        source: impl Into<String>,
        priority: i32,
        listens_self: bool,
        callback: impl Fn(&T, &T, bool, &str) + Send + Sync + 'static,
    ) -> bool {
        let source = source.into();
        let mut listeners = self.listeners.lock().unwrap_or_else(|e| e.into_inner());
        listeners.retain(|h| h.source != source);
        listeners.push(Holder { source, priority, listens_self, callback: Box::new(callback) });
        listeners.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.source.cmp(&b.source)));
        true
    }

    /// Unregisters the listener registered under `source`, if any. Returns
    /// whether one was removed.
    pub fn unregister(&self, source: &str) -> bool {
        let mut listeners = self.listeners.lock().unwrap_or_else(|e| e.into_inner());
        let before = listeners.len();
        listeners.retain(|h| h.source != source);
        listeners.len() != before
    }

    /// Notifies every registered listener whose source differs from
    /// `source` (or that opted into self-notification via
    /// `listens_self`), unless `previous == next`.
    pub fn on_maybe_change(&self, previous: &T, next: &T, valid: bool, source: &str) {
        if previous == next {
            return;
        }
        let listeners = self.listeners.lock().unwrap_or_else(|e| e.into_inner());
        for holder in listeners.iter() {
            if holder.listens_self || holder.source != source {
                (holder.callback)(previous, next, valid, source);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn notifies_other_listeners_but_not_the_source_by_default() {
        let list: ListenerList<i32> = ListenerList::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        list.register("widget-a", 0, false, move |_prev, _next, _valid, _src| {
            calls2.fetch_add(1, Ordering::SeqCst);
        });
        list.on_maybe_change(&1, &2, true, "widget-a");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "source should not self-notify by default");
        list.on_maybe_change(&2, &3, true, "widget-b");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn listens_self_true_receives_its_own_change() {
        let list: ListenerList<i32> = ListenerList::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = calls.clone();
        list.register("widget-a", 0, true, move |_, _, _, _| {
            calls2.fetch_add(1, Ordering::SeqCst);
        });
        list.on_maybe_change(&1, &2, true, "widget-a");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reregistering_the_same_source_replaces_the_old_listener() {
        let list: ListenerList<i32> = ListenerList::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let c1 = calls.clone();
        list.register("widget-a", 0, false, move |_, _, _, _| {
            c1.fetch_add(1, Ordering::SeqCst);
        });
        let c2 = calls.clone();
        list.register("widget-a", 0, false, move |_, _, _, _| {
            c2.fetch_add(100, Ordering::SeqCst);
        });
        list.on_maybe_change(&1, &2, true, "widget-b");
        assert_eq!(calls.load(Ordering::SeqCst), 100, "only the newest registration for a source should fire");
    }

    #[test]
    fn no_change_short_circuits() {
        let list: ListenerList<i32> = ListenerList::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let c1 = calls.clone();
        list.register("widget-a", 0, true, move |_, _, _, _| {
            c1.fetch_add(1, Ordering::SeqCst);
        });
        list.on_maybe_change(&5, &5, true, "widget-b");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
