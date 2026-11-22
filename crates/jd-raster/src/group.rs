//! Layer groups (Photoshop's folders), kept in the flat bottom-to-top layer list: a group's
//! contents sit directly below it, and each layer names its group in [`Layer::parent`]. A
//! group and everything inside it is a contiguous *block* ending with the group itself.
//!
//! How groups look is in [`crate::render`].

use std::ops::RangeInclusive;

use crate::{Document, Group, Layer, LayerId};

impl Layer {
    pub fn is_group(&self) -> bool {
        self.group.is_some()
    }
}

impl Document {
    /// Index of the layer `id`.
    pub fn index_of_layer(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// Whether `id` is `ancestor` or inside it (at any depth).
    pub fn is_within(&self, id: LayerId, ancestor: LayerId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.layer(c).and_then(|l| l.parent);
        }
        false
    }

    /// The block of layer `index`: itself, plus everything inside it if it's a group.
    pub fn block(&self, index: usize) -> RangeInclusive<usize> {
        let id = self.layers[index].id;
        let mut start = index;
        while start > 0 && self.is_within(self.layers[start - 1].id, id) {
            start -= 1;
        }
        start..=index
    }

    /// How deeply a layer is nested (0: not in a group).
    pub fn depth(&self, id: LayerId) -> usize {
        let mut d = 0;
        let mut cur = self.layer(id).and_then(|l| l.parent);
        while let Some(c) = cur {
            d += 1;
            cur = self.layer(c).and_then(|l| l.parent);
        }
        d
    }

