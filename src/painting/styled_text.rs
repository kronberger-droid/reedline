use nu_ansi_term::Style;

use crate::terminal_extensions::semantic_prompt::{PromptKind, SemanticPromptMarkers};
use crate::Prompt;

use super::utils::strip_ansi;

/// A representation of a buffer with styling, used for doing syntax highlighting
#[derive(Clone)]
pub struct StyledText {
    /// The component, styled parts of the text
    pub buffer: Vec<(Style, String)>,
}

impl Default for StyledText {
    fn default() -> Self {
        Self::new()
    }
}

impl StyledText {
    /// Construct a new `StyledText`
    pub const fn new() -> Self {
        Self { buffer: vec![] }
    }

    /// Add a new styled string to the buffer
    pub fn push(&mut self, styled_string: (Style, String)) {
        self.buffer.push(styled_string);
    }

    /// Overwrite the style of `[from, to)` with `new_style`, replacing the
    /// foreground, background, and attributes of any text in that range.
    pub fn style_range(&mut self, from: usize, to: usize, new_style: Style) {
        self.map_range(from, to, |_| new_style);
    }

    /// Composite `new_style` over the styles already in `[from, to)` rather than
    /// replacing them (see [`overlay`]): `new_style`'s foreground/background win
    /// where set, and the existing text's colors and attributes show through
    /// otherwise. `new_style`'s *attributes* (bold, reverse, …) are intentionally
    /// not applied — callers wanting those should use [`style_range`]. Use this
    /// for selection highlighting that stays readable over syntax colors.
    pub fn overlay_range(&mut self, from: usize, to: usize, new_style: Style) {
        self.map_range(from, to, |s| overlay(s, new_style));
    }

    /// Apply `f` to the style of every run overlapping `[from, to)`, splitting
    /// runs at the boundaries so only the overlapping slice is transformed.
    /// Rebuilds the run buffer; empty pieces are dropped.
    ///
    /// `from`/`to` are byte offsets and must fall on `char` boundaries (callers
    /// pass grapheme-aligned positions from `get_selection`).
    fn map_range(&mut self, from: usize, to: usize, f: impl Fn(Style) -> Style) {
        let (from, to) = (from.min(to), to.max(from));
        let mut out = Vec::with_capacity(self.buffer.len());
        let mut start = 0;
        for (style, text) in std::mem::take(&mut self.buffer) {
            let len = text.len();
            let a = from.saturating_sub(start).min(len);
            let b = to.saturating_sub(start).min(len);
            if a > 0 {
                out.push((style, text[..a].to_string()));
            } // before
            if b > a {
                out.push((f(style), text[a..b].to_string()));
            } // inside (needs to be styled)
            if b < len {
                out.push((style, text[b..].to_string()));
            } // outside
            start += len;
        }
        self.buffer = out;
    }

    /// Render the styled string. We use the insertion point to render around so that
    /// we can properly write out the styled string to the screen and find the correct
    /// place to put the cursor. This assumes a logic that prints the first part of the
    /// string, saves the cursor position, prints the second half, and then restores
    /// the cursor position
    ///
    /// Also inserts the multiline continuation prompt with optional semantic markers
    pub fn render_around_insertion_point(
        &self,
        insertion_point: usize,
        prompt: &dyn Prompt,
        use_ansi_coloring: bool,
        semantic_markers: Option<&dyn SemanticPromptMarkers>,
    ) -> (String, String) {
        let mut current_idx = 0;
        let mut left_string = String::new();
        let mut right_string = String::new();

        let multiline_prompt = prompt.render_prompt_multiline_indicator();
        let prompt_style = Style::new().fg(prompt.get_prompt_multiline_color());

        for pair in &self.buffer {
            if current_idx >= insertion_point {
                right_string.push_str(&render_as_string(
                    pair,
                    &prompt_style,
                    &multiline_prompt,
                    semantic_markers,
                ));
            } else if pair.1.len() + current_idx <= insertion_point {
                left_string.push_str(&render_as_string(
                    pair,
                    &prompt_style,
                    &multiline_prompt,
                    semantic_markers,
                ));
            } else if pair.1.len() + current_idx > insertion_point {
                let offset = insertion_point - current_idx;

                let left_side = pair.1[..offset].to_string();
                let right_side = pair.1[offset..].to_string();

                left_string.push_str(&render_as_string(
                    &(pair.0, left_side),
                    &prompt_style,
                    &multiline_prompt,
                    semantic_markers,
                ));
                right_string.push_str(&render_as_string(
                    &(pair.0, right_side),
                    &prompt_style,
                    &multiline_prompt,
                    semantic_markers,
                ));
            }
            current_idx += pair.1.len();
        }

        if use_ansi_coloring {
            (left_string, right_string)
        } else {
            (strip_ansi(&left_string), strip_ansi(&right_string))
        }
    }

