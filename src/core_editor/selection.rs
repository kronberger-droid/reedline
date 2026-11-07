use std::ops::Range;

use crate::core_editor::Cursor;

/// Every cursor in a buffer, with one of them marked primary.
///
/// Invariant, established by every constructor: `ranges` is never empty,
/// sorted by start, with no two overlapping (see [`Cursor::overlaps`]), and
/// `primary_index` is in bounds. Mutators that move a range may break the
/// sort or merge order; restoring it after a command is the engine's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    ranges: Vec<Cursor>,
    primary_index: usize,
}

impl Selection {
    /// Builds a selection from `primary` and any number of other cursors,
    /// sorted and merged.
    pub fn new(primary: Cursor, cursors: impl IntoIterator<Item = Cursor>) -> Self {
        let mut cursors: Vec<Cursor> = cursors.into_iter().collect();

        let primary_index = normalize(&mut cursors, primary);
        Self {
            ranges: cursors,
            primary_index,
        }
    }

    /// A selection of just `cursor`.
    pub fn single(cursor: Cursor) -> Self {
        Self::new(cursor, [])
    }

    /// The primary cursor.
    pub fn primary(&self) -> Cursor {
        self.ranges[self.primary_index]
    }

    /// Replaces the primary cursor in place. Does not re-sort or merge.
    pub fn set_primary(&mut self, cursor: Cursor) {
        self.ranges[self.primary_index] = cursor;
    }

    /// Maps every range but the primary through a splice that replaced
    /// `splice` with `new_len` bytes (see [`Cursor::map_through`]). The
    /// primary is the mutator's to place. Does not re-sort or merge: a
    /// secondary clamped onto the primary stays a separate range.
    pub fn map_others(&mut self, splice: Range<usize>, new_len: usize) {
        for (i, r) in self.ranges.iter_mut().enumerate() {
            if i != self.primary_index {
                *r = r.map_through(&splice, new_len)
            }
        }
    }
}

impl Default for Selection {
    /// A single point at the start of the buffer.
    fn default() -> Self {
        Self::single(Cursor::default())
    }
}

fn normalize(cursors: &mut Vec<Cursor>, primary: Cursor) -> usize {
    let mut primary = primary;
    cursors.push(primary);
    if cursors.len() < 2 {
        return 0;
    }

    cursors.sort_unstable_by_key(|c| (c.start(), c.end()));

    let mut merged: Vec<Cursor> = Vec::with_capacity(cursors.len());
    for c in cursors.drain(..) {
        match merged.last_mut() {
            Some(last) if last.overlaps(c) => {
                let hull = last.hull(c);
                if *last == primary || c == primary {
                    primary = hull;
                }
                *last = hull;
            }
            _ => merged.push(c),
        }
    }

    *cursors = merged;
    let idx = cursors.iter().position(|c| *c == primary);
    debug_assert!(idx.is_some(), "the primary survives the merge");
    idx.unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn c(anchor: usize, head: usize) -> Cursor {
        Cursor::new(anchor, head)
    }

    #[rstest]
    #[case(vec![], c(2, 2), vec![c(2, 2)], 0)] // lone primary
    #[case(vec![c(6, 8), c(0, 2)], c(3, 4), vec![c(0, 2), c(3, 4), c(6, 8)], 1)] // unsorted input is sorted
    #[case(vec![c(0, 1)], c(5, 6), vec![c(0, 1), c(5, 6)], 1)] // primary index follows the sort
    #[case(vec![c(2, 5)], c(0, 3), vec![c(0, 5)], 0)] // overlapping cursors merge
    #[case(vec![c(3, 5)], c(0, 3), vec![c(0, 3), c(3, 5)], 0)] // touching cursors stay separate
    #[case(vec![c(0, 3), c(8, 9)], c(2, 5), vec![c(0, 5), c(8, 9)], 0)] // primary merges into the hull
    #[case(vec![c(0, 1), c(4, 6)], c(5, 8), vec![c(0, 1), c(4, 8)], 1)] // primary merges, index after the sort
    #[case(vec![c(0, 3), c(2, 5)], c(4, 8), vec![c(0, 8)], 0)] // a chain of merges ends as one
    #[case(vec![c(2, 4)], c(2, 4), vec![c(2, 4)], 0)] // a duplicate of the primary collapses
    #[case(vec![c(2, 5)], c(2, 2), vec![c(2, 5)], 0)] // point at a range's start is absorbed
    #[case(vec![c(3, 0)], c(5, 2), vec![c(5, 0)], 0)] // two backward cursors stay backward
    fn normalize_follows_the_table(
        #[case] mut cursors: Vec<Cursor>,
        #[case] primary: Cursor,
        #[case] expected: Vec<Cursor>,
        #[case] expected_index: usize,
    ) {
        let index = normalize(&mut cursors, primary);
        assert_eq!(cursors, expected);
        assert_eq!(index, expected_index);
    }

    #[test]
    fn new_builds_a_normalized_selection() {
        let selection = Selection::new(c(5, 6), [c(0, 1), c(5, 6)]);
        assert_eq!(selection.ranges, vec![c(0, 1), c(5, 6)]);
        assert_eq!(selection.primary_index, 1);
    }

    #[test]
    fn map_others_leaves_the_primary_alone() {
        let mut selection = Selection::new(c(5, 6), [c(0, 2), c(9, 12)]);

        // A deletion before every range: the primary would have moved to
        // (4, 5) if it were mapped.
        selection.map_others(0..1, 0);
        assert_eq!(selection.ranges, vec![c(0, 1), c(5, 6), c(8, 11)]);
        assert_eq!(selection.primary(), c(5, 6));

        // An insertion after the primary, inside the last range.
        selection.map_others(10..10, 3);
        assert_eq!(selection.ranges, vec![c(0, 1), c(5, 6), c(8, 14)]);
        assert_eq!(selection.primary(), c(5, 6));
    }

    #[test]
    fn map_others_may_leave_an_overlap() {
        let mut selection = Selection::new(c(2, 3), [c(5, 9)]);

        // The deletion covers the secondary's anchor, which clamps to 1, and
        // its head shifts to 3: it now overlaps the untouched primary. Merging
        // is the normalise step's job, not this one's.
        selection.map_others(1..7, 0);
        assert_eq!(selection.ranges, vec![c(2, 3), c(1, 3)]);
        assert_eq!(selection.primary(), c(2, 3));
    }
}
