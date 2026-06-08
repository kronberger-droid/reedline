use crate::{
    core_editor::{
        graphemes::{next_grapheme_boundary, prev_grapheme_boundary},
        word,
    },
    enums::{Direction, MotionTarget, WordEdge},
    FindStop,
};

/// A resolved motion: where the cursor head lands, and whether an operator
/// acting over it consumes the grapheme at `head`.
///
/// `inclusive` is vim's inclusive-vs-exclusive *motion* classification — a
/// property of the motion itself (e.g. a word *end* is inclusive), which
/// operators honor. The cursor lands on `head` either way; only an operator's
/// range extends one grapheme further when `inclusive`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Movement {
    pub(crate) head: usize,
    pub(crate) inclusive: bool,
}

/// Resolve a public [`MotionTarget`] against `buf`, relative to `origin`.
///
/// Total over every variant — a target with no resolution yet (`Find`) stays at
/// `origin` (a no-op) rather than panicking, so a target constructed from config
/// or another mode can never crash the editor. Context-aware (takes `buf`), so
/// line/buffer edges resolve correctly where a context-free conversion couldn't.
pub(crate) fn resolve_motion(buf: &str, origin: usize, target: MotionTarget) -> Movement {
    let exclusive = |head| Movement {
        head,
        inclusive: false,
    };
    match target {
        MotionTarget::Grapheme(Direction::Forward) => {
            exclusive(next_grapheme_boundary(buf, origin))
        }
        MotionTarget::Grapheme(Direction::Backward) => {
            exclusive(prev_grapheme_boundary(buf, origin))
        }
        MotionTarget::Word {
            kind,
            edge,
            direction,
        } => Movement {
            head: word::locate_word(buf, origin, kind, edge, direction == Direction::Forward),
            // A word *end* is the word's last grapheme; operating to it consumes
            // that grapheme (vim classifies `e`/`E` as inclusive).
            inclusive: edge == WordEdge::End && direction == Direction::Forward,
        },
        MotionTarget::Offset(n) => exclusive(n.min(buf.len())),
        MotionTarget::BufferEdge(Direction::Backward) => exclusive(0),
        MotionTarget::BufferEdge(Direction::Forward) => exclusive(buf.len()),
        MotionTarget::LineEdge(Direction::Backward) => {
            exclusive(buf[..origin].rfind('\n').map_or(0, |i| i + 1))
        }
        MotionTarget::LineEdge(Direction::Forward) => {
            exclusive(buf[origin..].find('\n').map_or(buf.len(), |i| origin + i))
        }
        // Character search (vi `f`/`t`/`F`/`T`). A miss stays at `origin` (a
        // no-op) rather than panicking. Forward find is inclusive (`df` eats the
        // target char); backward is exclusive.
        MotionTarget::Find {
            ch,
            direction,
            stop,
        } => {
            find_char(buf, origin, ch, direction, stop).map_or(exclusive(origin), |head| Movement {
                head,
                inclusive: direction == Direction::Forward,
            })
        }
    }
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
        let m = resolve_motion("foo bar", 0, word(WordEdge::End, Direction::Forward));
        assert_eq!(
            m,
            Movement {
                head: 2,
                inclusive: true
            }
        ); // on the last 'o'
        let m = resolve_motion("foo bar", 0, word(WordEdge::Start, Direction::Forward));
        assert!(!m.inclusive);
        let m = resolve_motion("foo bar", 7, word(WordEdge::End, Direction::Backward));
        assert!(!m.inclusive);
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
            Movement {
                head: 4,
                inclusive: true
            }
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
            Movement {
                head: 3,
                inclusive: true
            }
        );
    }

    #[test]
    fn resolve_motion_find_backward_on_lands_on_char() {
        // `F f` from `r` (origin 6) — land *on* the previous `f` (byte 0).
        // Backward find is an exclusive motion (vim `F`/`T`).
        assert_eq!(
            resolve_motion("foo bar", 6, find('f', Direction::Backward, FindStop::On)),
            Movement {
                head: 0,
                inclusive: false
            }
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
            Movement {
                head: 1,
                inclusive: false
            }
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
    fn resolve_motion_find_absent_char_stays_put() {
        // Totality: an unfindable char is a no-op, never a panic.
        assert_eq!(
            resolve_motion("foo bar", 3, find('z', Direction::Forward, FindStop::On)),
            Movement {
                head: 3,
                inclusive: false
            }
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
            Movement {
                head: 2,
                inclusive: false
            }
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
}
