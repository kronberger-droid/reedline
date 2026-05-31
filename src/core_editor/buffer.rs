use super::selection::Selection;
use super::LineBuffer;

/// Owns the editable text and the selection.
///
/// A thin coordinator between [`Editor`](super::Editor) and [`LineBuffer`].
/// The selection's *head* is the line buffer's insertion point; this type
/// stores the *anchor* and materializes a [`Selection`] on demand. (Once
/// the cursor moves into the buffer wholesale, head moves here too.)
pub(crate) struct Buffer {
    pub(crate) line_buffer: LineBuffer,
    /// Anchor of the active selection; `None` when nothing is selected.
    anchor: Option<usize>,
}

impl Buffer {
    pub(crate) fn new() -> Self {
        Self {
            line_buffer: LineBuffer::new(),
            anchor: None,
        }
    }

    /// The anchor of the active selection, if any.
    pub(crate) fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    /// Pin or clear the selection anchor directly.
    pub(crate) fn set_anchor(&mut self, anchor: Option<usize>) {
        self.anchor = anchor;
    }

    /// The current selection: head at the cursor, anchor where it was
    /// pinned — or the cursor itself when nothing is selected.
    pub(super) fn selection(&self) -> Selection {
        let head = self.line_buffer.insertion_point();
        Selection::new(self.anchor.unwrap_or(head), head)
    }

    /// Commit a [`Selection`]: move the cursor to its head and pin the
    /// anchor. An empty range clears the selection.
    pub(super) fn set_selection(&mut self, selection: Selection) {
        self.line_buffer.set_insertion_point(selection.head());
        self.anchor = (!selection.is_empty()).then(|| selection.anchor());
    }
}
