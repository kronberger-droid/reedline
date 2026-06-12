use crate::{
    core_editor::{
        graphemes::{next_grapheme_boundary, prev_grapheme_boundary},
        line,
        word::{self, categorize_char, CharClass},
        Cursor,
    },
    enums::{Direction, MotionTarget, WordEdge, WordKind},
    FindStop,
};

/// A resolved motion, as two byte positions:
/// - `head` — where the cursor lands (used by `Move`/`Extend`).
/// - `op_end` — the far edge an operator consumes (used by `Cut`/`Copy`/`Erase`).
///
/// They differ only for *inclusive* motions: a forward word-end (`e`) or find
/// (`f`/`t`) lands the cursor *on* a grapheme, but an operator eats it — so
/// `op_end` is one grapheme past `head`. For exclusive motions `op_end == head`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Movement {
    pub(crate) head: usize,
    pub(crate) op_end: usize,
}

/// The span an operator (`Cut`/`Copy`/`Erase`) acts over: a [`Cursor`] from
/// `origin` to the motion's `op_end`. `start()..end()` is the byte range to
/// consume — inclusivity and direction are already baked into `op_end`, so the
/// operator never has to reconsider them.
pub(crate) fn operator_span(buf: &str, origin: usize, target: MotionTarget) -> Cursor {
    Cursor::new(origin, resolve_motion(buf, origin, target).op_end)
}

/// Resolve a public [`MotionTarget`] against `buf`, relative to `origin`.
///
/// Total over every variant — a target that cannot land anywhere (a `Find` that
/// misses, a `Line` past the first/last line) stays at `origin` (a no-op) rather
/// than panicking, so a target constructed from config or another mode can never
/// crash the editor. Context-aware (takes `buf`), so line/buffer edges resolve
/// correctly where a context-free conversion couldn't.
pub(crate) fn resolve_motion(buf: &str, origin: usize, target: MotionTarget) -> Movement {
    let span = |head: usize, inclusive: bool| Movement {
        head,
        op_end: if inclusive {
            next_grapheme_boundary(buf, head)
        } else {
            head
        },
    };
    match target {
        MotionTarget::Grapheme(Direction::Forward) => {
            span(next_grapheme_boundary(buf, origin), false)
        }
        MotionTarget::Grapheme(Direction::Backward) => {
            span(prev_grapheme_boundary(buf, origin), false)
        }
        MotionTarget::Word {
            kind,
            edge,
            direction,
        } => {
            let head = word::locate_word(buf, origin, kind, edge, direction == Direction::Forward);
            let inclusive = edge == WordEdge::End && direction == Direction::Forward;
            span(head, inclusive)
        }
        MotionTarget::Offset(n) => span(n.min(buf.len()), false),
        MotionTarget::BufferEdge(Direction::Backward) => span(0, false),
        MotionTarget::BufferEdge(Direction::Forward) => span(buf.len(), false),
        MotionTarget::LineEdge(Direction::Backward) => {
            span(line::start_of_line(buf, origin), false)
        }
        // CRLF-aware via `end_of_line`: `$` stops before the `\r` of a `\r\n`
        // terminator, matching `LineBuffer::find_current_line_end`.
        MotionTarget::LineEdge(Direction::Forward) => span(line::end_of_line(buf, origin), false),
        // The adjacent line (`j`/`k`). Lands on the *start* of the line below /
        // above; on the first/last line it stays put (so `dj`/`dk` there only
        // affect the current line). Operators snap the span to whole lines.
        MotionTarget::Line(Direction::Forward) => {
            let head = line::start_of_next_line(buf, origin).unwrap_or(origin);
            span(head, false)
        }
        MotionTarget::Line(Direction::Backward) => {
            let line_start = line::start_of_line(buf, origin);
            let head = if line_start == 0 {
                origin
            } else {
                line::start_of_line(buf, line_start - 1)
            };
            span(head, false)
        }
        // Character search (vi `f`/`t`/`F`/`T`). A miss stays at `origin` (a
        // no-op) rather than panicking. Forward find is inclusive (`df` eats the
        // target char); backward is exclusive.
        MotionTarget::Find {
            ch,
            direction,
            stop,
        } => {
            let hit = find_char(buf, origin, ch, direction, stop);
            let inclusive = hit.is_some() && direction == Direction::Forward;
            span(hit.unwrap_or(origin), inclusive)
        }
    }
}

