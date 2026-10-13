//! Painter tests asserting on what reaches the screen, through
//! [`TestScreen`], rather than on the bytes the painter emits.

use super::*;
use crate::painting::test_screen::TestScreen;
use crate::PromptHistorySearch;
use pretty_assertions::assert_eq;
use std::borrow::Cow;

/// A prompt drawing fixed strings. Lines are built through
/// [`PromptLines::new`] from it, as the engine builds them, so a new field on
/// `PromptLines` does not reach these tests.
#[derive(Default)]
struct FixedPrompt {
    left: &'static str,
    indicator: &'static str,
    right: &'static str,
    right_on_last_line: bool,
}

impl FixedPrompt {
    fn new(left: &'static str, indicator: &'static str) -> Self {
        Self {
            left,
            indicator,
            ..Self::default()
        }
    }

    fn with_right(mut self, right: &'static str, on_last_line: bool) -> Self {
        self.right = right;
        self.right_on_last_line = on_last_line;
        self
    }
}

impl Prompt for FixedPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        self.left.into()
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        self.right.into()
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        self.indicator.into()
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        "".into()
    }

    fn render_prompt_history_search_indicator(
        &self,
        _history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        "".into()
    }

    fn right_prompt_on_last_line(&self) -> bool {
        self.right_on_last_line
    }
}

/// A painter wired to a [`TestScreen`] of the same size, with its prompt
/// anchored on `anchor_row` as if `initialize_prompt_position` had measured it.
struct Session {
    painter: Painter,
    screen: TestScreen,
}

impl Session {
    fn new(width: u16, height: u16, anchor_row: u16) -> Self {
        let mut painter = Painter::new(W::capture());
        painter.terminal_size = (width, height);
        // Pinned rather than read from `TERM`, which differs between a
        // developer's shell and CI.
        painter.term_is_dumb = false;
        painter.prompt_start_row = PromptStartRow::Verified(anchor_row);
        Self {
            painter,
            screen: TestScreen::new(width, height),
        }
    }

    /// As [`Session::new`], with `output` already on screen above the anchor,
    /// left there by the host before the prompt.
    fn after_output(width: u16, height: u16, output: &[&str]) -> Self {
        let rows = u16::try_from(output.len()).expect("a test prints a few rows");
        let mut session = Self::new(width, height, rows);
        for line in output {
            session.screen.feed(line.as_bytes());
            session.screen.feed(b"\r\n");
        }
        session
    }

    /// Paint `prompt` with the cursor between `before` and `after`.
    fn paint(&mut self, prompt: &FixedPrompt, before: &str, after: &str) {
        let mode = PromptEditMode::Default;
        let lines = PromptLines::new(prompt, mode.clone(), None, before, after, "");
        self.painter
            .repaint_buffer(prompt, &lines, mode, None, false, &None)
            .expect("repaint_buffer failed");
        self.screen.sync(self.painter.stdout.captured());
    }

    /// Commit `messages` above the prompt block, as the external printer
    /// does. The engine repaints right after; a test does that with
    /// [`Session::paint`].
    fn commit(&mut self, messages: &[&str]) {
        self.painter
            .print_external_message(messages.iter().map(|m| (*m).to_owned()).collect())
            .expect("print_external_message failed");
        self.screen.sync(self.painter.stdout.captured());
    }

    /// Visible rows and scrollback together, oldest first, trailing blank
    /// rows dropped: everything the user can still scroll back to.
    fn history(&mut self) -> Vec<String> {
        let mut rows = self.screen.scrollback_rows();
        rows.extend(self.screen.rows());
        while rows.last().is_some_and(String::is_empty) {
            rows.pop();
        }
        rows
    }
}

#[test]
fn paints_below_the_host_output_without_touching_it() {
    let mut session = Session::after_output(20, 6, &["out1", "out2"]);

    session.paint(&FixedPrompt::new("> ", ""), "hello", "");

    assert_eq!(session.screen.row(0), "out1");
    assert_eq!(session.screen.row(1), "out2");
    assert_eq!(session.screen.row(2), "> hello");
    assert_eq!(session.screen.cursor(), (7, 2));
    assert!(session.screen.cursor_visible());
    assert_eq!(session.screen.scrolled(), 0);
}

#[test]
fn cursor_sits_between_the_text_before_and_after_it() {
    let mut session = Session::new(20, 6, 1);

    session.paint(&FixedPrompt::new("> ", ""), "hel", "lo");

    assert_eq!(session.screen.row(1), "> hello");
    assert_eq!(session.screen.cursor(), (5, 1));
}

#[test]
fn repaint_draws_over_the_previous_frame() {
    let mut session = Session::new(20, 6, 1);
    let prompt = FixedPrompt::new("> ", "");

    session.paint(&prompt, "a longer line", "");
    session.paint(&prompt, "short", "");

    assert_eq!(session.screen.row(1), "> short");
    assert_eq!(session.screen.row(2), "");
    assert_eq!(session.screen.scrolled(), 0);
}

// #1141: text ending exactly on the margin leaves the terminal's wrap pending,
// and terminals disagree about restoring that state. The cursor belongs on the
// next row's home column, placed there absolutely.
#[test]
fn text_filling_the_width_puts_the_cursor_on_the_next_row() {
    let mut session = Session::new(10, 6, 1);

    session.paint(&FixedPrompt::new("> ", ""), "abcdefgh", "");

    assert_eq!(session.screen.row(1), "> abcdefgh");
    assert_eq!(session.screen.cursor(), (0, 2));
}

