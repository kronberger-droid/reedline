use unicode_segmentation::{Graphemes, UnicodeSegmentation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Token<'a> {
    Escape(&'a str),
    Grapheme(&'a str),
}

struct Tokens<'a> {
    rest: &'a str,
    plain: Graphemes<'a>,
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        match (self.rest.is_empty(), self.plain.next()) {
            (_, Some(g)) => Some(Token::Grapheme(g)),
            (true, None) => None,
            (false, None) => {
                if self.rest.starts_with('\x1b') {
                    let offset = escape_len(self.rest);
                    let (head, tail) = self.rest.split_at(offset);
                    self.rest = tail;
                    Some(Token::Escape(head))
                } else {
                    let offset = self.rest.find('\x1b').unwrap_or(self.rest.len());
                    let (head, tail) = self.rest.split_at(offset);
                    self.rest = tail;
                    self.plain = head.graphemes(true);
                    self.plain.next().map(Token::Grapheme)
                }
            }
        }
    }
}

pub(super) fn tokens(text: &str) -> impl Iterator<Item = Token<'_>> {
    Tokens {
        rest: text,
        plain: "".graphemes(true),
    }
}

pub(super) fn escape_len(text: &str) -> usize {
    let bytes = text.as_bytes();
    match bytes.get(1) {
        None => 1,
        Some(b'[') => {
            let end = 2 + bytes[2..]
                .iter()
                .take_while(|b| matches!(b, 0x20..=0x3f))
                .count();
            match bytes.get(end) {
                Some(0x40..=0x7e) => end + 1,
                _ => end,
            }
        }
        Some(b']' | b'P' | b'^' | b'_') => {
            match bytes[2..].iter().position(|b| matches!(b, 0x07 | 0x1b)) {
                None => text.len(),
                Some(i) => {
                    let at = 2 + i;
                    match (bytes[at], bytes.get(at + 1)) {
                        (0x07, _) => at + 1,
                        (_, Some(b'\\')) => at + 2,
                        _ => at,
                    }
                }
            }
        }

        Some(_) => {
            let end = 1 + bytes[1..]
                .iter()
                .take_while(|b| matches!(b, 0x20..=0x2f))
                .count();
            match bytes.get(end) {
                Some(0x30..=0x7e) => end + 1,
                _ => 1,
            }
        }
    }
}
