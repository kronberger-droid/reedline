// Background threads that print while the user keeps editing.
//
// Two kinds of output: messages sent through the `ExternalPrinter` are
// committed above the prompt and scroll into history, while the `LiveRegion`
// holds lines that are repainted in place above the prompt and never reach
// scrollback (a counter here; a job list or progress bar in a shell).
//
// to run:
// cargo run --example external_printer

use {
    reedline::{DefaultPrompt, ExternalPrinter, LiveRegion, Reedline, Signal},
    std::thread,
    std::thread::sleep,
    std::time::Duration,
};

fn main() {
    let printer = ExternalPrinter::default();

    // grab a sender per producer thread; only the engine holds the receiving end
    let p_sender_slow = printer.sender();
    let p_sender_fast = printer.sender();

    // external printer that prints a message every second
    thread::spawn(move || {
        let mut i = 1;
        loop {
            sleep(Duration::from_secs(1));
            assert!(p_sender_slow
                .send(format!("Message {i} delivered.\nWith two lines!"))
                .is_ok());
            i += 1;
        }
    });

    // external printer that prints a bunch of messages after 3 seconds
    thread::spawn(move || {
        sleep(Duration::from_secs(3));
        for _ in 0..10 {
            sleep(Duration::from_millis(1));
            assert!(p_sender_fast.send("Fast Hello !".to_string()).is_ok());
        }
    });

    // live region: a counter and a spinner, replaced in place four times a
    // second. Every `set` between two polls collapses into one repaint.
    let region = LiveRegion::default();
    let region_writer = region.clone();
    thread::spawn(move || {
        let spinner = ['|', '/', '-', '\\'];
        let mut tick = 0usize;
        loop {
            sleep(Duration::from_millis(250));
            tick += 1;
            region_writer.set(vec![
                format!("{} live region: tick {tick}", spinner[tick % spinner.len()]),
                "  (updated in place, not in scrollback)".to_string(),
            ]);
        }
    });

    let mut line_editor = Reedline::create()
        .with_external_printer(printer)
        .with_live_region(region);
    let prompt = DefaultPrompt::default();

    loop {
        if let Ok(sig) = line_editor.read_line(&prompt) {
            match sig {
                Signal::Success(buffer) => {
                    println!("We processed: {buffer}");
                }
                Signal::CtrlD | Signal::CtrlC => {
                    println!("\nAborted!");
                    break;
                }
                _ => {}
            }
            continue;
        }
        break;
    }
}
