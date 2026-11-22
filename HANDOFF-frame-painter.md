# Frame painter v2: handoff

Working notes for `feat/frame-painter-v2`. Drop this file before the PR
merges.

State as of 2026-11-22, rebased onto `upstream/main` (`452867e`, with the
terminal-taken signal from #1297).

## Decision: replace, not flag

The frame painter replaces the streaming painter in this PR. There is no
user-facing switch and no env var. The branch ends with `print_small_buffer`,
`print_large_buffer`, `compute_layout`, `skip_buffer_lines`,
`render_snapshot` and `screen_to_buffer_offset` deleted.

Until then `Painter::paints_frames` is a crate-internal switch, on by
default, and `Painter::frame_can_paint` sends every paint the frame cannot
draw yet to the streaming painter. Each region the frame learns removes one
condition from it. The existing painter tests are the regression suite and
run against the frame. A test of the streaming painter's own mechanics opts
out with `paint_streaming_for_test` and goes away with it.

## What exists

`src/painting/frame/`, pure, no terminal:

| File | What |
|---|---|
| `token.rs` | `tokens(text)`: escapes and graphemes |
| `grid.rs` | `Frame`, `Row`, `Cell`, `Position { row, col }`. In a row: slot (place in `cells`), column (on screen), index (buffer grapheme) |
| `builder.rs` | the one grapheme walk. `position()` resolves a pending wrap, `rows_drawn()` leaves out the row a last newline opened |
| `layout.rs` | `LayoutInput` (left, indicator, continuation, before, after, hint, width) and `layout()` |
| `write.rs` | `write_frame(frame, top, anchor, width, height, out)`, full redraw from the anchor; `make_room(anchor, visible, height) -> Room { scroll, anchor }` |

Painter side:

- `StyledText::render_buffer_around`: the buffer with bare newlines, so the
  continuation takes no buffer index. The public
  `render_around_insertion_point` keeps its output.
- `PromptLines::continuation`, `with_bare_buffer`, `frame_buffer()`. The
  engine renders the bare buffer only while `paints_frames` is on.
- `src/painting/painter/frame.rs`: `frame_can_paint`, `repaint_frame`,
  `PromptParts`, `record_frame_exit`, `paint_streaming_for_test`.

`repaint_frame` order, which has to hold: reset and hide, prompt parts,
layout, `top`, `resolve_resize(cursor.row - top + 1)`,
`reconcile_stale_anchor`, height and `top` again, then `make_room`, scroll,
`mark_verified`, `write_frame`, exit state, cursor shape and show,
`end_frame`, flush. Nothing is drawn before the terminal queries finish.
No synchronized update of its own (#1302).

Screen tests at the end of `painter/screen_tests.rs`. `both_painters` paints
the same thing through both painters and compares rows, scrollback and
cursor; six cases match, and three more after a resize. The resize tests
start from the block as the terminal reflowed it, since `TestScreen` cannot
reflow.

## Scope: parity on the merits

No API redesign in this PR. Where the two painters differ, the one that is
wrong gets fixed: a streaming painter bug goes to `main` as its own PR, so
`both_painters` stays an honest oracle and the fix ships before the frame.

## Where the streaming painter is wrong

All three on a buffer taller than the screen, found with the resize tests.
The frame is right in each, so tall-buffer tests assert on the frame alone.

- **An empty indicator scrolls a row out on every paint.** `compute_layout`
  counts `prompt_indicator.lines()`, which is 0 for `""`, so `extra_rows`
  comes out one short and the paint emits one row too many. Prompt `"> "`,
  no indicator, `1\n2\n3\n4\n5` on three rows: scrollback gains a `2` per
  paint. Same family as the ignored
  `large_buffer_of_one_wrapped_line_stays_on_screen`.
- **A paint that jumps straight to a tall buffer draws over the host output
  above it.** The large-buffer branch sets the anchor to row 0 and clears
  from there. The frame scrolls those rows into scrollback first.
- **A tall buffer ending in a newline loses its last row.** `lines()` drops
  the trailing empty line: the screen shows `2 3 4` with the cursor on the
  `4` row, where the frame shows `3 4` and the cursor on the empty row.

The first is fixed on `fix/large-buffer-empty-indicator` (one commit on
`upstream/main`, not pushed), to go to `main` as its own PR. The other two
go deeper into `print_large_buffer` and are left to the frame.

## Settled along the way

- A newline that ends the prompt adds no prompt row. That row is the
  buffer's, and the right prompt "on the last line" goes there, as the
  streaming painter places it today.
- Only the parts the `Prompt` trait already gives go into `LayoutInput`.
  No header/before/after parts and no new public trait until the layout has
  rules for them (#526).
- `top` is a frame row, `anchor` a screen row; frame row `r` is drawn on
  screen row `anchor + r - top`.
- `large_buffer` after a frame paint is `frame.height() > height`. Every
  reader of the field is streaming code that sets it again before reading,
  so it goes with the streaming painter.
- A tall frame keeps its top while the cursor's row fits below it, as the
  streaming painter does: `viewport_top` skips rows only to bring the
  cursor's row onto the last screen row. It used to show the bottom of the
  frame up to the cursor's row, which dropped the first prompt row of a
  frame one row too tall. `a_large_buffer_keeps_the_prompts_trailing_newline`
  caught it. Revisit when menus become frame rows: the viewport then has to
  keep some menu rows on screen too, as `large_buffer_offset` does today.

## What turning the switch on took

Ten lib tests failed. One was the viewport rule above. The rest:

- Five hint tests in `engine.rs` painted on an unsized painter, 0 by 0,
  where the frame draws nothing. They are sized now and use a short prompt,
  since a working directory long enough to wrap splits the bytes they
  search. The real start path turns an unreported size into 80 by 24; a
  `Resize(0, 0)` event still reaches the painter as it is.
- Byte assertions on the streaming painter's output moved to `TestScreen`,
  or opt out with `paint_streaming_for_test` where the bytes are the point
  (erasing first for a line of uncertain width).

## Next

1. Done: the resize screen tests. `top` in the `resolve_resize` argument is
   not pinned by them: the saturating subtraction gives the same anchor
   without it unless the terminal's cursor sits lower than the frame expects.
2. Done: `paints_frames` is on by default. **Clicks do nothing after a frame
   paint until step 3**, so the branch is not for daily use before it.
3. Clicks: `Frame::index_at(Position)` plus grapheme to byte on the raw
   buffer, replacing `last_layout` for frame paints. Watch the selection's
   blank cell (`styled_text.rs`, `shows_on_a_blank_cell`), which takes a
   buffer index in the frame.
4. Regions, one `frame_can_paint` condition each, each first as a
   `both_painters` case: status line and footer, right prompt, menus,
   semantic markers (OSC 133 as marks). Menus keep returning their painted
   strings through the `Menu` trait; the frame lays those out as rows.
5. Delete the streaming painter and the switch.

`CellStyle` and the row diff come after this PR: the streaming painter
redraws in full too, so neither is parity.

## Open

- **The frame places the cursor by its own column count.** `write_frame`
  ends with a `MoveTo` to the column the layout worked out. The streaming
  painter leaves the cursor where the terminal put it after printing, or
  saves and restores it, so it stays right on a terminal that measures a
  grapheme differently (ambiguous-width characters, some emoji). On such a
  terminal the frame's cursor lands a column off, and a row the frame
  thinks is full wraps early. Not seen on a real terminal yet; the
  `an_ambiguous_width` case shows the bytes. One way out for the common
  case: write the cursor's row last and skip the move when the cursor sits
  at the end of it.
- Multi-line hints get their continuation from `continue_hint_lines` in the
  engine. Harmless in the frame (hint cells take no index), but the layout
  could own it like the buffer's.
- `Room` has no doc of its own; `make_room`'s covers both fields.
- **Prompt scroll over ssh to Windows: fix committed, not yet confirmed.**
  Daren's recording (macOS, `ssh` to Windows, nu 0.117.1, two-row prompt
  with a right prompt) scrolls one row per keystroke once the prompt sits
  on the bottom row, leaving a copy of the first prompt row each paint.
  Seen with both painters. Turning off `shell_integration.osc133` was the
  only thing that changed it, which ruled out the right-prompt-in-the-last-
  column lead. Cause, as far as we can tell: the painter sent OSC 133 `A`
  on every paint. `A` starts a fresh line when the cursor is off column 0,
  and ConPTY passes it through ahead of the cursor move written before it.
  `2fe8f94` (and `1e8c302` on v1, #1299) sends `A` once per prompt and
  `P;k=i` on repaints. Still open:
  - Daren reruns with the fix. His `osc133` toggle also turned off mouse
    click mode and the per-keystroke `B` and `P;k=r`. If the scroll
    survives, those are next.
  - Which terminal he uses on the Mac. Ghostty does a fresh-line on `A`,
    Kitty does not. The bytes would settle it:
    `script -q ~/rprompt.log ssh windows`.

## Lessons from the OSC 133 bug

v1's frame copied the streaming painter's per-paint `A`, and the tests
could not see it. Before the semantic markers region (Next, step 4):

- **Marks are terminal state, not cells.** `A` starts a fresh line, arms
  `click_events` and leaves a mark on its row. Track "the terminal holds
  the prompt mark on row R" in the painter (`prompt_marked` today),
  separate from any frame cache. Change it only on events that move or
  wipe the mark, never when the cache is lost. v1 re-sent `A` from every
  `forget_frame`, so on every idle poll.
- **The marker belongs to the rewrite of the prompt's cells.** A diffed
  frame that leaves the prompt row alone sends no marker. One that rewrites
  prompt cells sends one before them, since Ghostty tags each cell with
  the semantic state the cursor has when it is written, and after `B`
  that is input. The lifecycle above only picks which: `A` for a new or
  moved prompt, `P` otherwise. Whatever the paint rewrote, the last marker
  it sends is `B`, so the next keystroke's text counts as input.
- **`write_frame` emits nothing that moves the cursor except its own
  moves.** List the exceptions and keep them off the per-paint path: `A`
  (fresh line), a cell in the last column (pending wrap), `\r\n`. A test
  that scans one paint's bytes for them is cheap.
- **`both_painters` is the wrong oracle for markers.** It treats the
  streaming painter as correct, and `vt100` drops OSC 133, so the test
  screen never shows a fresh line from `A`. Test markers against the
  protocol instead: one `A` per prompt, and the cursor in column 0 when
  it arrives. Better, teach `TestScreen` OSC 133 (row marks and Ghostty's
  fresh-line on `A`), so a stray scroll shows up as a screen diff.
- **Seen with both painters means shared code.** Look at markers, the
  `Prompt` trait and the prompt rendering before either painter's layout.
  The right-prompt lead was a layout theory, and layout is what the two
  painters do not share.

## Not pushed

The fork holds the rebase onto `452867e` up to `9077398`. Everything after
it is local, and a plain push is enough:
`git push origin feat/frame-painter-v2`.
