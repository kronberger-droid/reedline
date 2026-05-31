#![allow(dead_code)]

/// A selection range.
///
/// Uses gap indexing — `anchor` and `head` represent positions *between* bytes,
/// not bytes themselves. Ranges are inclusive on the left and exclusive on the
/// right, regardless of anchor/head ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Selection {
    /// The anchor of the range: the side that doesn't move when extending.
    anchor: usize,
    /// The head of the range, moved when extending.
    head: usize,
}

impl Selection {
    /// The fixed enpoint, which doesn't move when extending
    pub(super) fn anchor(&self) -> usize {
        self.anchor
    }

    /// The moving endpouint, the cursor
    pub(super) fn head(&self) -> usize {
        self.head
    }
}

/// The direction a range extends in.
///
/// `Forward` when `head >= anchor`, `Backward` when `head < anchor`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Forward,
    Backward,
}

impl Selection {
    pub(super) fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// A zero-width range at `head`.
    pub(super) fn point(head: usize) -> Self {
        Self::new(head, head)
    }

    /// Start of the range
    pub(super) fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// End of the range
    pub(super) fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// Total length of the range.
    pub(super) fn len(&self) -> usize {
        self.end() - self.start()
    }

    /// `true` when anchor and head are at the same position.
    pub(super) fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// `Forward` when `head >= anchor`, `Backward` otherwise.
    pub(super) fn direction(&self) -> Direction {
        if self.head < self.anchor {
            Direction::Backward
        } else {
            Direction::Forward
        }
    }

    /// Swap anchor and head.
    pub(super) fn flip(self) -> Self {
        Self {
            anchor: self.head,
            head: self.anchor,
        }
    }

    /// Return the range if it already points in `direction`, otherwise flip it.
    pub(super) fn with_direction(self, direction: Direction) -> Self {
        if self.direction() == direction {
            self
        } else {
            self.flip()
        }
    }

    /// Grow the range to cover at least `[from, to]`, preserving anchor/head
    /// ordering.
    ///
    /// If the range is currently `Forward`, the anchor can only move left and
    /// the head can only move right. If `Backward`, the roles are inverted.
    pub(super) fn extend(self, from: usize, to: usize) -> Self {
        debug_assert!(from <= to);
        if self.anchor <= self.head {
            Self {
                anchor: self.anchor.min(from),
                head: self.head.max(to),
            }
        } else {
            Self {
                anchor: self.anchor.max(to),
                head: self.head.min(from),
            }
        }
    }

    /// `true` if `pos` lies inside the range (left-inclusive, right-exclusive).
    pub(super) fn contains(&self, pos: usize) -> bool {
        self.start() <= pos && pos < self.end()
    }
}

/// A position spec the editor resolves against the buffer.
///
/// Inert data: a mode emits it, the editor resolves it to a byte offset
/// relative to an origin (the head). Line and regex boundaries come later.
pub(super) enum Boundary {
    /// An absolute byte position.
    Offset(usize),
    /// One grapheme right of the origin.
    GraphemeRight,
    /// One grapheme left of the origin.
    GraphemeLeft,
    /// The end of the next word right of the origin.
    WordRight,
}

/// Identifies one endpoint of a [`Selection`].
pub(super) enum Side {
    Anchor,
    Head,
}

/// How one endpoint moves when a [`SetSelection`] is applied.
///
/// `T` is a [`Boundary`] before resolution, a `usize` after.
pub(super) enum End<T> {
    /// Leave the endpoint at its pre-transform position.
    Keep,
    /// Move the endpoint to `T`.
    To(T),
    /// Set the endpoint to the pre-transform value of `Side`.
    Pin(Side),
}

/// An [`End`] targeting an unresolved [`Boundary`] — what a mode emits.
type EndSpec = End<Boundary>;
/// An [`End`] targeting a byte position — what [`Selection::transform`] takes.
type EndPos = End<usize>;

/// How both endpoints change: the one primitive every edit lowers to.
pub(super) struct SetSelection<B> {
    pub(super) anchor: End<B>,
    pub(super) head: End<B>,
}

