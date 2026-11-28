//! Wait for terminal input without spinning on a terminal that went away
//!
//! When the far end of a pty closes, the tty stays readable for good while
//! `read` returns 0 or EIO. crossterm's unix event source retries that read
//! without end (crossterm-rs/crossterm#793), so a `crossterm::event::read`
//! already blocked when the terminal disappears never returns and pins a core.
//! A process that ignores SIGHUP, or never receives it, then runs forever.
//!
//! On unix reedline therefore does the waiting itself. It polls the tty next to
//! a SIGWINCH self-pipe and calls into crossterm only once poll says the tty is
//! alive, and then only for a short look. The self-pipe wakes the wait on a
//! resize, which crossterm learns about through its own signal pipe and reports
//! on the look that follows.
//!
//! A hangup fails the read with `UnexpectedEof` only if two things agree: poll
//! reports `POLLHUP`/`POLLERR`, and the fd no longer answers as a terminal. A
//! host is likely to exit on that error, so poll alone is not trusted with it.
//! Where the two disagree the watch retires and crossterm waits as before.
//!
//! What this cannot close: a hangup that lands while crossterm is having its
//! look. With `use-dev-tty` that costs the rest of the look. crossterm's default
//! source never leaves its read loop, so there the spin is still possible, in
//! a window of a millisecond per wakeup.
//!
//! Elsewhere, and on unix when the tty cannot be polled (macOS `poll` rejects
//! some devices with `POLLNVAL`), these are crossterm's own functions.

use std::{io, time::Duration};

use crossterm::event::{self, Event};

/// [`crossterm::event::poll`], but fails instead of spinning on a dead tty
pub(crate) fn poll(timeout: Duration) -> io::Result<bool> {
    #[cfg(unix)]
    if let Some(watch) = unix::watch() {
        return watch.poll(timeout);
    }
    event::poll(timeout)
}

/// [`crossterm::event::read`], but fails instead of spinning on a dead tty
pub(crate) fn read() -> io::Result<Event> {
    #[cfg(unix)]
    if let Some(watch) = unix::watch() {
        while !watch.poll(Duration::MAX)? {}
    }
    event::read()
}

/// Fail if the tty is already gone, before anything asks it for the cursor
/// position: `crossterm::cursor::position` spins on a dead tty as well
pub(crate) fn ensure_tty_alive() -> io::Result<()> {
    #[cfg(unix)]
    if let Some(watch) = unix::watch() {
        watch.wait(Some(Duration::ZERO))?;
    }
    Ok(())
}

#[cfg(unix)]
mod unix {
    use std::{
        io::{self, Read},
        os::{
            fd::{AsFd, OwnedFd},
            unix::net::UnixStream,
        },
        sync::{
            atomic::{AtomicBool, Ordering},
            OnceLock,
        },
        time::{Duration, Instant},
    };

    use crossterm::event;
    use rustix::event::{PollFd, PollFlags, Timespec};

    /// How many wakeups in a row may find the tty readable and crossterm
    /// without an event, before crossterm does the waiting itself
    ///
    /// Each look should drain the tty, so repeats mean crossterm reads some
    /// other fd than the one watched here, and waiting on this one would spin.
    pub(super) const MAX_IDLE_WAKEUPS: u32 = 3;

    /// How long crossterm gets to look for an event once the wait has woken
    ///
    /// `event::poll(Duration::ZERO)` skips what crossterm's `use-dev-tty`
    /// source, which nushell enables, has already read and parsed but not yet
    /// handed out, and it does not read the tty either. Any nonzero timeout
    /// gets past that, and an event that is ready returns at once.
    pub(super) const CROSSTERM_LOOK: Duration = Duration::from_millis(1);

    /// What poll's answer for the tty says about the terminal
    #[derive(Debug, PartialEq, Eq)]
    pub(super) enum Line {
        Alive,
        /// Poll reports a hangup and the fd is no terminal any more
        Gone,
        /// Poll reports a hangup on an fd that still answers as a terminal
        Doubtful,
    }