/// Resolve a [`MotionTarget`] as a *selection-first* (Helix-style) motion: the
/// returned [`Cursor`] is the **gap-indexed** range the motion travels over,
/// exactly Helix's convention — `start()..end()` is the covered byte span and
/// the caret grapheme is [`Cursor::caret`]. For a forward motion the head is
/// the motion's `op_end` (one past the last covered grapheme); for a backward
/// motion the head is the resolved position and the anchor sits one past the
/// origin grapheme so the origin stays covered.
///
/// Word targets follow Helix's anchor rule (`movement::word_move` /
/// `range_to_target`): the anchor normally stays at the origin, but when the
/// motion's boundary lies immediately at the cursor — the head "effectively
/// starts on a boundary" — the anchor hops past the origin grapheme, and a
/// forward word-*start* additionally selects through the *next* boundary
/// (this is what makes repeated `w` walk word by word instead of sticking on
/// the space between two words).
///
/// Total like [`resolve_motion`]: a motion that cannot move collapses to a
/// point at `origin` rather than panicking. Consumers under the vi-style
/// inclusive convention (`RestPolicy::OnGrapheme`) pull the range's high end
/// back one grapheme — see `Editor::select_to_target`.
pub(crate) fn resolve_selection(buf: &str, origin: usize, target: MotionTarget) -> Cursor {
    let m = resolve_motion(buf, origin, target);
    if m.head == origin {
        return Cursor::point(origin);
    }

    if m.head < origin {
        // Backward: cover the origin grapheme (anchor one past it), unless
        // the boundary the motion seeks sits immediately before it — `b`
        // pressed on a word's first grapheme selects only what lies behind
        // the word (Helix re-anchors past the cursor grapheme).
        let anchor = if word_boundary_at_origin(buf, origin, target, Direction::Backward) {
            origin
        } else {
            next_grapheme_boundary(buf, origin)
        };
        return Cursor::new(anchor, m.head);
    }

    // Forward.
    let next = next_grapheme_boundary(buf, origin);
    let on_boundary = word_boundary_at_origin(buf, origin, target, Direction::Forward);
    let (anchor, movement) = match target {
        // A forward word-start whose target is the very next grapheme: the
        // anchor hops onto it and the span runs to the *following* word start.
        MotionTarget::Word {
            edge: WordEdge::Start,
            ..
        } if on_boundary => (next, resolve_motion(buf, next, target)),
        // A forward word-end pressed on a word's last grapheme already resolves
        // to the next word's end; only the anchor excludes the origin grapheme.
        MotionTarget::Word {
            edge: WordEdge::End,
            ..
        } if on_boundary => (next, m),
        _ => (origin, m),
    };
    Cursor::new(anchor, movement.op_end.max(anchor))
}

/// Helix's `reached_target` evaluated at the cursor itself: does the boundary
/// `target` travels to lie directly between the origin grapheme and its
/// `direction`-side neighbor? Only meaningful for word targets — every other
/// target returns `false` (their spans need no anchor adjustment).
fn word_boundary_at_origin(
    buf: &str,
    origin: usize,
    target: MotionTarget,
    direction: Direction,
) -> bool {
    let MotionTarget::Word { kind, edge, .. } = target else {
        return false;
    };
    // `a` is the grapheme under the cursor, `b` its neighbor in travel
    // direction; both reduced to their first scalar, like the classifier-based
    // scans in `word::locate_word`.
    let Some(a) = buf[origin..].chars().next() else {
        return false;
    };
    let b = match direction {
        Direction::Forward => buf[next_grapheme_boundary(buf, origin)..].chars().next(),
        Direction::Backward => buf[prev_grapheme_boundary(buf, origin)..].chars().next(),
    };
    let Some(b) = b else {
        return false;
    };
    let is_boundary = match kind {
        WordKind::Small => word::is_word_boundary(a, b),
        WordKind::Big => word::is_long_word_boundary(a, b),
    };
    // Mirrors Helix's `reached_target`: a *start* must land on a word grapheme;
    // an *end* must leave one. (Backward travel only uses the `Start` form —
    // `b`/`B`; the cursor grapheme plays the "leaving" role there.)
    let class_ok = match (edge, direction) {
        (WordEdge::Start, Direction::Forward) => {
            matches!(categorize_char(b), CharClass::Word | CharClass::Punctuation)
        }
        (WordEdge::End, Direction::Forward) | (WordEdge::Start, Direction::Backward) => {
            matches!(categorize_char(a), CharClass::Word | CharClass::Punctuation)
        }
        (WordEdge::End, Direction::Backward) => {
            matches!(categorize_char(b), CharClass::Word | CharClass::Punctuation)
        }
    };
    is_boundary && class_ok
}

