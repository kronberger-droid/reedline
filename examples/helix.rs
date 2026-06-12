// Create a reedline object with the experimental Helix edit mode.
//
//     cargo run --example helix --features helix
//
// Helix is selection-first: motions select the text they travel over, and
// operators act on that selection. Watch the highlighted span follow `w`/`b`/
// `e`/`f` before you press `d`, `c` or `y`.

use reedline::{DefaultPrompt, Helix, Reedline, Signal};
use std::io;

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
  d/c/y      delete / change / yank the selection (or the cursor char)
  p/P        paste after / before,  u/U  undo / redo,  r<ch>  replace
  %          select all,  ~  switch case

Submit with Enter (returns to insert). Abort with Ctrl-C, quit with Ctrl-D."
    );

    let prompt = DefaultPrompt::default();
    let mut line_editor = Reedline::create().with_edit_mode(Box::new(Helix::default()));

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
