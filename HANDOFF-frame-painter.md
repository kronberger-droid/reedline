# Frame painter v2: handoff

Working notes for `feat/frame-painter-v2`. Drop this file before the PR
merges.

State as of 2026-11-20, head `2fe8f94`, rebased onto `upstream/main`
(`f1d6069`, with the prompt footer from #1242).

## Decision: replace, not flag

The frame painter replaces the streaming painter in this PR. There is no
user-facing switch and no env var. The branch ends with `print_small_buffer`,
`print_large_buffer`, `compute_layout`, `skip_buffer_lines`,
`render_snapshot` and `screen_to_buffer_offset` deleted.

Until then `Painter::paints_frames` is a crate-internal switch (off by
default), and `Painter::frame_can_paint` sends every paint the frame cannot
draw yet to the streaming painter. Each region the frame learns removes one
condition from it. The existing painter tests are the regression suite:
once the switch is on by default they run against the frame.

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
  `PromptParts`, `record_frame_exit`, `paint_frames_for_test`.

`repaint_frame` order, which has to hold: reset and hide, prompt parts,
layout, `top`, `resolve_resize(cursor.row - top + 1)`,
`reconcile_stale_anchor`, height and `top` again, then `make_room`, scroll,
`mark_verified`, `write_frame`, exit state, cursor shape and show,
`end_frame`, flush. Nothing is drawn before the terminal queries finish.
No synchronized update of its own (#1302).

Screen tests at the end of `painter/screen_tests.rs`. `both_painters` paints
the same thing through both painters and compares rows, scrollback and
cursor; six cases match.

## Settled along the way

- A newline that ends the prompt adds no prompt row. That row is the
  buffer's, and the right prompt "on the last line" goes there, as the
  streaming painter places it today.
- Only the parts the `Prompt` trait already gives go into `LayoutInput`.
  No header/before/after parts and no new public trait until the layout has
  rules for them (#526).
- `top` is a frame row, `anchor` a screen row; frame row `r` is drawn on
  screen row `anchor + r - top`.

## With the switch on by default: 10 of 2543 lib tests fail

Measured on `30dcc6e`, then reverted. No screen-level regression seen; not
debugged one by one.

| Tests | Why | Likely |
|---|---|---|
| `a_paint_that_erases_first_leaves_nothing_behind` (2 cases), `a_large_buffer_keeps_the_prompts_trailing_newline` | assert streaming bytes (`\x1b[2;1H\x1b[J`, `ab\r\n> Z`) | rewrite on `TestScreen` or drop with the streaming path; the screen half of the first passes |
| 5 hint tests in `engine.rs` | `contains` on captured bytes | same, but check each: these may run at a tiny width, which splits the hint across rows |
| `a_paint_that_reserves_nothing_draws_no_status_rows` | `last_layout.expect` | expected until clicks read the frame |
| `a_plain_paint_sends_no_frame_marker::large_buffer` | `large_buffer` is `frame.height() > height` here, `required_lines >= height` in the streaming painter | decide what `large_buffer` still means |

## Next

1. A resize screen test: `handle_resize`, then a paint through
   `resolve_resize` with `top`.
2. Work through the 10 failures above, then turn `paints_frames` on by
   default.
3. Clicks: `Frame::index_at(Position)` plus grapheme to byte on the raw
   buffer, replacing `last_layout` for frame paints. Watch the selection's
   blank cell (`styled_text.rs`, `shows_on_a_blank_cell`), which takes a
   buffer index in the frame.
4. Regions, one `frame_can_paint` condition each, each first as a
   `both_painters` case: status line and footer, right prompt, menus,
   semantic markers (OSC 133 as marks), then `CellStyle` and the row diff.
5. Delete the streaming painter and the switch.

## Open

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

The rebase rewrote commits already on the fork:
`git push --force-with-lease origin feat/frame-painter-v2`.
