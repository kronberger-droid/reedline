mod helix_keybindings;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
pub use helix_keybindings::{default_helix_insert_keybindings, default_helix_normal_keybindings};

use crate::{
    Direction, EditCommand, EditMode, FindStop, Keybindings, MotionTarget, PromptEditMode,
    PromptHelixMode, ReedlineEvent, WordEdge,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelixMode {
    Normal,
    Insert,
    Select,
}
/// A prefix key waiting for its argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// `f`/`F`/`t`/`T` are waiting for the character to find.
    Find {
        direction: Direction,
        stop: FindStop,
    },
    /// `r` is waiting for the replacement character.
    Replace,
}

/// Every parse_event will result in one of three outcomes:
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Absorb the `ReedlineRawEvent` -> change state -> continue parsing
    Absorb(Pending),
    /// Execute an `Action` matching the completed sequence
    Execute(Action),
    /// Reject a miss-typed sequence
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    SelectingMotion(MotionTarget),
    CollapsingMotion(MotionTarget),
    Collapse(Direction),
    Deselect,
    OnSelection(Op),
    Submit,
    ChangeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Delete,
    Cut,
    Change,
    Yank,
    Replace(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Action {
    count: usize,
    verb: Verb,
    next_mode: Option<HelixMode>,
}

/// This parses incoming input `Event`s like a Helix/Kakoune-style editor: motions are
/// selection first, lowered onto the editor's [`MotionTarget`](crate::MotionTarget) verb vocabulary.
#[derive(Debug, Clone)]
pub struct Helix {
    /// Keybinding lookup table for insert mode
    insert_keybindings: Keybindings,
    /// Keybinding lookup table for normal mode
    normal_keybindings: Keybindings,
    mode: HelixMode,
    /// Count prefix being accumulated (`3w`).
    count: Option<usize>,
    /// Prefix key waiting for its argument (`f`/`r`).
    pending: Option<Pending>,
}

impl EditMode for Helix {
    fn parse_event(&mut self, event: crate::ReedlineRawEvent) -> crate::ReedlineEvent {
        match event.into() {
            Event::Key(key) => match self.mode {
                HelixMode::Insert => self
                    .insert_keybindings
                    .find_binding(key.modifiers, key.code)
                    .unwrap_or_else(|| {
                        if key.modifiers == KeyModifiers::NONE && key.code == KeyCode::Enter {
                            ReedlineEvent::Enter
                        } else {
                            ReedlineEvent::None
                        }
                    }),
                _ => self.dispatch(key),
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
    fn edit_mode(&self) -> crate::PromptEditMode {
        match self.mode {
            HelixMode::Insert => PromptEditMode::Helix(PromptHelixMode::Insert),
            HelixMode::Normal => PromptEditMode::Helix(PromptHelixMode::Normal),
            HelixMode::Select => PromptEditMode::Helix(PromptHelixMode::Select),
        }
    }
}

impl Helix {
    fn dispatch(&mut self, key: KeyEvent) -> ReedlineEvent {
        let outcome = match (self.pending.take(), key.code) {
            // Handle a pending key event
            (Some(pending), _) => complete_pending(pending, self.count.unwrap_or(1), key),
            // Handle a count modifier
            (None, KeyCode::Char(c @ '0'..='9'))
                if key.modifiers == KeyModifiers::NONE && (c != '0' || self.count.is_some()) =>
            {
                self.count = Some(
                    self.count
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(c.to_digit(10).unwrap_or(0) as usize),
                );
                return ReedlineEvent::None;
            }
            // Do a table lookup, else use the helix machine,
            // we don't handle insert mode in dispatch
            (None, code) => {
                if self.count.is_none() {
                    // Esc must always reach the machine, otherwise modes get stranded
                    if code != KeyCode::Esc {
                        if let Some(event) =
                            self.normal_keybindings.find_binding(key.modifiers, code)
                        {
                            return event;
                        }
                    }
                }
                interpret(self.mode, self.count.unwrap_or(1), key)
            }
        };

        match outcome {
            Outcome::Absorb(pending) => {
                self.pending = Some(pending);
                ReedlineEvent::None
            }
            Outcome::Execute(action) => {
                self.count = None;
                let event = lower(action, self.mode);
                if let Some(next_mode) = action.next_mode {
                    self.mode = next_mode;
                }
                event
            }
            Outcome::Reject => {
                self.count = None;
                ReedlineEvent::None
            }
        }
    }
}
impl Default for Helix {
    fn default() -> Self {
        Self {
            insert_keybindings: default_helix_insert_keybindings(),
            normal_keybindings: default_helix_normal_keybindings(),
            mode: HelixMode::Insert,
            count: None,
            pending: None,
        }
    }
}

/// Complete a pending sequence
fn complete_pending(pending: Pending, count: usize, key: KeyEvent) -> Outcome {
    let ch = match key.code {
        KeyCode::Char(ch) if is_typeable(key.modifiers) => ch,
        _ => return Outcome::Reject,
    };

    match pending {
        Pending::Find { direction, stop } => Outcome::Execute(Action {
            count,
            verb: Verb::SelectingMotion(MotionTarget::Find {
                ch,
                direction,
                stop,
            }),
            next_mode: None,
        }),
        Pending::Replace => Outcome::Execute(Action {
            count,
            verb: Verb::OnSelection(Op::Replace(ch)),
            next_mode: None,
        }),
    }
}

/// Interpret a state
fn interpret(mode: HelixMode, count: usize, key: KeyEvent) -> Outcome {
    // reject any non typeable char, this has to be changed when Alt-d is introduced
    if let KeyCode::Char(_) = key.code {
        if !is_typed_char(key.modifiers) {
            return Outcome::Reject;
        }
    }
    match key.code {
        KeyCode::Char(ch) => match ch {
            'f' => Outcome::Absorb(Pending::Find {
                direction: Direction::Forward,
                stop: FindStop::On,
            }),
            'F' => Outcome::Absorb(Pending::Find {
                direction: Direction::Backward,
                stop: FindStop::On,
            }),
            't' => Outcome::Absorb(Pending::Find {
                direction: Direction::Forward,
                stop: FindStop::Before,
            }),
            'T' => Outcome::Absorb(Pending::Find {
                direction: Direction::Backward,
                stop: FindStop::Before,
            }),
            'r' => Outcome::Absorb(Pending::Replace),
            'w' => Outcome::Execute(Action {
                count,
                verb: Verb::SelectingMotion(MotionTarget::Word {
                    kind: crate::WordKind::Word,
                    edge: WordEdge::Start,
                    direction: Direction::Forward,
                }),
                next_mode: None,
            }),
            'b' => Outcome::Execute(Action {
                count,
                verb: Verb::SelectingMotion(MotionTarget::Word {
                    kind: crate::WordKind::Word,
                    edge: WordEdge::Start,
                    direction: Direction::Backward,
                }),
                next_mode: None,
            }),
            'e' => Outcome::Execute(Action {
                count,
                verb: Verb::SelectingMotion(MotionTarget::Word {
                    kind: crate::WordKind::Word,
                    edge: WordEdge::End,
                    direction: Direction::Forward,
                }),
                next_mode: None,
            }),
            'W' => Outcome::Execute(Action {
                count,
                verb: Verb::SelectingMotion(MotionTarget::Word {
                    kind: crate::WordKind::LongWord,
                    edge: WordEdge::Start,
                    direction: Direction::Forward,
                }),
                next_mode: None,
            }),
            'B' => Outcome::Execute(Action {
                count,
                verb: Verb::SelectingMotion(MotionTarget::Word {
                    kind: crate::WordKind::LongWord,
                    edge: WordEdge::Start,
                    direction: Direction::Backward,
                }),
                next_mode: None,
            }),
            'E' => Outcome::Execute(Action {
                count,
                verb: Verb::SelectingMotion(MotionTarget::Word {
                    kind: crate::WordKind::LongWord,
                    edge: WordEdge::End,
                    direction: Direction::Forward,
                }),
                next_mode: None,
            }),
            'l' => Outcome::Execute(Action {
                count,
                verb: Verb::CollapsingMotion(MotionTarget::Grapheme(Direction::Forward)),
                next_mode: None,
            }),
            'h' => Outcome::Execute(Action {
                count,
                verb: Verb::CollapsingMotion(MotionTarget::Grapheme(Direction::Backward)),
                next_mode: None,
            }),
            'v' => match mode {
                HelixMode::Normal => Outcome::Execute(Action {
                    count,
                    verb: Verb::ChangeMode,
                    next_mode: Some(HelixMode::Select),
                }),
                HelixMode::Select => Outcome::Execute(Action {
                    count,
                    verb: Verb::ChangeMode,
                    next_mode: Some(HelixMode::Normal),
                }),
                _ => Outcome::Reject,
            },
            'i' => Outcome::Execute(Action {
                count,
                verb: Verb::Collapse(Direction::Backward),
                next_mode: Some(HelixMode::Insert),
            }),
            'a' => Outcome::Execute(Action {
                count,
                verb: Verb::Collapse(Direction::Forward),
                next_mode: Some(HelixMode::Insert),
            }),
            'd' => Outcome::Execute(Action {
                count,
                verb: Verb::OnSelection(Op::Cut),
                next_mode: Some(HelixMode::Normal),
            }),
            'c' => Outcome::Execute(Action {
                count,
                verb: Verb::OnSelection(Op::Change),
                next_mode: Some(HelixMode::Insert),
            }),
            'y' => Outcome::Execute(Action {
                count,
                verb: Verb::OnSelection(Op::Yank),
                next_mode: Some(HelixMode::Normal),
            }),
            _ => Outcome::Reject,
        },
        KeyCode::Enter => Outcome::Execute(Action {
            count,
            verb: Verb::Submit,
            next_mode: Some(HelixMode::Insert),
        }),
        KeyCode::Esc => match mode {
            HelixMode::Normal => Outcome::Execute(Action {
                count,
                verb: Verb::Deselect,
                next_mode: None,
            }),
            HelixMode::Select => Outcome::Execute(Action {
                count,
                verb: Verb::ChangeMode,
                next_mode: Some(HelixMode::Normal),
            }),
            HelixMode::Insert => Outcome::Reject,
        },
        _ => Outcome::Reject,
    }
}

/// Lowers an `Action` onto `ReedlineEvent`
fn lower(action: Action, mode: HelixMode) -> ReedlineEvent {
    let event = match action.verb {
        Verb::SelectingMotion(target) => match mode {
            HelixMode::Normal => {
                ReedlineEvent::Edit(vec![EditCommand::Select(target); action.count])
            }
            HelixMode::Select => {
                ReedlineEvent::Edit(vec![EditCommand::Extend(target); action.count])
            }
            HelixMode::Insert => {
                // unreachable at runtime: dispatch guards against insert mode
                ReedlineEvent::None
            }
        },
        Verb::CollapsingMotion(target) => match mode {
            HelixMode::Normal => ReedlineEvent::Edit(vec![EditCommand::Move(target); action.count]),
            HelixMode::Select => {
                ReedlineEvent::Edit(vec![EditCommand::Extend(target); action.count])
            }
            HelixMode::Insert => {
                // unreachable at runtime: dispatch guards against insert mode
                ReedlineEvent::None
            }
        },
        Verb::OnSelection(op) => match op {
            Op::Cut => ReedlineEvent::Edit(vec![EditCommand::CutSelection]),
            Op::Change => ReedlineEvent::Edit(vec![EditCommand::CutSelection]),
            Op::Yank => ReedlineEvent::Edit(vec![EditCommand::CopySelection]),
            Op::Replace(ch) => ReedlineEvent::Edit(vec![EditCommand::ReplaceChar(ch)]),
            Op::Delete => {
                // TODO: delete without touching the cut buffer (Alt-d)
                ReedlineEvent::Edit(vec![EditCommand::CutSelection])
            }
        },
        Verb::Collapse(dir) => ReedlineEvent::Edit(vec![EditCommand::CollapseSelection(dir)]),
        Verb::Deselect => ReedlineEvent::Multiple(vec![ReedlineEvent::Esc, ReedlineEvent::Repaint]),
        Verb::ChangeMode => ReedlineEvent::None,
        Verb::Submit => {
            return ReedlineEvent::Enter;
        }
    };

    if action.next_mode.is_some() {
        ReedlineEvent::Multiple(vec![event, ReedlineEvent::Repaint])
    } else {
        event
    }
}

fn is_typeable(modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::NONE || modifiers == KeyModifiers::SHIFT
}

/// Modifier sets under which a `KeyCode::Char` is *typed text* (data), not a chord
fn is_typed_char(modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::NONE
        || modifiers == KeyModifiers::SHIFT
        || modifiers == KeyModifiers::CONTROL | KeyModifiers::ALT
        || modifiers == KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT
}