// A wide grapheme with one column left wraps whole, leaving that column blank.
#[test]
fn wide_grapheme_without_room_wraps_to_the_next_row() {
    let mut session = Session::new(5, 6, 1);

    session.paint(&FixedPrompt::new("> ", ""), "ab中", "");

    assert_eq!(session.screen.row(1), "> ab");
    assert_eq!(session.screen.row(2), "中");
    assert_eq!(session.screen.cursor(), (2, 2));
}

#[test]
fn right_prompt_on_last_line_of_a_multiline_prompt() {
    let mut session = Session::new(20, 6, 1);
    let prompt = FixedPrompt::new("abc\n", "> ").with_right("R", true);

    session.paint(&prompt, "hi", "");

    assert_eq!(session.screen.row(1), "abc");
    assert_eq!(session.screen.row(2), format!("> hi{}R", " ".repeat(15)));
    assert_eq!(session.screen.cursor(), (4, 2));
}

// The commit starts where the painter says the block starts, erases from
// there, and prints the messages in its place, so nothing of the old block
// survives under them.
#[test]
fn external_messages_replace_the_block_at_the_anchor() {
    let mut session = Session::new(20, 10, 5);

    session.paint(&FixedPrompt::new("> ", ""), "old", "");
    session.commit(&["one", "two"]);

    assert_eq!(session.screen.rows()[4..8], ["", "one", "two", ""]);
    assert_eq!(
        session.painter.prompt_start_row,
        PromptStartRow::Verified(7)
    );
}

// The case from a real terminal: a left prompt long enough that the first row
// wraps, and a message landing mid-edit. The old path moved up from the cursor
// by a byte count and left a copy of the prompt row above each batch. The
// message commits above the block, and exactly one prompt is drawn under it.
#[test]
fn external_messages_commit_above_the_prompt_and_leave_nothing_behind() {
    let mut session = Session::new(20, 6, 0);
    let prompt = FixedPrompt::new("~/project>", "");

    session.paint(&prompt, "some things!", "");
    session.commit(&["msg"]);
    session.paint(&prompt, "some things!", "");

    // The prompt and buffer wrap at 20 columns onto a second row.
    assert_eq!(
        session.screen.rows()[..4],
        ["msg", "~/project>some thing", "s!", ""]
    );
    assert_eq!(
        session.painter.prompt_start_row,
        PromptStartRow::Verified(1)
    );
}

// A message wider than the screen takes the rows it wraps onto, and the
// prompt goes below all of them.
#[test]
fn a_wrapped_external_message_pushes_the_prompt_below_its_rows() {
    let mut session = Session::new(20, 6, 2);
    let prompt = FixedPrompt::new("> ", "");

    session.paint(&prompt, "", "");
    session.commit(&[&"x".repeat(30)]);
    session.paint(&prompt, "", "");

    assert_eq!(session.screen.row(2), "x".repeat(20));
    assert_eq!(session.screen.row(3), "x".repeat(10));
    assert_eq!(session.screen.row(4), ">");
    assert_eq!(
        session.painter.prompt_start_row,
        PromptStartRow::Verified(4)
    );
}

// Messages arriving at the bottom of the screen scroll the earlier ones into
// history in order, and the prompt never does.
#[test]
fn external_messages_at_the_bottom_scroll_into_history_in_order() {
    let mut session = Session::new(20, 4, 1);
    session.painter.bottom_confirmed_at = Some(4);
    let prompt = FixedPrompt::new("> ", "");

    session.paint(&prompt, "", "");
    for msg in ["m1", "m2", "m3", "m4", "m5"] {
        session.commit(&[msg]);
        session.paint(&prompt, "", "");
    }

    assert_eq!(session.history(), ["", "m1", "m2", "m3", "m4", "m5", ">"]);
}

// As above with the indicator folded into the left prompt. `prompt_height`
// counts rows with `str::lines()`, which yields no row after a trailing
// newline, so the prompt reads as one row and the right prompt lands a row
// too high.
#[test]
#[ignore = "prompt_height reads a trailing newline as no row"]
fn right_prompt_on_last_line_after_a_prompt_ending_in_newline() {
    let mut session = Session::new(20, 6, 1);
    let prompt = FixedPrompt::new("abc\n", "").with_right("R", true);

    session.paint(&prompt, "hi", "");

    assert_eq!(session.screen.row(1), "abc");
    assert_eq!(session.screen.row(2), format!("hi{}R", " ".repeat(17)));
    assert_eq!(session.screen.cursor(), (2, 2));
}

// A buffer taller than the screen is drawn as a viewport onto its tail. Which
// rows scroll off is decided by `compute_layout` counting `str::lines()`, so a
// single line wrapping past the bottom reads as fitting: the paint emits one
// row too many and the terminal scrolls, on every paint.
#[test]
#[ignore = "compute_layout counts logical lines, not wrapped rows"]
fn large_buffer_of_one_wrapped_line_stays_on_screen() {
    let mut session = Session::new(80, 5, 0);
    let prompt = FixedPrompt::new("> ", "");
    let buffer = "x".repeat(400);

    session.paint(&prompt, &buffer, "");
    session.paint(&prompt, &buffer, "");

    assert_eq!(session.screen.scrolled(), 0);
    assert_eq!(session.screen.row(4), "xx");
    assert_eq!(session.screen.cursor(), (2, 4));
}
