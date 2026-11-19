use super::{indices, texts};
use crate::painting::frame::grid::{Frame, Position};
use crate::painting::frame::layout::{layout, LayoutInput};
use pretty_assertions::assert_eq;
use rstest::rstest;

/// `> ` as the prompt, `: ` as the continuation, no hint, 10 columns.
fn input<'a>(before: &'a str, after: &'a str) -> LayoutInput<'a> {
    LayoutInput {
        prompt_left: "",
        indicator: "> ",
        continuation: ": ",
        before,
        after,
        hint: "",
        width: 10,
    }
}

fn laid_out(before: &str, after: &str) -> Frame {
    layout(&input(before, after))
}

#[test]
fn the_cursor_sits_between_before_and_after() {
    let frame = laid_out("ab", "cd");
    assert_eq!(texts(&frame), ["> abcd"]);
    assert_eq!(frame.cursor, Position { row: 0, col: 4 });
    assert_eq!(
        indices(&frame.rows[0]),
        [None, None, Some(0), Some(1), Some(2), Some(3)]
    );
    assert_eq!(frame.prompt_rows, 1);
}

#[test]
fn a_prompt_over_several_rows_puts_the_buffer_after_its_last() {
    let frame = layout(&LayoutInput {
        prompt_left: "~/dir\n",
        ..input("x", "")
    });
    assert_eq!(texts(&frame), ["~/dir", "> x"]);
    assert_eq!(frame.prompt_rows, 2);
    assert_eq!(frame.cursor, Position { row: 1, col: 3 });
}

#[test]
fn a_newline_that_ends_the_prompt_adds_no_prompt_row() {
    // The row it opens is the buffer's, where the right prompt goes.
    let frame = layout(&LayoutInput {
        prompt_left: "info\n",
        indicator: "",
        ..input("x", "")
    });
    assert_eq!(texts(&frame), ["info", "x"]);
    assert_eq!(frame.prompt_rows, 1);
    assert_eq!(frame.cursor, Position { row: 1, col: 1 });
}

#[test]
fn a_prompt_that_fills_its_row_takes_only_that_row() {
    // The wrap the prompt owes belongs to the buffer's first grapheme.
    let frame = layout(&LayoutInput {
        indicator: "> ab",
        width: 4,
        ..input("x", "")
    });
    assert_eq!(texts(&frame), ["> ab", "x"]);
    assert_eq!(frame.prompt_rows, 1);
    assert_eq!(frame.cursor, Position { row: 1, col: 1 });
}

#[rstest]
#[case::lf("a\nb", "")]
#[case::crlf("a\r\nb", "")]
#[case::after_the_cursor("a", "\nb")]
fn the_continuation_follows_each_newline_and_takes_no_index(
    #[case] before: &str,
    #[case] after: &str,
) {
    let frame = laid_out(before, after);
    assert_eq!(texts(&frame), ["> a", ": b"]);
    // The newline is index 1, thus `b` is 2 and not 4.
    assert_eq!(indices(&frame.rows[1]), [None, None, Some(2)]);
}

#[test]
fn a_cursor_on_an_empty_line_sits_after_the_continuation() {
    let frame = laid_out("a\n", "");
    assert_eq!(texts(&frame), ["> a", ": "]);
    assert_eq!(frame.cursor, Position { row: 1, col: 2 });
}

#[test]
fn the_index_runs_on_from_before_into_after_across_lines() {
    let frame = laid_out("a\nb", "c\nd");
    assert_eq!(texts(&frame), ["> a", ": bc", ": d"]);
    assert_eq!(indices(&frame.rows[1]), [None, None, Some(2), Some(3)]);
    assert_eq!(indices(&frame.rows[2]), [None, None, Some(5)]);
}

#[test]
fn the_hint_follows_the_buffer_and_takes_no_index() {
    let frame = layout(&LayoutInput {
        hint: "cd",
        ..input("ab", "")
    });
    assert_eq!(texts(&frame), ["> abcd"]);
    assert_eq!(
        indices(&frame.rows[0]),
        [None, None, Some(0), Some(1), None, None]
    );
    // The hint does not move the cursor.
    assert_eq!(frame.cursor, Position { row: 0, col: 4 });
}

#[test]
fn a_zero_width_screen_lays_out_one_column() {
    let frame = layout(&LayoutInput {
        indicator: "",
        width: 0,
        ..input("ab", "")
    });
    assert_eq!(texts(&frame), ["a", "b", ""]);
    assert_eq!(frame.cursor, Position { row: 2, col: 0 });
}
