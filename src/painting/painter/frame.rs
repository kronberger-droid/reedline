//! The frame painter: lay the paint out as a `Frame` first, then draw it
//! from the prompt's anchor row.
//!
//! The streaming painter in `painter.rs` still draws what the frame cannot
//! yet, see [`Painter::frame_can_paint`]. Each of its conditions goes away
//! as the frame learns to draw that part, and the streaming painter with
//! the last of them.

use nu_ansi_term::Style;

use crate::painting::frame::{layout, make_room, write_frame, LayoutInput};

use super::*;

impl Painter {
    /// Whether this paint can go through the frame layout, which draws the
    /// prompt, the buffer and the hint, and nothing else yet.
    pub(super) fn frame_can_paint(
        &self,
        lines: &PromptLines,
        menu: Option<&ReedlineMenu>,
        reserved_status_rows: u16,
        reserved_footer_rows: u16,
    ) -> bool {
        self.paints_frames
            && menu.is_none()
            && reserved_status_rows == 0
            && reserved_footer_rows == 0
            && lines.prompt_str_right.is_empty()
            && self.semantic_markers.is_none()
    }

    /// Lay the paint out as a frame and draw it from the anchor row.
    pub(super) fn repaint_frame(
        &mut self,
        prompt: &dyn Prompt,
        lines: &PromptLines,
        prompt_mode: PromptEditMode,
        use_ansi_coloring: bool,
        cursor_config: &Option<CursorConfig>,
    ) -> Result<()> {
        // As `repaint_buffer`: clear styling an earlier command left on.
        self.stdout.queue(SetAttribute(Attribute::Reset))?;
        self.hide_cursor()?;

        // Escapes stay in the strings: the layout keeps each where it was
        // written, and `write_frame` sends it with its row.
        let (left, indicator, continuation) = if use_ansi_coloring {
            (
                format!(
                    "{}{}",
                    prompt.get_prompt_color().prefix(),
                    lines.prompt_str_left
                ),
                format!(
                    "{}{}\x1b[0m",
                    prompt.get_indicator_color().prefix(),
                    lines.prompt_indicator
                ),
                Style::new()
                    .fg(prompt.get_prompt_multiline_color())
                    .paint(&*lines.continuation)
                    .to_string(),
            )
        } else {
            (
                lines.prompt_str_left.to_string(),
                lines.prompt_indicator.to_string(),
                lines.continuation.to_string(),
            )
        };
        let (before, after) = lines.frame_buffer();
        let input = LayoutInput {
            prompt_left: &left,
            indicator: &indicator,
            continuation: &continuation,
            before,
            after,
            hint: &lines.hint,
            width: self.screen_width(),
        };

        let width = input.width;
        let frame = layout(&input);

        self.prompt_height = frame.prompt_rows();

        // Everything that asks the terminal runs before anything is drawn.
        // Only one of these two does anything in a paint: `Resized` and
        // `Stale` are different states. The reconcile can grow the height,
        // which moves `top`, so it is worked out again after.
        let top = frame.viewport_top(self.screen_height());
        self.prompt_start_row
            .resolve_resize(frame.cursor().row.saturating_sub(top).saturating_add(1));
        self.reconcile_stale_anchor();

        // Unreachable in normal flow (initialize_prompt_position runs first);
        // in release, home to row 0 rather than draw over the content there.
        let anchor_uninitialized = self.prompt_start_row == PromptStartRow::Unverified;
        debug_assert!(
            !anchor_uninitialized,
            "repaint_frame reached before initialize_prompt_position"
        );

        let height = self.screen_height();
        let top = frame.viewport_top(height);
        let visible = frame.height().saturating_sub(top).min(height);

        // No synchronized update of its own, as in `repaint_buffer` (#1302).
        // One `print_external_message` left open is still ended below.
        let painted = (|| -> Result<()> {
            let room = make_room(self.prompt_start_row.last_known_row(), visible, height);
            if room.scroll > 0 {
                self.queue_universal_scroll(room.scroll)?;
            }
            self.prompt_start_row.mark_verified(room.anchor);

            write_frame(&frame, top, room.anchor, width, height, &mut self.stdout)?;

            // What the exit and suspend paths read, as `repaint_buffer`
            // records it.
            self.last_required_lines = frame.height();
            self.large_buffer = frame.height() > height;
            self.exit_right_prompt = None;
            // Click mapping reads the streaming layout, which a frame paint
            // does not make: clicks do nothing until it reads the frame.
            self.last_layout = None;
            let after_cursor = lines
                .exit_after_cursor
                .as_ref()
                .unwrap_or(&lines.after_cursor);
            self.after_cursor_lines = (!after_cursor.is_empty()).then(|| after_cursor.to_string());

            let shape = cursor_config
                .as_ref()
                .and_then(|shapes| shapes.shape_for(&prompt_mode));
            if let Some(shape) = shape {
                self.stdout.queue(shape)?;
            }
            self.stdout.queue(cursor::Show)?;
            self.cursor_hidden = false;
            Ok(())
        })();
        let ended = self.end_frame().and_then(|()| self.stdout.flush());
        painted.and(ended)
    }

    /// Paint through the frame layout where it can.
    #[cfg(test)]
    pub(crate) fn paint_frames_for_test(&mut self) {
        self.paints_frames = true;
    }
}
