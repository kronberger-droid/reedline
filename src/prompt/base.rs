use {
    crate::core_editor::{RestPolicy, SelectionExtent},
    crossterm::style::Color,
    serde::{Deserialize, Serialize},
    std::{
        borrow::Cow,
        fmt::{Display, Formatter},
    },
    strum::{EnumIter, EnumString, IntoDiscriminant},
};

/// The default color for the prompt, indicator, and right prompt
pub static DEFAULT_PROMPT_COLOR: Color = Color::Green;
pub static DEFAULT_PROMPT_MULTILINE_COLOR: nu_ansi_term::Color = nu_ansi_term::Color::LightBlue;
pub static DEFAULT_INDICATOR_COLOR: Color = Color::Cyan;
pub static DEFAULT_PROMPT_RIGHT_COLOR: Color = Color::AnsiValue(5);

/// The current success/failure of the history search
pub enum PromptHistorySearchStatus {
    /// Success for the search
    Passing,

    /// Failure to find the search
    Failing,
}

/// A representation of the history search
pub struct PromptHistorySearch {
    /// The status of the search
    pub status: PromptHistorySearchStatus,

    /// The search term used during the search
    pub term: String,
}

impl PromptHistorySearch {
    /// A constructor to create a history search
    pub const fn new(status: PromptHistorySearchStatus, search_term: String) -> Self {
        PromptHistorySearch {
            status,
            term: search_term,
        }
    }
}

/// Modes that the prompt can be in
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub enum PromptEditMode {
    /// The default mode
    #[default]
    Default,

    /// Emacs normal mode
    Emacs,

    /// A vi-specific mode
    Vi(PromptViMode),

    /// A helix-specific mode (normal, select, or insert)
    Helix(HelixMode),

    /// A custom mode
    Custom(String),
}

impl PromptEditMode {
    pub(crate) fn rest_policy(&self) -> RestPolicy {
        match self {
            PromptEditMode::Vi(PromptViMode::Normal) => RestPolicy::OnGrapheme,
            // Visual selections are min-width-1: the cursor always covers at
            // least the grapheme it sits on, so an empty point widens to a block.
            PromptEditMode::Vi(PromptViMode::Visual) => RestPolicy::Block,
            // Helix normal and select both rest a one-grapheme block cursor.
            PromptEditMode::Helix(HelixMode::Normal | HelixMode::Select) => RestPolicy::Block,
            PromptEditMode::Vi(PromptViMode::Insert)
            | PromptEditMode::Helix(HelixMode::Insert)
            | PromptEditMode::Default
            | PromptEditMode::Emacs => RestPolicy::Between,
            // No catch-all `_ =>` arm over the variants on purpose: a future
            // variant then fails to compile here until it is given an explicit
            // policy, rather than silently defaulting. The `_` below only
            // ignores the custom mode's name.
            PromptEditMode::Custom(_) => RestPolicy::Between,
        }
    }

    /// Whether a non-empty cursor in this mode is an *intentional selection*
    /// rather than the resting caret.
    ///
    /// Needed because a block mode rests one grapheme wide, so cursor shape alone
    /// can't tell vi-visual's `v`-started selection (which must be protected — a
    /// history hint must not clobber it) from helix-normal's resting block (a
    /// plain caret that should behave like vi-normal's). The visual/select modes
    /// answer `true`; the caret modes (normal/insert/emacs) `false`.
    pub(crate) fn is_selection_mode(&self) -> bool {
        matches!(
            self,
            PromptEditMode::Vi(PromptViMode::Visual) | PromptEditMode::Helix(HelixMode::Select)
        )
    }

    /// How a *selecting* motion places its head in this mode (the selection-model
    /// axis, orthogonal to [`rest_policy`](Self::rest_policy)).
    pub(crate) fn selection_extent(&self) -> SelectionExtent {
        match self {
            // Vi normal/visual sweep the block cursor over the grapheme it lands
            // on (vim's inclusive visual: `vw` selects "foo b").
            PromptEditMode::Vi(_) => SelectionExtent::CoverLanding,
            // Helix is block-but-gap-indexed: `w` selects "foo " (caret on the
            // space), not vi-visual's "foo b". The bar modes never form a block
            // selection, and `op_end` is already exclusive there, so `Span` is the
            // natural (and only sensible) reading for them too.
            PromptEditMode::Helix(_)
            | PromptEditMode::Default
            | PromptEditMode::Emacs
            | PromptEditMode::Custom(_) => SelectionExtent::Span,
        }
    }
}

/// The vi-specific modes that the prompt can be in
#[derive(Serialize, Deserialize, Clone, Debug, EnumIter, Default, PartialEq, Eq)]
pub enum PromptViMode {
    /// The default mode
    #[default]
    Normal,

