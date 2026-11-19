//! The writer's output run through a terminal emulator: these assert on
//! what reaches the screen, not on the bytes.

use crate::painting::frame::grid::Frame;
use crate::painting::frame::layout::{layout, LayoutInput};
use crate::painting::frame::write::write_frame;
use crate::painting::test_screen::TestScreen;
use pretty_assertions::assert_eq;

/// A frame of `> ` and the buffer, the cursor between `before` and
/// `after`, no continuation.
fn frame(before: &str, after: &str, width: u16) -> Frame {
    layout(&prompt_input(before, after, width))
}

/// Buffer lines only, no prompt: row `i` of the frame reads line `i`.
fn lines(before: &str, after: &str, width: u16) -> Frame {
    layout(&LayoutInput {
        indicator: "",
        ..prompt_input(before, after, width)
    })
}

fn prompt_input<'a>(before: &'a str, after: &'a str, width: u16) -> LayoutInput<'a> {
    LayoutInput {
        prompt_left: "",
        indicator: "> ",
        continuation: "",
        before,
        after,
        hint: "",
        width,
    }
}

/// A `width` by `height` screen holding `before`, then `frame` written
/// over it.
fn screen(
    before: &[u8],
    frame: &Frame,
    top: u16,
    anchor: u16,
    width: u16,
    height: u16,
) -> TestScreen {
    let mut screen = TestScreen::new(width, height);
    screen.feed(before);
    let mut out = Vec::new();
    write_frame(frame, top, anchor, width, height, &mut out).unwrap();
    screen.feed(&out);
    screen
}

#[test]
fn rows_go_down_from_the_anchor_and_leave_the_rows_above() {
    let screen = screen(b"out", &frame("ab", "", 10), 0, 1, 10, 4);
    assert_eq!(screen.rows(), ["out", "> ab", "", ""]);
    assert_eq!(screen.cursor(), (4, 1));
}

#[test]
fn a_row_replaces_all_that_was_on_it() {
    let screen = screen(b"\r\nxxxxxxxxxx", &frame("ab", "", 10), 0, 1, 10, 3);
    assert_eq!(screen.row(1), "> ab");
}

#[test]
fn the_rows_below_the_frame_are_cleared() {
    let screen = screen(
        b"out\r\nold\r\nold\r\nold",
        &frame("ab", "", 10),
        0,
        1,
        10,
        4,
    );
    assert_eq!(screen.rows(), ["out", "> ab", "", ""]);
}

#[test]
fn a_row_that_fills_the_width_keeps_its_last_cell() {
    // Clearing to the end of the line from a pending wrap erases the cell
    // the cursor is still on.
    let frame = frame("ab", "", 4);
    assert_eq!(frame.height(), 2);
    let screen = screen(b"", &frame, 0, 0, 4, 3);
    assert_eq!(screen.rows(), ["> ab", "", ""]);
    assert_eq!(screen.cursor(), (0, 1));
}

#[test]
fn the_cursor_goes_to_its_row_and_column_of_the_frame() {
    let screen = screen(b"out", &lines("a\nbc", "d", 10), 0, 1, 10, 4);
    assert_eq!(screen.rows(), ["out", "a", "bcd", ""]);
    assert_eq!(screen.cursor(), (2, 2));
}

#[test]
fn a_frame_taller_than_the_screen_shows_from_top() {
    let frame = lines("1\n2\n3\n4\n5", "", 10);
    let top = frame.viewport_top(3);
    assert_eq!(top, 2);
    let screen = screen(b"", &frame, top, 0, 10, 3);
    assert_eq!(screen.rows(), ["3", "4", "5"]);
    // Frame row 4 is screen row 2: counted from `top`, not from row 0.
    assert_eq!(screen.cursor(), (1, 2));
}

#[test]
fn rows_past_the_bottom_of_the_screen_are_not_written() {
    // A `MoveTo` past the last row lands on the last row, so writing them
    // would draw each over the one before.
    let frame = lines("", "1\n2\n3\n4\n5", 10);
    let mut screen = screen(b"", &frame, frame.viewport_top(3), 0, 10, 3);
    assert_eq!(screen.rows(), ["1", "2", "3"]);
    assert_eq!(screen.cursor(), (0, 0));
    assert_eq!(screen.scrolled(), 0);
}

#[test]
fn an_escape_is_sent_and_takes_no_column() {
    let screen = screen(b"", &frame("\x1b[31mab\x1b[0m", "c", 10), 0, 0, 10, 2);
    assert_eq!(screen.row(0), "> abc");
    assert_eq!(screen.cursor(), (4, 0));
}
