use crate::painting::frame::builder::Builder;
use crate::painting::frame::grid::{Frame, Position, Row};
use pretty_assertions::assert_eq;
use rstest::rstest;

/// Each row's graphemes joined, escapes included.
fn texts(frame: &Frame) -> Vec<String> {
    frame
        .rows
        .iter()
        .map(|row| row.cells.iter().map(|c| c.grapheme.as_str()).collect())
        .collect()
}

fn indices(row: &Row) -> Vec<Option<usize>> {
    row.cells.iter().map(|c| c.index).collect()
}

/// `text` put as prompt text into a builder of width 4, then the frame
/// with the cursor wherever the walk stopped.
fn build(text: &str) -> Frame {
    let mut builder = Builder::new(4);
    builder.put(text, None);
    let cursor = builder.position();
    builder.frame(cursor, 1)
}

#[test]
fn text_wraps_at_the_width() {
    assert_eq!(texts(&build("abcdef")), ["abcd", "ef"]);
}

#[test]
fn a_filled_row_leaves_the_wrap_pending() {
    let mut builder = Builder::new(4);
    builder.put("abcd", None);
    // The next grapheme would start the next row, but nothing opened it.
    assert_eq!(builder.position(), Position { row: 1, col: 0 });
    let frame = builder.frame(Position { row: 0, col: 3 }, 1);
    assert_eq!(texts(&frame), ["abcd"]);
}

#[test]
fn the_frame_opens_the_row_a_pending_wrap_puts_the_cursor_on() {
    let frame = build("abcd");
    assert_eq!(texts(&frame), ["abcd", ""]);
    assert_eq!(frame.cursor, Position { row: 1, col: 0 });
}

#[rstest]
#[case::lf("ab\ncd")]
#[case::crlf("ab\r\ncd")]
fn a_newline_breaks_the_row_and_takes_one_index(#[case] text: &str) {
    let mut builder = Builder::new(4);
    let mut index = 0;
    builder.put(text, Some(&mut index));
    let frame = builder.frame(Position { row: 1, col: 2 }, 1);
    assert_eq!(texts(&frame), ["ab", "cd"]);
    assert_eq!(indices(&frame.rows[0]), [Some(0), Some(1)]);
    // The newline is index 2 and has no cell.
    assert_eq!(indices(&frame.rows[1]), [Some(3), Some(4)]);
    assert_eq!(index, 5);
}

#[test]
fn a_wide_grapheme_that_does_not_fit_starts_the_next_row() {
    let frame = build("abcあ");
    assert_eq!(texts(&frame), ["abc", "あ"]);
    // The last column of row 0 stays blank.
    assert_eq!(frame.rows[0].width(), 3);
    assert_eq!(frame.rows[1].width(), 2);
}

#[test]
fn the_index_runs_on_across_puts_and_skips_prompt_text() {
    let mut builder = Builder::new(8);
    let mut index = 0;
    builder.put("> ", None);
    builder.put("ab", Some(&mut index));
    builder.put("cd", Some(&mut index));
    let frame = builder.frame(Position { row: 0, col: 6 }, 1);
    assert_eq!(
        indices(&frame.rows[0]),
        [None, None, Some(0), Some(1), Some(2), Some(3)]
    );
    assert_eq!(index, 4);
}

#[test]
fn an_escape_stays_in_place_at_no_width_and_takes_no_index() {
    let mut builder = Builder::new(4);
    let mut index = 0;
    builder.put("a\x1b[31mb", Some(&mut index));
    let frame = builder.frame(Position { row: 0, col: 2 }, 1);
    let row = &frame.rows[0];
    assert_eq!(texts(&frame), ["a\x1b[31mb"]);
    assert_eq!(row.cells[1].grapheme, "\x1b[31m");
    assert_eq!(row.cells[1].width, 0);
    assert_eq!(indices(row), [Some(0), None, Some(1)]);
    assert_eq!(row.width(), 2);
    assert_eq!(index, 2);
}

#[test]
fn an_escape_does_not_count_toward_the_wrap() {
    // Four glyphs fill the row; the escape between them takes no column.
    assert_eq!(texts(&build("ab\x1b[1mcde")), ["ab\x1b[1mcd", "e"]);
}

#[test]
fn a_prompt_ending_in_a_newline_puts_the_buffer_on_its_own_row() {
    let mut builder = Builder::new(4);
    let mut index = 0;
    builder.put("> \n", None);
    builder.put("x", Some(&mut index));
    let cursor = builder.position();
    let frame = builder.frame(cursor, 1);
    assert_eq!(texts(&frame), ["> ", "x"]);
    assert_eq!(cursor, Position { row: 1, col: 1 });
    assert_eq!(indices(&frame.rows[1]), [Some(0)]);
}
