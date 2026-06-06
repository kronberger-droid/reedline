//! Character classification for word-boundary detection — the shared substrate
//! for vi, emacs, and helix word motions.
//!
//! Every "word" notion in the editor is built on one classifier: each character
//! is sorted into a [`CharClass`], and a word boundary is a *transition* between
//! classes. The flavors differ only in which transitions count:
//! - **small word** (`w`/`b`/`e`): any class change is a boundary.
//! - **big WORD** (`W`/`B`/`E`): only whitespace/EOL transitions count, so a run
//!   of `Word` and `Punctuation` together is one WORD.
//!
//! Modes pick a flavor; the resolver (`locate`) scans with the matching
//! predicate. Keeping the classifier here — mode-agnostic and tested in isolation
//! — means vi-word, vi-WORD, emacs-word, and helix-word are thin variations over
//! one definition rather than eight ad-hoc functions.

use crate::core_editor::graphemes::prev_grapheme_boundary;
use crate::enums::{WordEdge, WordKind};

/// Classification of a character for word-boundary detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // resolver (`locate`) lands in a following commit
pub(crate) enum CharClass {
    /// Alphanumeric or `_` — the characters that make up a "word".
    Word,
    /// Anything else that isn't whitespace or a line ending.
    Punctuation,
    /// Spaces, tabs, etc.
    Whitespace,
    /// Line endings, kept separate from whitespace so a `\n` always forms a
    /// boundary with adjacent spaces (a word motion never glides across lines).
    Eol,
}

/// Sort `ch` into a [`CharClass`].
#[allow(dead_code)] // resolver (`locate`) lands in a following commit
pub(crate) fn categorize_char(ch: char) -> CharClass {
    match ch {
        '\n' => CharClass::Eol,
        ch if ch.is_alphanumeric() || ch == '_' => CharClass::Word,
        ch if ch.is_whitespace() => CharClass::Whitespace,
        _ => CharClass::Punctuation,
    }
}

/// `true` if a *small word* boundary lies between `a` and `b` — any class change.
#[allow(dead_code)] // resolver (`locate`) lands in a following commit
pub(crate) fn is_word_boundary(a: char, b: char) -> bool {
    categorize_char(a) != categorize_char(b)
}

/// `true` if a *big WORD* boundary lies between `a` and `b` — a class change,
/// except `Word`↔`Punctuation`, which stay fused into one WORD.
#[allow(dead_code)] // resolver (`locate`) lands in a following commit
pub(crate) fn is_long_word_boundary(a: char, b: char) -> bool {
    match (categorize_char(a), categorize_char(b)) {
        (CharClass::Word, CharClass::Punctuation) | (CharClass::Punctuation, CharClass::Word) => {
            false
        }
        (a, b) => a != b,
    }
}

/// Byte offset of the word boundary reached from `origin`, scanning `forward`
/// (or backward), using `kind`'s boundary predicate and landing on `edge`.
///
/// This is the single resolver the 8 ad-hoc `LineBuffer::*_index` functions
/// collapse into. The `(forward, edge)` pairs map to vi motions:
/// - `(true,  Start)` → `w` / `W`   (next word's first char)
/// - `(true,  End)`   → `e` / `E`   (next word's last char, inclusive)
/// - `(false, Start)` → `b` / `B`   (previous word's first char)
#[allow(dead_code)] // producer is the `Boundary::Word` locate arm + re-lowered motions
pub(crate) fn locate_word(
    buf: &str,
    origin: usize,
    kind: WordKind,
    edge: WordEdge,
    forward: bool,
) -> usize {
    // The only thing `kind` changes is which transitions count as a boundary.
    let is_boundary: fn(char, char) -> bool = match kind {
        WordKind::Small => is_word_boundary,
        WordKind::Big => is_long_word_boundary,
    };

    let chars: Vec<(usize, char)> = buf.char_indices().collect();

    // Is char `i` the `edge` of a word? A word excludes whitespace/EOL, so its
    // `Start` is a non-whitespace char with a boundary on its left (or buffer
    // start), and its `End` one with a boundary on its right (or buffer end).
    let is_target = |i: usize| -> bool {
        let ch = chars[i].1;
        if ch.is_whitespace() {
            return false;
        }
        match edge {
            WordEdge::Start => i == 0 || is_boundary(chars[i - 1].1, ch),
            WordEdge::End => i + 1 == chars.len() || is_boundary(ch, chars[i + 1].1),
        }
    };

    if forward {
        // first target strictly after origin
        for (i, &(byte, _)) in chars.iter().enumerate() {
            if byte > origin && is_target(i) {
                return byte;
            }
        }
        // none: `w` runs to the buffer end; `e` rests on the last grapheme
        match edge {
            WordEdge::Start => buf.len(),
            WordEdge::End => prev_grapheme_boundary(buf, buf.len()),
        }
    } else {
        // nearest target strictly before origin
        for (i, &(byte, _)) in chars.iter().enumerate().rev() {
            if byte < origin && is_target(i) {
                return byte;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorize_each_class() {
        assert_eq!(categorize_char('a'), CharClass::Word);
        assert_eq!(categorize_char('Z'), CharClass::Word);
        assert_eq!(categorize_char('7'), CharClass::Word);
        assert_eq!(categorize_char('_'), CharClass::Word);
        assert_eq!(categorize_char('é'), CharClass::Word); // unicode alphanumeric
        assert_eq!(categorize_char(' '), CharClass::Whitespace);
        assert_eq!(categorize_char('\t'), CharClass::Whitespace);
        assert_eq!(categorize_char('\n'), CharClass::Eol);
        assert_eq!(categorize_char('.'), CharClass::Punctuation);
        assert_eq!(categorize_char('-'), CharClass::Punctuation);
    }

    #[test]
    fn small_word_boundary_is_any_class_change() {
        assert!(is_word_boundary('a', '.')); // Word → Punctuation
        assert!(is_word_boundary('.', 'a')); // Punctuation → Word
        assert!(is_word_boundary('a', ' ')); // Word → Whitespace
        assert!(is_word_boundary(' ', '\n')); // Whitespace → Eol
        assert!(!is_word_boundary('a', 'b')); // both Word
        assert!(!is_word_boundary('.', ',')); // both Punctuation
    }

    #[test]
    fn big_word_boundary_fuses_word_and_punctuation() {
        // Word↔Punctuation is NOT a big-WORD boundary (e.g. `foo.bar` is one WORD)
        assert!(!is_long_word_boundary('o', '.'));
        assert!(!is_long_word_boundary('.', 'b'));
        // but whitespace/eol transitions still are
        assert!(is_long_word_boundary('a', ' '));
        assert!(is_long_word_boundary('.', ' '));
        assert!(is_long_word_boundary(' ', '\n'));
        assert!(!is_long_word_boundary('a', 'b')); // same class, no boundary
    }
}