impl<T> End<T> {
    /// Resolve a `To` target through `locate`; `Keep`/`Pin` pass through.
    fn resolve(self, locate: impl Fn(T) -> usize) -> EndPos {
        match self {
            End::Keep => End::Keep,
            End::Pin(s) => End::Pin(s),
            End::To(t) => End::To(locate(t)),
        }
    }
}

impl SetSelection<Boundary> {
    /// Resolve both endpoints' boundaries to byte positions.
    pub(super) fn resolve(
        self,
        locate: impl Fn(Boundary) -> usize,
    ) -> SetSelection<usize> {
        SetSelection {
            anchor: self.anchor.resolve(&locate),
            head: self.head.resolve(&locate),
        }
    }
}

impl Selection {
    /// Apply a resolved [`SetSelection`].
    ///
    /// Both endpoints read from the pre-transform snapshot `(a0, h0)`,
    /// so swaps and crossings commit atomically.
    pub(super) fn transform(self, op: SetSelection<usize>) -> Self {
        let (a0, h0) = (self.anchor, self.head);
        let eval = |end: EndPos, own: usize| match end {
            End::Keep => own,
            End::To(p) => p,
            End::Pin(Side::Anchor) => a0,
            End::Pin(Side::Head) => h0,
        };
        Selection::new(eval(op.anchor, a0), eval(op.head, h0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flip_using_transform() {
        let range = Selection::new(10, 12);

        let op = SetSelection {
            anchor: End::Pin(Side::Head),
            head: End::Pin(Side::Anchor),
        };

        assert_eq!(range.transform(op), range.flip());
    }

    #[test]
    fn extend_to_usung_transform() {
        let range = Selection::new(10, 12);

        let op = SetSelection {
            anchor: End::To(10),
            head: End::To(15),
        };

        assert_eq!(range.transform(op), range.extend(10, 15));
    }
    #[test]
    fn contains() {
        let range = Selection::new(10, 12);

        assert!(!range.contains(9));
        assert!(range.contains(10));
        assert!(range.contains(11));
        assert!(!range.contains(12));
        assert!(!range.contains(13));

        let range = Selection::new(9, 6);
        assert!(!range.contains(9));
        assert!(range.contains(7));
        assert!(range.contains(6));
    }

    #[test]
    fn point_constructs_empty_range_at_head() {
        let range = Selection::point(5);
        assert_eq!(range.start(), 5);
        assert_eq!(range.end(), 5);
        assert!(range.is_empty());
    }

    #[test]
    fn new_preserves_anchor_and_head_order() {
        let forward = Selection::new(2, 5);
        assert_eq!(forward.direction(), Direction::Forward);

        let backward = Selection::new(5, 2);
        assert_eq!(backward.direction(), Direction::Backward);
    }

    #[test]
    fn start_returns_lower_of_anchor_and_head() {
        assert_eq!(Selection::new(2, 5).start(), 2);
        assert_eq!(Selection::new(5, 2).start(), 2);
    }

    #[test]
    fn end_returns_higher_of_anchor_and_head() {
        assert_eq!(Selection::new(2, 5).end(), 5);
        assert_eq!(Selection::new(5, 2).end(), 5);
    }

    #[test]
    fn start_and_end_agree_for_empty_range() {
        let range = Selection::point(7);
        assert_eq!(range.start(), range.end());
    }

    #[test]
    fn len_is_zero_for_empty_range() {
        assert_eq!(Selection::point(7).len(), 0);
    }

    #[test]
    fn len_ignores_direction() {
        assert_eq!(Selection::new(2, 5).len(), 3);
        assert_eq!(Selection::new(5, 2).len(), 3);
    }

    #[test]
    fn is_empty_true_when_anchor_equals_head() {
        assert!(Selection::new(5, 5).is_empty());
        assert!(Selection::point(0).is_empty());
    }

    #[test]
    fn is_empty_false_for_nonzero_width() {
        assert!(!Selection::new(2, 5).is_empty());
        assert!(!Selection::new(5, 2).is_empty());
    }

    #[test]
    fn direction_forward_when_head_greater_than_anchor() {
        assert_eq!(
            Selection::new(2, 5).direction(),
            Direction::Forward
        );
    }

    #[test]
    fn direction_backward_when_head_less_than_anchor() {
        assert_eq!(
            Selection::new(5, 2).direction(),
            Direction::Backward
        );
    }

    #[test]
    fn direction_forward_for_empty_range() {
        assert_eq!(
            Selection::point(5).direction(),
            Direction::Forward
        );
    }

    #[test]
    fn flip_swaps_anchor_and_head() {
        let flipped = Selection::new(2, 5).flip();
        assert_eq!(flipped, Selection::new(5, 2));
    }

    #[test]
    fn flip_twice_returns_original() {
        let range = Selection::new(2, 5);
        assert_eq!(range.flip().flip(), range);
    }

    #[test]
    fn flip_of_empty_range_is_unchanged() {
        let range = Selection::point(5);
        assert_eq!(range.flip(), range);
    }

    #[test]
    fn with_direction_noop_when_already_forward() {
        let range = Selection::new(2, 5);
        assert_eq!(range.with_direction(Direction::Forward), range);
    }

    #[test]
    fn with_direction_noop_when_already_backward() {
        let range = Selection::new(5, 2);
        assert_eq!(range.with_direction(Direction::Backward), range);
    }

    #[test]
    fn with_direction_flips_forward_to_backward() {
        let range = Selection::new(2, 5);
        assert_eq!(
            range.with_direction(Direction::Backward),
            Selection::new(5, 2)
        );
    }

    #[test]
    fn with_direction_flips_backward_to_forward() {
        let range = Selection::new(5, 2);
        assert_eq!(
            range.with_direction(Direction::Forward),
            Selection::new(2, 5)
        );
    }

    #[test]
    fn with_direction_on_empty_range_stays_forward() {
        let range = Selection::point(5);
        assert_eq!(range.with_direction(Direction::Forward), range);
        // Empty range is already Forward, so asking for Backward flips it —
        // which is still the same point, since anchor == head.
        assert_eq!(range.with_direction(Direction::Backward), range);
    }

    #[test]
    fn extend_forward_shrinks_anchor_left() {
        let range = Selection::new(5, 8);
        assert_eq!(range.extend(2, 3), Selection::new(2, 8));
    }

    #[test]
    fn extend_forward_grows_head_right() {
        let range = Selection::new(2, 5);
        assert_eq!(range.extend(6, 8), Selection::new(2, 8));
    }

    #[test]
    fn extend_forward_grows_both_sides() {
        let range = Selection::new(4, 6);
        assert_eq!(range.extend(2, 8), Selection::new(2, 8));
    }

    #[test]
    fn extend_forward_noop_when_range_already_covers() {
        let range = Selection::new(1, 9);
        assert_eq!(range.extend(3, 5), range);
    }

    #[test]
    fn extend_backward_preserves_direction() {
        let range = Selection::new(8, 2);
        let result = range.extend(4, 6);
        assert_eq!(result.direction(), Direction::Backward);
    }

    #[test]
    fn extend_backward_grows_head_left() {
        let range = Selection::new(8, 5);
        assert_eq!(range.extend(2, 3), Selection::new(8, 2));
    }

    #[test]
    fn extend_backward_grows_anchor_right() {
        let range = Selection::new(5, 2);
        assert_eq!(range.extend(6, 8), Selection::new(8, 2));
    }

    #[test]
    fn extend_from_empty_range_stays_forward() {
        let range = Selection::point(5);
        let result = range.extend(3, 7);
        assert_eq!(result.direction(), Direction::Forward);
        assert_eq!(result, Selection::new(3, 7));
    }

    #[test]
    fn extend_with_zero_width_target_is_safe() {
        let range = Selection::new(2, 5);
        assert_eq!(range.extend(3, 3), range);
    }

    #[test]
    fn contains_false_for_empty_range() {
        let range = Selection::point(5);
        assert!(!range.contains(5));
        assert!(!range.contains(4));
        assert!(!range.contains(6));
    }

    #[test]
    fn contains_is_direction_agnostic() {
        let forward = Selection::new(2, 5);
        let backward = Selection::new(5, 2);
        for pos in 0..=6 {
            assert_eq!(
                forward.contains(pos),
                backward.contains(pos),
                "mismatch at {pos}"
            );
        }
    }
}
