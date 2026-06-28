use std::cmp::Ordering;

use crate::core_editor::{
    graphemes::{
        ensure_grapheme_boundary_next, ensure_grapheme_boundary_prev, next_grapheme_boundary,
        prev_grapheme_boundary,
    },
    Cursor,
};

/// Where the cursor's head may *rest* after an edit, per editor paradigm.
///
/// Applied at the single commit boundary by [`commit`], layered on top of the
/// universal coherence pass ([`recohere`]). Each edit mode maps to one variant;
/// the core never inspects the mode, only the resulting policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestPolicy {
    /// The head may rest anywhere between graphemes, including past the last one
    /// (at `buf.len()`). Emacs / Vi insert — the ordinary text caret.
    Between,
    /// The head may not rest past the last grapheme: a head at the end is pulled
    /// back onto the final grapheme. Vi normal mode — the cursor invariant
    /// enforced structurally here instead of by a maintenance command.
    OnGrapheme,
    /// The resting cursor always covers exactly one grapheme: a point is widened
    /// onto the grapheme to its right, or — at the buffer end, where there is
    /// none — onto the grapheme to its left. Mirrors Helix's `Range::min_width_1`.
    /// The policy for Vi visual mode, whose selection is always at least the
    /// grapheme it sits on.
    ///
    /// The line terminator is *not* a coverable cell here: a point at the end of
    /// an interior line widens backward onto the line's last grapheme, never
    /// forward onto the `\n` — matching vim, whose visual cursor never sits on the
    /// newline.
    Block,
    /// Like [`Block`](Self::Block), but the line terminator **is** a coverable
    /// cell: a point sitting at the end of an interior line widens *forward* onto
    /// the `\n` (or `\r\n`) so the block rests on the newline itself, the way
    /// Helix's block cursor can. At the true buffer end (no terminator to the
    /// right) it still widens backward onto the last grapheme, exactly like
    /// `Block`. The policy for Helix normal and select modes.
    BlockEol,
}

impl RestPolicy {
    /// Whether this policy lets the block cursor *rest on* the line terminator
    /// (treating `\n`/`\r\n` as an ordinary one-grapheme cell) rather than
    /// skipping past it. Only [`BlockEol`](Self::BlockEol) does — the movement
    /// layer reads this to decide whether a grapheme step may land on the
    /// newline.
    pub(crate) fn rests_on_line_terminator(self) -> bool {
        matches!(self, RestPolicy::BlockEol)
    }
}

/// Make a cursor *coherent* for `buf`: clamp both ends into `[0, buf.len()]` and
/// snap them to grapheme boundaries, expanding outward so whole graphemes stay
/// covered — the low end floors, the high end ceils, and a point floors while
/// staying a point.
///
/// Universal across edit modes and idempotent, so it is safe to run after any
/// edit. The mode-specific resting rule is layered on top by [`commit`].
fn recohere(buf: &str, c: Cursor) -> Cursor {
    let len = buf.len();

    let head = c.head().min(len);
    let anchor = c.anchor().min(len);

    let (anchor, head) = match anchor.cmp(&head) {
        // point: both floor to keep it a point
        Ordering::Equal => {
            let pos = ensure_grapheme_boundary_prev(buf, anchor);
            (pos, pos)
        }
        // forward: anchor (low) floors, head (high) ceils
        Ordering::Less => (
            ensure_grapheme_boundary_prev(buf, anchor),
            ensure_grapheme_boundary_next(buf, head),
        ),
        // backward: anchor (high) ceils, head (low) floors
        Ordering::Greater => (
            ensure_grapheme_boundary_next(buf, anchor),
            ensure_grapheme_boundary_prev(buf, head),
        ),
    };

    Cursor::new(anchor, head)
}

