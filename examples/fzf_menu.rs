// An fzf-style picker built as a *menu* rather than as an interactive completer.
// cargo run --example fzf_menu
//
// [Ctrl-T] opens the picker. Keep typing to refilter, [Up]/[Down] to move,
// [Enter] to accept — the selection replaces the whole line, not just the token
// under the cursor.
//
// The matching is done by an external process: `fzf --filter <query>` when fzf
// is on PATH, otherwise a built-in subsequence match. That process runs on a
// worker thread and only ever answers "which candidates match?" — it never
// draws to the terminal and never reads keys. Reedline keeps the terminal for
// the whole interaction and paints the results itself.
//
// Three pieces make this work, and they are the reason a heavy external matcher
// does not have to block the line editor:
//
//   * `ReedlineMenu::WithCompleter` gives the menu its own completer, separate
//     from the engine's Tab completer (still bound to [Tab] here).
//   * `InputMode::FullBuffer` / `OutputMode::FullBuffer` hand the whole line to
//     the matcher and replace the whole line on selection.
//   * `CompletionResult::Stale`/`Pending` plus `poll_completion` keep typing
//     responsive while a slow match is still running: the previous results stay
//     on screen, and the menu refreshes by itself once the new ones land.

use reedline::{
    default_emacs_keybindings, ColumnarMenu, Completer, CompletionResult, CompletionStatus,
    DefaultCompleter, DefaultPrompt, Emacs, InputMode, KeyCode, KeyModifiers, Keybindings,
    MenuBuilder, OutputMode, Reedline, ReedlineEvent, ReedlineMenu, Signal, Span, Suggestion,
    Suggestions,
};
use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;

const MENU_NAME: &str = "fzf_menu";

/// Rank `candidates` against `query` with fzf's own matcher, without fzf taking
/// over the terminal. `--filter` makes it a plain stdin-to-stdout filter.
///
/// `None` means fzf could not be used at all (not installed, spawn failed), as
/// opposed to `Some(vec![])`, which is fzf saying nothing matched.
fn fzf_filter(query: &str, candidates: &[String]) -> Option<Vec<String>> {
    let mut child = Command::new("fzf")
        .arg("--filter")
        .arg(query)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdin = child.stdin.take()?;
    for candidate in candidates {
        // Ignore write errors: fzf may close stdin before we are done.
        let _ = writeln!(stdin, "{candidate}");
    }
    drop(stdin);

    // fzf exits 1 when nothing matched, which is a result, not a failure.
    let output = child.wait_with_output().ok()?;
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect(),
    )
}

/// Fallback matcher for when fzf is not installed: case-insensitive
/// subsequence match, so "gcm" still finds "git commit".
fn subsequence_filter(query: &str, candidates: &[String]) -> Vec<String> {
    let needle: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    candidates
        .iter()
        .filter(|candidate| {
            let mut chars = candidate
                .to_lowercase()
                .chars()
                .collect::<Vec<_>>()
                .into_iter();
            needle.iter().all(|wanted| chars.any(|c| c == *wanted))
        })
        .cloned()
        .collect()
}

/// Run the matcher off the main thread so a slow match never blocks typing.
///
/// Queries are coalesced: if the user typed three more characters while a match
/// was running, the next round skips straight to the newest query.
fn spawn_matcher(
    candidates: Arc<Vec<String>>,
) -> (Sender<String>, Receiver<(String, Vec<String>)>) {
    let (query_tx, query_rx) = mpsc::channel::<String>();
    let (result_tx, result_rx) = mpsc::channel::<(String, Vec<String>)>();

    thread::spawn(move || {
        while let Ok(mut query) = query_rx.recv() {
            while let Ok(newer) = query_rx.try_recv() {
                query = newer;
            }

            let matches = fzf_filter(&query, &candidates)
                .unwrap_or_else(|| subsequence_filter(&query, &candidates));

            if result_tx.send((query, matches)).is_err() {
                break;
            }
        }
    });

    (query_tx, result_rx)
}

/// A completer that hands the query to an external matcher and reports back
/// through [`CompletionStatus`] instead of blocking on it.
struct FuzzyPicker {
    queries: Sender<String>,
    results: Receiver<(String, Vec<String>)>,
    /// Query the cached `matches` belong to; `None` before the first result.
    shown: Option<String>,
    matches: Suggestions,
    /// Newest query handed to the matcher that has not been answered yet.
    in_flight: Option<String>,
}

