//! What one paint draws, as the painter has it, and the layout that turns
//! it into a [`Frame`].

use super::{builder::Builder, grid::Frame};

/// Everything [`layout`] draws, in the strings the painter already holds.
///
/// Each part arrives finished: highlighted, colored, CRLF-coerced. The
/// layout places them and does not look inside beyond escapes and
/// graphemes.
pub(crate) struct LayoutInput<'a> {
    /// `Prompt::render_prompt_left`. May span rows; the buffer starts
    /// after its last one.
    pub(crate) prompt_left: &'a str,
    /// `Prompt::render_prompt_indicator`, or the history search one.
    pub(crate) indicator: &'a str,
    /// `Prompt::render_prompt_multiline_indicator`, drawn after every
    /// newline of the buffer. Never buffer text, so it takes no index.
    pub(crate) continuation: &'a str,
    /// The buffer before the cursor, without continuation prompts.
    pub(crate) before: &'a str,
    /// The buffer from the cursor on, without continuation prompts.
    pub(crate) after: &'a str,
    /// The hint after the buffer. Empty for none.
    pub(crate) hint: &'a str,
    /// Columns of the screen.
    pub(crate) width: u16,
}

/// Lay `input` out into a frame `input.width` columns wide.
pub(crate) fn layout(input: &LayoutInput<'_>) -> Frame {
    let mut builder = Builder::new(input.width.max(1));
    builder.put(input.prompt_left, None);
    builder.put(input.indicator, None);

    let prompt_rows = builder.rows_drawn();

    let mut index: usize = 0;

    put_buffer(&mut builder, input.before, input.continuation, &mut index);
    let cursor = builder.position();

    put_buffer(&mut builder, input.after, input.continuation, &mut index);
    builder.put(input.hint, None);

    builder.frame(cursor, prompt_rows)
}

/// Put buffer `text`, numbered from `index`, with `continuation` after
/// every newline.
fn put_buffer(builder: &mut Builder, text: &str, continuation: &str, index: &mut usize) {
    for line in text.split_inclusive('\n') {
        builder.put(line, Some(index));
        if line.ends_with('\n') {
            builder.put(continuation, None);
        }
    }
}
