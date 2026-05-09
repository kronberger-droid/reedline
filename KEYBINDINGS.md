# Default Keybindings

This document lists the default keybindings shipped with reedline for **emacs**, **vi insert**, and **vi normal** modes. The lists below cover the keybinding map (key chord → action). Vi normal mode also responds to single-key motion commands (`h`, `j`, `w`, `b`, …) interpreted by the vi parser; those are not part of the keybinding map and are not listed here.

To override any of these bindings, see the [Integrate with custom keybindings](README.md#integrate-with-custom-keybindings) example in the README.

## Common bindings

The bindings in this section apply to **emacs** and **vi insert** modes. **Vi normal** mode shares the control, navigation, and selection bindings, but not the editing bindings (vi normal uses motion commands instead).

### Control

| Key | Action |
|---|---|
| `Esc` | Mode-specific (e.g. switch to vi normal mode) |
| `Ctrl+C` | Interrupt the current input |
| `Ctrl+D` | Send EOF (exit on empty buffer) |
| `Ctrl+L` | Clear the screen |
| `Ctrl+R` | Open history search |
| `Ctrl+O` | Open the buffer in an external editor |

### Navigation

| Key | Action |
|---|---|
| `Up` / `Down` | History scroll (or menu up/down when a menu is open) |
| `Left` / `Right` | Move cursor (or menu left/right; `Right` accepts an inline history hint at end of line) |
| `Ctrl+Left` / `Ctrl+Right` | Move by word (`Ctrl+Right` accepts a word from the history hint when available) |
| `Home` / `Ctrl+A` | Move to line start |
| `End` / `Ctrl+E` | Move to line end (accepts the full history hint when at end of line) |
| `Ctrl+Home` / `Ctrl+End` | Move to buffer start / end |
| `Ctrl+P` / `Ctrl+N` | History up / down (also navigates an open menu) |
| `Alt+<` / `Alt+>` | Jump to buffer start / end |
| `Shift+Alt+,` / `Shift+Alt+.` | Jump to buffer start / end (kitty keyboard protocol) |

### Editing

These are part of the *common* set but are excluded from vi normal mode.

| Key | Action |
|---|---|
| `Backspace` | Delete the grapheme before the cursor |
| `Delete` | Delete the grapheme after the cursor |
| `Ctrl+Backspace` / `Ctrl+W` | Delete the word before the cursor (no cut buffer; emacs overrides `Ctrl+W` to cut, see below) |
| `Ctrl+Delete` | Delete the word after the cursor (no cut buffer) |
| `Ctrl+H` | Delete the grapheme before the cursor (alias for `Backspace`) |
| `Ctrl+Shift+X` | Cut selection to system clipboard *(requires `system_clipboard` feature)* |
| `Ctrl+Shift+C` | Copy selection to system clipboard *(requires `system_clipboard` feature)* |
| `Ctrl+Shift+V` | Paste from system clipboard *(requires `system_clipboard` feature)* |
| `Alt+Enter` / `Shift+Enter` | Insert a newline (continue editing on a new line) |
| `Ctrl+J` | Submit the current input |

### Selection

| Key | Action |
|---|---|
| `Shift+Up` / `Shift+Down` | Extend selection one line up / down |
| `Shift+Left` / `Shift+Right` | Extend selection one character left / right |
| `Ctrl+Shift+Left` / `Ctrl+Shift+Right` | Extend selection one word left / right |
| `Shift+Home` / `Shift+End` | Extend selection to line start / end |
| `Ctrl+Shift+Home` / `Ctrl+Shift+End` | Extend selection to buffer start / end |
| `Ctrl+Shift+A` | Select all |

## Emacs mode

In addition to all common bindings (control, navigation, editing, selection), emacs mode adds:

| Key | Action |
|---|---|
| `Enter` | Submit the current input |
| `Ctrl+B` | Move cursor left (or menu left when a menu is open) |
| `Ctrl+F` | Move cursor right (also accepts a history hint at end of line) |
| `Ctrl+G` | Redo |
| `Ctrl+Z` | Undo |
| `Ctrl+Y` | Paste cut buffer (yank) |
| `Ctrl+W` | Cut word before the cursor (overrides the common `BackspaceWord` to use the cut buffer) |
| `Ctrl+K` | Kill (cut) from cursor to line end |
| `Ctrl+U` | Cut from line start to cursor |
| `Ctrl+T` | Swap the two graphemes around the cursor |
| `Alt+D` | Cut word after the cursor |
| `Alt+Left` / `Alt+B` | Move one word left |
| `Alt+Right` / `Alt+F` | Move one word right (accepts a word from the history hint when available) |
| `Alt+Backspace` / `Alt+M` | Delete word before the cursor |
| `Alt+Delete` | Delete word after the cursor |
| `Alt+U` / `Alt+L` | Uppercase / lowercase the current word |
| `Alt+C` | Capitalize the character at the cursor |

## Vi insert mode

Vi insert mode uses the common bindings as-is — control, navigation, editing, and selection — with no extra keys. Press `Esc` to switch to vi normal mode.

## Vi normal mode

Vi normal mode uses the common control, navigation, and selection bindings (no editing — vi normal uses motion commands instead), plus the following vi-specific remappings:

| Key | Action |
|---|---|
| `Backspace` | Move cursor left (vi-default behavior) |
| `Delete` | Delete the grapheme at the cursor |

Vi normal mode also responds to single-key motion commands (`h`, `j`, `k`, `l`, `w`, `e`, `b`, `i`, `a`, …) handled by the vi parser. Those are documented separately.
