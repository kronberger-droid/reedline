mod helix_keybindings;

pub use helix_keybindings::{default_helix_insert_keybindings, default_helix_normal_keybindings};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use super::EditMode;
use crate::{
    edit_mode::keybindings::Keybindings,
    enums::{EditCommand, ReedlineEvent, ReedlineRawEvent},
    Direction, FindStop, MotionTarget, PromptEditMode, PromptViMode, WordEdge, WordKind,
};

/// The mode the Helix machine is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelixMode {
    /// Selection-first command mode: every motion *replaces* the selection
    /// with the span it travels over (`EditCommand::Select`).
    Normal,
    /// Ordinary text entry.
    Insert,
    /// Helix's select/extend mode (`v`): motions keep the anchor and only move
    /// the head (`EditCommand::Extend`).
    Select,
}

/// A prefix key waiting for its argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// `f`/`F`/`t`/`T` — waiting for the character to find.
    Find {
        direction: Direction,
        stop: FindStop,
    },
    /// `r` — waiting for the replacement character.
    Replace,
    /// `g` — waiting for the goto key (`gg`/`ge`/`gh`/`gl`/`gs`).
    Goto,
}

/// This parses incoming input `Event`s like a Helix-style editor: motions are
/// selection-first, lowered onto the editor's [`MotionTarget`] verb vocabulary.
///
/// - In **normal** mode a word/find motion emits [`EditCommand::Select`], so
///   the selection always covers the span the cursor just travelled —
///   `w` selects up to the next word, `d` then deletes it. `h`/`l` and the
///   goto motions (`gh`, `gl`, `gg`, `ge`, `gs`) emit [`EditCommand::Move`]
///   and collapse the selection, like Helix's `move_*` commands.
/// - **Select** mode (`v`) switches every motion to [`EditCommand::Extend`].
/// - Operators act on the selection, falling back to the grapheme under the
///   cursor (Helix's "the cursor is a one-grapheme selection"): `d`/`c` lower
///   to [`EditCommand::CutChar`], `y` to [`EditCommand::CopyChar`].
///
/// Known deviations from Helix, inherited from the line-editor context: the
/// cursor cannot rest on a line's `\n` cell (the `OnGrapheme` rest policy pulls
/// it onto the last grapheme, like vi), `x` selects the line's content rather
/// than extending line-by-line, and `i` collapses the selection at the head
/// instead of jumping to its start.
pub struct Helix {
    insert_keybindings: Keybindings,
    normal_keybindings: Keybindings,
    mode: HelixMode,
    /// Count prefix being accumulated (`3w`).
    count: Option<usize>,
    /// Prefix key waiting for its argument (`f`/`r`/`g`).
    pending: Option<Pending>,
}

impl Default for Helix {
    fn default() -> Self {
        Helix {
            insert_keybindings: default_helix_insert_keybindings(),
            normal_keybindings: default_helix_normal_keybindings(),
            mode: HelixMode::Insert,
            count: None,
            pending: None,
        }
    }
}

