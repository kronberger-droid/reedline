// Create a reedline object with the experimental Helix edit mode.
//
//     cargo run --example helix --features helix
//
// Helix is selection-first: motions select the text they travel over, and
// operators act on that selection. Watch the highlighted span follow `w`/`b`/
// `e`/`f` before you press `d`, `c` or `y`.
//
// The demo styles the experience like Helix itself: a blinking bar cursor in
// insert mode, a steady block in normal/select mode (resting on exactly one
// grapheme — the min-width-1 invariant), a bluish selection highlight, and a
// NOR/INS marker in the prompt.

use crossterm::cursor::SetCursorStyle;
use nu_ansi_term::{Color, Style};
use reedline::{
    CursorConfig, DefaultPrompt, Helix, Prompt, PromptEditMode, PromptHistorySearch, PromptViMode,
    Reedline, Signal,
};
use std::borrow::Cow;
use std::io;

/// A prompt that shows which Helix mode you are in, like Helix's statusline.
struct HelixPrompt {
    inner: DefaultPrompt,
}

impl Prompt for HelixPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        self.inner.render_prompt_left()
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        self.inner.render_prompt_right()
    }

    fn render_prompt_indicator(&self, edit_mode: PromptEditMode) -> Cow<'_, str> {
        match edit_mode {
            PromptEditMode::Helix(PromptViMode::Normal) => " NOR ❯ ".into(),
            PromptEditMode::Helix(PromptViMode::Insert) => " INS ❯ ".into(),
            other => self.inner.render_prompt_indicator(other),
        }
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        self.inner.render_prompt_multiline_indicator()
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        self.inner
            .render_prompt_history_search_indicator(history_search)
    }
}

fn main() -> io::Result<()> {
    println!(
        "Helix edit mode demo. You start in insert mode; type away.

  Esc        normal mode        i/a/I/A/o/O  back to insert
  h/l        move by grapheme   j/k          history (or lines)
  w/b/e      select word motion (W/B/E for WORDS), counts work: 3w
  f/t/F/T    select to a character
  gh/gl/gs   line start / line end / first non-blank
  gg/ge      buffer start / buffer end
  x          select the current line
  v          select mode (motions extend), ;  collapse, Alt-;  flip ends
  d/c/y      delete / change / yank the selection (or the cursor grapheme)
  p/P        paste after / before,  u/U  undo / redo,  r<ch>  replace
  %          select all,  ~  switch case

Submit with Enter (returns to insert). Abort with Ctrl-C, quit with Ctrl-D."
    );

    let cursor_config = CursorConfig {
        vi_insert: Some(SetCursorStyle::BlinkingBar),
        vi_normal: Some(SetCursorStyle::SteadyBlock),
        emacs: None,
    };
    // Helix-style selection: a bluish background under the selected span.
    let selection_style = Style::new().on(Color::Rgb(45, 60, 95)).fg(Color::White);

    let prompt = HelixPrompt {
        inner: DefaultPrompt::default(),
    };
    let mut line_editor = Reedline::create()
        .with_edit_mode(Box::new(Helix::default()))
        .with_cursor_config(cursor_config)
        .with_visual_selection_style(selection_style);

    loop {
        let sig = line_editor.read_line(&prompt)?;
        match sig {
            Signal::Success(buffer) => {
                println!("We processed: {buffer}");
            }
            Signal::CtrlD | Signal::CtrlC => {
                println!("\nAborted!");
                break Ok(());
            }
            _ => {}
        }
    }
}