    /// Visible itself and in every group around it.
    pub fn effectively_visible(&self, id: LayerId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.layer(c) {
                Some(l) if l.visible => cur = l.parent,
                _ => return false,
            }
        }
        true
    }

    /// Layer > New > Group: an empty group above the active layer.
    pub fn add_group(&mut self, name: impl Into<String>) -> LayerId {
        let id = self.add_layer(name);
        if let Some(l) = self.layer_mut(id) {
            l.group = Some(Group { pass_through: true, open: true });
        }
        id
    }

    /// Group Layers (Ctrl+G): puts the layer `id` (and its contents) into a new group in its
    /// place. Returns the group.
    pub fn group_layer(&mut self, id: LayerId, name: impl Into<String>) -> Option<LayerId> {
        let index = self.index_of_layer(id)?;
        let parent = self.layers[index].parent;
        let group_id = LayerId(self.next_id);
        self.next_id += 1;
        let mut group = Layer::new(group_id, name.into(), self.width, self.height);
        group.group = Some(Group { pass_through: true, open: true });
        group.parent = parent;
        self.layers[index].parent = Some(group_id);
        self.layers.insert(index + 1, group);
        self.active = Some(group_id);
        self.mask_target = false;
        Some(group_id)
    }

    /// Ungroup (Shift+Ctrl+G): removes the group, keeping what was inside it in its place.
    pub fn ungroup(&mut self, id: LayerId) -> bool {
        let Some(index) = self.index_of_layer(id) else { return false };
        if !self.layers[index].is_group() {
            return false;
        }
        let parent = self.layers[index].parent;
        for l in &mut self.layers {
            if l.parent == Some(id) {
                l.parent = parent;
            }
        }
        self.layers.remove(index);
        // The top layer that was inside it becomes active (or the one below).
        let next = index.saturating_sub(1).min(self.layers.len().saturating_sub(1));
        self.active = self.layers.get(next).map(|l| l.id);
        self.mask_target = false;
        true
    }

    /// Moves the block of `id` so it lands at `slot` (an insertion index in the current list,
    /// 0 = bottom) inside `parent`. Refuses to move a group into itself.
    pub fn move_block(&mut self, id: LayerId, slot: usize, parent: Option<LayerId>) -> bool {
        let Some(index) = self.index_of_layer(id) else { return false };
        if parent.is_some_and(|p| self.is_within(p, id)) || parent.is_some_and(|p| !self.layer(p).is_some_and(Layer::is_group)) {
            return false;
        }
        let block = self.block(index);
        let (start, end) = (*block.start(), *block.end());
        if slot >= start && slot <= end + 1 && self.layers[index].parent == parent {
            return false;
        }
        if slot > start && slot <= end {
            return false;
        }
        let moved: Vec<Layer> = self.layers.drain(start..=end).collect();
        let at = if slot > end { slot - moved.len() } else { slot };
        let at = at.min(self.layers.len());
        let top = moved.len() - 1;
        for (k, mut l) in moved.into_iter().enumerate() {
            if k == top {
                l.parent = parent;
            }
            self.layers.insert(at + k, l);
        }
        true
    }

    /// Moves the block of `id` past its neighbouring sibling block, `delta` times up (+) or
    /// down (-). Returns whether it moved.
    pub fn move_layer(&mut self, id: LayerId, delta: isize) -> bool {
        let mut moved = false;
        for _ in 0..delta.unsigned_abs() {
            let Some(index) = self.index_of_layer(id) else { break };
            let parent = self.layers[index].parent;
            let block = self.block(index);
            let slot = if delta > 0 {
                // The sibling block above starts right after ours (unless that's our own
                // group's entry: we're already at its top) and ends with the sibling itself.
                let above = index + 1;
                if above >= self.layers.len() || Some(self.layers[above].id) == parent {
                    break;
                }
                let Some(end) = (above..self.layers.len()).find(|&j| self.layers[j].parent == parent) else { break };
                end + 1
            } else {
                let start = *block.start();
                if start == 0 {
                    break;
                }
                let below = start - 1;
                if self.layers[below].parent != parent {
                    break;
                }
                *self.block(below).start()
            };
            if !self.move_block(id, slot, parent) {
                break;
            }
            moved = true;
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(d: &Document) -> Vec<String> {
        d.layers().iter().map(|l| format!("{}{}", "-".repeat(d.depth(l.id)), l.name)).collect()
    }

    fn doc() -> (Document, [LayerId; 3]) {
        let mut d = Document::blank("t", 10, 10);
        let a = d.add_layer("a");
        let b = d.add_layer("b");
        let c = d.add_layer("c");
        (d, [a, b, c])
    }

    #[test]
    fn group_ungroup_and_blocks() {
        let (mut d, [_, b, c]) = doc();
        let g = d.group_layer(b, "G").unwrap();
        assert_eq!(names(&d), ["a", "-b", "G", "c"]);
        assert_eq!(d.block(2), 1..=2);
        // New layers go next to the active one, in the same group.
        d.set_active(b);
        d.add_layer("b2");
        assert_eq!(names(&d), ["a", "-b", "-b2", "G", "c"]);
        assert!(d.effectively_visible(b));
        d.layer_mut(g).unwrap().visible = false;
        assert!(!d.effectively_visible(b));
        // Moving the group moves its contents too.
        assert!(d.move_layer(g, -1));
        assert_eq!(names(&d), ["-b", "-b2", "G", "a", "c"]);
        assert!(d.move_layer(g, 1));
        assert!(d.move_layer(g, 1));
        assert_eq!(names(&d), ["a", "c", "-b", "-b2", "G"]);
        // Dragging c into the group (just below the group entry: its top).
        assert!(d.move_block(c, 4, Some(g)));
        assert_eq!(names(&d), ["a", "-b", "-b2", "-c", "G"]);
        // A group can't go inside itself.
        assert!(!d.move_block(g, 1, Some(g)));
        assert!(d.ungroup(g));
        assert_eq!(names(&d), ["a", "b", "b2", "c"]);
    }

    #[test]
    fn merging_groups() {
        use crate::composite::pixel_srgb8;
        let mut d = Document::from_rgba8("t", 10, 10, &[0, 0, 255, 255].repeat(100));
        let a = d.add_layer("a");
        d.layer_mut(a).unwrap().pixels.set_pixel(1, 1, [255, 0, 0, 255]);
        let b = d.add_layer("b");
        d.layer_mut(b).unwrap().pixels.set_pixel(2, 2, [0, 255, 0, 255]);
        let g = d.group_layer(b, "G").unwrap();
        d.set_active(a);
        d.move_block(a, 2, Some(g));
        assert_eq!(names(&d), ["Background", "-a", "-b", "G"]);
        let before = [pixel_srgb8(&d, 1, 1), pixel_srgb8(&d, 2, 2)];
        // Merge Down on a group merges the group.
        d.set_active(g);
        assert!(d.merge_down());
        assert_eq!(names(&d), ["Background", "G"]);
        assert_eq!([pixel_srgb8(&d, 1, 1), pixel_srgb8(&d, 2, 2)], before);

        // Merge Visible leaves hidden groups (and what's in them) alone.
        let c = d.add_layer("c");
        let h = d.group_layer(c, "H").unwrap();
        d.layer_mut(h).unwrap().visible = false;
        assert!(d.merge_visible());
        assert_eq!(names(&d), ["Background", "-c", "H"]);
        assert_eq!(pixel_srgb8(&d, 2, 2), before[1]);
    }

    #[test]
    fn deleting_and_duplicating_a_group_takes_its_contents() {
        let (mut d, [_, b, _]) = doc();
        let g = d.group_layer(b, "G").unwrap();
        let copy = d.duplicate_layer(g).unwrap();
        assert_eq!(names(&d), ["a", "-b", "G", "-b", "G copy", "c"], "only the group is renamed, like Photoshop");
        assert!(d.layers()[3].parent == Some(copy));
        d.remove_layer(g);
        assert_eq!(names(&d), ["a", "-b", "G copy", "c"]);
    }
}