impl std::fmt::Debug for Helix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Helix")
            .field("mode", &self.mode)
            .field("count", &self.count)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl Helix {
    /// Creates a Helix editor using defined keybindings.
    pub fn new(insert_keybindings: Keybindings, normal_keybindings: Keybindings) -> Self {
        Self {
            insert_keybindings,
            normal_keybindings,
            ..Default::default()
        }
    }

    fn reset_sequence(&mut self) {
        self.count = None;
        self.pending = None;
    }

    /// Take the accumulated count and emit `command` that many times in one
    /// edit batch.
    fn repeat_edit(&mut self, command: EditCommand) -> ReedlineEvent {
        let count = self.count.take().unwrap_or(1).max(1);
        ReedlineEvent::Edit(vec![command; count])
    }

    /// A selection-first motion: `Select` (re-anchor) in normal mode, `Extend`
    /// (keep the anchor) in select mode.
    fn motion_event(&mut self, target: MotionTarget) -> ReedlineEvent {
        let command = if self.mode == HelixMode::Select {
            EditCommand::Extend(target)
        } else {
            EditCommand::Select(target)
        };
        self.repeat_edit(command)
    }

    /// A goto-style motion: `Move` (collapse) in normal mode, `Extend` in
    /// select mode — Helix's `move_*` commands never plant a selection.
    fn move_event(&mut self, target: MotionTarget) -> ReedlineEvent {
        let command = if self.mode == HelixMode::Select {
            EditCommand::Extend(target)
        } else {
            EditCommand::Move(target)
        };
        self.repeat_edit(command)
    }

    fn set_pending(&mut self, pending: Pending) -> ReedlineEvent {
        self.pending = Some(pending);
        ReedlineEvent::None
    }

    /// Switch to insert mode, running `commands` first (under insert's rest
    /// policy — the engine relays the new policy before executing them, so
    /// e.g. `a` can step past the last grapheme).
    fn enter_insert(&mut self, commands: Vec<EditCommand>) -> ReedlineEvent {
        self.count = None;
        self.mode = HelixMode::Insert;
        if commands.is_empty() {
            // No movement to run: deselect explicitly (`Esc`) so a leftover
            // selection doesn't linger into insert mode.
            ReedlineEvent::Multiple(vec![ReedlineEvent::Esc, ReedlineEvent::Repaint])
        } else {
            ReedlineEvent::Multiple(vec![ReedlineEvent::Edit(commands), ReedlineEvent::Repaint])
        }
    }

    fn handle_esc(&mut self) -> ReedlineEvent {
        self.reset_sequence();
        match self.mode {
            // Unlike vi, leaving insert does not step the cursor left; the
            // `OnGrapheme` rest policy alone pulls a line-end caret back onto
            // the last grapheme.
            HelixMode::Insert | HelixMode::Normal => {
                self.mode = HelixMode::Normal;
                ReedlineEvent::Multiple(vec![ReedlineEvent::Esc, ReedlineEvent::Repaint])
            }
            // Leaving select mode keeps the selection, like Helix.
            HelixMode::Select => {
                self.mode = HelixMode::Normal;
                ReedlineEvent::Repaint
            }
        }
    }

    fn handle_pending(&mut self, pending: Pending, c: char) -> ReedlineEvent {
        match pending {
            Pending::Find { direction, stop } => self.motion_event(MotionTarget::Find {
                ch: c,
                direction,
                stop,
            }),
            Pending::Replace => {
                self.count = None;
                ReedlineEvent::Edit(vec![EditCommand::ReplaceChar(c)])
            }
            Pending::Goto => {
                let select = self.mode == HelixMode::Select;
                match c {
                    'h' => self.move_event(MotionTarget::LineEdge(Direction::Backward)),
                    'l' => self.move_event(MotionTarget::LineEdge(Direction::Forward)),
                    'g' => self.move_event(MotionTarget::BufferEdge(Direction::Backward)),
                    'e' => self.move_event(MotionTarget::BufferEdge(Direction::Forward)),
                    's' => {
                        self.count = None;
                        ReedlineEvent::Edit(vec![EditCommand::MoveToLineNonBlankStart { select }])
                    }
                    _ => {
                        self.reset_sequence();
                        ReedlineEvent::None
                    }
                }
            }
        }
    }

    fn handle_normal_key(&mut self, c: char) -> ReedlineEvent {
        // Count prefix. A bare `0` is unbound (like Helix), so it only counts
        // as a digit when a count is already being accumulated.
        if let Some(digit) = c.to_digit(10) {
            if digit != 0 || self.count.is_some() {
                self.count = Some(
                    self.count
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(digit as usize),
                );
                return ReedlineEvent::None;
            }
        }

        let word = |kind, edge, direction| MotionTarget::Word {
            kind,
            edge,
            direction,
        };

        match c {
            'h' => self.move_event(MotionTarget::Grapheme(Direction::Backward)),
            'l' => self.move_event(MotionTarget::Grapheme(Direction::Forward)),
            'j' | 'k' if self.mode == HelixMode::Select => {
                self.count = None;
                ReedlineEvent::Edit(vec![if c == 'j' {
                    EditCommand::MoveLineDown { select: true }
                } else {
                    EditCommand::MoveLineUp { select: true }
                }])
            }
            'j' => {
                self.count = None;
                ReedlineEvent::UntilFound(vec![ReedlineEvent::MenuDown, ReedlineEvent::Down])
            }
            'k' => {
                self.count = None;
                ReedlineEvent::UntilFound(vec![ReedlineEvent::MenuUp, ReedlineEvent::Up])
            }
            'w' => self.motion_event(word(WordKind::Small, WordEdge::Start, Direction::Forward)),
            'W' => self.motion_event(word(WordKind::Big, WordEdge::Start, Direction::Forward)),
            'e' => self.motion_event(word(WordKind::Small, WordEdge::End, Direction::Forward)),
            'E' => self.motion_event(word(WordKind::Big, WordEdge::End, Direction::Forward)),
            'b' => self.motion_event(word(WordKind::Small, WordEdge::Start, Direction::Backward)),
            'B' => self.motion_event(word(WordKind::Big, WordEdge::Start, Direction::Backward)),
            'f' => self.set_pending(Pending::Find {
                direction: Direction::Forward,
                stop: FindStop::On,
            }),
            't' => self.set_pending(Pending::Find {
                direction: Direction::Forward,
                stop: FindStop::Before,
            }),
            'F' => self.set_pending(Pending::Find {
                direction: Direction::Backward,
                stop: FindStop::On,
            }),
            'T' => self.set_pending(Pending::Find {
                direction: Direction::Backward,
                stop: FindStop::Before,
            }),
            'g' => self.set_pending(Pending::Goto),
            'r' => self.set_pending(Pending::Replace),
            // Select the current line: jump to its start, then select through
            // its end.
            'x' => {
                self.count = None;
                ReedlineEvent::Edit(vec![
                    EditCommand::Move(MotionTarget::LineEdge(Direction::Backward)),
                    EditCommand::Select(MotionTarget::LineEdge(Direction::Forward)),
                ])
            }
            // Collapse the selection onto the cursor; the engine's `Esc`
            // handling is exactly that (it also closes an open menu).
            ';' => {
                self.count = None;
                ReedlineEvent::Esc
            }
            'v' => {
                self.count = None;
                self.mode = if self.mode == HelixMode::Select {
                    HelixMode::Normal
                } else {
                    HelixMode::Select
                };
                ReedlineEvent::Repaint
            }
            'i' => self.enter_insert(vec![]),
            'a' => self.enter_insert(vec![EditCommand::Move(MotionTarget::Grapheme(
                Direction::Forward,
            ))]),
            'I' => self.enter_insert(vec![EditCommand::MoveToLineNonBlankStart { select: false }]),
            'A' => self.enter_insert(vec![EditCommand::MoveToLineEnd { select: false }]),
            'o' => self.enter_insert(vec![EditCommand::InsertNewlineBelow]),
            'O' => self.enter_insert(vec![EditCommand::InsertNewlineAbove]),
            // Operators: the selection, or the grapheme under the cursor.
            'd' => {
                self.count = None;
                self.mode = HelixMode::Normal;
                ReedlineEvent::Edit(vec![EditCommand::CutChar])
            }
            'c' => self.enter_insert(vec![EditCommand::CutChar]),
            'y' => {
                self.count = None;
                self.mode = HelixMode::Normal;
                ReedlineEvent::Edit(vec![EditCommand::CopyChar])
            }
            'p' => self.repeat_edit(EditCommand::PasteCutBufferAfter),
            'P' => self.repeat_edit(EditCommand::PasteCutBufferBefore),
            'u' => self.repeat_edit(EditCommand::Undo),
            'U' => self.repeat_edit(EditCommand::Redo),
            '~' => self.repeat_edit(EditCommand::SwitchcaseChar),
            '%' => {
                self.count = None;
                ReedlineEvent::Edit(vec![EditCommand::SelectAll])
            }
            _ => {
                self.count = None;
                ReedlineEvent::None
            }
        }
    }
}