    /// Apply the ANSI style formatting to the full string.
    pub fn render_simple(&self) -> String {
        self.buffer
            .iter()
            .map(|(style, text)| style.paint(text).to_string())
            .collect()
    }

    /// Get the unformatted text as a single continuous string.
    pub fn raw_string(&self) -> String {
        self.buffer.iter().map(|(_, str)| str.as_str()).collect()
    }
}

fn render_as_string(
    renderable: &(Style, String),
    prompt_style: &Style,
    multiline_prompt: &str,
    semantic_markers: Option<&dyn SemanticPromptMarkers>,
) -> String {
    let mut rendered = String::new();

    // Build the formatted multiline prompt with optional semantic markers
    let formatted_multiline_prompt = if let Some(markers) = semantic_markers {
        // Wrap multiline indicator with secondary prompt markers:
        // \n + A;k=s + multiline_prompt + B
        format!(
            "\n{}{}{}",
            markers.prompt_start(PromptKind::Secondary),
            multiline_prompt,
            markers.command_input_start()
        )
    } else {
        format!("\n{multiline_prompt}")
    };

    for (line_number, line) in renderable.1.split('\n').enumerate() {
        if line_number != 0 {
            rendered.push_str(&prompt_style.paint(&formatted_multiline_prompt).to_string());
        }
        rendered.push_str(&renderable.0.paint(line).to_string());
    }
    rendered
}

/// Merge `sel` over `old`: `sel`'s foreground/background win where set, and the
/// rest of `old` — its foreground fallback and attributes (bold, italic, …) —
/// shows through. This is what lets a selection *tint* text without discarding
/// the underlying syntax highlighting.
fn overlay(old: Style, sel: Style) -> Style {
    Style {
        foreground: sel.foreground.or(old.foreground),
        background: sel.background.or(old.background),
        ..old
    }
}

#[cfg(test)]
mod test {
    use nu_ansi_term::{Color, Style};

    use crate::StyledText;

