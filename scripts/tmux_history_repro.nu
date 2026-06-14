#!/usr/bin/env nu
#
# Reproduction harness for #1062 — reedline duplicating the prompt into tmux
# scrollback when the prompt is anchored at row 0.
#
# Mechanism under test:
#   reedline's repaint does `MoveTo(0, anchor_row)` + `Clear(FromCursorDown)`.
#   When anchor_row == 0 the cursor sits on tmux's home cell (0,0); with
#   `scroll-on-clear on` (tmux default) tmux copies the whole screen into
#   scrollback (grid_view_clear_history) on every repaint → the prompt piles
#   up in history. The fix clears from column 1 so the cx==0 guard misses.
#
# Strategy: drive a *detached* tmux session, synchronize on observable grid
# state (never bare sleeps), then count how many times the sentinel prompt
# appears in scrollback. Buggy ≫ 1; fixed ≈ 1. Run the A/B over
# `scroll-on-clear off|on` to prove causation.
#
# Usage: nu scripts/tmux_history_repro.nu

# ── Config ───────────────────────────────────────────────────────────────────

# TODO: how do you want to launch the binary under test inside the session?
#   Option A: build the demo first, run the binary directly (no cargo noise in
#             the pane): `cargo build --example demo` then point at
#             target/debug/examples/demo.
#   Option B: `nu -n` with `$env.PROMPT_COMMAND` set to the sentinel.
# Whichever you pick, the launched program MUST render a fixed, unique left
# prompt (the sentinel) so counting is unambiguous. The stock examples/demo
# and custom_prompt render changing/right-prompt content — consider copying
# examples/custom_prompt.rs to a dedicated example with a STATIC left prompt
# and no per-render counter.
const LAUNCH_CMD = "TODO-command-that-runs-the-binary-under-test"

# TODO: the sentinel string your prompt renders. Used both to detect "ready to
# type" and to count duplicates — so it must be unique and appear exactly once
# per rendered prompt line.
const MARKER = ">"

# TODO: pass/fail threshold. With the fix, the marker should appear roughly
# once (the live prompt). Pick the line you want to assert on, e.g. <= 2.
const MAX_OK_COUNT = 2

const SESSION = "rl-repro"
const COLS = 80
const ROWS = 24
const POLL_TRIES = 40
const POLL_DELAY = 50ms

# ── De-flaking primitive: synchronize on grid state ──────────────────────────

# Poll the visible pane until `marker` shows up, or error out after a bounded
# number of tries. This is what replaces sleep-based timing.
def wait-for [marker: string] {
    for _ in 1..$POLL_TRIES {
        if ((tmux capture-pane -t $SESSION -p) | str contains $marker) { return }
        sleep $POLL_DELAY
    }
    error make {msg: $"timed out waiting for '($marker)' in pane"}
}

# ── Session lifecycle ────────────────────────────────────────────────────────

def setup-session [] {
    teardown   # in case a previous run left one around
    tmux new-session -d -s $SESSION -x $COLS -y $ROWS $LAUNCH_CMD

    # TODO: wait until the program has drawn its first prompt before driving it.
    #   wait-for $MARKER
}

def teardown [] {
    # tmux errors if the session is gone; swallow that.
    try { tmux kill-session -t $SESSION }
}

# ── Driving input ────────────────────────────────────────────────────────────

# Send the keystroke sequence that forces repaints at row 0.
def drive-input [] {
    # Anchor the prompt at the very top of the pane.
    tmux send-keys -t $SESSION C-l
    # TODO: re-sync after the clear (the screen state changed):
    #   wait-for $MARKER

    # TODO: send the keystrokes that trigger repaints. Each character typed is
    # one repaint_buffer → one potential snapshot-to-history pre-fix. Decide:
    #   - what to type (e.g. "echo hello"),
    #   - whether to synchronize after each key (robust) or send a literal
    #     block (fast),
    #   - whether to also exercise the menu path (print_menu) by opening a
    #     completion/history menu and paging through it.
    #
    # Per-key skeleton:
    #   for c in ("echo hello" | split chars) {
    #       tmux send-keys -t $SESSION -l $c
    #       # optional: wait-for ($MARKER + <expected buffer so far>)
    #   }
}

# ── Measurement ──────────────────────────────────────────────────────────────

# Count marker occurrences across the FULL scrollback (history + visible).
def count-marker [] {
    tmux capture-pane -t $SESSION -p -J -S -
        | lines
        | where ($it | str contains $MARKER)
        | length
}

# Run one trial under a given `scroll-on-clear` setting and return the count.
def run-trial [scroll_on_clear: string] {
    # TODO: a large history-limit so duplicates aren't truncated before we count
    # them. Set it before the session is created, or as a global:
    #   tmux set -g history-limit 100000
    tmux set -g scroll-on-clear $scroll_on_clear

    setup-session
    drive-input
    let count = (count-marker)
    teardown

    {scroll_on_clear: $scroll_on_clear, marker_count: $count}
}

# ── Entry point ──────────────────────────────────────────────────────────────

def main [] {
    # A/B to prove the mechanism: `off` should always be clean; `on` reveals
    # the bug on an unpatched build and stays clean on a patched one.
    let results = [
        (run-trial "off")
        (run-trial "on")
    ]
    print ($results | table)

    # TODO: decide the assertion you actually want this harness to enforce:
    #   - Regression guard on a PATCHED build: assert the "on" count <= MAX_OK_COUNT.
    #   - Causation demo on an UNPATCHED build: assert "on" count > "off" count.
    # Then surface pass/fail (exit nonzero / `error make`) so CI can use it.
    #
    #   let on = ($results | where scroll_on_clear == "on" | first | get marker_count)
    #   if $on > $MAX_OK_COUNT { error make {msg: $"FAIL: ($on) prompts in scrollback"} }

    # TODO: restore the user's tmux `scroll-on-clear`/`history-limit` if you
    # changed global options above (or scope them to the session instead of -g).
}