impl FuzzyPicker {
    fn new(candidates: Vec<String>) -> Self {
        let (queries, results) = spawn_matcher(Arc::new(candidates));
        Self {
            queries,
            results,
            shown: None,
            matches: Suggestions::from(vec![]),
            in_flight: None,
        }
    }
}

/// Every match replaces the entire line, so each suggestion spans the whole
/// query. `OutputMode::FullBuffer` makes the replacement range explicit too,
/// which keeps things right even for a result that arrives a keystroke late.
fn to_suggestions(query: &str, matches: Vec<String>) -> Suggestions {
    matches
        .into_iter()
        .map(|value| Suggestion {
            value,
            span: Span::new(0, query.len()),
            ..Suggestion::default()
        })
        .collect::<Vec<_>>()
        .into()
}

impl Completer for FuzzyPicker {
    fn complete(&mut self, line: &str, _pos: usize) -> CompletionResult {
        // `InputMode::FullBuffer` means `line` is the whole buffer, so the
        // entire line is the query — exactly what you would have typed at an
        // `fzf` prompt.
        if self.shown.as_deref() == Some(line) {
            return CompletionResult::Fresh(self.matches.clone());
        }

        if self.in_flight.as_deref() != Some(line) {
            if self.queries.send(line.to_string()).is_err() {
                // Matcher thread is gone; keep showing what we have.
                return CompletionResult::Fresh(self.matches.clone());
            }
            self.in_flight = Some(line.to_string());
        }

        // Keep the previous matches on screen while the new ones compute,
        // instead of blanking the menu on every keystroke.
        CompletionResult::stale_or_pending(self.matches.clone())
    }

    fn poll_completion(&mut self) -> CompletionStatus {
        let mut landed = false;

        loop {
            match self.results.try_recv() {
                // Answers to queries the user has already typed past are
                // dropped; the matcher is already working on the newest one.
                Ok((query, matches)) if self.in_flight.as_deref() == Some(query.as_str()) => {
                    self.matches = to_suggestions(&query, matches);
                    self.shown = Some(query);
                    self.in_flight = None;
                    landed = true;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.in_flight = None;
                    break;
                }
            }
        }

        if landed {
            CompletionStatus::Ready
        } else if self.in_flight.is_some() {
            CompletionStatus::Pending
        } else {
            CompletionStatus::Idle
        }
    }
}

fn add_keybindings(keybindings: &mut Keybindings) {
    // Ctrl-T, as in fzf's own widget. Overrides the emacs SwapGraphemes binding.
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('t'),
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu(MENU_NAME.to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Up,
        ReedlineEvent::UntilFound(vec![ReedlineEvent::MenuUp, ReedlineEvent::Up]),
    );
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Down,
        ReedlineEvent::UntilFound(vec![ReedlineEvent::MenuDown, ReedlineEvent::Down]),
    );
}

