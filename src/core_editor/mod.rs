mod clip_buffer;
mod cursor;
mod edit_stack;
mod editor;
mod graphemes;
mod line_buffer;
mod rest_policy;
mod set_selection;
mod word;

#[cfg(feature = "system_clipboard")]
pub(crate) use clip_buffer::get_system_clipboard;
pub(crate) use clip_buffer::{get_local_clipboard, Clipboard, ClipboardMode};
pub(crate) use cursor::Cursor;
pub use editor::Editor;
pub use line_buffer::LineBuffer;
pub(crate) use rest_policy::{commit, RestPolicy};
pub(crate) use set_selection::resolve_motion;
