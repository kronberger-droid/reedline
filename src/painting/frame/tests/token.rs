use crate::painting::frame::token::{escape_len, tokens, Token};
use pretty_assertions::assert_eq;
use rstest::rstest;

fn text_of<'a>(token: &Token<'a>) -> &'a str {
    match token {
        Token::Escape(text) | Token::Grapheme(text) => text,
    }
}

#[rstest]
// CSI: parameters, intermediates, one final byte.
#[case::sgr("\x1b[31m", 5)]
#[case::sgr_with_text_after("\x1b[1;32;40mrest", 10)]
#[case::private_parameter("\x1b[?25l", 6)]
#[case::intermediate_byte("\x1b[0 q", 5)]
// String sequences run to BEL or ST.
#[case::osc_ended_by_bel("\x1b]133;A;k=i\x07text", 12)]
#[case::osc_ended_by_st("\x1b]8;;http://x\x1b\\link", 15)]
#[case::dcs("\x1bPq\x1b\\x", 5)]
// Two-byte escapes, with optional intermediates.
#[case::charset("\x1b(B", 3)]
#[case::save_cursor("\x1b7", 2)]
fn escape_len_covers_a_whole_sequence(#[case] text: &str, #[case] expected: usize) {
    assert_eq!(escape_len(text), expected);
}

/// A sequence that never finishes ends where it stops being one. It must not
/// take a byte that has a meaning of its own with it: a swallowed newline is
/// a row the layout never counts.
#[rstest]
#[case::lone_esc("\x1b", 1)]
#[case::csi_cut_short("\x1b[", 2)]
#[case::csi_before_non_ascii("\x1b[1ä", 3)]
#[case::csi_before_newline("\x1b[1\nx", 3)]
#[case::csi_before_another_esc("\x1b[1\x1b[31m", 3)]
#[case::esc_before_newline("\x1b\nx", 1)]
#[case::esc_before_non_ascii("\x1bä", 1)]
#[case::osc_never_ended("\x1b]0;title", 9)]
fn escape_len_stops_a_malformed_sequence_early(#[case] text: &str, #[case] expected: usize) {
    assert_eq!(escape_len(text), expected);
    assert!(text.is_char_boundary(expected));
}

#[test]
fn tokens_split_escapes_from_graphemes() {
    let got: Vec<_> = tokens("a\x1b[31mb\u{301}\x1b[0m").collect();
    assert_eq!(
        got,
        vec![
            Token::Grapheme("a"),
            Token::Escape("\x1b[31m"),
            Token::Grapheme("b\u{301}"),
            Token::Escape("\x1b[0m"),
        ]
    );
}

#[test]
fn empty_text_has_no_tokens() {
    assert_eq!(tokens("").count(), 0);
}

#[test]
fn adjacent_escapes_stay_separate() {
    let got: Vec<_> = tokens("\x1b[1m\x1b[31m\x1b]8;;u\x07").collect();
    assert_eq!(
        got,
        vec![
            Token::Escape("\x1b[1m"),
            Token::Escape("\x1b[31m"),
            Token::Escape("\x1b]8;;u\x07"),
        ]
    );
}

/// The layout counts buffer graphemes to map a click back to the buffer, so
/// the split has to be the one `unicode-segmentation` gives the editor.
#[rstest]
#[case::crlf("a\r\nb", &["a", "\r\n", "b"])]
#[case::lone_newline("a\nb", &["a", "\n", "b"])]
#[case::wide("あい", &["あ", "い"])]
#[case::zwj_emoji("👨‍👩‍👧x", &["👨‍👩‍👧", "x"])]
#[case::combining("e\u{301}e", &["e\u{301}", "e"])]
fn graphemes_are_the_editors(#[case] text: &str, #[case] expected: &[&str]) {
    let got: Vec<_> = tokens(text).collect();
    let expected: Vec<_> = expected.iter().map(|g| Token::Grapheme(g)).collect();
    assert_eq!(got, expected);
}

/// An escape cuts a cluster in two: the terminal has already drawn the base
/// by the time the sequence arrives, so the mark after it is on its own.
#[test]
fn an_escape_between_a_base_and_its_mark_splits_the_cluster() {
    let got: Vec<_> = tokens("e\x1b[31m\u{301}").collect();
    assert_eq!(
        got,
        vec![
            Token::Grapheme("e"),
            Token::Escape("\x1b[31m"),
            Token::Grapheme("\u{301}"),
        ]
    );
}

/// Nothing is dropped and nothing is invented: the tokens are the text, cut
/// up. Every `ESC` opens an `Escape`, and no grapheme carries one.
#[rstest]
#[case("")]
#[case("plain")]
#[case("\x1b[32m~/src\x1b[0m \x1b[1;34m❯\x1b[0m ")]
#[case("\x1b]133;A\x1b\\abc\r\n> \x1b]133;B\x1b\\")]
#[case("a\x1b")]
#[case("a\x1b[")]
#[case("a\x1b[1ä\x1b\nb")]
#[case("\x1b]0;never ended ä\nx")]
#[case("\x1b\x1b\x1b[m")]
fn tokens_are_the_text_cut_up(#[case] text: &str) {
    let got: Vec<_> = tokens(text).collect();
    let joined: String = got.iter().map(text_of).collect();
    assert_eq!(joined, text);
    for token in &got {
        match token {
            Token::Escape(seq) => {
                assert!(seq.starts_with('\x1b'), "{seq:?} does not open with ESC");
                // An ST terminator is the one place a second ESC belongs.
                let body = &seq[1..];
                let body = body.strip_suffix("\x1b\\").unwrap_or(body);
                assert!(!body.contains('\x1b'), "{seq:?} runs into the next ESC");
            }
            Token::Grapheme(grapheme) => {
                assert!(!grapheme.contains('\x1b'), "{grapheme:?} carries an ESC")
            }
        }
    }
}