    /// Judge `revents`, asking `still_a_tty` only about a reported hangup
    pub(super) fn judge(revents: PollFlags, still_a_tty: impl FnOnce() -> bool) -> Line {
        if !revents.intersects(PollFlags::HUP | PollFlags::ERR) {
            Line::Alive
        } else if still_a_tty() {
            Line::Doubtful
        } else {
            Line::Gone
        }
    }

    pub(super) struct Watch {
        tty: OwnedFd,
        winch: UnixStream,
        /// Cleared for good once poll called a hangup the tty does not confirm
        trusted: AtomicBool,
    }

    static WATCH: OnceLock<Option<Watch>> = OnceLock::new();

    /// The watch, set up on first use while the tty is still alive, and gone
    /// again once it has retired
    pub(super) fn watch() -> Option<&'static Watch> {
        WATCH
            .get_or_init(Watch::new)
            .as_ref()
            .filter(|watch| watch.trusted())
    }

    impl Watch {
        fn new() -> Option<Self> {
            // Stdin if it is a terminal, else /dev/tty, the same choice crossterm
            // makes. Decided once: `isatty` fails on a hung up tty.
            let stdin = io::stdin();
            let tty: OwnedFd = if rustix::termios::isatty(&stdin) {
                stdin.as_fd().try_clone_to_owned().ok()?
            } else {
                std::fs::File::open("/dev/tty").ok()?.into()
            };

            // signal-hook runs SIGWINCH actions in the order they were added.
            // With crossterm's first, its pipe holds the resize by the time this
            // one wakes the wait, so the poll that follows cannot miss it.
            // Starting crossterm's reader adds its action; skip that on a tty
            // already gone, where crossterm would spin.
            if probe(&tty).is_some_and(|tty| !tty.intersects(PollFlags::HUP | PollFlags::ERR)) {
                let _ = event::poll(Duration::ZERO);
            }

            Self::from_tty(tty)
        }

        /// Watch `tty`, unless poll cannot
        pub(super) fn from_tty(tty: OwnedFd) -> Option<Self> {
            // A tty poll cannot watch would wake the wait at once, every time
            if probe(&tty)?.contains(PollFlags::NVAL) {
                return None;
            }

            let (winch, notify) = UnixStream::pair().ok()?;
            winch.set_nonblocking(true).ok()?;
            signal_hook::low_level::pipe::register(signal_hook::consts::SIGWINCH, notify).ok()?;

            Some(Self {
                tty,
                winch,
                trusted: AtomicBool::new(true),
            })
        }

        /// Whether poll's word on this tty still counts
        pub(super) fn trusted(&self) -> bool {
            self.trusted.load(Ordering::Relaxed)
        }

        pub(super) fn poll(&self, timeout: Duration) -> io::Result<bool> {
            self.poll_with(timeout, event::poll)
        }

        /// [`Self::poll`], with `look` standing in for `crossterm::event::poll`
        pub(super) fn poll_with(
            &self,
            timeout: Duration,
            mut look: impl FnMut(Duration) -> io::Result<bool>,
        ) -> io::Result<bool> {
            let deadline = Instant::now().checked_add(timeout);
            // What is left of the timeout: `None` once it has run out,
            // `Some(None)` for one that never does
            let left = || match deadline {
                Some(deadline) => deadline
                    .checked_duration_since(Instant::now())
                    .filter(|left| !left.is_zero())
                    .map(Some),
                None => Some(None),
            };

            // Probe first, so crossterm is never entered on a dead tty
            self.wait(Some(Duration::ZERO))?;
            let mut idle_wakeups = 0;
            loop {
                if !self.trusted() {
                    // Retired: this poll, and every one after it, is crossterm's
                    let rest = left().map_or(Duration::ZERO, |left| left.unwrap_or(Duration::MAX));
                    return look(rest);
                }
                if look(CROSSTERM_LOOK)? {
                    return Ok(true);
                }
                let Some(remaining) = left() else {
                    return Ok(false);
                };
                if self.wait(remaining)? {
                    idle_wakeups += 1;
                    if idle_wakeups >= MAX_IDLE_WAKEUPS {
                        return look(remaining.unwrap_or(Duration::MAX));
                    }
                } else {
                    idle_wakeups = 0;
                }
            }
        }

        /// Wait for input or a resize, `None` meaning no timeout. Returns whether
        /// the tty is readable, and fails once the tty has hung up.
        pub(super) fn wait(&self, timeout: Option<Duration>) -> io::Result<bool> {
            let timeout = timeout.and_then(|t| Timespec::try_from(t).ok());
            let mut fds = [
                PollFd::new(&self.tty, PollFlags::IN),
                PollFd::new(&self.winch, PollFlags::IN),
            ];
            match rustix::event::poll(&mut fds, timeout.as_ref()) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => return Ok(false),
                Err(err) => return Err(err.into()),
            }

            let tty = fds[0].revents();
            match judge(tty, || rustix::termios::isatty(&self.tty)) {
                Line::Alive => {}
                Line::Gone => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the terminal went away",
                    ));
                }
                Line::Doubtful => {
                    // Poll would repeat this on every wait. Better to lose the
                    // guard here than to end a session that is alive.
                    self.trusted.store(false, Ordering::Relaxed);
                    return Ok(false);
                }
            }
            if !fds[1].revents().is_empty() {
                let mut buf = [0u8; 64];
                while matches!((&self.winch).read(&mut buf), Ok(n) if n > 0) {}
            }
            Ok(tty.contains(PollFlags::IN))
        }
    }

    /// What poll reports for `tty` right now, `None` if poll fails
    fn probe(tty: &OwnedFd) -> Option<PollFlags> {
        let mut fds = [PollFd::new(tty, PollFlags::IN)];
        rustix::event::poll(&mut fds, Some(&Timespec::default())).ok()?;
        Some(fds[0].revents())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        io::{ErrorKind, Write},
        os::fd::OwnedFd,
        time::{Duration, Instant},
    };

    use rustix::{
        event::PollFlags,
        fs::{Mode, OFlags},
        pty::{grantpt, openpt, ptsname, unlockpt, OpenptFlags},
    };

    use super::unix::{judge, Line, Watch, CROSSTERM_LOOK, MAX_IDLE_WAKEUPS};

    /// A pty pair: the master, played by the terminal, and the watched slave
    fn pty() -> (std::fs::File, Watch) {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let path = ptsname(&master, Vec::new()).unwrap();
        let slave: OwnedFd = rustix::fs::open(
            path.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )
        .unwrap();
        // Raw, as under reedline: in canonical mode a key is not readable
        // until a newline completes the line
        let mut termios = rustix::termios::tcgetattr(&slave).unwrap();
        termios.make_raw();
        rustix::termios::tcsetattr(&slave, rustix::termios::OptionalActions::Now, &termios)
            .unwrap();
        let watch = Watch::from_tty(slave).expect("poll can watch a pty");
        (master.into(), watch)
    }

    #[test]
    fn live_idle_tty_is_neither_readable_nor_an_error() {
        let (_master, watch) = pty();
        assert!(!watch.wait(Some(Duration::ZERO)).unwrap());
    }

    /// Wait until `watch` reports the tty readable or fails, sitting out
    /// wakeups from SIGWINCH, which tests running in parallel raise
    fn wait_past_resizes(watch: &Watch) -> std::io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !watch.wait(Some(Duration::from_secs(5)))? {
            assert!(Instant::now() < deadline, "the wait never woke");
        }
        Ok(())
    }

    #[test]
    fn typed_input_makes_the_tty_readable() {
        let (mut master, watch) = pty();
        master.write_all(b"x").unwrap();
        wait_past_resizes(&watch).unwrap();
    }

    #[test]
    fn closed_terminal_is_unexpected_eof() {
        let (master, watch) = pty();
        drop(master);
        let err = watch.wait(Some(Duration::ZERO)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
        assert!(watch.trusted(), "a confirmed hangup is no reason to retire");
    }

    #[test]
    fn closing_the_terminal_wakes_a_blocked_wait() {
        let (master, watch) = pty();
        let closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(master);
        });
        let err = wait_past_resizes(&watch).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
        closer.join().unwrap();
    }

    #[test]
    fn resize_wakes_a_blocked_wait() {
        let (_master, watch) = pty();
        let start = Instant::now();
        let raiser = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            signal_hook::low_level::raise(signal_hook::consts::SIGWINCH).unwrap();
        });
        assert!(!watch.wait(Some(Duration::from_secs(5))).unwrap());
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "woke on the timeout"
        );
        raiser.join().unwrap();
    }

    #[test]
    fn a_hangup_counts_only_once_the_fd_is_no_terminal() {
        let hangups = [
            PollFlags::HUP,
            PollFlags::ERR,
            PollFlags::IN | PollFlags::HUP,
        ];
        for revents in hangups {
            assert_eq!(judge(revents, || false), Line::Gone, "{revents:?}");
            assert_eq!(judge(revents, || true), Line::Doubtful, "{revents:?}");
        }
        for revents in [PollFlags::empty(), PollFlags::IN] {
            let alive = judge(revents, || panic!("asked about a tty poll calls alive"));
            assert_eq!(alive, Line::Alive, "{revents:?}");
        }
    }

    #[test]
    fn a_dead_tty_fails_the_poll_before_crossterm_is_asked() {
        let (master, watch) = pty();
        drop(master);
        let err = watch
            .poll_with(Duration::from_secs(5), |_| {
                panic!("crossterm was asked about a dead tty")
            })
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_ready_event_ends_the_poll_on_the_first_look() {
        let (_master, watch) = pty();
        let mut looks = Vec::new();
        let ready = watch.poll_with(Duration::from_secs(5), |timeout| {
            looks.push(timeout);
            Ok(true)
        });
        assert!(ready.unwrap());
        assert_eq!(looks, [CROSSTERM_LOOK]);
    }

    #[test]
    fn an_idle_poll_runs_out_its_timeout() {
        let (_master, watch) = pty();
        let timeout = Duration::from_millis(50);
        let start = Instant::now();
        let mut looks = Vec::new();
        let ready = watch.poll_with(timeout, |look| {
            looks.push(look);
            Ok(false)
        });
        assert!(!ready.unwrap());
        assert!(start.elapsed() >= timeout, "returned early");
        assert!(start.elapsed() < Duration::from_secs(2), "overslept");
        assert!(
            looks.iter().all(|look| *look == CROSSTERM_LOOK),
            "an idle tty is never handed over: {looks:?}"
        );
    }

    #[test]
    fn input_crossterm_never_takes_is_handed_over() {
        let (mut master, watch) = pty();
        // Nothing reads this byte, so every wait finds the tty readable
        master.write_all(b"x").unwrap();
        let timeout = Duration::from_secs(30);
        let start = Instant::now();
        let mut looks = Vec::new();
        let ready = watch.poll_with(timeout, |look| {
            looks.push(look);
            Ok(false)
        });
        assert!(!ready.unwrap());
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "spun on the timeout"
        );

        let (handed_over, short_looks) = looks.split_last().unwrap();
        assert!(
            *handed_over > Duration::from_secs(20),
            "crossterm gets the rest of the timeout: {handed_over:?}"
        );
        assert!(short_looks.len() >= MAX_IDLE_WAKEUPS as usize);
        assert!(short_looks.iter().all(|look| *look == CROSSTERM_LOOK));
    }
}
