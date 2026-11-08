//! Bounded undo / redo built on copy-on-write snapshots.
//!
//! Layers hold their buffers in `Arc`s, so a snapshot of the whole document
//! costs a few pointer copies; only buffers that an operation subsequently
//! modifies are duplicated. History is bounded by both entry count and an
//! estimate of the *unique* memory it retains.

use crate::document::Document;
use crate::image::Plane;
use std::collections::HashSet;

#[derive(Clone)]
pub struct Snapshot {
    pub label: String,
    pub doc: Document,
    pub selection: Option<Plane>,
}

pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for History {
    fn default() -> Self {
        History { undo: Vec::new(), redo: Vec::new(), max_entries: 80, max_bytes: 3 << 30 }
    }
}

impl History {
    pub fn push(&mut self, s: Snapshot, current: &Document) {
        self.undo.push(s);
        self.redo.clear();
        self.trim(current);
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
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
    /// Pop the last undo entry, pushing `current` onto the redo stack.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let s = self.undo.pop()?;
        self.redo.push(Snapshot { label: s.label.clone(), ..current });
        Some(s)
    }
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let s = self.redo.pop()?;
        self.undo.push(Snapshot { label: s.label.clone(), ..current });
        Some(s)
    }
    pub fn len(&self) -> usize {
        self.undo.len()
    }
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    /// Unique bytes retained by history beyond the current document.
    pub fn retained_bytes(&self, current: &Document) -> usize {
        let mut seen: HashSet<usize> = HashSet::new();
        let mut note = |p: usize| seen.insert(p);
        for l in &current.layers {
            note(std::sync::Arc::as_ptr(&l.pixels.data) as usize);
            if let Some(m) = &l.mask {
                note(std::sync::Arc::as_ptr(&m.data) as usize);
            }
            if let Some(o) = &l.original {
                note(std::sync::Arc::as_ptr(&o.data) as usize);
            }
        }
        let mut total = 0;
        for s in self.undo.iter().chain(self.redo.iter()) {
            for l in &s.doc.layers {
                if seen.insert(std::sync::Arc::as_ptr(&l.pixels.data) as usize) {
                    total += l.pixels.bytes();
                }
                if let Some(m) = &l.mask {
                    if seen.insert(std::sync::Arc::as_ptr(&m.data) as usize) {
                        total += m.data.len();
                    }
                }
                if let Some(o) = &l.original {
                    if seen.insert(std::sync::Arc::as_ptr(&o.data) as usize) {
                        total += o.bytes();
                    }
                }
            }
            if let Some(sel) = &s.selection {
                if seen.insert(std::sync::Arc::as_ptr(&sel.data) as usize) {
                    total += sel.data.len();
                }
            }
        }
        total
    }

    fn trim(&mut self, current: &Document) {
        while self.undo.len() > self.max_entries {
            self.undo.remove(0);
        }
        while self.undo.len() > 1 && self.retained_bytes(current) > self.max_bytes {
            self.undo.remove(0);
        }
    }
}
