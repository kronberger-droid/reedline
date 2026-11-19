//! The grapheme walk that fills a [`Frame`]'s rows.
//!
//! Every decision about which row a grapheme lands on goes through
//! [`advance_grapheme`], the same step the streaming painter maps clicks
//! with, so the frame and the old painter cannot count rows differently.

use crate::painting::{
    frame::{
        grid::{Cell, Position},
        token::{tokens, Token},
    },
    utils::advance_grapheme,
};

use super::grid::{Frame, Row};

/// Lays text out into rows of `width` columns, one `put` at a time.
///
/// `row` and `col` are where the next grapheme would be drawn, before any
/// wrap it still owes. After a grapheme fills a row, `col` equals `width`:
/// the terminal does not wrap until the next grapheme arrives, and neither
/// does the builder.
pub(super) struct Builder {
    width: u16,
    rows: Vec<Row>,
    row: u16,
    col: u16,
}

impl Builder {
    /// An empty walk: one empty row, nothing written.
    pub(super) fn new(width: u16) -> Self {
        Builder {
            width,
            rows: vec![Row::default()],
            row: 0,
            col: 0,
        }
    }

    /// The row the last grapheme was drawn on, before any wrap it owes.
    /// Unlike [`Builder::position`], a filled row is still this row.
    pub(super) fn row(&self) -> u16 {
        self.row
    }

    pub(super) fn rows_drawn(&self) -> u16 {
        if self.col == 0 && self.row != 0 {
            self.row
        } else {
            self.row + 1
        }
    }

    /// Where the next grapheme would start, which is where a cursor placed
    /// now belongs.
    ///
    /// A filled row resolves to the first column of the next one, since
    /// that is where the terminal puts the next grapheme. The row is not
    /// opened: if nothing follows, the frame keeps the full row as its last,
    /// unless [`Builder::frame`] is told the cursor is below it.
    pub(super) fn position(&self) -> Position {
        if self.col >= self.width {
            Position {
                row: self.row.saturating_add(1),
                col: 0,
            }
        } else {
            Position {
                row: self.row,
                col: self.col,
            }
        }
    }

    /// Append `text`, wrapping at the width and breaking rows at newlines.
    ///
    /// With `index`, each grapheme takes the next buffer index, and the
    /// counter is left past the last one, so a later `put` numbers on from
    /// there. A newline takes an index but no cell, thus a click maps past
    /// it; an escape takes a cell but no index, thus it shifts no click.
    /// Without `index`, as for prompt text, no cell is buffer text.
    pub(super) fn put(&mut self, text: &str, mut index: Option<&mut usize>) {
        for token in tokens(text) {
            match token {
                Token::Escape(seq) => self.escape(seq),
                Token::Grapheme(g) => {
                    let at = index.as_deref_mut().map(|i| std::mem::replace(i, *i + 1));
                    match g {
                        "\n" | "\r\n" => self.newline(),
                        _ => self.glyph(g, at),
                    }
                }
            }
        }
    }

    /// The finished frame, with its cursor at `cursor`.
    ///
    /// The cursor's row is opened if no grapheme reached it, as after a
    /// filled row, so [`Frame::cursor`] always names a row that exists.
    pub(super) fn frame(mut self, cursor: Position, prompt_rows: u16) -> Frame {
        self.ensure_row(cursor.row);
        Frame {
            rows: self.rows,
            cursor,
            prompt_rows,
        }
    }

    /// Draw one grapheme where [`advance_grapheme`] puts it, which may be
    /// the next row, and step past its width.
    fn glyph(&mut self, grapheme: &str, index: Option<usize>) {
        let ((row, col), width) = advance_grapheme((self.row, self.col), grapheme, self.width);
        self.ensure_row(row);
        self.rows[row as usize].cells.push(Cell {
            grapheme: String::from(grapheme),
            width,
            index,
        });
        self.row = row;
        self.col = col.saturating_add(width);
    }

    /// Keep `seq` where it was written, as a cell of no width.
    ///
    /// The frame does not interpret escapes yet. Keeping each in place means
    /// the frame writes the bytes in the order the terminal would have
    /// read them. A sequence that arrives while a wrap is owed stays on the
    /// filled row, where the terminal's cursor still is.
    fn escape(&mut self, seq: &str) {
        self.ensure_row(self.row);
        self.rows[self.row as usize].cells.push(Cell {
            grapheme: String::from(seq),
            width: 0,
            index: None,
        });
    }

    /// Open empty rows until `row` exists.
    fn ensure_row(&mut self, row: u16) {
        while self.rows.len() <= row as usize {
            self.rows.push(Row::default());
        }
    }

    /// Start the next row, opening it even if nothing is drawn on it.
    fn newline(&mut self) {
        self.col = 0;
        self.row = self.row.saturating_add(1);
        self.ensure_row(self.row);
    }
}
