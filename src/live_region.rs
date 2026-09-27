use std::sync::{Arc, Mutex, MutexGuard};

/// Handle to the live region: host-defined lines drawn above the prompt,
/// repainted in place and never committed to scrollback.
///
/// The host keeps one handle and gives clones to the threads that produce
/// status. Those call [`set`](Self::set) whenever their lines change;
/// `read_line` reads the handle once per poll and repaints if anything is
/// pending. Committed output, lines that should scroll into history, goes
/// through [`ExternalPrinter`](crate::ExternalPrinter) instead.
///
/// ```
/// use reedline::{LiveRegion, Reedline};
///
/// let region = LiveRegion::default();
/// let editor = Reedline::create().with_live_region(region.clone());
///
/// // From any thread, at any time:
/// region.set(vec!["job 3: compiling 41/120".into()]);
/// # let _ = editor;
/// ```
#[derive(Clone, Default, Debug)]
pub struct LiveRegion {
    pending: Arc<Mutex<Option<Vec<String>>>>,
}

impl LiveRegion {
    /// Replace the region with `lines`, one row each.
    ///
    /// Updates coalesce: several calls between two polls leave one pending
    /// update holding the last `lines`, and the editor repaints once. A
    /// producer can therefore call this as often as it likes.
    pub fn set(&self, lines: Vec<String>) {
        *self.slot() = Some(lines);
    }

    /// Remove every line from the region.
    ///
    /// This is an update like any other: the next poll sees it and repaints
    /// without the rows.
    pub fn clear(&self) {
        self.set(Vec::new());
    }

    /// Take the pending update, if any, leaving nothing pending.
    ///
    /// `None` means nothing changed since the last call. `Some` with an empty
    /// vector is a [`clear`](Self::clear) and must be applied.
    pub(crate) fn take_update(&self) -> Option<Vec<String>> {
        self.slot().take()
    }

    /// Whether a producer holds a clone of this handle.
    ///
    /// The engine only needs to poll for updates while someone can make
    /// them; a handle nobody cloned costs nothing.
    pub(crate) fn is_shared(&self) -> bool {
        Arc::strong_count(&self.pending) > 1
    }

    /// Lock the slot. A producer that panicked mid-`set` poisons the mutex,
    /// but the critical section is one assignment, so the slot is always a
    /// whole value and the poison can be ignored.
    fn slot(&self) -> MutexGuard<'_, Option<Vec<String>>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_handle_has_nothing_pending() {
        let region = LiveRegion::default();
        assert_eq!(region.take_update(), None);
    }

    #[test]
    fn sets_between_polls_coalesce_to_the_last_one() {
        let region = LiveRegion::default();
        region.set(vec!["one".into()]);
        region.set(vec!["two".into(), "three".into()]);

        assert_eq!(
            region.take_update(),
            Some(vec!["two".to_string(), "three".to_string()])
        );
        assert_eq!(region.take_update(), None);
    }

    #[test]
    fn clear_is_an_update_with_no_lines() {
        let region = LiveRegion::default();
        region.set(vec!["status".into()]);
        region.clear();

        assert_eq!(region.take_update(), Some(Vec::new()));
    }

    #[test]
    fn a_clone_writes_to_the_same_slot() {
        let region = LiveRegion::default();
        let producer = region.clone();
        producer.set(vec!["from the clone".into()]);

        assert_eq!(
            region.take_update(),
            Some(vec!["from the clone".to_string()])
        );
    }

    #[test]
    fn is_shared_follows_the_clones() {
        let region = LiveRegion::default();
        assert!(!region.is_shared());

        let producer = region.clone();
        assert!(region.is_shared());

        drop(producer);
        assert!(!region.is_shared());
    }

    #[test]
    fn a_poisoned_slot_still_serves_the_last_value() {
        let region = LiveRegion::default();
        let poisoner = region.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.pending.lock().unwrap();
            panic!("poison the slot");
        })
        .join();

        region.set(vec!["after".into()]);
        assert_eq!(region.take_update(), Some(vec!["after".to_string()]));
    }
}