/// Normalize a cursor at the one commit boundary: first [`recohere`] it (universal
/// coherence), then apply the mode's [`RestPolicy`] resting rule.
///
/// Total and idempotent — `commit(buf, commit(buf, c, p), p) == commit(buf, c, p)`
/// for every input — so it can run after every command without the cursor drifting.
pub(crate) fn commit(buf: &str, c: Cursor, policy: RestPolicy) -> Cursor {
    let c = recohere(buf, c);
    let len = buf.len();
    match policy {
        RestPolicy::Between => c,
        RestPolicy::OnGrapheme => {
            // A *bare* cursor (an empty point) may not rest past the last grapheme
            // of its line: pull it back off the line terminator (or the buffer end)
            // onto that grapheme. Skip when the line is empty — the char before is
            // itself a terminator (or the buffer start), so column 0 is the only
            // cell and pulling back would cross into the previous line. A selection
            // may legitimately cover the final grapheme (head at the boundary,
            // caret one grapheme inward), so its head is left alone — inclusivity
            // is geometric now, not a query-time `+1`.
            let head = c.head();
            let past_last_grapheme = head == len || buf[head..].starts_with(['\n', '\r']);
            let line_has_grapheme = head > 0 && !buf[..head].ends_with(['\n', '\r']);
            if c.is_empty() && past_last_grapheme && line_has_grapheme {
                Cursor::point(prev_grapheme_boundary(buf, head))
            } else {
                c
            }
        }
        // `Block` treats the line terminator as a wall (widen backward off it);
        // `BlockEol` treats it as a coverable cell (widen forward onto it). Only a
        // resting *point* needs adjusting — an existing selection is already a range.
        RestPolicy::Block => widen_block(buf, c, false),
        RestPolicy::BlockEol => widen_block(buf, c, true),
    }
}

