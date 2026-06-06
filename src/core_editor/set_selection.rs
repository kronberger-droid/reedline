use crate::{
    core_editor::{
        graphemes::{next_grapheme_boundary, prev_grapheme_boundary},
        word, Cursor,
    },
    enums::{MotionTarget, WordEdge, WordKind},
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

    pub(crate) fn motion(boundary: Boundary, extend: bool) -> Self {
        SetSelection {
            anchor: if extend { End::Keep } else { End::To(boundary) },
            head: End::To(boundary),
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

impl From<MotionTarget> for Boundary {
    fn from(value: MotionTarget) -> Self {
        use crate::enums::Direction::{Backward, Forward};
        match value {
            MotionTarget::Grapheme(Forward) => Boundary::GraphemeRight,
            MotionTarget::Grapheme(Backward) => Boundary::GraphemeLeft,
            MotionTarget::Word {
                kind,
                edge,
                direction,
            } => Boundary::Word {
                kind,
                edge,
                forward: direction == Forward,
            },
            MotionTarget::Offset(n) => Boundary::Offset(n),
            MotionTarget::LineEdge(_) | MotionTarget::BufferEdge(_) | MotionTarget::Find { .. } => {
                todo!()
            }
        }
    }
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
}