impl EditMode for Helix {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        match event.into() {
            Event::Key(KeyEvent {
                code, modifiers, ..
            }) => match (self.mode, modifiers, code) {
                (_, KeyModifiers::NONE, KeyCode::Esc) => self.handle_esc(),
                (HelixMode::Normal | HelixMode::Select, KeyModifiers::ALT, KeyCode::Char(';')) => {
                    // Alt-; — flip the selection's cursor and anchor.
                    self.count = None;
                    ReedlineEvent::Edit(vec![EditCommand::SwapCursorAndAnchor])
                }
                (HelixMode::Normal | HelixMode::Select, modifier, KeyCode::Char(c)) => {
                    // A pending prefix consumes the next character outright
                    // (`f<char>` must even swallow chars a binding would claim).
                    if let Some(pending) = self.pending.take() {
                        if modifier == KeyModifiers::NONE || modifier == KeyModifiers::SHIFT {
                            return self.handle_pending(pending, c);
                        }
                        self.reset_sequence();
                        return ReedlineEvent::None;
                    }

                    if let Some(event) = self
                        .normal_keybindings
                        .find_binding(modifier, KeyCode::Char(c.to_ascii_lowercase()))
                    {
                        event
                    } else if modifier == KeyModifiers::NONE || modifier == KeyModifiers::SHIFT {
                        self.handle_normal_key(c)
                    } else {
                        ReedlineEvent::None
                    }
                }
                (HelixMode::Insert, modifier, KeyCode::Char(c)) => {
                    // Mixed modifiers (e.g. 'alt gr' keyboards) still insert;
                    // same normalization as `Vi`.
                    let c = match modifier {
                        KeyModifiers::NONE => c,
                        _ => c.to_ascii_lowercase(),
                    };

                    self.insert_keybindings
                        .find_binding(modifier, KeyCode::Char(c))
                        .unwrap_or_else(|| {
                            if modifier == KeyModifiers::NONE
                                || modifier == KeyModifiers::SHIFT
                                || modifier == KeyModifiers::CONTROL | KeyModifiers::ALT
                                || modifier
                                    == KeyModifiers::CONTROL
                                        | KeyModifiers::ALT
                                        | KeyModifiers::SHIFT
                            {
                                ReedlineEvent::Edit(vec![EditCommand::InsertChar(
                                    if modifier == KeyModifiers::SHIFT {
                                        c.to_ascii_uppercase()
                                    } else {
                                        c
                                    },
                                )])
                            } else {
                                ReedlineEvent::None
                            }
                        })
                }
                (HelixMode::Normal | HelixMode::Select, _, _) => self
                    .normal_keybindings
                    .find_binding(modifiers, code)
                    .unwrap_or_else(|| {
                        if modifiers == KeyModifiers::NONE && code == KeyCode::Enter {
                            self.reset_sequence();
                            self.mode = HelixMode::Insert;
                            ReedlineEvent::Enter
                        } else {
                            ReedlineEvent::None
                        }
                    }),
                (HelixMode::Insert, _, _) => self
                    .insert_keybindings
                    .find_binding(modifiers, code)
                    .unwrap_or_else(|| {
                        if modifiers == KeyModifiers::NONE && code == KeyCode::Enter {
                            ReedlineEvent::Enter
                        } else {
                            ReedlineEvent::None
                        }
                    }),
            },

            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(button),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }) => ReedlineEvent::Mouse {
                column,
                row,
                button: button.into(),
            },
            Event::Mouse(_) => ReedlineEvent::None,
            Event::Resize(width, height) => ReedlineEvent::Resize(width, height),
            Event::FocusGained => ReedlineEvent::None,
            Event::FocusLost => ReedlineEvent::None,
            Event::Paste(body) => ReedlineEvent::Edit(vec![EditCommand::InsertString(
                body.replace("\r\n", "\n").replace('\r', "\n"),
            )]),
        }
    }

    fn edit_mode(&self) -> PromptEditMode {
        match self.mode {
            HelixMode::Normal | HelixMode::Select => PromptEditMode::Vi(PromptViMode::Normal),
            HelixMode::Insert => PromptEditMode::Vi(PromptViMode::Insert),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> ReedlineRawEvent {
        ReedlineRawEvent::try_from(Event::Key(KeyEvent::new(code, modifiers))).unwrap()
    }

    fn chr(c: char) -> ReedlineRawEvent {
        let modifiers = if c.is_ascii_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        key(KeyCode::Char(c), modifiers)
    }

    fn normal() -> Helix {
        Helix {
            mode: HelixMode::Normal,
            ..Default::default()
        }
    }

    fn word(kind: WordKind, edge: WordEdge, direction: Direction) -> MotionTarget {
        MotionTarget::Word {
            kind,
            edge,
            direction,
        }
    }

    #[test]
    fn defaults_to_insert_and_inserts_chars() {
        let mut helix = Helix::default();
        assert!(matches!(
            helix.edit_mode(),
            PromptEditMode::Vi(PromptViMode::Insert)
        ));
        assert_eq!(
            helix.parse_event(chr('a')),
            ReedlineEvent::Edit(vec![EditCommand::InsertChar('a')])
        );
    }

    #[test]
    fn esc_enters_normal_without_step_back() {
        // Helix does not walk the cursor left on leaving insert; only the
        // rest-policy commit settles a line-end caret.
        let mut helix = Helix::default();
        assert_eq!(
            helix.parse_event(key(KeyCode::Esc, KeyModifiers::NONE)),
            ReedlineEvent::Multiple(vec![ReedlineEvent::Esc, ReedlineEvent::Repaint])
        );
        assert!(matches!(
            helix.edit_mode(),
            PromptEditMode::Vi(PromptViMode::Normal)
        ));
    }

    #[rstest]
    #[case('w', WordKind::Small, WordEdge::Start, Direction::Forward)]
    #[case('W', WordKind::Big, WordEdge::Start, Direction::Forward)]
    #[case('e', WordKind::Small, WordEdge::End, Direction::Forward)]
    #[case('E', WordKind::Big, WordEdge::End, Direction::Forward)]
    #[case('b', WordKind::Small, WordEdge::Start, Direction::Backward)]
    #[case('B', WordKind::Big, WordEdge::Start, Direction::Backward)]
    fn word_motions_select_in_normal_mode(
        #[case] c: char,
        #[case] kind: WordKind,
        #[case] edge: WordEdge,
        #[case] direction: Direction,
    ) {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(chr(c)),
            ReedlineEvent::Edit(vec![EditCommand::Select(word(kind, edge, direction))])
        );
    }

    #[test]
    fn word_motion_extends_in_select_mode() {
        let mut helix = normal();
        assert_eq!(helix.parse_event(chr('v')), ReedlineEvent::Repaint);
        assert_eq!(
            helix.parse_event(chr('w')),
            ReedlineEvent::Edit(vec![EditCommand::Extend(word(
                WordKind::Small,
                WordEdge::Start,
                Direction::Forward
            ))])
        );
        // `v` toggles back to normal.
        assert_eq!(helix.parse_event(chr('v')), ReedlineEvent::Repaint);
        assert!(matches!(helix.mode, HelixMode::Normal));
    }

    #[test]
    fn count_repeats_motion() {
        let mut helix = normal();
        assert_eq!(helix.parse_event(chr('3')), ReedlineEvent::None);
        let target = word(WordKind::Small, WordEdge::Start, Direction::Forward);
        assert_eq!(
            helix.parse_event(chr('w')),
            ReedlineEvent::Edit(vec![EditCommand::Select(target); 3])
        );
        assert_eq!(helix.count, None);
    }

    #[test]
    fn find_waits_for_char_then_selects() {
        let mut helix = normal();
        assert_eq!(helix.parse_event(chr('f')), ReedlineEvent::None);
        assert_eq!(
            helix.parse_event(chr('x')),
            ReedlineEvent::Edit(vec![EditCommand::Select(MotionTarget::Find {
                ch: 'x',
                direction: Direction::Forward,
                stop: FindStop::On,
            })])
        );
    }

    #[test]
    fn till_backward_uses_stop_before() {
        let mut helix = normal();
        let _ = helix.parse_event(chr('T'));
        assert_eq!(
            helix.parse_event(chr('a')),
            ReedlineEvent::Edit(vec![EditCommand::Select(MotionTarget::Find {
                ch: 'a',
                direction: Direction::Backward,
                stop: FindStop::Before,
            })])
        );
    }

    #[rstest]
    #[case('h', MotionTarget::LineEdge(Direction::Backward))]
    #[case('l', MotionTarget::LineEdge(Direction::Forward))]
    #[case('g', MotionTarget::BufferEdge(Direction::Backward))]
    #[case('e', MotionTarget::BufferEdge(Direction::Forward))]
    fn goto_motions_move_in_normal_mode(#[case] c: char, #[case] target: MotionTarget) {
        let mut helix = normal();
        let _ = helix.parse_event(chr('g'));
        assert_eq!(
            helix.parse_event(chr(c)),
            ReedlineEvent::Edit(vec![EditCommand::Move(target)])
        );
    }

    #[test]
    fn h_and_l_collapse_in_normal_extend_in_select() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(chr('l')),
            ReedlineEvent::Edit(vec![EditCommand::Move(MotionTarget::Grapheme(
                Direction::Forward
            ))])
        );
        let _ = helix.parse_event(chr('v'));
        assert_eq!(
            helix.parse_event(chr('h')),
            ReedlineEvent::Edit(vec![EditCommand::Extend(MotionTarget::Grapheme(
                Direction::Backward
            ))])
        );
    }

    #[test]
    fn d_cuts_selection_or_char() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(chr('d')),
            ReedlineEvent::Edit(vec![EditCommand::CutChar])
        );
        assert!(matches!(helix.mode, HelixMode::Normal));
    }

    #[test]
    fn c_cuts_and_enters_insert() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(chr('c')),
            ReedlineEvent::Multiple(vec![
                ReedlineEvent::Edit(vec![EditCommand::CutChar]),
                ReedlineEvent::Repaint,
            ])
        );
        assert!(matches!(helix.mode, HelixMode::Insert));
    }

    #[test]
    fn y_copies_selection_or_char() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(chr('y')),
            ReedlineEvent::Edit(vec![EditCommand::CopyChar])
        );
    }

    #[test]
    fn a_steps_right_then_inserts() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(chr('a')),
            ReedlineEvent::Multiple(vec![
                ReedlineEvent::Edit(vec![EditCommand::Move(MotionTarget::Grapheme(
                    Direction::Forward
                ))]),
                ReedlineEvent::Repaint,
            ])
        );
        assert!(matches!(helix.mode, HelixMode::Insert));
    }

    #[test]
    fn x_selects_the_current_line() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(chr('x')),
            ReedlineEvent::Edit(vec![
                EditCommand::Move(MotionTarget::LineEdge(Direction::Backward)),
                EditCommand::Select(MotionTarget::LineEdge(Direction::Forward)),
            ])
        );
    }

    #[test]
    fn semicolon_collapses_selection() {
        let mut helix = normal();
        assert_eq!(helix.parse_event(chr(';')), ReedlineEvent::Esc);
    }

    #[test]
    fn alt_semicolon_flips_cursor_and_anchor() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(key(KeyCode::Char(';'), KeyModifiers::ALT)),
            ReedlineEvent::Edit(vec![EditCommand::SwapCursorAndAnchor])
        );
    }

    #[test]
    fn replace_waits_for_char() {
        let mut helix = normal();
        assert_eq!(helix.parse_event(chr('r')), ReedlineEvent::None);
        assert_eq!(
            helix.parse_event(chr('z')),
            ReedlineEvent::Edit(vec![EditCommand::ReplaceChar('z')])
        );
    }

    #[test]
    fn esc_cancels_pending_sequence() {
        let mut helix = normal();
        let _ = helix.parse_event(chr('2'));
        let _ = helix.parse_event(chr('f'));
        let _ = helix.parse_event(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(helix.count, None);
        assert_eq!(helix.pending, None);
        // The next char is interpreted fresh, not as a find argument.
        assert_eq!(
            helix.parse_event(chr('d')),
            ReedlineEvent::Edit(vec![EditCommand::CutChar])
        );
    }

    #[test]
    fn esc_from_select_keeps_selection_and_returns_to_normal() {
        let mut helix = normal();
        let _ = helix.parse_event(chr('v'));
        assert_eq!(
            helix.parse_event(key(KeyCode::Esc, KeyModifiers::NONE)),
            ReedlineEvent::Repaint
        );
        assert!(matches!(helix.mode, HelixMode::Normal));
    }

    #[test]
    fn enter_in_normal_submits_and_enters_insert() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(key(KeyCode::Enter, KeyModifiers::NONE)),
            ReedlineEvent::Enter
        );
        assert!(matches!(helix.mode, HelixMode::Insert));
    }

    #[test]
    fn select_mode_j_extends_line_down() {
        let mut helix = normal();
        let _ = helix.parse_event(chr('v'));
        assert_eq!(
            helix.parse_event(chr('j')),
            ReedlineEvent::Edit(vec![EditCommand::MoveLineDown { select: true }])
        );
    }

    #[test]
    fn undo_and_redo() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(chr('u')),
            ReedlineEvent::Edit(vec![EditCommand::Undo])
        );
        assert_eq!(
            helix.parse_event(chr('U')),
            ReedlineEvent::Edit(vec![EditCommand::Redo])
        );
    }

    #[test]
    fn ctrl_c_uses_common_control_binding() {
        let mut helix = normal();
        assert_eq!(
            helix.parse_event(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            ReedlineEvent::CtrlC
        );
    }

    #[test]
    fn paste_event_produces_insert_string() {
        let mut helix = Helix::default();
        let paste = ReedlineRawEvent::try_from(Event::Paste("hello".to_string())).unwrap();
        assert_eq!(
            helix.parse_event(paste),
            ReedlineEvent::Edit(vec![EditCommand::InsertString("hello".to_string())])
        );
    }
}
