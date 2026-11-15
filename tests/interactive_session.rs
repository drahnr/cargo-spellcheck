#![cfg(all(
    target_os = "linux",
    any(feature = "hunspell", feature = "spellbook", feature = "zet")
))]

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::pty::{openpty, Winsize};
use nix::sys::termios::{tcgetattr, LocalFlags};
use nix::unistd::setsid;

const DEADLINE: Duration = Duration::from_secs(30);
const PROMPT: &str = "Apply this suggestion [y,n,q,a=accept word,d,j,e,?]?";
const UNKNOWN: &str = "Zzyzxqvocab";
const MISTAKES: &str = "Zzyzxqvocab Spelchekr Thirdwrng";

// openpty cannot atomically set CLOEXEC. Serialize only descriptor setup and
// spawn, so another session cannot inherit a not-yet-marked PTY descriptor.
static SPAWN_LOCK: Mutex<()> = Mutex::new(());

struct Backend {
    section: &'static str,
    checker: &'static str,
}

// Prefer Hunspell for the original tests when available; Rust-only builds use
// the first compiled Rust backend instead.
const BACKENDS: &[Backend] = &[
    #[cfg(feature = "hunspell")]
    Backend {
        section: "Hunspell",
        checker: "hunspell",
    },
    #[cfg(feature = "spellbook")]
    Backend {
        section: "Spellbook",
        checker: "spellbook",
    },
    #[cfg(feature = "zet")]
    Backend {
        section: "ZSpell",
        checker: "zet",
    },
];

struct Fixture {
    root: PathBuf,
    config: String,
    checkers: String,
}

impl Fixture {
    fn new(persistent: bool, sources: &[(&str, &str)]) -> Self {
        Self::with_backends(persistent, sources, &BACKENDS[..1])
    }