    fn get_styled_text_template() -> (super::StyledText, Style, Style) {
        let before_style = Style::new().on(Color::Black);
        let after_style = Style::new().on(Color::Red);
        (
            super::StyledText {
                buffer: vec![
                    (before_style, "aaa".into()),
                    (before_style, "bbb".into()),
                    (before_style, "ccc".into()),
                ],
            },
            before_style,
            after_style,
        )
    }
    #[test]
    fn style_range_partial_update_one_part() {
        let (styled_text_template, before_style, after_style) = get_styled_text_template();
        let mut styled_text = styled_text_template.clone();
        styled_text.style_range(0, 1, after_style);
        assert_eq!(styled_text.buffer[0], (after_style, "a".into()));
        assert_eq!(styled_text.buffer[1], (before_style, "aa".into()));
        assert_eq!(styled_text.buffer[2], (before_style, "bbb".into()));
        assert_eq!(styled_text.buffer[3], (before_style, "ccc".into()));
    }
    #[test]
    fn style_range_complete_update_one_part() {
        let (styled_text_template, before_style, after_style) = get_styled_text_template();
        let mut styled_text = styled_text_template.clone();
        styled_text.style_range(0, 3, after_style);
        assert_eq!(styled_text.buffer[0], (after_style, "aaa".into()));
        assert_eq!(styled_text.buffer[1], (before_style, "bbb".into()));
        assert_eq!(styled_text.buffer[2], (before_style, "ccc".into()));
        assert_eq!(styled_text.buffer.len(), 3);
    }
    #[test]
    fn style_range_update_over_boundary() {
        let (styled_text_template, before_style, after_style) = get_styled_text_template();
        let mut styled_text = styled_text_template;
        styled_text.style_range(0, 5, after_style);
        assert_eq!(styled_text.buffer[0], (after_style, "aaa".into()));
        assert_eq!(styled_text.buffer[1], (after_style, "bb".into()));
        assert_eq!(styled_text.buffer[2], (before_style, "b".into()));
        assert_eq!(styled_text.buffer[3], (before_style, "ccc".into()));
    }
    #[test]
    fn style_range_update_over_part() {
        let (styled_text_template, before_style, after_style) = get_styled_text_template();
        let mut styled_text = styled_text_template;
        styled_text.style_range(1, 7, after_style);
        assert_eq!(styled_text.buffer[0], (before_style, "a".into()));
        assert_eq!(styled_text.buffer[1], (after_style, "aa".into()));
        assert_eq!(styled_text.buffer[2], (after_style, "bbb".into()));
        assert_eq!(styled_text.buffer[3], (after_style, "c".into()));
        assert_eq!(styled_text.buffer[4], (before_style, "cc".into()));
    }
    #[test]
    fn style_range_last_letter() {
        let (_, before_style, after_style) = get_styled_text_template();
        let mut styled_text = StyledText {
            buffer: vec![(before_style, "asdf".into())],
        };
        styled_text.style_range(3, 4, after_style);
        assert_eq!(styled_text.buffer[0], (before_style, "asd".into()));
        assert_eq!(styled_text.buffer[1], (after_style, "f".into()));
    }
    #[test]
    fn style_range_from_second_to_last() {
        let (_, before_style, after_style) = get_styled_text_template();
        let mut styled_text = StyledText {
            buffer: vec![(before_style, "asdf".into())],
        };
        styled_text.style_range(2, 3, after_style);
        assert_eq!(styled_text.buffer[0], (before_style, "as".into()));
        assert_eq!(styled_text.buffer[1], (after_style, "d".into()));
        assert_eq!(styled_text.buffer[2], (before_style, "f".into()));
    }
    #[test]
    fn regression_style_range_cargo_run() {
        let (_, before_style, after_style) = get_styled_text_template();
        let mut styled_text = StyledText {
            buffer: vec![
                (before_style, "cargo".into()),
                (before_style, " ".into()),
                (before_style, "run".into()),
            ],
        };
        styled_text.style_range(8, 7, after_style);
        assert_eq!(styled_text.buffer[0], (before_style, "cargo".into()));
        assert_eq!(styled_text.buffer[1], (before_style, " ".into()));
        assert_eq!(styled_text.buffer[2], (before_style, "r".into()));
        assert_eq!(styled_text.buffer[3], (after_style, "u".into()));
        assert_eq!(styled_text.buffer[4], (before_style, "n".into()));
    }

    #[test]
    fn test_render_multiline_without_semantic_markers() {
        let style = Style::new();
        let renderable = (style, "line1\nline2".to_string());
        let prompt_style = Style::new();
        let multiline_prompt = "::: ";

        // Without semantic markers, just get newline + multiline prompt
        let result = super::render_as_string(&renderable, &prompt_style, multiline_prompt, None);
        assert!(result.contains("\n::: "));
        assert!(!result.contains("\x1b]133;A;k=s"));
    }

