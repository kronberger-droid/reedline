#![allow(dead_code)]

use crate::core_editor::{next_grapheme_boundary, prev_grapheme_boundary};

use super::range::{Direction, HelixRange};

/// Move the head one grapheme in `direction`, leaving the anchor in place.
///
/// At buffer edges the head is clamped: extending past the end leaves the head
/// at `buf.len()`, extending past the start leaves it at `0`.
///
/// # Panics
///
/// Panics if `range.head()` is not on a UTF-8 character boundary in `buf`.
pub(super) fn extend_grapheme(range: HelixRange, buf: &str, direction: Direction) -> HelixRange {
    let new_head = match direction {
        Direction::Forward => next_grapheme_boundary(buf, range.head()),
        Direction::Backward => prev_grapheme_boundary(buf, range.head()),
    };
    HelixRange::new(range.anchor(), new_head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extend_forward_from_empty_grows_one_grapheme() {
        let range = HelixRange::point(0);
        assert_eq!(
            extend_grapheme(range, "abc", Direction::Forward),
            HelixRange::new(0, 1),
        );
    }

    #[test]
    fn extend_backward_from_empty_grows_one_grapheme() {
        let range = HelixRange::point(2);
        assert_eq!(
            extend_grapheme(range, "abc", Direction::Backward),
            HelixRange::new(2, 1),
        );
    }

    #[test]
    fn extend_forward_at_end_of_buffer_clamps() {
        let range = HelixRange::point(3);
        assert_eq!(
            extend_grapheme(range, "abc", Direction::Forward),
            HelixRange::new(3, 3),
        );
    }

    #[test]
    fn extend_backward_at_start_clamps() {
        let range = HelixRange::point(0);
        assert_eq!(
            extend_grapheme(range, "abc", Direction::Backward),
            HelixRange::new(0, 0),
        );
    }

    #[test]
    fn extend_forward_over_cjk_grapheme() {
        // "日本" — '日' is 3 bytes. Head jumps from 0 to 3.
        let range = HelixRange::point(0);
        assert_eq!(
            extend_grapheme(range, "日本", Direction::Forward),
            HelixRange::new(0, 3),
        );
    }

    #[test]
    fn extend_forward_over_combining_mark() {
        // "e\u{0301}" — base + combining acute, 3 bytes total but one grapheme.
        // Head must skip the whole grapheme, not stop mid-codepoint at byte 1.
        let range = HelixRange::point(0);
        assert_eq!(
            extend_grapheme(range, "e\u{0301}", Direction::Forward),
            HelixRange::new(0, 3),
        );
    }

    #[test]
    fn extend_forward_over_zwj_emoji_sequence() {
        // Family ZWJ emoji — 18 bytes, one grapheme.
        let range = HelixRange::point(0);
        assert_eq!(
            extend_grapheme(range, "👨‍👩‍👧!", Direction::Forward),
            HelixRange::new(0, 18),
        );
    }

    #[test]
    fn extend_preserves_anchor() {
        let range = HelixRange::new(2, 4);
        let extended = extend_grapheme(range, "abcdef", Direction::Forward);
        assert_eq!(extended.anchor(), 2);
        assert_eq!(extended.head(), 5);
    }

    #[test]
    fn extend_can_flip_direction_through_empty() {
        // Backward range new(2, 1). Extending forward shrinks head: 1 -> 2.
        // Now empty at 2. Extending forward again grows head: 2 -> 3,
        // producing a Forward range new(2, 3).
        let backward = HelixRange::new(2, 1);
        let collapsed = extend_grapheme(backward, "abc", Direction::Forward);
        assert_eq!(collapsed, HelixRange::new(2, 2));
        assert!(collapsed.is_empty());

        let forward = extend_grapheme(collapsed, "abc", Direction::Forward);
        assert_eq!(forward, HelixRange::new(2, 3));
        assert_eq!(forward.direction(), Direction::Forward);
    }
}