// we either find it or not.
fn find_char(
    buf: &str,
    origin: usize,
    ch: char,
    direction: Direction,
    stop: FindStop,
) -> Option<usize> {
    let hit = match direction {
        Direction::Forward => {
            let start = next_grapheme_boundary(buf, origin);
            buf[start..].find(ch).map(|rel| start + rel)
        }
        Direction::Backward => buf[..origin].rfind(ch),
    }?;

    Some(match (direction, stop) {
        (_, FindStop::On) => hit,
        (Direction::Forward, FindStop::Before) => prev_grapheme_boundary(buf, hit),
        (Direction::Backward, FindStop::Before) => next_grapheme_boundary(buf, hit),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WordKind;

    fn word(edge: WordEdge, direction: Direction) -> MotionTarget {
        MotionTarget::Word {
            kind: WordKind::Small,
            edge,
            direction,
        }
    }

    #[test]
    fn resolve_motion_marks_forward_word_end_inclusive() {
        // Only a forward word *end* is inclusive; starts and backward motions are not.
        // forward word-end is inclusive: lands on the last 'o' (2), op_end one past (3)
        let m = resolve_motion("foo bar", 0, word(WordEdge::End, Direction::Forward));
        assert_eq!(m, Movement { head: 2, op_end: 3 });
        // starts and backward motions are exclusive: op_end == head
        let m = resolve_motion("foo bar", 0, word(WordEdge::Start, Direction::Forward));
        assert_eq!(m.op_end, m.head);
        let m = resolve_motion("foo bar", 7, word(WordEdge::End, Direction::Backward));
        assert_eq!(m.op_end, m.head);
    }

    #[test]
    fn resolve_motion_handles_line_and_buffer_edges() {
        let buf = "ab\ncd\nef";
        // line edges resolve against the *current* line (context-aware)
        assert_eq!(
            resolve_motion(buf, 4, MotionTarget::LineEdge(Direction::Backward)).head,
            3
        );
        assert_eq!(
            resolve_motion(buf, 4, MotionTarget::LineEdge(Direction::Forward)).head,
            5
        );
        assert_eq!(
            resolve_motion(buf, 4, MotionTarget::BufferEdge(Direction::Backward)).head,
            0
        );
        assert_eq!(
            resolve_motion(buf, 4, MotionTarget::BufferEdge(Direction::Forward)).head,
            8
        );
    }

    use crate::enums::FindStop;

    /// Build a `Find` target — the `f`/`t`/`F`/`T` family.
    fn find(ch: char, direction: Direction, stop: FindStop) -> MotionTarget {
        MotionTarget::Find {
            ch,
            direction,
            stop,
        }
    }

    #[test]
    fn resolve_motion_find_forward_on_lands_on_char() {
        // `foo bar`:  f0 o1 o2 _3 b4 a5 r6
        // `f b` — land *on* the next `b` after origin.
        // Forward find is an inclusive motion (vim `f`/`t`).
        assert_eq!(
            resolve_motion("foo bar", 0, find('b', Direction::Forward, FindStop::On)),
            Movement { head: 4, op_end: 5 } // inclusive: op_end one past 'b'
        );
    }

    #[test]
    fn resolve_motion_find_forward_before_stops_short() {
        // `t b` — stop one grapheme *short* of the next `b` (byte 3).
        assert_eq!(
            resolve_motion(
                "foo bar",
                0,
                find('b', Direction::Forward, FindStop::Before)
            ),
            Movement { head: 3, op_end: 4 } // inclusive: op_end one past byte 3
        );
    }

    #[test]
    fn resolve_motion_find_backward_on_lands_on_char() {
        // `F f` from `r` (origin 6) — land *on* the previous `f` (byte 0).
        // Backward find is an exclusive motion (vim `F`/`T`).
        assert_eq!(
            resolve_motion("foo bar", 6, find('f', Direction::Backward, FindStop::On)),
            Movement { head: 0, op_end: 0 } // backward is exclusive
        );
    }

    #[test]
    fn resolve_motion_find_backward_before_stops_short() {
        // `T f` from origin 6 — stop one grapheme short, i.e. just *after*
        // the `f` (byte 1).
        assert_eq!(
            resolve_motion(
                "foo bar",
                6,
                find('f', Direction::Backward, FindStop::Before)
            ),
            Movement { head: 1, op_end: 1 } // backward is exclusive
        );
    }

    #[test]
    fn resolve_motion_find_searches_strictly_past_origin() {
        // The char *at* origin doesn't count — search starts past it, like
        // `locate_word`. Origin 4 is `b`; forward-find `b` skips it and,
        // finding no other, stays put.
        assert_eq!(
            resolve_motion("foo bar", 4, find('b', Direction::Forward, FindStop::On)).head,
            4
        );
    }

    #[test]
    fn resolve_motion_find_before_replay_from_landing_spot_is_stuck() {
        // `t` lands one grapheme short of the target; replaying the same Find
        // (`;`) from that landing spot searches from the next grapheme — the
        // target char itself — re-finds the *same* occurrence, and lands back
        // where it began. Vim (default cpoptions) skips to the next occurrence
        // instead; reedline keeps the historical stuck behavior, pinned here so
        // any future change to it is deliberate.
        let t = find('x', Direction::Forward, FindStop::Before);
        // "axbxc": x@1, x@3. From 0 (adjacent to x@1): stays at 0.
        assert_eq!(resolve_motion("axbxc", 0, t).head, 0);
        // From 2 (adjacent to x@3): stays at 2.
        assert_eq!(resolve_motion("axbxc", 2, t).head, 2);
    }

    #[test]
    fn resolve_motion_find_absent_char_stays_put() {
        // Totality: an unfindable char is a no-op, never a panic.
        assert_eq!(
            resolve_motion("foo bar", 3, find('z', Direction::Forward, FindStop::On)),
            Movement { head: 3, op_end: 3 } // miss: no-op at origin
        );
    }

    #[test]
    fn resolve_motion_find_before_respects_grapheme_boundaries() {
        // `a→b`:  a0  →1..4 (3-byte arrow)  b4.  `t b` must land at the
        // *start* of `→` (byte 1), not byte 3 — proof the impl steps a
        // grapheme, not a single byte.
        assert_eq!(
            resolve_motion("a→b", 0, find('b', Direction::Forward, FindStop::Before)).head,
            1
        );
        // backward `T a` from `b` (origin 4): one grapheme *after* `a` is
        // also the start of `→` (byte 1).
        assert_eq!(
            resolve_motion("a→b", 4, find('a', Direction::Backward, FindStop::Before)).head,
            1
        );
    }

    #[test]
    fn resolve_motion_find_backward_finds_adjacent_char() {
        // `fab`:  f0 a1 b2.  `F a` from `b` (origin 2) must land on the `a`
        // *immediately* left of the cursor (byte 1) — the backward search
        // looks at the char right before origin, it does not skip a grapheme.
        assert_eq!(
            resolve_motion("fab", 2, find('a', Direction::Backward, FindStop::On)).head,
            1
        );
    }

    #[test]
    fn resolve_motion_find_backward_searches_strictly_before_origin() {
        // Mirror of the forward case: the char *at* origin is excluded. Origin
        // 0 is `b`; backward-find `b` has nothing before it and stays put.
        assert_eq!(
            resolve_motion("bab", 0, find('b', Direction::Backward, FindStop::On)).head,
            0
        );
    }

    // --- selection-first resolution (Helix motions) ---
    //
    // `resolve_selection` returns a gap-indexed Cursor, exactly Helix's Range
    // convention: `start()..end()` is the covered byte span, the caret rests
    // on the grapheme before a forward head. Expectations are checked against
    // Helix's `movement::word_move` semantics.

    #[test]
    fn resolve_selection_word_start_selects_through_the_gap() {
        // `w` from 'f' in "foo bar": Range(0, 4) covers "foo " — the caret on
        // the space *before* the next word, not on 'b'.
        let sel = resolve_selection("foo bar", 0, word(WordEdge::Start, Direction::Forward));
        assert_eq!((sel.anchor(), sel.head()), (0, 4));
        assert_eq!(sel.caret("foo bar"), 3);
    }

    #[test]
    fn resolve_selection_word_start_hops_when_cursor_touches_the_boundary() {
        // "foo bar baz", caret on the space at 3 — exactly where the previous
        // `w` parked it. Helix re-anchors past the boundary and selects the
        // *next* span "bar " (4..8); a naive span-from-origin would collapse
        // onto the space and stick there forever.
        let sel = resolve_selection("foo bar baz", 3, word(WordEdge::Start, Direction::Forward));
        assert_eq!((sel.anchor(), sel.head()), (4, 8));
    }

    #[test]
    fn resolve_selection_word_start_from_space_before_last_word() {
        // "a b", caret on the space: the hop lands on 'b' and the span is just
        // that final word.
        let sel = resolve_selection("a b", 1, word(WordEdge::Start, Direction::Forward));
        assert_eq!((sel.anchor(), sel.head()), (2, 3));
    }

    #[test]
    fn resolve_selection_word_end_includes_cursor_unless_on_word_end() {
        // `e` from 'f': Range(0, 3) covers "foo", caret on the last 'o'.
        let sel = resolve_selection("foo bar", 0, word(WordEdge::End, Direction::Forward));
        assert_eq!((sel.anchor(), sel.head()), (0, 3));
        // `e` from the last 'o' (a word end): Helix re-anchors past the cursor
        // grapheme — Range(3, 7) covers " bar", the 'o' is *not* included.
        let sel = resolve_selection("foo bar", 2, word(WordEdge::End, Direction::Forward));
        assert_eq!((sel.anchor(), sel.head()), (3, 7));
    }

    #[test]
    fn resolve_selection_word_back_excludes_cursor_on_word_start() {
        // `b` from 'b' (a word start): Range(4, 0) covers "foo " — the 'b'
        // itself is excluded, caret on 'f'.
        let sel = resolve_selection("foo bar", 4, word(WordEdge::Start, Direction::Backward));
        assert_eq!((sel.anchor(), sel.head()), (4, 0));
        assert_eq!(sel.caret("foo bar"), 0);
    }

    #[test]
    fn resolve_selection_word_back_includes_cursor_mid_word() {
        // `b` from 'a' (mid-word): Range(6, 4) covers "ba" — the cursor
        // grapheme stays covered (anchor one past it).
        let sel = resolve_selection("foo bar", 5, word(WordEdge::Start, Direction::Backward));
        assert_eq!((sel.anchor(), sel.head()), (6, 4));
    }

    #[test]
    fn resolve_selection_find_spans_origin_to_hit() {
        // `f b`: Range(0, 5) covers "foo b", caret on the found char.
        let sel = resolve_selection("foo bar", 0, find('b', Direction::Forward, FindStop::On));
        assert_eq!((sel.anchor(), sel.head()), (0, 5));
        assert_eq!(sel.caret("foo bar"), 4);
        // `t b` stops one short.
        let sel = resolve_selection(
            "foo bar",
            0,
            find('b', Direction::Forward, FindStop::Before),
        );
        assert_eq!((sel.anchor(), sel.head()), (0, 4));
        // backward `F f` from 'r': the cursor grapheme stays covered
        // (anchor 7), head on the hit.
        let sel = resolve_selection("foo bar", 6, find('f', Direction::Backward, FindStop::On));
        assert_eq!((sel.anchor(), sel.head()), (7, 0));
    }

    #[test]
    fn resolve_selection_at_buffer_end_covers_the_tail() {
        // `w` on the last grapheme runs to the buffer end and still covers it
        // (Helix's early-out keeps the block); a missed find is a no-op point.
        let sel = resolve_selection("foo", 2, word(WordEdge::Start, Direction::Forward));
        assert_eq!((sel.anchor(), sel.head()), (2, 3));
        let sel = resolve_selection("foo bar", 3, find('z', Direction::Forward, FindStop::On));
        assert!(sel.is_empty());
    }

    #[test]
    fn resolve_selection_line_edge_covers_the_line_content() {
        // Forward to the line edge (Helix `x`'s second half): the span runs to
        // the line end, caret resting on the last grapheme, `\n` untouched.
        let sel = resolve_selection("ab\ncd", 0, MotionTarget::LineEdge(Direction::Forward));
        assert_eq!((sel.anchor(), sel.head()), (0, 2));
        assert_eq!(sel.caret("ab\ncd"), 1);
    }

    // --- line / buffer edges (`0`/`$`/`gg`/`G`) ---
    //
    // The whole reason `LineEdge` and `BufferEdge` are distinct targets is
    // multiline: `$` must stop at the next `\n`, not run to the buffer end.
    // `"ab\ncd"` has bytes a0 b1 \n2 c3 d4, len 5.

    #[test]
    fn resolve_motion_line_edge_forward_stops_at_newline() {
        // `$` from inside the first line lands *at* the `\n`, not the buffer end.
        assert_eq!(
            resolve_motion("ab\ncd", 0, MotionTarget::LineEdge(Direction::Forward)),
            Movement { head: 2, op_end: 2 } // line edge is exclusive
        );
    }

    #[test]
    fn resolve_motion_line_edge_forward_stops_before_crlf() {
        // On a CRLF-terminated line `$` lands before the `\r`, matching
        // `LineBuffer::find_current_line_end` — both delegate to `end_of_line`.
        assert_eq!(
            resolve_motion("ab\r\ncd", 0, MotionTarget::LineEdge(Direction::Forward)).head,
            2
        );
    }

    #[test]
    fn resolve_motion_line_edge_backward_stops_at_line_start() {
        // `0` from the second line lands at that line's start (byte 3), not 0.
        assert_eq!(
            resolve_motion("ab\ncd", 4, MotionTarget::LineEdge(Direction::Backward)).head,
            3
        );
    }

    #[test]
    fn resolve_motion_buffer_edge_spans_whole_buffer() {
        // `G` / `gg` ignore line breaks — start is 0, end is the buffer length.
        assert_eq!(
            resolve_motion("ab\ncd", 0, MotionTarget::BufferEdge(Direction::Forward)).head,
            5
        );
        assert_eq!(
            resolve_motion("ab\ncd", 4, MotionTarget::BufferEdge(Direction::Backward)).head,
            0
        );
    }

    #[test]
    fn resolve_motion_line_targets_the_adjacent_line() {
        let buf = "ab\ncd\nef"; // ab@0-1 \n@2 cd@3-4 \n@5 ef@6-7
                                // from "cd" (origin 4): down → start of "ef", up → start of "ab"
        assert_eq!(
            resolve_motion(buf, 4, MotionTarget::Line(Direction::Forward)).head,
            6
        );
        assert_eq!(
            resolve_motion(buf, 4, MotionTarget::Line(Direction::Backward)).head,
            0
        );
        // no adjacent line → stay put (last line down, first line up)
        assert_eq!(
            resolve_motion(buf, 7, MotionTarget::Line(Direction::Forward)).head,
            7
        );
        assert_eq!(
            resolve_motion(buf, 1, MotionTarget::Line(Direction::Backward)).head,
            1
        );
    }
}
