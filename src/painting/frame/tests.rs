//! Tests for the frame painter, one module per source file beside this one.

use crate::painting::frame::grid::{Frame, Row};

mod builder;
mod grid;
mod layout;
mod token;
mod write;

/// Each row's graphemes joined, escapes included.
fn texts(frame: &Frame) -> Vec<String> {
    frame
        .rows
        .iter()
        .map(|row| row.cells.iter().map(|c| c.grapheme.as_str()).collect())
        .collect()
}

/// The buffer index of each cell in `row`.
fn indices(row: &Row) -> Vec<Option<usize>> {
    row.cells.iter().map(|c| c.index).collect()
}