    /// Insertion mode
    Insert,

    /// Visual (selection) mode — like normal, but the cursor carries a
    /// min-width-1 selection that motions extend.
    Visual,
}

/// The helix-specific modes that the prompt can be in
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, EnumIter, Default)]
pub enum HelixMode {
    /// Selection-first command mode
    #[default]
    Normal,

    /// Select/extend mode (`v`)
    Select,

    /// Insertion mode
    Insert,
}

/// This is the discriminant type for [`PromptEditMode`]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, EnumIter, EnumString)]
#[strum(ascii_case_insensitive)]
pub enum PromptEditModeDiscriminants {
    /// The default mode
    #[default]
    Default,

    /// Emacs normal mode
    Emacs,

    /// Vi normal mode
    #[strum(serialize = "ViNormal", serialize = "vi_normal")]
    ViNormal,

    /// Vi insert mode
    #[strum(serialize = "ViInsert", serialize = "vi_insert")]
    ViInsert,

    /// Helix normal mode
    #[strum(serialize = "HelixNormal", serialize = "helix_normal")]
    HelixNormal,

    /// Helix select mode
    #[strum(serialize = "HelixSelect", serialize = "helix_select")]
    HelixSelect,

    /// Helix insert mode
    #[strum(serialize = "HelixInsert", serialize = "helix_insert")]
    HelixInsert,

    /// A custom mode
    Custom,
}

impl From<PromptViMode> for PromptEditMode {
    fn from(value: PromptViMode) -> Self {
        Self::Vi(value)
    }
}

impl Display for PromptEditMode {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        use PromptViMode as Vi;
        match self {
            Self::Default => write!(f, "Default"),
            Self::Emacs => write!(f, "Emacs"),
            Self::Vi(Vi::Normal) => write!(f, "Vi_Normal"),
            Self::Vi(Vi::Insert) => write!(f, "Vi_Insert"),
            Self::Vi(Vi::Visual) => write!(f, "Vi_Visual"),
            Self::Helix(HelixMode::Normal) => write!(f, "Helix_Normal"),
            Self::Helix(HelixMode::Select) => write!(f, "Helix_Select"),
            Self::Helix(HelixMode::Insert) => write!(f, "Helix_Insert"),
            Self::Custom(s) => write!(f, "Custom_{s}"),
        }
    }
}

impl IntoDiscriminant for PromptEditMode {
    type Discriminant = PromptEditModeDiscriminants;

    fn discriminant(&self) -> Self::Discriminant {
        use PromptViMode as Vi;
        match self {
            Self::Default => Self::Discriminant::Default,
            Self::Emacs => Self::Discriminant::Emacs,
            // Visual shares Normal's discriminant: it uses the normal-mode
            // keybindings, differing only in selection geometry.
            Self::Vi(Vi::Normal | Vi::Visual) => Self::Discriminant::ViNormal,
            Self::Vi(Vi::Insert) => Self::Discriminant::ViInsert,
            Self::Helix(HelixMode::Normal) => Self::Discriminant::HelixNormal,
            Self::Helix(HelixMode::Select) => Self::Discriminant::HelixSelect,
            Self::Helix(HelixMode::Insert) => Self::Discriminant::HelixInsert,
            Self::Custom(_) => Self::Discriminant::Custom,
        }
    }
}

/// API to provide a custom prompt.
///
/// Implementors have to provide [`str`]-based content which will be
/// displayed before the `LineBuffer` is drawn.
pub trait Prompt: Send {
    /// Provide content of the left full prompt
    fn render_prompt_left(&self) -> Cow<'_, str>;
    /// Provide content of the right full prompt
    fn render_prompt_right(&self) -> Cow<'_, str>;
    /// Render the prompt indicator (Last part of the prompt that changes based on the editor mode)
    fn render_prompt_indicator(&self, prompt_mode: PromptEditMode) -> Cow<'_, str>;
    /// Indicator to show before explicit new lines
    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str>;
    /// Render the prompt indicator for `Ctrl-R` history search
    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str>;
    /// Get the default prompt color
    fn get_prompt_color(&self) -> Color {
        DEFAULT_PROMPT_COLOR
    }
    /// Get the default multiline prompt color
    fn get_prompt_multiline_color(&self) -> nu_ansi_term::Color {
        DEFAULT_PROMPT_MULTILINE_COLOR
    }
    /// Get the default indicator color
    fn get_indicator_color(&self) -> Color {
        DEFAULT_INDICATOR_COLOR
    }
    /// Get the default right prompt color
    fn get_prompt_right_color(&self) -> Color {
        DEFAULT_PROMPT_RIGHT_COLOR
    }

    /// Whether to render right prompt on the last line
    fn right_prompt_on_last_line(&self) -> bool {
        false
    }
}
