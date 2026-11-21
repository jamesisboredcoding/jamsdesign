//! Snapshot undo/redo. Sequences are small (clips reference media, they don't hold it), so
//! whole snapshots are cheap and can't get out of step with the edits.

#[derive(Debug, Clone)]
pub struct History<T> {
    undo: Vec<(String, T)>,
    redo: Vec<(String, T)>,
    limit: usize,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit: 200 }
    }
}

impl<T> History<T> {
    /// Records the state from before an edit called `label`.
    pub fn push(&mut self, label: impl Into<String>, before: T) {
        self.undo.push((label.into(), before));
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Swaps `current` for the state before the last edit; returns the edit's label.
    pub fn undo(&mut self, current: &mut T) -> Option<String> {
        let (label, before) = self.undo.pop()?;
        let after = std::mem::replace(current, before);
        self.redo.push((label.clone(), after));
        Some(label)
    }

    pub fn redo(&mut self, current: &mut T) -> Option<String> {
        let (label, after) = self.redo.pop()?;
        let before = std::mem::replace(current, after);
        self.undo.push((label.clone(), before));
        Some(label)
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(l, _)| l.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(l, _)| l.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_round_trip_and_new_edits_clear_redo() {
        let mut h = History::default();
        let mut v = 1;
        h.push("one", v);
        v = 2;
        h.push("two", v);
        v = 3;
        assert_eq!(h.undo(&mut v).as_deref(), Some("two"));
        assert_eq!(v, 2);
        assert_eq!(h.undo(&mut v).as_deref(), Some("one"));
        assert_eq!(v, 1);
        assert_eq!(h.undo(&mut v), None);
        assert_eq!(h.redo(&mut v).as_deref(), Some("one"));
        assert_eq!(v, 2);
        h.push("three", v);
        v = 9;
        assert_eq!(h.redo(&mut v), None);
        assert_eq!(h.undo_label(), Some("three"));
    }
}
