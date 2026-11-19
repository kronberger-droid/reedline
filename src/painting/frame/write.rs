use super::grid::Frame;
use std::io::{Result, Write};

use crossterm::{
    cursor::MoveTo,
    style::Print,
    terminal::{Clear, ClearType},
    QueueableCommand,
};

/// Draw `frame` on the screen with its row `top` on screen row `anchor`,
/// and leave the cursor where the frame puts it.
///
/// Draws only what fits between `anchor` and the bottom of the screen and
/// clears the rows below. Making room first, by scrolling, is the
/// caller's job, as are flushing and hiding the cursor while drawing.
/// A row that fills the width is not cleared to its end: the terminal's
/// cursor still sits on its last cell, and the clear would erase it.
pub(crate) fn write_frame(
    frame: &Frame,
    top: u16,
    anchor: u16,
    width: u16,
    height: u16,
    out: &mut impl Write,
) -> Result<()> {
    let visible = frame
        .height()
        .saturating_sub(top)
        .min(height.saturating_sub(anchor));

    for i in 0..visible {
        out.queue(MoveTo(0, anchor + i))?;
        let row = &frame.rows[(top + i) as usize];
        let line: String = row.cells.iter().map(|c| c.grapheme.as_str()).collect();
        out.queue(Print(line))?;

        if row.width() < width {
            out.queue(Clear(ClearType::UntilNewLine))?;
        }
    }

    let below = anchor + visible;
    if below < height {
        out.queue(MoveTo(0, below))?
            .queue(Clear(ClearType::FromCursorDown))?;
    }

    out.queue(MoveTo(
        frame.cursor.col,
        anchor.saturating_add(frame.cursor.row).saturating_sub(top),
    ))?;
    Ok(())
}
