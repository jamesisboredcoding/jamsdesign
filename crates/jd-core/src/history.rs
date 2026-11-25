//! Undo/redo by whole snapshots, shared by every app: Photosplash's raster documents,
//! Blobr's vector documents and Cinematica's sequences. Snapshots are cheap because the
//! documents share their heavy parts (`Arc` tiles and nodes).

const DEFAULT_LIMIT: usize = 200;

#[derive(Debug, Clone)]
struct Step<T> {
    label: String,
    /// The state before this step (on the redo stack: the state to restore).
    before: T,
}

#[derive(Debug, Clone)]
pub struct History<T> {
    undo: Vec<Step<T>>,
    /// Steps undone, most recent last.
    redo: Vec<Step<T>>,
    /// Edits sharing this key (one drag, one slider) collapse into one step.
    gesture: Option<String>,
    /// `undo.len()` when last saved; `None` if that state can't be reached any more.
    saved_at: Option<usize>,
    /// Counts every change (new or merged edits, undo, redo), so a save that ran in the
    /// background can tell whether the document changed meanwhile.
    version: u64,
    /// Most undo steps kept (0: the default).
    limit: usize,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), gesture: None, saved_at: None, version: 0, limit: 0 }
    }
}

impl<T> History<T> {
    /// History for a document just opened or saved (counts as saved).
    pub fn new_saved() -> Self {
        Self { saved_at: Some(0), ..Self::default() }
    }

    /// Call before changing the document, with its current state. Consecutive calls with
    /// the same `gesture` make one step (until [`end_gesture`](Self::end_gesture)).
    pub fn record(&mut self, label: &str, before: &T, gesture: Option<&str>)
    where
        T: Clone,
    {
        self.version += 1;
        if let Some(key) = gesture
            && self.gesture.as_deref() == Some(key)
        {
            return;
        }
        self.gesture = gesture.map(str::to_owned);
        self.add(label.to_owned(), before.clone());
    }

    /// Records the state from before an edit called `label` (no gesture).
    pub fn push(&mut self, label: impl Into<String>, before: T) {
        self.version += 1;
        self.gesture = None;
        self.add(label.into(), before);
    }

    fn add(&mut self, label: String, before: T) {
        if self.saved_at.is_some_and(|s| s > self.undo.len()) {
            // The saved state was in the redo branch we're about to throw away.
            self.saved_at = None;
        }
        self.redo.clear();
        self.undo.push(Step { label, before });
        self.trim();
    }

