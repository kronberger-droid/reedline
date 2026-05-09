#[cfg(feature = "helix")]
use unicode_segmentation::GraphemeCursor;
use unicode_segmentation::UnicodeSegmentation;

/// Byte index of the next grapheme boundary at or after `pos`.
///
/// Returns `buf.len()` if there is no grapheme after `pos`.
///
/// # Panics
///
/// Panics if `pos` is not on a UTF-8 character boundary in `buf`.
pub fn next_grapheme_boundary(buf: &str, pos: usize) -> usize {
    buf[pos..]
        .grapheme_indices(true)
        .nth(1)
        .map(|(i, _)| pos + i)
        .unwrap_or(buf.len())
}

/// Byte index of the previous grapheme boundary before `pos`.
///
/// Returns `0` if there is no grapheme before `pos`.
///
/// # Panics
///
/// Panics if `pos` is not on a UTF-8 character boundary in `buf`.
pub fn prev_grapheme_boundary(buf: &str, pos: usize) -> usize {
    buf[..pos]
        .grapheme_indices(true)
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// `true` when `pos` falls on a grapheme boundary in `buf`.
///
/// Unlike [`next_grapheme_boundary`] and [`prev_grapheme_boundary`], this is
/// defensive: it does not panic on positions that lie inside a multi-byte
/// codepoint or past `buf.len()`, and returns `false` for those.
#[cfg(feature = "helix")]
pub fn is_grapheme_boundary(buf: &str, pos: usize) -> bool {
    if pos > buf.len() || !buf.is_char_boundary(pos) {
        return false;
    }
    GraphemeCursor::new(pos, buf.len(), true)
        .is_boundary(buf, 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- next_grapheme_boundary ---------------------------------------------

    #[test]
    fn next_advances_one_ascii_char() {
        assert_eq!(next_grapheme_boundary("abc", 0), 1);
    }

    #[test]
    fn next_returns_buf_len_when_at_end() {
        assert_eq!(next_grapheme_boundary("abc", 3), 3);
    }

    #[test]
    fn next_on_empty_buffer_returns_zero() {
        assert_eq!(next_grapheme_boundary("", 0), 0);
    }

    #[test]
    fn next_skips_two_byte_utf8_grapheme() {
        assert_eq!(next_grapheme_boundary("café!", 3), 5);
    }

    #[test]
    fn next_at_end_returns_buf_len() {
        let buf = "café";
        assert_eq!(next_grapheme_boundary(buf, 3), buf.len());
    }

    #[test]
    fn next_treats_combining_mark_as_single_grapheme() {
        assert_eq!(next_grapheme_boundary("e\u{0301}", 0), 3);
    }

    #[test]
    fn next_advances_one_cjk_char() {
        assert_eq!(next_grapheme_boundary("日本", 0), 3);
    }

    #[test]
    fn next_skips_zwj_emoji_sequence_as_one() {
        // family-emoji + `!`. From 0, skip the whole 18-byte sequence and land on `!`
        let prefix = "👨‍👩‍👧";
        assert_eq!(next_grapheme_boundary("👨‍👩‍👧!", 0), prefix.len());
    }

    // --- prev_grapheme_boundary ---------------------------------------------

    #[test]
    fn prev_retreats_one_ascii_char() {
        assert_eq!(prev_grapheme_boundary("abc", 2), 1);
    }

    #[test]
    fn prev_at_zero_returns_zero() {
        assert_eq!(prev_grapheme_boundary("abc", 0), 0);
    }

    #[test]
    fn prev_retreats_past_two_byte_utf8_grapheme() {
        // from byte 5 (end of "café") retreat past `é` to byte 3 (its start)
        let buf = "café";
        assert_eq!(prev_grapheme_boundary(buf, buf.len()), 3);
    }

    #[test]
    fn prev_retreats_past_combining_mark() {
        // 'a' + combined 'é' (3 bytes). From end, retreat past combined grapheme to byte 1
        let buf = "ae\u{0301}";
        assert_eq!(prev_grapheme_boundary(buf, buf.len()), 1);
    }

    #[test]
    fn prev_retreats_past_zwj_emoji_sequence() {
        // 'a' + family-emoji (18 bytes). From end, retreat past the family to byte 1
        let buf = "a👨‍👩‍👧";
        assert_eq!(prev_grapheme_boundary(buf, buf.len()), 1);
    }

    // --- round-trip ----------------------------------------------------------

    #[test]
    fn next_then_prev_returns_to_origin_for_ascii() {
        let buf = "abc";
        for (pos, _) in buf.grapheme_indices(true) {
            assert_eq!(
                prev_grapheme_boundary(buf, next_grapheme_boundary(buf, pos)),
                pos,
                "round-trip failed at pos {pos}"
            );
        }
    }

    #[test]
    fn next_then_prev_returns_to_origin_for_unicode() {
        // mix ASCII, multi-byte, combining mark, and ZWJ emoji
        let buf = "a日e\u{0301}👨‍👩‍👧";
        for (pos, _) in buf.grapheme_indices(true) {
            assert_eq!(
                prev_grapheme_boundary(buf, next_grapheme_boundary(buf, pos)),
                pos,
                "round-trip failed at pos {pos}"
            );
        }
    }

    // --- is_grapheme_boundary -----------------------------------------------

    #[cfg(feature = "helix")]
    mod is_grapheme_boundary {
        use super::*;

        #[test]
        fn true_at_zero() {
            assert!(is_grapheme_boundary("abc", 0));
            assert!(is_grapheme_boundary("", 0));
        }

        #[test]
        fn true_at_buf_len() {
            assert!(is_grapheme_boundary("abc", 3));
        }

        #[test]
        fn true_at_ascii_positions() {
            for pos in 0..=3 {
                assert!(is_grapheme_boundary("abc", pos), "pos {pos}");
            }
        }

        #[test]
        fn false_past_buf_len() {
            assert!(!is_grapheme_boundary("abc", 4));
        }

        #[test]
        fn false_inside_multi_byte_codepoint() {
            // Byte 4 of "café" lies inside the 'é' codepoint — not a UTF-8 boundary.
            assert!(!is_grapheme_boundary("café", 4));
        }

        #[test]
        fn false_between_base_and_combining_mark() {
            // "e\u{0301}" — byte 1 is a UTF-8 char boundary but the combining
            // acute is part of the same grapheme, so byte 1 is not a grapheme boundary.
            assert!(!is_grapheme_boundary("e\u{0301}", 1));
        }

        #[test]
        fn true_at_every_grapheme_index() {
            let buf = "a日e\u{0301}👨‍👩‍👧";
            for (pos, _) in buf.grapheme_indices(true) {
                assert!(is_grapheme_boundary(buf, pos), "pos {pos}");
            }
            assert!(is_grapheme_boundary(buf, buf.len()));
        }
    }
}
