//! The screen as data: [`Frame`] holds [`Row`]s, a row holds [`Cell`]s.
//!
//! Three different numbers name a place in a row, and none stands in for
//! another:
//!
//! ```text
//! cells:     >   ·   a   あ   b
//! slot:      0   1   2   3    4      where the cell is in `Row::cells`
//! column:    0   1   2   3-4  5      where it is drawn on screen
//! index:     -   -   0   1    2      which buffer grapheme it is
//! ```
//!
//! [`Row::column_of`] turns a slot into a column, [`Row::index_at`] a
//! column into a buffer index. A place in the whole frame, row and column
//! together, is a [`Position`].

/// A place in the frame: `row` counted from the frame's first row,
/// `col` from the left edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Position {
    pub(crate) row: u16,
    pub(crate) col: u16,
}

/// One grapheme on screen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Cell {
    /// What is drawn. Owned, since a frame is kept after the strings it was
    /// laid out from are gone, for the next paint to compare against.
    pub(super) grapheme: String,
    /// Columns the grapheme takes: 2 for a wide character, 1 for most.
    pub(super) width: u16,
    /// Which grapheme of the editing buffer this is, counted from the start
    /// of the buffer, newlines included. `None` for everything that is not
    /// buffer text: prompt, hint, padding, menu.
    ///
    /// Not drawn. It is there so a mouse click on this cell can be turned
    /// into a cursor position. A count of graphemes and not a byte offset:
    /// the buffer text arrives highlighted and CRLF-coerced, so its bytes
    /// are no longer the editor's, but its graphemes are.
    pub(super) index: Option<usize>,
}

/// One screen row: its cells from left to right.
///
/// Wrapping has already happened when a row is built, so a row is never
/// wider than the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Row {
    pub(super) cells: Vec<Cell>,
}

impl Row {
    /// Columns drawn on this row: every cell's width, added up.
    pub(super) fn width(&self) -> u16 {
        self.column_of(self.cells.len())
    }

    /// The column the cell at `slot` starts on, which is the width of the
    /// cells before it. A `slot` past the last cell gives the row's width,
    /// the column the next cell would start on.
    ///
    /// `slot` is a place in [`Row::cells`], not a buffer index.
    pub(super) fn column_of(&self, slot: usize) -> u16 {
        self.cells.iter().take(slot).map(|c| c.width).sum()
    }

    /// The buffer index of the cell drawn at `column`.
    ///
    /// `None` when the cell there is not buffer text, or no cell reaches
    /// that column. A wide cell answers for each column it covers.
    pub(super) fn index_at(&self, column: u16) -> Option<usize> {
        let mut start = 0;
        for cell in &self.cells {
            let end = start + cell.width;
            if (start..end).contains(&column) {
                return cell.index;
            }
            start = end;
        }
        None
    }
}

/// Everything one paint draws, laid out and not yet written: the rows from
/// top to bottom and where the cursor goes.
///
/// Row 0 is the first row of the prompt, wherever on the screen that ends
/// up. A frame knows nothing of the screen it will be drawn on; it can be
/// taller than the screen, and [`Frame::viewport_top`] says which part to
/// show then.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Frame {
    pub(super) rows: Vec<Row>,
    /// Where the cursor goes. Always names a row that exists in `rows`.
    pub(super) cursor: Position,
    /// Rows the prompt takes, wrapping included. At least 1.
    pub(super) prompt_rows: u16,
}

impl Frame {
    /// Where the cursor goes.
    pub(crate) fn cursor(&self) -> Position {
        self.cursor
    }

    /// Rows the prompt draws on, wrapping included. At least 1.
    pub(crate) fn prompt_rows(&self) -> u16 {
        self.prompt_rows
    }

    /// Rows the frame needs on screen: the length of `rows` as a `u16`.
    pub(crate) fn height(&self) -> u16 {
        u16::try_from(self.rows.len()).unwrap_or(u16::MAX)
    }

    /// The buffer index of the cell drawn at `at`, for click mapping.
    pub(crate) fn index_at(&self, at: Position) -> Option<usize> {
        self.rows.get(at.row as usize)?.index_at(at.col)
    }

    /// The first row to show when only `screen_rows` rows fit.
    ///
    /// The top of the frame stays for as long as the cursor's row fits below
    /// it, so the prompt is the last thing to go. Rows are skipped only to
    /// bring the cursor's row onto the last screen row, and what does not fit
    /// below the cursor is cut, as the streaming painter cuts it. With no
    /// screen at all the cursor's row is still not skipped.
    ///
    /// ```text
    /// 6 rows, screen of 5, cursor on row 5  ->  1   rows 1..=5 show
    /// 6 rows, screen of 5, cursor on row 0  ->  0   row 5 is cut
    /// 3 rows, screen of 5                   ->  0   it fits
    /// ```
    pub(crate) fn viewport_top(&self, screen_rows: u16) -> u16 {
        self.cursor
            .row
            .saturating_add(1)
            .saturating_sub(screen_rows)
            .min(self.cursor.row)
    }
}