fn candidates() -> Vec<String> {
    [
        "git status --short --branch",
        "git commit --amend --no-edit",
        "git rebase --interactive origin/main",
        "git log --oneline --graph --decorate",
        "git switch --create feature/fzf-menu",
        "cargo run --example fzf_menu",
        "cargo test --all-features",
        "cargo clippy --all-targets -- -D warnings",
        "cargo build --release",
        "cargo doc --open",
        "ls --color=auto -lah",
        "rg --hidden --glob '!.git' TODO",
        "fd --type file --extension rs",
        "tar czf archive.tar.gz ./src",
        "curl -sSL https://example.com/api | jq '.items[]'",
        "docker compose up --detach",
        "docker image prune --all",
        "kubectl get pods --all-namespaces",
        "ssh user@example.com -p 2222",
        "rsync -avz ./dist/ user@example.com:/srv/www/",
        "systemctl --user restart pipewire",
        "journalctl --user -u pipewire --since '1 hour ago'",
        "ffmpeg -i input.mp4 -vf scale=1280:-1 output.mp4",
        "python -m http.server 8000",
        "nu -c 'ls | where size > 1mb'",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn main() -> io::Result<()> {
    let picker = ColumnarMenu::default()
        .with_name(MENU_NAME)
        .with_columns(1)
        .with_column_padding(2)
        .with_marker("fzf> ")
        // The matcher sees the whole line, and a pick replaces the whole line.
        .with_input_mode(InputMode::FullBuffer)
        .with_output_mode(OutputMode::FullBuffer);

    let fzf_menu = ReedlineMenu::WithCompleter {
        menu: Box::new(picker),
        completer: Box::new(FuzzyPicker::new(candidates())),
    };

    // An ordinary Tab completion menu alongside it, to show the picker is not
    // in the way of the engine's own completer.
    let completion_menu = ReedlineMenu::EngineCompleter(Box::new(
        ColumnarMenu::default().with_name("completion_menu"),
    ));

    let mut keybindings = default_emacs_keybindings();
    add_keybindings(&mut keybindings);

    let mut line_editor = Reedline::create()
        .with_completer(Box::new(DefaultCompleter::new_with_wordlen(
            candidates(),
            2,
        )))
        .with_menu(fzf_menu)
        .with_menu(completion_menu)
        // The picker stays open while the line empties out, the way fzf does.
        .with_persistent_menus(true)
        .with_edit_mode(Box::new(Emacs::new(keybindings)));

    let prompt = DefaultPrompt::default();

    println!("[Ctrl-T] fuzzy picker  ·  [Tab] completions  ·  [Ctrl-D] quit");
    if Command::new("fzf")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        println!("(fzf not found on PATH — falling back to the built-in subsequence matcher)");
    }

    loop {
        match line_editor.read_line(&prompt)? {
            Signal::Success(buffer) => println!("We processed: {buffer}"),
            Signal::CtrlD | Signal::CtrlC => {
                println!("\nAborted!");
                break Ok(());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Drive the picker the way the engine does — `complete` on a keystroke,
    /// `poll_completion` once per loop turn — until the matcher answers.
    fn settle(picker: &mut FuzzyPicker, query: &str) -> CompletionResult {
        let first = picker.complete(query, query.len());
        assert!(
            first.is_pending() || matches!(first, CompletionResult::Stale(_)),
            "a query the matcher has not answered yet must not block"
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match picker.poll_completion() {
                CompletionStatus::Ready => return picker.complete(query, query.len()),
                CompletionStatus::Pending => thread::sleep(Duration::from_millis(5)),
                CompletionStatus::Idle => panic!("matcher went idle without answering"),
            }
        }
        panic!("matcher did not answer within the deadline");
    }

    #[test]
    fn subsequence_fallback_matches_initials() {
        let matches = subsequence_filter("gcm", &candidates());
        assert!(matches.iter().any(|m| m == "git commit --amend --no-edit"));
        assert!(!matches.iter().any(|m| m == "cargo build --release"));
    }

    #[test]
    fn results_land_through_poll_completion() {
        let mut picker = FuzzyPicker::new(candidates());

        let settled = settle(&mut picker, "cargo");
        assert!(
            matches!(settled, CompletionResult::Fresh(_)),
            "once the matcher has answered the query, its results are authoritative"
        );
        assert!(
            settled
                .suggestions()
                .iter()
                .all(|s| s.value.contains("carg")),
            "every suggestion should come from the matcher"
        );
        // Whole-line replacement: the span covers the entire query.
        assert!(settled
            .suggestions()
            .iter()
            .all(|s| s.span == Span::new(0, "cargo".len())));

        // A repeat of the same query is answered from cache, with nothing in flight.
        assert!(matches!(
            picker.complete("cargo", 5),
            CompletionResult::Fresh(_)
        ));
        assert_eq!(picker.poll_completion(), CompletionStatus::Idle);
    }

    #[test]
    fn typing_ahead_keeps_the_previous_results_on_screen() {
        let mut picker = FuzzyPicker::new(candidates());
        let settled = settle(&mut picker, "git");
        let shown = settled.suggestions().len();
        assert!(shown > 0, "expected some matches for \"git\"");

        // The next keystroke arrives before the matcher has answered: the menu
        // keeps showing the previous matches instead of blanking.
        match picker.complete("git c", 5) {
            CompletionResult::Stale(values) => assert_eq!(values.len(), shown),
            other => panic!("expected the previous results to be kept, got {other:?}"),
        }
    }
}
