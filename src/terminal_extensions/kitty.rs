use crossterm::{event, execute};

/// Helper managing proper setup and teardown of the kitty keyboard enhancement protocol
///
/// Note that, currently, only the following support this protocol:
/// * [kitty terminal](https://sw.kovidgoyal.net/kitty/)
/// * [foot terminal](https://codeberg.org/dnkl/foot/issues/319)
/// * [WezTerm terminal](https://wezfurlong.org/wezterm/config/lua/config/enable_kitty_keyboard.html)
/// * [notcurses library](https://github.com/dankamongmen/notcurses/issues/2131)
/// * [neovim text editor](https://github.com/neovim/neovim/pull/18181)
/// * [kakoune text editor](https://github.com/mawww/kakoune/issues/4103)
/// * [dte text editor](https://gitlab.com/craigbarnes/dte/-/issues/138)
///
/// Refer to <https://sw.kovidgoyal.net/kitty/keyboard-protocol/> if you're curious.
#[derive(Default)]
pub(crate) struct KittyProtocolGuard {
    /// Explicit user preference. `None` is the default ("auto"): enable when the
    /// terminal supports it. `Some(true)`/`Some(false)` force the protocol on/off.
    preference: Option<bool>,
    active: bool,
    /// Caches whether the terminal supports the kitty protocol; `None` means we haven't checked yet
    /// and `Some(bool)` stores a cached answer.
    support_kitty_protocol: Option<bool>,
}

impl KittyProtocolGuard {
    /// Record an explicit on/off preference, overriding the auto default.
    pub fn set(&mut self, enable: bool) {
        self.preference = Some(enable);
    }

    /// Resolve whether the protocol should be active right now.
    ///
    /// An explicit opt-out wins immediately. Otherwise (auto default or explicit
    /// opt-in) we enable only if the terminal supports it, caching the
    /// side-effecting support check so it runs at most once.
    fn should_enable(&mut self) -> bool {
        if self.preference == Some(false) {
            return false;
        }

        *self
            .support_kitty_protocol
            .get_or_insert_with(super::kitty_protocol_available)
    }

    pub fn enter(&mut self) {
        if !self.active && self.should_enable() {
            let _ = execute!(
                std::io::stdout(),
                event::PushKeyboardEnhancementFlags(
                    event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                )
            );

            self.active = true;
        }
    }
    pub fn exit(&mut self) {
        if self.active {
            let _ = execute!(std::io::stdout(), event::PopKeyboardEnhancementFlags);
            self.active = false;
        }
    }
}

impl Drop for KittyProtocolGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = execute!(std::io::stdout(), event::PopKeyboardEnhancementFlags);
        }
    }
}
