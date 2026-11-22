//! Undo/redo by whole-document snapshots.
//!
//! Snapshots are cheap: a `Document` clone shares every tile (`Arc`), so a step only costs
//! the layer list plus whatever tiles the edit actually replaced.

use crate::Document;

const LIMIT: usize = 100;

#[derive(Debug, Clone)]
struct Step {
    label: String,
    /// The document *before* this step was applied.
    before: Document,
}

#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Step>,
    /// Steps undone, most recent last. Each holds the state to restore on redo.
    redo: Vec<Step>,
    /// Edits sharing this key (e.g. one slider drag) collapse into one step.
    gesture: Option<String>,
    /// `undo.len()` when last saved; `None` if that state is no longer reachable.
    saved_at: Option<usize>,
}

impl History {
    /// History for a document freshly opened from disk (counts as saved).
    pub fn new_saved() -> Self {
        Self { saved_at: Some(0), ..Self::default() }
    }

    /// Call *before* mutating `doc`. With a `gesture` key, consecutive edits with the same key
    /// become a single undo step (until [`end_gesture`](Self::end_gesture)).
    pub fn record(&mut self, label: &str, doc: &Document, gesture: Option<&str>) {
        if let Some(key) = gesture
            && self.gesture.as_deref() == Some(key)
        {
            return;
        }
        self.gesture = gesture.map(str::to_owned);
        if self.saved_at.is_some_and(|s| s > self.undo.len()) {
            // The saved state was in the redo branch we're about to throw away.
            self.saved_at = None;
        }
        self.redo.clear();
        self.undo.push(Step { label: label.to_owned(), before: doc.clone() });
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
            self.saved_at = self.saved_at.and_then(|s| s.checked_sub(1));
        }
    }

    pub fn end_gesture(&mut self) {
        self.gesture = None;
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        self.gesture = None;
        let Some(step) = self.undo.pop() else { return false };
        let after = std::mem::replace(doc, step.before);
        self.redo.push(Step { label: step.label, before: after });
        true
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        self.gesture = None;
        let Some(step) = self.redo.pop() else { return false };
        let before = std::mem::replace(doc, step.before);
        self.undo.push(Step { label: step.label, before });
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn mark_saved(&mut self) {
        self.mark_saved_at(self.undo.len());
    }

    /// Number of applied steps; identifies the current state for [`mark_saved_at`](Self::mark_saved_at).
    pub fn depth(&self) -> usize {
        self.undo.len()
    }

    /// Marks the state at `depth` as saved (a background save started at that depth).
    pub fn mark_saved_at(&mut self, depth: usize) {
        self.gesture = None;
        self.saved_at = Some(depth);
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo.len())
    }

    /// Labels for a history panel: done steps (oldest first), then undone ones (next first).
    /// Returns (done, undone).
    pub fn labels(&self) -> (Vec<&str>, Vec<&str>) {
        (
            self.undo.iter().map(|s| s.label.as_str()).collect(),
            self.redo.iter().rev().map(|s| s.label.as_str()).collect(),
        )
    }

    /// Undo or redo until exactly `done` steps are applied.
    pub fn jump_to(&mut self, done: usize, doc: &mut Document) {
        while self.undo.len() > done && self.undo(doc) {}
        while self.undo.len() < done && self.redo(doc) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opacity(doc: &Document) -> f32 {
        doc.layers()[0].opacity
    }

    fn set(doc: &mut Document, v: f32) {
        let id = doc.layers()[0].id;
        doc.layer_mut(id).unwrap().opacity = v;
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut doc = Document::new("t", 4, 4);
        let mut h = History::new_saved();
        assert!(!h.is_dirty());
        h.record("Opacity", &doc, None);
        set(&mut doc, 0.5);
        assert!(h.is_dirty());
        assert!(h.undo(&mut doc));
        assert_eq!(opacity(&doc), 1.0);
        assert!(!h.is_dirty());
        assert!(h.redo(&mut doc));
        assert_eq!(opacity(&doc), 0.5);
        assert!(!h.redo(&mut doc));
    }

    #[test]
    fn gestures_coalesce() {
        let mut doc = Document::new("t", 4, 4);
        let mut h = History::default();
        for v in [0.9, 0.8, 0.7] {
            h.record("Opacity", &doc, Some("opacity"));
            set(&mut doc, v);
        }
        h.end_gesture();
        h.record("Opacity", &doc, Some("opacity"));
        set(&mut doc, 0.1);
        assert_eq!(h.labels().0.len(), 2);
        h.undo(&mut doc);
        assert_eq!(opacity(&doc), 0.7);
        h.undo(&mut doc);
        assert_eq!(opacity(&doc), 1.0);
    }

    #[test]
    fn new_edit_after_undo_drops_redo_and_saved_state() {
        let mut doc = Document::new("t", 4, 4);
        let mut h = History::new_saved();
        h.record("a", &doc, None);
        set(&mut doc, 0.5);
        h.mark_saved();
        h.undo(&mut doc);
        h.record("b", &doc, None);
        set(&mut doc, 0.2);
        assert!(!h.can_redo());
        assert!(h.is_dirty());
        h.undo(&mut doc);
        assert!(h.is_dirty(), "saved state (0.5) is gone, so nothing is clean");
    }

    #[test]
    fn jump_to_moves_both_ways() {
        let mut doc = Document::new("t", 4, 4);
        let mut h = History::default();
        for v in [0.9, 0.8, 0.7] {
            h.record("x", &doc, None);
            set(&mut doc, v);
        }
        h.jump_to(1, &mut doc);
        assert_eq!(opacity(&doc), 0.9);
        h.jump_to(3, &mut doc);
        assert_eq!(opacity(&doc), 0.7);
    }
}