/// Widen a resting *point* into a one-grapheme block; an existing range is left
/// untouched. `cover_terminator` decides the behavior at the end of an interior
/// line: when `true` the block widens *forward* onto the `\n`/`\r\n` (Helix —
/// the newline is a cell); when `false` it widens *backward* onto the line's last
/// grapheme (Vi visual — the newline is a wall). At the true buffer end both
/// widen backward, since there is no grapheme to the right.
fn widen_block(buf: &str, c: Cursor, cover_terminator: bool) -> Cursor {
    if !c.is_empty() {
        return c;
    }
    let head = c.head();
    let next = next_grapheme_boundary(buf, head);
    let on_terminator = buf[head..].starts_with(['\n', '\r']);
    if next > head && (cover_terminator || !on_terminator) {
        // widen forward onto the grapheme to the right (the `\n` too, for
        // `BlockEol`): [head, next)
        c.move_head(next)
    } else if head > 0 && !buf[..head].ends_with(['\n', '\r']) {
        // nothing coverable to the right (buffer end, or a wall under `Block`), so
        // cover the last grapheme of the line instead: [prev, head)
        Cursor::new(prev_grapheme_boundary(buf, head), head)
    } else {
        // empty buffer, or an empty line whose only cell is the terminator we
        // refuse to cover (`Block`): stay a zero-width point
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Buffers used across tests:
    //   "hello"   — ASCII, len 5, grapheme starts 0,1,2,3,4
    //   "caf\u{e9}" — precomposed é (2 bytes), len 5, last grapheme starts at 3
    //   "ae\u{0301}" — e + combining acute, len 4; "é" grapheme is [1,4),
    //                  so byte 2 is a char boundary but *not* a grapheme boundary
    const MIXED: &str = "a日e\u{0301}👨‍👩‍👧"; // ASCII + CJK + combining + ZWJ emoji

    // --- recohere ------------------------------------------------------------

    #[test]
    fn recohere_is_identity_on_aligned_cursors() {
        let b = "hello";
        assert_eq!(recohere(b, Cursor::new(1, 3)), Cursor::new(1, 3));
        assert_eq!(recohere(b, Cursor::new(3, 1)), Cursor::new(3, 1));
        assert_eq!(recohere(b, Cursor::point(2)), Cursor::point(2));
    }

    #[test]
    fn recohere_clamps_past_end_to_point_at_len() {
        assert_eq!(recohere("hello", Cursor::new(10, 12)), Cursor::point(5));
    }

    #[test]
    fn recohere_floors_mid_grapheme_point() {
        // point at byte 2 sits inside "é" [1,4) → floors to its start, 1
        assert_eq!(recohere("ae\u{0301}", Cursor::point(2)), Cursor::point(1));
    }

    #[test]
    fn recohere_expands_selection_outward() {
        // anchor 0 (low) floors to 0, head 2 (high, mid-grapheme) ceils to 4
        assert_eq!(recohere("ae\u{0301}", Cursor::new(0, 2)), Cursor::new(0, 4));
    }

    #[test]
    fn recohere_is_idempotent() {
        for a in (0..=MIXED.len()).filter(|&i| MIXED.is_char_boundary(i)) {
            for h in (0..=MIXED.len()).filter(|&i| MIXED.is_char_boundary(i)) {
                let once = recohere(MIXED, Cursor::new(a, h));
                assert_eq!(recohere(MIXED, once), once, "anchor={a} head={h}");
            }
        }
    }

    // --- commit: Between -----------------------------------------------------

    #[test]
    fn between_leaves_head_past_last_grapheme() {
        assert_eq!(
            commit("hello", Cursor::point(5), RestPolicy::Between),
            Cursor::point(5)
        );
    }

    #[test]
    fn between_only_recoheres() {
        // mid-grapheme point still floors (that's recohere); the policy adds nothing
        assert_eq!(
            commit("ae\u{0301}", Cursor::point(2), RestPolicy::Between),
            Cursor::point(1)
        );
    }

    // --- commit: OnGrapheme --------------------------------------------------

    #[test]
    fn on_grapheme_pulls_point_back_from_end() {
        assert_eq!(
            commit("hello", Cursor::point(5), RestPolicy::OnGrapheme),
            Cursor::point(4)
        );
    }

    #[test]
    fn on_grapheme_pulls_back_over_multibyte_grapheme() {
        // "café": last grapheme é starts at byte 3
        assert_eq!(
            commit("caf\u{e9}", Cursor::point(5), RestPolicy::OnGrapheme),
            Cursor::point(3)
        );
    }

    #[test]
    fn on_grapheme_leaves_midbuffer_point() {
        assert_eq!(
            commit("hello", Cursor::point(2), RestPolicy::OnGrapheme),
            Cursor::point(2)
        );
    }

    #[test]
    fn on_grapheme_rests_on_trailing_empty_line() {
        // A buffer ending in `\n` has an empty last line; a head at `len` sits
        // on it and must NOT be pulled back over the newline. This is the
        // `dd`-on-the-last-line case: deleting the final line leaves the cursor
        // on the new (empty) last line rather than jumping up a line.
        assert_eq!(
            commit("a\n", Cursor::point(2), RestPolicy::OnGrapheme),
            Cursor::point(2)
        );
        assert_eq!(
            commit("hello\n", Cursor::point(6), RestPolicy::OnGrapheme),
            Cursor::point(6)
        );
    }

    #[test]
    fn on_grapheme_pulls_back_from_interior_line_end() {
        // A point past the last grapheme of an *interior* line (the gap before the
        // `\n`) is pulled back onto that grapheme, like at the buffer end — vi
        // normal's caret never sits on the terminator. "abc\ndef": gap 3 -> 'c' (2).
        assert_eq!(
            commit("abc\ndef", Cursor::point(3), RestPolicy::OnGrapheme),
            Cursor::point(2)
        );
        // `\r\n` terminator: pull back over the whole grapheme. "ab\r\ncd": gap 2 -> 'b' (1).
        assert_eq!(
            commit("ab\r\ncd", Cursor::point(2), RestPolicy::OnGrapheme),
            Cursor::point(1)
        );
    }

    #[test]
    fn on_grapheme_rests_on_interior_empty_line() {
        // An empty interior line ("abc\n\ndef": the byte-4 newline is that line's
        // column 0) keeps the point — pulling back would cross up a line.
        assert_eq!(
            commit("abc\n\ndef", Cursor::point(4), RestPolicy::OnGrapheme),
            Cursor::point(4)
        );
    }

    #[test]
    fn on_grapheme_empty_buffer_is_noop() {
        assert_eq!(
            commit("", Cursor::point(0), RestPolicy::OnGrapheme),
            Cursor::point(0)
        );
    }

    #[test]
    fn on_grapheme_leaves_selection_head_at_end() {
        // A selection may cover the final grapheme: its head at `len` is left
        // alone (inclusivity is geometric now). Only a bare *point* is pulled
        // back off the end — see `on_grapheme_pulls_back_from_end`.
        assert_eq!(
            commit("hello", Cursor::new(0, 5), RestPolicy::OnGrapheme),
            Cursor::new(0, 5)
        );
    }

    // --- commit: Block -------------------------------------------------------

    #[test]
    fn block_widens_point_to_one_grapheme() {
        assert_eq!(
            commit("hello", Cursor::point(2), RestPolicy::Block),
            Cursor::new(2, 3)
        );
    }

    #[test]
    fn block_widens_over_multibyte_grapheme() {
        // "café": point at start of é (byte 3) widens to cover it, (3,5)
        assert_eq!(
            commit("caf\u{e9}", Cursor::point(3), RestPolicy::Block),
            Cursor::new(3, 5)
        );
    }

    #[test]
    fn block_point_at_end_widens_backward() {
        // no grapheme to the right at the end, so the block covers the last one:
        // point(5) → [4,5), caret on the 'o'
        assert_eq!(
            commit("hello", Cursor::point(5), RestPolicy::Block),
            Cursor::new(4, 5)
        );
    }

    #[test]
    fn block_widens_backward_over_multibyte_at_end() {
        // "café": point at end (5) → block covers the 2-byte é → [3,5)
        assert_eq!(
            commit("caf\u{e9}", Cursor::point(5), RestPolicy::Block),
            Cursor::new(3, 5)
        );
    }

    #[test]
    fn block_empty_buffer_stays_empty() {
        assert_eq!(
            commit("", Cursor::point(0), RestPolicy::Block),
            Cursor::point(0)
        );
    }

    #[test]
    fn block_does_not_rest_on_crlf_terminator() {
        // A point on the `\r` of a CRLF terminator must not widen forward over
        // the line break; it covers the last real grapheme of the line instead.
        // Mirrors the `['\n','\r']` guard OnGrapheme already uses. "ab\r\ncd":
        // point at byte 2 (the `\r`) → block over 'b' at [1,2), not [2,4).
        assert_eq!(
            commit("ab\r\ncd", Cursor::point(2), RestPolicy::Block),
            Cursor::new(1, 2)
        );
    }

    #[test]
    fn block_leaves_existing_selection() {
        assert_eq!(
            commit("hello", Cursor::new(1, 3), RestPolicy::Block),
            Cursor::new(1, 3)
        );
    }

    // --- commit: BlockEol ----------------------------------------------------

    #[test]
    fn block_eol_rests_on_interior_newline() {
        // The Helix block: a point at the end of an interior line widens *forward*
        // onto the `\n` (the cell `Block` would refuse). "ab\ncd": point(2) → [2,3).
        assert_eq!(
            commit("ab\ncd", Cursor::point(2), RestPolicy::BlockEol),
            Cursor::new(2, 3)
        );
    }

    #[test]
    fn block_eol_rests_on_crlf_terminator() {
        // Widens forward over the whole `\r\n` grapheme. "ab\r\ncd": point(2) → [2,4).
        assert_eq!(
            commit("ab\r\ncd", Cursor::point(2), RestPolicy::BlockEol),
            Cursor::new(2, 4)
        );
    }

    #[test]
    fn block_eol_covers_empty_interior_line() {
        // An empty interior line's only cell *is* the terminator; `BlockEol` covers
        // it rather than staying a point. "ab\n\ncd": point(3) → [3,4) on the 2nd `\n`.
        assert_eq!(
            commit("ab\n\ncd", Cursor::point(3), RestPolicy::BlockEol),
            Cursor::new(3, 4)
        );
    }

    #[test]
    fn block_eol_widens_backward_at_buffer_end() {
        // No terminator (nor any grapheme) to the right at the true end, so it
        // behaves like `Block`: point(5) → [4,5) on the last 'o'.
        assert_eq!(
            commit("hello", Cursor::point(5), RestPolicy::BlockEol),
            Cursor::new(4, 5)
        );
    }

    #[test]
    fn block_eol_widens_forward_midline_like_block() {
        // Away from any terminator it is identical to `Block`: point(2) → [2,3).
        assert_eq!(
            commit("hello", Cursor::point(2), RestPolicy::BlockEol),
            Cursor::new(2, 3)
        );
    }

    #[test]
    fn block_eol_leaves_existing_selection() {
        assert_eq!(
            commit("ab\ncd", Cursor::new(0, 3), RestPolicy::BlockEol),
            Cursor::new(0, 3)
        );
    }

    // --- commit: idempotency across every policy and char boundary -----------

    #[test]
    fn commit_is_idempotent() {
        for policy in [
            RestPolicy::Between,
            RestPolicy::OnGrapheme,
            RestPolicy::Block,
            RestPolicy::BlockEol,
        ] {
            for a in (0..=MIXED.len()).filter(|&i| MIXED.is_char_boundary(i)) {
                for h in (0..=MIXED.len()).filter(|&i| MIXED.is_char_boundary(i)) {
                    let once = commit(MIXED, Cursor::new(a, h), policy);
                    assert_eq!(
                        commit(MIXED, once, policy),
                        once,
                        "policy={policy:?} anchor={a} head={h}"
                    );
                }
            }
        }
    }
}
