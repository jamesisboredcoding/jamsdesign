//! Undo/redo by whole-document snapshots, like `jd-raster`'s history.
//!
//! Snapshots are cheap: a `Document` clone shares every node (`Arc`), so a step only costs the
//! node map plus the nodes the edit replaced.

use crate::Document;

const DEFAULT_LIMIT: usize = 200;

#[derive(Debug, Clone)]
struct Step {
    label: String,
    /// The document before this step.
    before: Document,
}

#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Edits sharing this key (one drag, one slider) collapse into a single step.
    gesture: Option<String>,
    /// `undo.len()` when last saved; `None` if that state can't be reached any more.
    saved_at: Option<usize>,
    limit: usize,
}

impl History {
    /// History for a document just opened or saved (counts as saved).
    pub fn new_saved() -> Self {
        Self { saved_at: Some(0), ..Self::default() }
    }

    /// Call before changing `doc`. Consecutive calls with the same `gesture` make one step
    /// (until [`end_gesture`](Self::end_gesture)).
    pub fn record(&mut self, label: &str, doc: &Document, gesture: Option<&str>) {
        if let Some(key) = gesture
            && self.gesture.as_deref() == Some(key)
        {
            return;
        }
        self.gesture = gesture.map(str::to_owned);
        if self.saved_at.is_some_and(|s| s > self.undo.len()) {
            self.saved_at = None;
        }
        self.redo.clear();
        self.undo.push(Step { label: label.to_owned(), before: doc.clone() });
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
    pub fn undo(&mut self, doc: &mut Document) -> Option<String> {
        self.gesture = None;
        let step = self.undo.pop()?;
        let after = std::mem::replace(doc, step.before);
        self.redo.push(Step { label: step.label.clone(), before: after });
        Some(step.label)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<String> {
        self.gesture = None;
        let step = self.redo.pop()?;
        let before = std::mem::replace(doc, step.before);
        self.undo.push(Step { label: step.label.clone(), before });
        Some(step.label)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Labels of the undoable steps, oldest first.
    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.undo.iter().map(|s| s.label.as_str())
    }

    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.undo.len());
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo.len())
    }

    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit.max(1);
    }
}

/// A document with its history: apply a labelled batch of ops as one undoable step.
#[derive(Debug, Clone, Default)]
pub struct Editor {
    pub doc: Document,
    pub history: History,
}

impl Editor {
    pub fn new(doc: Document) -> Self {
        Self { doc, history: History::new_saved() }
    }

    /// Apply `ops` as one undo step named `label`. A failed batch leaves no step behind.
    pub fn apply(
        &mut self,
        label: &str,
        ops: &[crate::Op],
        measure: &dyn crate::TextMeasure,
    ) -> Result<crate::Applied, crate::OpError> {
        let before = self.doc.clone();
        let done = self.doc.apply(ops, measure)?;
        self.history.record(label, &before, None);
        Ok(done)
    }

    /// Like [`apply`](Self::apply), but consecutive batches with the same `gesture` key (every
    /// step of one drag) make a single undo step. Call [`end_gesture`](Self::end_gesture) when
    /// the gesture ends.
    pub fn apply_gesture(
        &mut self,
        label: &str,
        gesture: &str,
        ops: &[crate::Op],
        measure: &dyn crate::TextMeasure,
    ) -> Result<crate::Applied, crate::OpError> {
        let before = self.doc.clone();
        let done = self.doc.apply(ops, measure)?;
        self.history.record(label, &before, Some(gesture));
        Ok(done)
    }

    pub fn end_gesture(&mut self) {
        self.history.end_gesture();
    }

    pub fn undo(&mut self) -> Option<String> {
        self.history.undo(&mut self.doc)
    }

    pub fn redo(&mut self) -> Option<String> {
        self.history.redo(&mut self.doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::NodeSpec;
    use crate::{ApproxMeasure, Op};

    #[test]
    fn undo_and_redo_restore_whole_batches() {
        let mut ed = Editor::new(Document::new());
        let p = ed.doc.pages()[0];
        assert!(!ed.history.is_dirty());
        ed.apply("Add frame", &[Op::create(p, NodeSpec::frame("F", 10.0, 10.0)), Op::create(p, NodeSpec::rect("R", 5.0, 5.0))], &ApproxMeasure)
            .unwrap();
        assert_eq!(ed.doc.len(), 3);
        assert!(ed.history.is_dirty());
        assert_eq!(ed.undo().as_deref(), Some("Add frame"));
        assert_eq!(ed.doc.len(), 1);
        assert!(!ed.history.is_dirty());
        assert_eq!(ed.redo().as_deref(), Some("Add frame"));
        assert_eq!(ed.doc.len(), 3);
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut ed = Editor::new(Document::new());
        let p = ed.doc.pages()[0];
        let r = ed.apply("Add", &[Op::create(p, NodeSpec::rect("R", 5.0, 5.0))], &ApproxMeasure).unwrap().created[0];
        for x in 1..=5 {
            let patch = crate::Patch { x: Some(x as f64 * 10.0), ..crate::Patch::default() };
            ed.apply_gesture("Move", "drag", &[Op::set(r, patch)], &ApproxMeasure).unwrap();
        }
        ed.end_gesture();
        assert_eq!(ed.doc.node(r).unwrap().props.x, 50.0);
        assert_eq!(ed.undo().as_deref(), Some("Move"));
        assert_eq!(ed.doc.node(r).unwrap().props.x, 0.0, "the whole drag undid at once");
        assert_eq!(ed.undo().as_deref(), Some("Add"));
    }

    #[test]
    fn failed_batches_leave_no_step() {
        let mut ed = Editor::new(Document::new());
        assert!(ed.apply("Bad", &[Op::Delete { id: crate::NodeId(999).into() }], &ApproxMeasure).is_err());
        assert!(!ed.history.can_undo());
    }
}