    #[test]
    fn test_render_multiline_with_semantic_markers() {
        use crate::terminal_extensions::semantic_prompt::Osc133Markers;
        let style = Style::new();
        let renderable = (style, "line1\nline2".to_string());
        let prompt_style = Style::new();
        let multiline_prompt = "::: ";
        let markers = Osc133Markers;

        // With semantic markers, should wrap multiline prompt with A;k=s and B
        let result =
            super::render_as_string(&renderable, &prompt_style, multiline_prompt, Some(&markers));
        // The result should contain the secondary prompt marker before ::: and B after
        assert!(result.contains("\x1b]133;A;k=s\x1b\\"));
        assert!(result.contains("\x1b]133;B\x1b\\"));
    }

    #[test]
    fn test_render_single_line_no_markers_emitted() {
        use crate::terminal_extensions::semantic_prompt::Osc133Markers;
        let style = Style::new();
        let renderable = (style, "single line".to_string());
        let prompt_style = Style::new();
        let multiline_prompt = "::: ";
        let markers = Osc133Markers;

        // Single line should not emit any markers
        let result =
            super::render_as_string(&renderable, &prompt_style, multiline_prompt, Some(&markers));
        assert!(!result.contains("\x1b]133;A;k=s"));
        assert!(!result.contains("\x1b]133;B"));
    }

    #[test]
    fn overlay_fn_tints_bg_and_keeps_fg() {
        let syntax = Style::new().fg(Color::Green).bold();
        let selection = Style::new().on(Color::Blue); // background-only
        let merged = super::overlay(syntax, selection);
        assert_eq!(merged.foreground, Some(Color::Green)); // syntax fg preserved
        assert_eq!(merged.background, Some(Color::Blue)); // selection bg applied
        assert!(merged.is_bold); // syntax attribute preserved
    }

    #[test]
    fn overlay_fn_selection_fg_wins_when_set() {
        let syntax = Style::new().fg(Color::Green);
        let selection = Style::new().fg(Color::White).on(Color::Blue);
        let merged = super::overlay(syntax, selection);
        assert_eq!(merged.foreground, Some(Color::White)); // selection fg wins
        assert_eq!(merged.background, Some(Color::Blue));
    }

    #[test]
    fn overlay_range_splits_one_run_and_tints_middle() {
        let syntax = Style::new().fg(Color::Green);
        let mut styled_text = super::StyledText {
            buffer: vec![(syntax, "hello".into())],
        };
        // tint [1, 3) with a background-only selection
        styled_text.overlay_range(1, 3, Style::new().on(Color::Blue));
        // "hello" → "h" | "el" | "lo"; only the middle gains the bg, fg preserved
        assert_eq!(styled_text.buffer[0], (syntax, "h".into()));
        assert_eq!(
            styled_text.buffer[1],
            (Style::new().fg(Color::Green).on(Color::Blue), "el".into())
        );
        assert_eq!(styled_text.buffer[2], (syntax, "lo".into()));
    }

    #[test]
    fn overlay_range_tints_each_run_with_its_own_color() {
        let green = Style::new().fg(Color::Green);
        let red = Style::new().fg(Color::Red);
        let mut styled_text = super::StyledText {
            buffer: vec![(green, "ab".into()), (red, "cd".into())],
        };
        // selection [1, 3) straddles the boundary: "b" (green) and "c" (red)
        styled_text.overlay_range(1, 3, Style::new().on(Color::Blue));
        // each run keeps its own fg; only the bg is tinted on the overlapping slice
        assert_eq!(styled_text.buffer[0], (green, "a".into()));
        assert_eq!(
            styled_text.buffer[1],
            (Style::new().fg(Color::Green).on(Color::Blue), "b".into())
        );
        assert_eq!(
            styled_text.buffer[2],
            (Style::new().fg(Color::Red).on(Color::Blue), "c".into())
        );
        assert_eq!(styled_text.buffer[3], (red, "d".into()));
    }
}
