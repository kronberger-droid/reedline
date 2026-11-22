use crate::painting::frame::grid::{Cell, Frame, Position, Row};
use pretty_assertions::assert_eq;
use rstest::rstest;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A row of `text`, a cell per grapheme. With `first`, the cells are buffer
/// text numbered from it; without, they are prompt text.
fn row(text: &str, first: Option<usize>) -> Row {
    let cells = text
        .graphemes(true)
        .enumerate()
        .map(|(i, grapheme)| Cell {
            grapheme: grapheme.to_owned(),
            width: grapheme.width() as u16,
            index: first.map(|first| first + i),
        })
        .collect();
    Row { cells }
}

/// `> ` as prompt, then `text` as buffer graphemes 0, 1, 2...
fn prompt_row(text: &str) -> Row {
    let mut prompt = row("> ", None);
    prompt.cells.extend(row(text, Some(0)).cells);
    prompt
}

/// A frame of `rows` empty rows, with the cursor on `cursor_row`.
fn frame(rows: usize, cursor_row: u16) -> Frame {
    Frame {
        rows: vec![Row::default(); rows],
        cursor: Position {
            row: cursor_row,
            col: 0,
        },
        prompt_rows: 1,
    }
}

#[rstest]
#[case::empty("", 0)]
#[case::narrow("abc", 3)]
#[case::wide("あい", 4)]
#[case::mixed("aあb", 4)]
fn a_row_is_as_wide_as_its_cells(#[case] text: &str, #[case] expected: u16) {
    assert_eq!(row(text, None).width(), expected);
}

#[rstest]
#[case::first(0, 0)]
#[case::after_a_narrow_cell(1, 1)]
#[case::after_a_wide_cell(2, 3)]
#[case::the_end(3, 4)]
#[case::past_the_end(99, 4)]
fn column_of_is_where_a_cell_starts(#[case] cell: usize, #[case] expected: u16) {
    assert_eq!(row("aあb", None).column_of(cell), expected);
}

#[rstest]
#[case::prompt(0, None)]
#[case::prompt_end(1, None)]
#[case::first_buffer_cell(2, Some(0))]
#[case::wide_cell_left_half(3, Some(1))]
#[case::wide_cell_right_half(4, Some(1))]
#[case::after_the_wide_cell(5, Some(2))]
#[case::past_the_text(6, None)]
fn index_at_names_the_buffer_grapheme_under_a_column(
    #[case] column: u16,
    #[case] expected: Option<usize>,
) {
    assert_eq!(prompt_row("aあb").index_at(column), expected);
}

#[test]
fn a_frame_looks_a_column_up_on_its_row() {
    let frame = Frame {
        rows: vec![prompt_row("ab"), row("cd", Some(3))],
        cursor: Position { row: 1, col: 2 },
        prompt_rows: 1,
    };
    assert_eq!(frame.height(), 2);
    assert_eq!(frame.index_at(Position { row: 0, col: 2 }), Some(0));
    assert_eq!(frame.index_at(Position { row: 1, col: 1 }), Some(4));
    assert_eq!(frame.index_at(Position { row: 2, col: 0 }), None);
}

/// The viewport scrolls down just far enough to show the cursor's row, so
/// the top of the frame stays until then, and never past the cursor's row.
#[rstest]
#[case::fits(3, 0, 5, 0)]
#[case::fits_exactly(5, 4, 5, 0)]
#[case::cursor_at_the_bottom(6, 5, 5, 1)]
#[case::cursor_at_the_top(6, 0, 5, 0)]
#[case::cursor_on_a_row_that_fits(20, 2, 5, 0)]
#[case::cursor_on_the_last_row_that_fits(20, 4, 5, 0)]
#[case::cursor_near_the_end(20, 18, 5, 14)]
#[case::no_screen(6, 5, 0, 5)]
fn viewport_top_keeps_the_cursor_row_on_screen(
    #[case] rows: usize,
    #[case] cursor_row: u16,
    #[case] screen_rows: u16,
    #[case] expected: u16,
) {
    assert_eq!(frame(rows, cursor_row).viewport_top(screen_rows), expected);
}
