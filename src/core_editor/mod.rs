mod clip_buffer;
mod edit_stack;
mod editor;
mod graphemes;
mod line_buffer;

#[cfg(feature = "system_clipboard")]
pub(crate) use clip_buffer::get_system_clipboard;
pub(crate) use clip_buffer::{get_local_clipboard, Clipboard, ClipboardMode};
pub use editor::Editor;
#[cfg(feature = "helix")]
pub(crate) use graphemes::{is_grapheme_boundary, next_grapheme_boundary, prev_grapheme_boundary};
pub use line_buffer::LineBuffer;