    fn with_backends(persistent: bool, sources: &[(&str, &str)], backends: &[Backend]) -> Self {
        assert!(!backends.is_empty());
        // Keep sources, the child's CWD, and correct_file's temporary files on
        // the same filesystem, even when /tmp and the checkout are not.
        let fixture = Self {
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!("interactive-session-{}", uuid::Uuid::new_v4())),
            config: backends
                .iter()
                .enumerate()
                .map(|(index, backend)| {
                    // Only the last backend names the target. In mixed builds,
                    // this proves it need not be set under Hunspell (or every backend).
                    let learning = if persistent && index == backends.len() - 1 {
                        "learning_dictionary = \"lingo.dic\"\n"
                    } else {
                        ""
                    };
                    format!(
                        "[{}]\nlang = \"en_US\"\nsearch_dirs = [\".\"]\nextra_dictionaries = [\"lingo.dic\"]\n{}skip_os_lookups = true\nuse_builtin = true\n",
                        backend.section, learning
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            checkers: backends
                .iter()
                .map(|backend| backend.checker)
                .collect::<Vec<_>>()
                .join(","),
        };
        fs::create_dir_all(&fixture.root).unwrap();
        fs::write(fixture.root.join("lingo.dic"), "0\n").unwrap();
        fs::write(fixture.root.join("spellcheck.toml"), &fixture.config).unwrap();
        for (name, source) in sources {
            // Avoid the existing doc-chunks trailing-newline extraction bug.
            assert!(!source.ends_with('\n'));
            fs::write(fixture.root.join(name), source).unwrap();
        }
        fixture
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).unwrap()
    }

    fn assert_config_unchanged(&self) {
        assert_eq!(self.read("spellcheck.toml"), self.config);
    }

    fn cli(&self, action: &str, paths: &[&str]) -> Session {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-spellcheck"));
        command
            .current_dir(&self.root)
            .env("TERM", "xterm")
            .env_remove("RUST_LOG")
            .args(["--cfg", "spellcheck.toml", action])
            .arg(format!("--checkers={}", self.checkers))
            .args(["--jobs=1", "--code=17"])
            .args(paths);
        Session::spawn(command)
    }

    fn check(&self, paths: &[&str], expected_code: i32) -> String {
        let mut session = self.cli("check", paths);
        session.finish(expected_code);
        assert!(!session.output().contains(PROMPT));
        session.output()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Session {
    child: Child,
    master: File,
    transcript: Vec<u8>,
    consumed: usize,
    eof: bool,
}

impl Session {
    fn spawn(mut command: Command) -> Self {
        let _spawn_guard = SPAWN_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let pty = openpty(
            Some(&Winsize {
                ws_row: 40,
                ws_col: 160,
                ws_xpixel: 0,
                ws_ypixel: 0,
            }),
            None,
        )
        .unwrap();
        // openpty transfers ownership of both descriptors to us.
        let master = unsafe { File::from_raw_fd(pty.master) };
        let slave = unsafe { File::from_raw_fd(pty.slave) };
        for descriptor in [master.as_raw_fd(), slave.as_raw_fd()] {
            fcntl(
                descriptor,
                FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
            )
            .unwrap();
        }
        fcntl(master.as_raw_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
        command
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        // Only async-signal-safe syscalls run between fork and exec. In
        // particular, don't use CommandExt::pre_exec to allocate or log.
        unsafe {
            command.pre_exec(|| {
                setsid().map_err(|error| io::Error::from_raw_os_error(error as i32))?;
                if nix::libc::ioctl(nix::libc::STDIN_FILENO, nix::libc::TIOCSCTTY, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        // Drop the parent's Stdio handles, so closing the child slave yields EOF.
        drop(command);
        Self {
            child,
            master,
            transcript: Vec::new(),
            consumed: 0,
            eof: false,
        }
    }

    fn output(&self) -> String {
        console::strip_ansi_codes(&String::from_utf8_lossy(&self.transcript)).into_owned()
    }

    fn read_available(&mut self) {
        if self.eof {
            return;
        }
        let mut bytes = [0; 8192];
        match self.master.read(&mut bytes) {
            Ok(0) => self.eof = true,
            Ok(count) => self.transcript.extend_from_slice(&bytes[..count]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            // Linux PTY masters report EIO once all slave handles are closed.
            Err(error) if error.raw_os_error() == Some(nix::libc::EIO) => self.eof = true,
            Err(error) => panic!("PTY read failed: {}\n{}", error, self.output()),
        }
        assert!(
            self.transcript.len() <= 2 * 1024 * 1024,
            "CLI produced excessive output"
        );
    }

    fn expect(&mut self, needle: &str) -> String {
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.read_available();
            if let Some(offset) = self.transcript[self.consumed..]
                .windows(needle.len())
                .position(|window| window == needle.as_bytes())
            {
                let end = self.consumed + offset + needle.len();
                let found = console::strip_ansi_codes(&String::from_utf8_lossy(
                    &self.transcript[self.consumed..end],
                ))
                .into_owned();
                self.consumed = end;
                return found;
            }
            assert!(
                Instant::now() < deadline && !self.eof,
                "CLI never produced {:?}\n{}",
                needle,
                self.output()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn prompt(&mut self, position: &str, file: &str) {
        let context = self.expect(PROMPT);
        assert!(
            context.contains(&format!("({}) {}", position, PROMPT)),
            "{}",
            context
        );
        assert!(context.contains(file), "{}", context);
        // The prompt is flushed by the first guard's cleanup, before the
        // event-reading guard exists. Its replacement list's final RestorePosition
        // is emitted under the latter guard, which stays raw until input arrives.
        self.expect("\x1b8");
    }

    fn send(&mut self, input: &[u8]) {
        // Callers first await the current redraw's completion. Sampling termios
        // alone could catch a temporary guard that is about to restore canonical
        // mode, turning Ctrl-C into a signal instead of a key event.
        let deadline = Instant::now() + DEADLINE;
        loop {
            let attributes = tcgetattr(self.master.as_raw_fd()).unwrap();
            if !attributes
                .local_flags
                .intersects(LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::ISIG)
            {
                break;
            }
            assert!(
                Instant::now() < deadline && !self.eof,
                "CLI did not enter raw mode\n{}",
                self.output()
            );
            self.read_available();
            thread::sleep(Duration::from_millis(10));
        }
        // The master is nonblocking; these tiny key sequences cannot hang even
        // if the child stops reading unexpectedly.
        self.master.write_all(input).unwrap();
    }

    fn expect_custom_cursor(&mut self, offset: usize) {
        // Unlike Show alone (also emitted by guard cleanup), this full sequence
        // is unique to the custom field's final cursor positioning before read().
        self.expect(&format!("\x1b7\x1b[?25h\x1b[0F\x1b[{}G", 5 + offset));
    }

    fn replace_custom(&mut self, replacement: &str) {
        assert!(replacement.is_ascii());
        self.send(b"e");
        self.expect_custom_cursor(0);
        // Custom input drops/reacquires raw mode for every event. Acknowledge
        // each character's redraw instead of queuing text across those transitions.
        for (offset, byte) in replacement.bytes().enumerate() {
            self.send(&[byte]);
            self.expect_custom_cursor(offset + 1);
        }
        self.send(b"\r");
    }

    fn finish(&mut self, expected_code: i32) {
        let deadline = Instant::now() + DEADLINE;
        let status: ExitStatus = loop {
            self.read_available();
            if let Some(status) = self.child.try_wait().unwrap() {
                if self.eof {
                    break status;
                }
            }
            assert!(
                Instant::now() < deadline,
                "CLI did not exit\n{}",
                self.output()
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(expected_code), "{}", self.output());
    }

    fn assert_prompt_count(&self, expected: usize) {
        assert_eq!(
            self.output().matches(PROMPT).count(),
            expected,
            "{}",
            self.output()
        );
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Also runs on assertion failures/timeouts; never leave a CLI waiting
        // for terminal input or an unreaped child behind. Child caches reaped
        // status, so kill() cannot signal a recycled PID after finish().
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn exercise_acceptance(backends: &[Backend], persistent: bool) {
    let repeated = "Zzyzxqvocab Zzyzxqvocab";
    let fixture = Fixture::with_backends(
        persistent,
        &[("first.md", repeated), ("second.md", UNKNOWN)],
        backends,
    );
    let paths = ["first.md", "second.md"];
    assert!(fixture
        .check(&paths, 17)
        .contains("Possible spelling mistake found"));

    let mut session = fixture.cli("fix", &paths);
    session.prompt("1/2", "first.md");
    session.send(b"a");
    // Both the already-queued duplicate and the later document must be skipped
    // without another keystroke; a regression fails on this finite deadline.
    session.finish(0);
    session.assert_prompt_count(1);
    assert_eq!(fixture.read("first.md"), repeated);
    assert_eq!(fixture.read("second.md"), UNKNOWN);
    assert_eq!(
        fixture.read("lingo.dic"),
        if persistent {
            "1\nZzyzxqvocab\n"
        } else {
            "0\n"
        }
    );
    fixture.assert_config_unchanged();
    let output = fixture.check(&paths, if persistent { 0 } else { 17 });
    assert_eq!(
        output.contains("Possible spelling mistake found"),
        !persistent,
        "{}",
        output
    );
}

#[test]
fn acceptance_suppresses_duplicates_saves_and_reloads_on_next_check() {
    for backend in BACKENDS {
        exercise_acceptance(std::slice::from_ref(backend), true);
    }
}

#[cfg(any(
    all(feature = "hunspell", feature = "spellbook"),
    all(feature = "hunspell", feature = "zet"),
    all(feature = "spellbook", feature = "zet")
))]
#[test]
fn mixed_backends_share_acceptance_and_a_single_learning_target() {
    // Session-only acceptance must also suppress queued and later diagnostics
    // from every spelling backend, without relying on a dictionary reload.
    for persistent in [false, true] {
        exercise_acceptance(BACKENDS, persistent);
    }
}

#[test]
fn quit_commits_accepted_words_and_selected_edits_but_stops_remaining_work() {
    let fixture = Fixture::new(
        true,
        &[("first.md", MISTAKES), ("remaining.md", "Thirdwrng")],
    );
    let mut session = fixture.cli("fix", &["first.md", "remaining.md"]);
    session.prompt("1/3", "first.md");
    session.send(b"a");
    session.prompt("2/3", "first.md");
    session.replace_custom("spellchecker");
    session.prompt("3/3", "first.md");
    // Neither kind of choice may be persisted before the session is committed.
    assert_eq!(fixture.read("first.md"), MISTAKES);
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    session.send(b"q");
    session.finish(17);
    session.assert_prompt_count(3);
    assert!(!session.output().contains("remaining.md"));

    let edited = fixture.read("first.md");
    assert!(edited.starts_with("Zzyzxqvocab "), "{}", edited);
    assert!(edited.ends_with(" Thirdwrng"), "{}", edited);
    assert!(!edited.contains("Spelchekr"), "{}", edited);
    assert_eq!(fixture.read("remaining.md"), "Thirdwrng");
    assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqvocab\n");
    fixture.assert_config_unchanged();
}

#[test]
fn ctrl_c_discards_both_accepted_words_and_pending_source_edits() {
    let fixture = Fixture::new(
        true,
        &[("first.md", MISTAKES), ("remaining.md", "Thirdwrng")],
    );
    let mut session = fixture.cli("fix", &["first.md", "remaining.md"]);
    session.prompt("1/3", "first.md");
    session.send(b"a");
    session.prompt("2/3", "first.md");
    session.replace_custom("spellchecker");
    session.prompt("3/3", "first.md");
    session.send(b"\x03");
    session.finish(130);
    session.assert_prompt_count(3);
    assert!(!session.output().contains("remaining.md"));
    assert_eq!(fixture.read("first.md"), MISTAKES);
    assert_eq!(fixture.read("remaining.md"), "Thirdwrng");
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    fixture.assert_config_unchanged();
    assert!(fixture
        .check(&["first.md"], 17)
        .contains("Possible spelling mistake found"));
}

#[test]
fn acceptance_without_learning_target_is_session_only() {
    let repeated = "Zzyzxqvocab Zzyzxqvocab";
    let fixture = Fixture::new(false, &[("first.md", repeated)]);
    let mut session = fixture.cli("fix", &["first.md"]);
    session.prompt("1/2", "first.md");
    session.send(b"a");
    session.finish(0);
    session.assert_prompt_count(1);
    assert_eq!(fixture.read("first.md"), repeated);
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    fixture.assert_config_unchanged();
    assert!(fixture
        .check(&["first.md"], 17)
        .contains("Possible spelling mistake found"));

    // Prove a fresh interactive process asks again, rather than merely trusting
    // that a nonzero check result came from some unrelated diagnostic.
    let mut next = fixture.cli("fix", &["first.md"]);
    next.prompt("1/2", "first.md");
    next.send(b"q");
    next.finish(0);
    assert_eq!(fixture.read("lingo.dic"), "0\n");
}

#[test]
fn custom_replacement_edits_source_without_learning_either_word() {
    let fixture = Fixture::new(true, &[("first.md", "Spelchekr")]);
    let mut session = fixture.cli("fix", &["first.md"]);
    session.prompt("1/1", "first.md");
    session.replace_custom(UNKNOWN);
    session.finish(17);
    session.assert_prompt_count(1);
    assert_eq!(fixture.read("first.md"), UNKNOWN);
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    fixture.assert_config_unchanged();
    let output = fixture.check(&["first.md"], 17);
    assert!(output.contains(UNKNOWN), "{}", output);
    assert!(
        output.contains("Possible spelling mistake found"),
        "{}",
        output
    );
}
