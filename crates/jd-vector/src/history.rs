//! Undo/redo by whole-document snapshots (the shared `jd_core::History`), and [`Editor`].
//!
//! Snapshots are cheap: a `Document` clone shares every node (`Arc`), so a step only costs the
//! node map plus the nodes the edit replaced.

use crate::Document;

/// Undo/redo for vector documents.
pub type History = jd_core::History<Document>;

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
