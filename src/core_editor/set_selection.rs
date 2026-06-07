use crate::{
    core_editor::{
        graphemes::{next_grapheme_boundary, prev_grapheme_boundary},
        word, Cursor,
    },
    enums::{Direction, MotionTarget, WordEdge, WordKind},
    FindStop,
};

/// A target an endpoint can move to, resolved against an origin (the head).
///
/// Inert data: a mode emits it, the editor resolves it to a byte offset via
/// [`locate`]. Only grapheme boundaries for now — word, line, and regex
/// boundaries come later (they need line-aware lowering).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Boundary {
    /// An absolute byte position (clamped into the buffer).
    ///
    // No caller until absolute-position moves are lowered through SetSelection.
    #[allow(dead_code)]
    Offset(usize),
    /// One grapheme right of the origin.
    GraphemeRight,
    /// One grapheme left of the origin.
    GraphemeLeft,
    /// A word boundary — vi `w`/`e`/`b` and their `W`/`E`/`B` variants, resolved
    /// via the shared classifier (see [`word::locate_word`]).
    #[allow(dead_code)] // producer lands when the word motions are re-lowered
    Word {
        kind: WordKind,
        edge: WordEdge,
        forward: bool,
    },
}

/// Identifies one endpoint of a [`Cursor`].
///
// No caller until `Pin` is used (flip/`o` swaps anchor↔head).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Anchor,
    Head,
}

/// How one endpoint moves when a [`SetSelection`] is applied.
///
/// `T` is a [`Boundary`] before resolution, a `usize` after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End<T> {
    /// Leave the endpoint at its pre-transform position.
    Keep,
    /// Move the endpoint to `T`.
    To(T),
    /// Set the endpoint to the pre-transform value of the given `Side`.
    ///
    // No caller until flip/`o` (atomic anchor↔head swap) is lowered.
    #[allow(dead_code)]
    Pin(Side),
}

/// How both endpoints change: the one primitive every motion/selection lowers
/// to. `B` is [`Boundary`] as emitted, `usize` once resolved.
///
/// A few examples of what lowers to this:
/// - move right (collapse): `{ anchor: To(GraphemeRight), head: To(GraphemeRight) }`
/// - extend right:          `{ anchor: Keep, head: To(GraphemeRight) }`
/// - flip (`o`):            `{ anchor: Pin(Head), head: Pin(Anchor) }`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SetSelection<B> {
    pub(crate) anchor: End<B>,
    pub(crate) head: End<B>,
}

impl<T> End<T> {
    /// Resolve a `To` target through `locate`; `Keep`/`Pin` pass through.
    fn resolve(self, locate: impl Fn(T) -> usize) -> End<usize> {
        match self {
            End::Keep => End::Keep,
            End::Pin(s) => End::Pin(s),
            End::To(t) => End::To(locate(t)),
        }
    }
}

impl SetSelection<Boundary> {
    /// Resolve both endpoints' boundaries to byte positions.
    pub(crate) fn resolve(self, locate: impl Fn(Boundary) -> usize) -> SetSelection<usize> {
        SetSelection {
            anchor: self.anchor.resolve(&locate),
            head: self.head.resolve(&locate),
        }
    }
}

impl<B: Copy> SetSelection<B> {
    /// A motion to `target`: collapse onto it, or keep the anchor when extending.
    /// Generic over the endpoint type so it serves both an unresolved
    /// [`Boundary`] and an already-resolved `usize` head.
    pub(crate) fn motion(target: B, extend: bool) -> Self {
        SetSelection {
            anchor: if extend { End::Keep } else { End::To(target) },
            head: End::To(target),
        }
    }
}

impl Cursor {
    /// Apply a resolved [`SetSelection`].
    ///
    /// Both endpoints read the pre-transform snapshot `(a0, h0)`, so swaps and
    /// crossings commit atomically — `Pin(Head)`/`Pin(Anchor)` see the *old*
    /// values, not values already updated this transform.
    pub(crate) fn transform(self, op: SetSelection<usize>) -> Self {
        let (a0, h0) = (self.anchor(), self.head());
        let resolve_end = |end: End<usize>, own: usize| match end {
            End::Keep => own,
            End::To(p) => p,
            End::Pin(Side::Anchor) => a0,
            End::Pin(Side::Head) => h0,
        };
        Cursor::new(resolve_end(op.anchor, a0), resolve_end(op.head, h0))
    }
}

/// Resolve a [`Boundary`] to a byte offset in `buf`, relative to `origin`.
///
/// Pure `&str`-based, like `commit` — no `Editor` needed, so it is unit-testable
/// in isolation. `origin` is normally the cursor head.
pub(crate) fn locate(buf: &str, origin: usize, boundary: Boundary) -> usize {
    match boundary {
        Boundary::Offset(n) => n.min(buf.len()),
        Boundary::GraphemeRight => next_grapheme_boundary(buf, origin),
        Boundary::GraphemeLeft => prev_grapheme_boundary(buf, origin),
        Boundary::Word {
            kind,
            edge,
            forward,
        } => word::locate_word(buf, origin, kind, edge, forward),
    }
}

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
        // Character search is not yet lowered through `MotionTarget`; vi `f`/`t`
        // use the dedicated `MoveRightUntil`/… path. No-op rather than panic.
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

    fn set(anchor: End<usize>, head: End<usize>) -> SetSelection<usize> {
        SetSelection { anchor, head }
    }

    #[test]
    fn transform_keep_leaves_endpoint() {
        let c = Cursor::new(2, 5);
        assert_eq!(c.transform(set(End::Keep, End::To(8))), Cursor::new(2, 8));
    }

    #[test]
    fn transform_collapse_moves_both() {
        let c = Cursor::new(2, 5);
        assert_eq!(c.transform(set(End::To(8), End::To(8))), Cursor::new(8, 8));
    }

    #[test]
    fn transform_pin_is_atomic_swap() {
        // flip: anchor takes the *old* head, head takes the *old* anchor
        let c = Cursor::new(2, 5);
        let flipped = c.transform(set(End::Pin(Side::Head), End::Pin(Side::Anchor)));
        assert_eq!(flipped, Cursor::new(5, 2));
    }

    #[test]
    fn resolve_maps_only_to_targets() {
        let op = SetSelection {
            anchor: End::Keep,
            head: End::To(Boundary::GraphemeRight),
        };
        // locate everything to 99 so we can see which arms were resolved
        let resolved = op.resolve(|_| 99);
        assert_eq!(resolved.anchor, End::Keep);
        assert_eq!(resolved.head, End::To(99));
    }

    #[test]
    fn locate_grapheme_and_offset() {
        let buf = "café"; // 'é' is 2 bytes; graphemes start 0,1,2,3, len 5
        assert_eq!(locate(buf, 2, Boundary::GraphemeRight), 3);
        assert_eq!(locate(buf, 3, Boundary::GraphemeRight), 5); // over the 2-byte é
        assert_eq!(locate(buf, 3, Boundary::GraphemeLeft), 2);
        assert_eq!(locate(buf, 0, Boundary::Offset(99)), 5); // clamped to len
    }

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
}