    /// Sets how many undo steps are kept, dropping the oldest if there are more.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(1);
        self.trim();
    }

    fn trim(&mut self) {
        let limit = if self.limit == 0 { DEFAULT_LIMIT } else { self.limit };
        if self.undo.len() > limit {
            let drop = self.undo.len() - limit;
            self.undo.drain(..drop);
            self.saved_at = self.saved_at.and_then(|s| s.checked_sub(drop));
        }
    }

    pub fn end_gesture(&mut self) {
        self.gesture = None;
    }

    /// Restores the state before the last step. Returns its label.
    pub fn undo(&mut self, current: &mut T) -> Option<String> {
        self.gesture = None;
        let step = self.undo.pop()?;
        self.version += 1;
        let after = std::mem::replace(current, step.before);
        self.redo.push(Step { label: step.label.clone(), before: after });
        Some(step.label)
    }

    pub fn redo(&mut self, current: &mut T) -> Option<String> {
        self.gesture = None;
        let step = self.redo.pop()?;
        self.version += 1;
        let before = std::mem::replace(current, step.before);
        self.undo.push(Step { label: step.label.clone(), before });
        Some(step.label)
    }

    /// Undo or redo until exactly `done` steps are applied.
    pub fn jump_to(&mut self, done: usize, current: &mut T) {
        while self.undo.len() > done && self.undo(current).is_some() {}
        while self.undo.len() < done && self.redo(current).is_some() {}
    }

    /// Folds the last step into the one before it (two batches that are one action for the
    /// user, like creating a style and then using it).
    pub fn merge_last(&mut self) {
        if self.undo.len() >= 2 {
            self.undo.pop();
            self.gesture = None;
            if self.saved_at.is_some_and(|s| s > self.undo.len()) {
                self.saved_at = None;
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    /// Labels for a history panel: done steps (oldest first), then undone ones (next first).
    pub fn labels(&self) -> (Vec<&str>, Vec<&str>) {
        (self.undo.iter().map(|s| s.label.as_str()).collect(), self.redo.iter().rev().map(|s| s.label.as_str()).collect())
    }

    /// Number of applied steps.
    pub fn depth(&self) -> usize {
        self.undo.len()
    }

    /// Changes so far; take it when a background save starts and hand it to
    /// [`mark_saved_if`](Self::mark_saved_if) when the save is done.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The current state is on disk. Ends a gesture, so later edits count as unsaved.
    pub fn mark_saved(&mut self) {
        self.gesture = None;
        self.saved_at = Some(self.undo.len());
    }

    /// A save of the state at `version` finished. If the document changed meanwhile, what's
    /// on disk is no state we can return to, so nothing counts as saved.
    pub fn mark_saved_if(&mut self, version: u64) {
        if version == self.version {
            self.mark_saved();
        } else {
            self.saved_at = None;
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_and_saved_state() {
        let mut h = History::new_saved();
        let mut v = 1;
        assert!(!h.is_dirty());
        h.record("a", &v, None);
        v = 2;
        assert!(h.is_dirty());
        assert_eq!(h.undo(&mut v).as_deref(), Some("a"));
        assert_eq!(v, 1);
        assert!(!h.is_dirty());
        assert_eq!(h.redo(&mut v).as_deref(), Some("a"));
        assert_eq!((v, h.redo(&mut v)), (2, None));
        h.mark_saved();
        h.undo(&mut v);
        h.push("b", v);
        v = 3;
        assert!(!h.can_redo());
        h.undo(&mut v);
        assert!(h.is_dirty(), "the saved state was thrown away with the redo branch");
    }

    #[test]
    fn gestures_coalesce_until_ended() {
        let mut h = History::default();
        let mut v = 0;
        for x in 1..=3 {
            h.record("Drag", &v, Some("drag"));
            v = x;
        }
        h.end_gesture();
        h.record("Drag", &v, Some("drag"));
        v = 9;
        assert_eq!(h.labels().0.len(), 2);
        h.undo(&mut v);
        assert_eq!(v, 3);
        h.jump_to(0, &mut v);
        assert_eq!(v, 0);
        h.jump_to(2, &mut v);
        assert_eq!(v, 9);
    }

    #[test]
    fn saving_mid_gesture_counts_later_edits() {
        let mut h = History::new_saved();
        let mut v = 0;
        h.record("Drag", &v, Some("drag"));
        v = 1;
        h.mark_saved();
        h.record("Drag", &v, Some("drag"));
        v = 2;
        assert!(h.is_dirty(), "the rest of the drag isn't on disk");
        let _ = v;
    }

    #[test]
    fn a_background_save_overtaken_by_edits_leaves_it_dirty() {
        // Full history: one edit during the save drops the oldest step, so the step count
        // matches the one the save started at even though the state differs.
        let mut h = History::new_saved();
        h.set_limit(3);
        let mut v = 0;
        for x in 1..=3 {
            h.record("x", &v, None);
            v = x;
        }
        let at_start = h.version();
        let depth = h.depth();
        h.record("stroke", &v, None);
        assert_eq!(h.depth(), depth);
        h.mark_saved_if(at_start);
        assert!(h.is_dirty());
        // A save with no edits meanwhile counts.
        let now = h.version();
        h.mark_saved_if(now);
        assert!(!h.is_dirty());
    }
}
