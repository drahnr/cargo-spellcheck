#![cfg(all(
    target_os = "linux",
    any(feature = "hunspell", feature = "spellbook", feature = "zet")
))]

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::style::{Color, Stylize};
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::pty::{openpty, Winsize};
use nix::sys::termios::{tcgetattr, LocalFlags};
use nix::unistd::setsid;

const DEADLINE: Duration = Duration::from_secs(30);
const PROMPT: &str = "Resolve this issue [y,n,q,a,d,j,e,?]?";
const WORD_PROMPT: &str = "Word [?]: ";
const AFFIX_PROMPT: &str = "Affixes [y,a,e,n,?]: ";
const SUFFIX_PROMPT: &str = "Suffix [y,e,n,?]: ";
const STEM_CHOICE_PROMPT: &str = "Stem [y,e,n,?]: ";
const EDIT_STEM_PROMPT: &str = "Stem [?]: ";
const CONFIRM_PROMPT: &str = "Confirm learning [y,e,n,?]: ";
const MAIN_COMMANDS: &[&str] = &[
    "Up", "Down", "Home", "End", "y", "Enter", "n", "q", "a", "d", "j", "e", "Esc", "Ctrl-C", "?",
];
const PICKER_COMMANDS: &[&str] = &[
    "Up", "Down", "Home", "End", "Enter", "y", "e", "n", "Esc", "Ctrl-C", "?",
];
const AFFIX_COMMANDS: &[&str] = &[
    "Up", "Down", "Home", "End", "Left", "Right", "Enter", "y", "a", "e", "n", "Esc", "Ctrl-C", "?",
];
const EDITOR_COMMANDS: &[&str] = &["Enter", "Esc", "Backspace", "Ctrl-U", "Ctrl-C", "?"];
const UNKNOWN: &str = "Zzyzxqvocab";
const MISTAKES: &str = "Zzyzxqvocab Spelchekr Thirdwrng";
const DEFAULT_DICTIONARY: &str = ".config/spellcheck/learnings.dic";
const LEGACY_DICTIONARY: &str = ".config/learnings.dic";

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
    // This switch controls explicit configuration, not whether learning persists:
    // false exercises the automatic default without naming it in extra_dictionaries.
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

    fn with_suffixes(sources: &[(&str, &str)], backends: &[Backend]) -> Self {
        Self::with_affixes(sources, backends, "SET UTF-8\nSFX S Y 1\nSFX S 0 s .\n")
    }

    fn with_affixes(sources: &[(&str, &str)], backends: &[Backend], aff: &str) -> Self {
        let mut fixture = Self::with_backends(true, sources, backends);
        fixture.config = fixture
            .config
            .replace("use_builtin = true", "use_builtin = false");
        fs::write(fixture.root.join("spellcheck.toml"), &fixture.config).unwrap();
        fs::write(fixture.root.join("en_US.aff"), aff).unwrap();
        // Every backend's debug initialization checks this sentinel word.
        fs::write(fixture.root.join("en_US.dic"), "1\nTest\n").unwrap();
        fixture
    }

    fn without_learning_target(mut self) -> Self {
        self.config = self
            .config
            .replace("learning_dictionary = \"lingo.dic\"\n", "");
        fs::write(self.root.join("spellcheck.toml"), &self.config).unwrap();
        self
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).unwrap()
    }

    fn assert_learning_saved(&self, configured: bool, entries: &str) {
        if configured {
            assert_eq!(self.read("lingo.dic"), entries);
            assert!(!self.root.join(".config").exists());
        } else {
            assert_eq!(self.read("lingo.dic"), "0\n");
            assert_eq!(self.read(DEFAULT_DICTIONARY), entries);
            assert!(!self.root.join(LEGACY_DICTIONARY).exists());
        }
    }

    fn assert_config_unchanged(&self) {
        assert_eq!(self.read("spellcheck.toml"), self.config);
    }

    fn cli(&self, action: &str, paths: &[&str]) -> Session {
        self.cli_with_columns(action, paths, 160)
    }

    fn cli_with_columns(&self, action: &str, paths: &[&str], columns: u16) -> Session {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-spellcheck"));
        command
            .current_dir(&self.root)
            .env("TERM", "xterm")
            .env_remove("RUST_LOG")
            .args(["--cfg", "spellcheck.toml", action])
            .arg(format!("--checkers={}", self.checkers))
            .args(["--jobs=1", "--code=17"])
            .args(paths);
        Session::spawn(command, columns)
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
    columns: u16,
}

impl Session {
    fn spawn(mut command: Command, columns: u16) -> Self {
        let _spawn_guard = SPAWN_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let pty = openpty(
            Some(&Winsize {
                ws_row: 40,
                ws_col: columns,
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
            columns,
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
        self.expect_any(&[needle]).1
    }

    fn expect_any(&mut self, needles: &[&str]) -> (usize, String) {
        assert!(!needles.is_empty() && needles.iter().all(|needle| !needle.is_empty()));
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.read_available();
            let found = needles
                .iter()
                .enumerate()
                .filter_map(|(index, needle)| {
                    self.transcript[self.consumed..]
                        .windows(needle.len())
                        .position(|window| window == needle.as_bytes())
                        .map(|offset| (offset, index, needle.len()))
                })
                .min_by_key(|(offset, _, _)| *offset);
            if let Some((offset, index, length)) = found {
                let end = self.consumed + offset + length;
                let found = console::strip_ansi_codes(&String::from_utf8_lossy(
                    &self.transcript[self.consumed..end],
                ))
                .into_owned();
                self.consumed = end;
                return (index, found);
            }
            assert!(
                Instant::now() < deadline && !self.eof,
                "CLI never produced any of {:?}\n{}",
                needles,
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
        // These spelling findings can be learned, so the custom field sits one
        // row above the bottom verbatim entry. Grammar findings keep it at bottom.
        self.expect(&format!("\x1b7\x1b[?25h\x1b[1A\x1b[{}G", 5 + offset));
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

    fn main_picker_redraw(&mut self, word: &str, active_idx: usize) -> usize {
        let start = self.consumed;
        self.expect("\x1b8");
        let raw = String::from_utf8_lossy(&self.transcript[start..self.consumed]);
        assert!(!raw.contains(PROMPT), "Repeated resolution prompt: {raw:?}");
        assert_main_picker_list(&raw, word, "...", active_idx)
    }

    fn custom_picker_redraw(&mut self, word: &str, custom: &str, offset: usize) -> usize {
        let start = self.consumed;
        // Editing can restore the field cursor before the list starts. Await the
        // list's save/restore pair, then the field's final cursor positioning.
        self.expect("\x1b7");
        self.expect("\x1b8");
        let raw = String::from_utf8_lossy(&self.transcript[start..self.consumed]);
        assert!(!raw.contains(PROMPT), "Repeated resolution prompt: {raw:?}");
        assert!(
            !console::strip_ansi_codes(&raw).contains('?'),
            "Help inserted text: {raw:?}"
        );
        let column = assert_main_picker_list(&raw, word, custom, 1);
        self.expect_custom_cursor(offset);
        column
    }

    fn command_help(&mut self, required: &[&str], optional: &[&str]) -> String {
        self.send(b"?");
        let help = self.expect("? - show commands\r\n");
        assert_command_help(&help, required, optional);
        help
    }

    fn confirmation_help(&mut self) {
        let help = self.command_help(&["y", "Enter", "n", "Esc", "e", "Ctrl-C", "?"], &[]);
        for (key, effect) in [
            ("Enter", "record entry"),
            ("y", "record entry"),
            ("n", "decline learning"),
            ("Esc", "decline learning"),
            ("e", "edit stem"),
            ("Ctrl-C", "discard choices and abort"),
        ] {
            assert_command_effect(&help, key, effect);
        }
        self.help_prompt(CONFIRM_PROMPT);
    }

    fn help_prompt(&mut self, prompt: &str) {
        let remount = self.expect(prompt);
        // Help remounts only the current question, not source context, previews,
        // startup warnings, or a different stage of the interaction.
        assert_eq!(
            remount.trim_start_matches(['\r', '\n']),
            console::strip_ansi_codes(prompt),
            "Unexpected output before remounted prompt: {remount:?}"
        );
    }

    fn main_help(&mut self, position: &str, word: &str, active_idx: usize) {
        let help = self.command_help(MAIN_COMMANDS, &[]);
        for (key, effect) in [
            ("Enter", "use selected action"),
            ("y", "use selected action"),
            ("n", "skip finding"),
            ("a", "open learning"),
            ("q", "apply choices and quit"),
            ("Esc", "apply choices and quit"),
            ("Ctrl-C", "discard choices and abort"),
            ("d", "skip remaining file"),
            ("j", "report unavailable navigation"),
            ("e", "edit replacement"),
        ] {
            assert_command_effect(&help, key, effect);
        }
        self.help_prompt(&format!("({position}) {PROMPT}"));
        self.main_picker_redraw(word, active_idx);
    }

    fn assert_no_learning_notices(&self) {
        let output = self.output();
        for notice in [
            "Persistent learning:",
            "Session-only learning",
            "Warning: no learning_dictionary",
        ] {
            assert!(!output.contains(notice), "{output:?}");
        }
    }

    fn assert_startup_learning_warning(&self, missing_target: Option<&Path>) {
        self.assert_no_learning_notices();
        let output = self.output();
        let Some(path) = missing_target else {
            assert!(!output.contains("Warning:"), "{output:?}");
            return;
        };
        assert!(path.is_absolute(), "{}", path.display());
        let warning = format!(
            "Warning: learning dictionary {} does not exist; a new file will be created when learned entries are saved.",
            path.display()
        );
        assert_eq!(
            output.lines().filter(|line| *line == warning).count(),
            1,
            "{output:?}"
        );
        assert_eq!(output.matches("Warning:").count(), 1, "{output:?}");
        let warning_position = output.find(&warning).unwrap();
        let prompt_position = output.find(PROMPT).expect("Missing resolution prompt");
        assert!(warning_position < prompt_position, "{output:?}");
    }

    fn learning_prompt(&mut self, word: &str, replacement: bool) {
        self.expect(&learning_question(word, replacement));
        self.assert_no_learning_notices();
    }

    fn accept_plain(&mut self, word: &str) {
        self.send(b"a");
        self.learning_prompt(word, false);
        self.send(b"\r");
        self.select_plain_fallback(word);
        self.send(b"y");
    }

    fn edit_learning_word(&mut self, initial: &str, edited: &str) {
        self.send(b"e");
        self.expect(&format!("{WORD_PROMPT}{initial}"));
        if initial != edited {
            self.send(b"\x15");
            self.expect(WORD_PROMPT);
            // Learning retains one raw guard across redraws, so UTF-8 text can
            // be queued here, unlike the main custom-replacement field.
            self.send(edited.as_bytes());
            self.expect(&format!("{WORD_PROMPT}{edited}"));
        }
        self.send(b"\r");
    }

    fn plain_fallback_menu(&mut self, word: &str) -> Vec<String> {
        let (prompt, menu) = self.expect_any(&[AFFIX_PROMPT, SUFFIX_PROMPT]);
        assert!(!menu.contains("Affix suggestions unavailable"), "{menu}");
        if prompt == 0 {
            // The automatic table and its expansion must both finish before
            // entering the unchanged manual flow. Backtracking may already be there.
            self.picker_redraw(AFFIX_PROMPT);
            self.send(b"a");
            self.expect(SUFFIX_PROMPT);
        }
        self.suffix_redraw(word, 0)
    }

    fn affix_menu(
        &mut self,
        rows: &[(&str, &str, &str)],
        active_idx: usize,
        with_prefix: bool,
    ) -> String {
        let menu = self.expect(AFFIX_PROMPT);
        assert!(!menu.contains("Affix suggestions unavailable"), "{menu}");
        self.affix_redraw(rows, active_idx, true, with_prefix)
    }

    fn affix_redraw(
        &mut self,
        rows: &[(&str, &str, &str)],
        active_idx: usize,
        header: bool,
        with_prefix: bool,
    ) -> String {
        let start = self.consumed;
        let (list, context) = self.picker_redraw(AFFIX_PROMPT);
        let raw = String::from_utf8_lossy(&self.transcript[start..self.consumed]);
        assert!(
            !raw.contains(AFFIX_PROMPT),
            "Repeated affix prompt: {raw:?}"
        );
        let before_list_raw = raw.split_once("\x1b7").unwrap().0;
        let before_list = console::strip_ansi_codes(before_list_raw);
        let headings = before_list
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        let heading = if header {
            assert!(before_list.starts_with("\r\n"), "{raw:?}");
            assert_eq!(
                headings.len(),
                1,
                "Expected one header after prompt: {raw:?}"
            );
            let heading = headings[0].trim_start_matches(' ');
            let header_line = before_list_raw
                .split_once("\r\n")
                .unwrap()
                .1
                .split("\r\n")
                .next()
                .unwrap();
            // Either spaces or an absolute cursor move may position the header,
            // but its first cell must start in the list's content column five.
            let positioned = header_line.replace("\x1b[5G", "    ");
            assert_eq!(
                console::strip_ansi_codes(&positioned),
                format!("    {heading}"),
                "Header must immediately follow the prompt and align with values: {raw:?}"
            );
            for color in [Color::Green, Color::Blue, Color::Black] {
                assert!(
                    !header_line
                        .contains(&format!("{}", crossterm::style::SetForegroundColor(color))),
                    "Header must stay default-colored: {raw:?}"
                );
            }
            assert!(
                !header_line.contains(&format!(
                    "{}",
                    crossterm::style::SetBackgroundColor(Color::Black)
                )),
                "Header must not be highlighted: {raw:?}"
            );
            Some(heading)
        } else {
            assert!(
                headings.is_empty(),
                "Repeated header on navigation: {raw:?}"
            );
            None
        };
        assert_affix_picker_list(&list, rows, active_idx, heading, with_prefix);
        let (prefix, stem, suffix) = rows[active_idx];
        let flags = [prefix, suffix]
            .into_iter()
            .filter(|flag| *flag != "-")
            .map(|flag| flag.strip_prefix('/').expect("Missing affix flag slash"))
            .collect::<String>();
        let selected = if flags.is_empty() {
            "-".to_owned()
        } else {
            format!("/{flags}")
        };
        assert_affix_strip(&context, &selected, self.columns);
        assert_picker_expansion_style(&context, stem)
    }

    fn affix_strip(&self) -> (String, String) {
        let raw = String::from_utf8_lossy(&self.transcript[..self.consumed]);
        let start = raw.rfind("\x1b7").expect("Missing saved context cursor");
        let rows = picker_context_rows(&raw[start..]);
        assert_eq!(rows.len(), 3, "{raw:?}");
        (
            console::strip_ansi_codes(rows[0]).into_owned(),
            console::strip_ansi_codes(rows[1]).into_owned(),
        )
    }

    fn picker_redraw(&mut self, prompt: &str) -> (String, String) {
        let start = self.consumed;
        // The shared list renderer restores once; the entire context (three
        // automatic rows, one manual row) uses a second save/restore block.
        // Input is safe only after BOTH restores.
        self.expect("\x1b8");
        let raw = String::from_utf8_lossy(&self.transcript[start..self.consumed]);
        let list_start = raw.find("\x1b7").expect("Missing saved picker cursor");
        let list = raw[list_start..].to_owned();
        let start = self.consumed;
        self.expect("\x1b8");
        let context = String::from_utf8_lossy(&self.transcript[start..self.consumed]).into_owned();
        assert!(!list.contains(prompt), "Repeated picker prompt: {list:?}");
        let rows = picker_context_rows(&context);
        assert_eq!(
            rows.len(),
            if prompt == AFFIX_PROMPT { 3 } else { 1 },
            "{context:?}"
        );
        for row in &rows {
            let text = console::strip_ansi_codes(row);
            assert!(
                console::measure_text_width(&text) < usize::from(self.columns),
                "{context:?}"
            );
        }
        let expansion = console::strip_ansi_codes(rows.last().unwrap());
        assert!(expansion.starts_with("Would match: "), "{context:?}");
        for color in [Color::Blue, Color::Black] {
            assert!(
                !context.contains(&format!("{}", crossterm::style::SetForegroundColor(color))),
                "Colored expansion context: {context:?}"
            );
        }
        // Green-on-black is reserved for changed affix spans; callers validate
        // those spans against the selected stem, not the list's selected row.
        (list, context)
    }

    fn suffix_redraw(&mut self, word: &str, active_idx: usize) -> Vec<String> {
        let (raw, context) = self.picker_redraw(SUFFIX_PROMPT);
        let context = assert_picker_expansion_style(&context, word);
        let rendered = selection_list_rows(&raw);
        let flags = rendered
            .iter()
            .skip(1)
            .map(|row| {
                let text = console::strip_ansi_codes(selection_row_content(row));
                let content = text
                    .strip_suffix("< suffix")
                    .expect("Expected suffix hint")
                    .trim_end_matches(' ');
                let flag = content
                    .strip_prefix('/')
                    .expect("Expected /FLAG suffix row");
                assert!(
                    !flag.is_empty() && !flag.chars().any(char::is_whitespace),
                    "Invalid suffix flag: {row:?}"
                );
                flag.to_owned()
            })
            .collect::<Vec<_>>();
        assert!(!flags.is_empty(), "Missing real suffix choices: {raw:?}");
        assert!(flags.windows(2).all(|pair| pair[0] < pair[1]), "{flags:?}");
        let mut rows = vec![(word.to_owned(), "plain")];
        rows.extend(flags.iter().map(|flag| (format!("/{flag}"), "suffix")));
        assert_learning_picker_list(&raw, &rows, active_idx);
        if active_idx == 0 {
            let full = format!("Would match: {word} (plain word; no affixes)");
            if let Some(visible) = context.strip_suffix('…') {
                assert!(full.starts_with(visible), "{context:?}");
            } else {
                assert_eq!(context, full);
            }
        }
        flags
    }

    fn stem_menu(&mut self, entries: &[&str]) -> String {
        self.expect(STEM_CHOICE_PROMPT);
        self.stem_redraw(entries, 0)
    }

    fn stem_redraw(&mut self, entries: &[&str], active_idx: usize) -> String {
        let (raw, context) = self.picker_redraw(STEM_CHOICE_PROMPT);
        let stem = entries[active_idx]
            .rsplit_once('/')
            .expect("Missing stem flag")
            .0;
        let context = assert_picker_expansion_style(&context, stem);
        let rows = entries
            .iter()
            .map(|entry| ((*entry).to_owned(), "stem"))
            .collect::<Vec<_>>();
        assert_learning_picker_list(&raw, &rows, active_idx);
        context
    }

    fn stem_editor(&mut self, initial: &str) {
        self.expect(&format!("{EDIT_STEM_PROMPT}{initial}"));
    }

    fn edit_stem(&mut self, initial: &str, edited: &str) {
        self.stem_editor(initial);
        if initial != edited {
            self.send(b"\x15");
            self.expect(EDIT_STEM_PROMPT);
            self.send(edited.as_bytes());
            self.expect(&format!("{EDIT_STEM_PROMPT}{edited}"));
        }
        self.send(b"\r");
    }

    fn select_plain_fallback(&mut self, word: &str) {
        self.plain_fallback_menu(word);
        self.send(b"\r");
        self.edit_stem(word, word);
        self.learning_confirmation(word, word);
    }

    fn learning_confirmation(&mut self, entry: &str, forms: &str) {
        let start = self.consumed;
        let preview = self.expect(CONFIRM_PROMPT);
        let raw = String::from_utf8_lossy(&self.transcript[start..self.consumed]);
        let highlighted = entry.with(Color::Green).on(Color::Black).bold();
        assert!(
            raw.contains(&format!("New entry: {highlighted}\r\n")),
            "{raw:?}"
        );
        assert!(
            preview.contains(&format!("New entry: {entry}\r\n")),
            "{}",
            preview
        );
        let suffix = if entry.contains('/') {
            ""
        } else {
            " (plain word; no affixes)"
        };
        assert!(
            preview.contains(&format!("Would match: {forms}{suffix}\r\n")),
            "{}",
            preview
        );
        let stem = entry.rsplit_once('/').map_or(entry, |(stem, _)| stem);
        let expansion = styled_expansion(stem, &format!("{forms}{suffix}"));
        assert!(
            raw.contains(&format!("\r\nWould match: {expansion}\r\n{CONFIRM_PROMPT}")),
            "Match preview must use cyan stem cores and bold green-on-black affix changes: {raw:?}"
        );
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
        self.assert_no_learning_notices();
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

fn learning_question(word: &str, replacement: bool) -> String {
    let highlighted = word.with(Color::Green).on(Color::Black).bold();
    if replacement {
        format!("Learn replacement \"{highlighted}\" [p,e,n,?]: ")
    } else {
        format!("Learn \"{highlighted}\" [p,e,n,?]: ")
    }
}

fn styled_expansion(stem: &str, forms: &str) -> String {
    if let Some(word) = forms.strip_suffix(" (plain word; no affixes)") {
        return format!("{} (plain word; no affixes)", word.with(Color::Cyan));
    }
    forms
        .split(", ")
        .map(|form| {
            let boundaries = stem
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(stem.len()))
                .collect::<Vec<_>>();
            // Color the longest unchanged stem core cyan, including when an
            // added prefix shifts it away from the start of the expanded form.
            let mut core = (0, 0);
            'search: for length in (1..boundaries.len()).rev() {
                for start in 0..boundaries.len() - length {
                    let candidate = &stem[boundaries[start]..boundaries[start + length]];
                    if let Some(offset) = form.find(candidate) {
                        core = (offset, offset + candidate.len());
                        break 'search;
                    }
                }
            }
            let highlight = |text: &str| {
                if text.is_empty() {
                    String::new()
                } else {
                    format!("{}", text.with(Color::Green).on(Color::Black).bold())
                }
            };
            format!(
                "{}{}{}",
                highlight(&form[..core.0]),
                if core.0 == core.1 {
                    String::new()
                } else {
                    format!("{}", form[core.0..core.1].with(Color::Cyan))
                },
                highlight(&form[core.1..])
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn picker_context_rows(raw: &str) -> Vec<&str> {
    assert!(!raw.contains(['\r', '\n']), "Wrapped context: {raw:?}");
    let context = raw
        .strip_prefix("\x1b7")
        .expect("Missing context cursor save")
        .strip_suffix("\x1b8")
        .expect("Missing context cursor restore");
    // MoveDown(1), MoveToColumn(0), Clear(CurrentLine) positions each row;
    // stripping ANSI alone would concatenate the strip, caret, and expansion.
    let mut rows = context.split("\x1b[1B\x1b[1G\x1b[2K");
    assert_eq!(rows.next(), Some(""), "{raw:?}");
    rows.collect()
}

fn assert_affix_strip(raw: &str, selected: &str, columns: u16) {
    let rows = picker_context_rows(raw);
    assert_eq!(rows.len(), 3, "{raw:?}");
    let strip = console::strip_ansi_codes(rows[0]);
    let caret = console::strip_ansi_codes(rows[1]);
    for row in &rows[..2] {
        for color in [Color::Green, Color::Blue, Color::Black] {
            assert!(
                !row.contains(&format!("{}", crossterm::style::SetForegroundColor(color))),
                "Strip and caret must stay default-colored: {raw:?}"
            );
        }
        assert!(
            !row.contains(&format!(
                "{}",
                crossterm::style::SetBackgroundColor(Color::Black)
            )),
            "Strip and caret must not be highlighted: {raw:?}"
        );
    }
    let tokens = strip.split_whitespace().collect::<Vec<_>>();
    for (index, token) in tokens.iter().enumerate() {
        assert!(
            *token == "-"
                || (token.starts_with('/') && token.len() > 1)
                || (*token == ".." && (index == 0 || index == tokens.len() - 1)),
            "Invalid or partially clipped strip label: {raw:?}"
        );
    }
    let flags = tokens
        .iter()
        .filter(|token| **token != "..")
        .map(|token| {
            if *token == "-" {
                ""
            } else {
                token.strip_prefix('/').unwrap()
            }
        })
        .collect::<Vec<_>>();
    assert!(
        flags.windows(2).all(|pair| pair[0] < pair[1]),
        "Visible alternatives must be globally alphabetically sorted: {raw:?}"
    );
    assert_eq!(
        tokens.iter().filter(|token| **token == selected).count(),
        1,
        "Selected flags must remain visible: {raw:?}"
    );
    assert_eq!(caret.trim(), "^", "Expected one caret: {raw:?}");
    let caret_column = console::measure_text_width(caret.split_once('^').unwrap().0);
    let center = usize::from(columns) / 2;
    assert!(
        (center.saturating_sub(1)..=center).contains(&caret_column),
        "Caret must stay centered in the {columns}-column terminal: {raw:?}"
    );
    let selected_start = strip
        .match_indices(selected)
        .find_map(|(offset, _)| {
            let before = &strip[..offset];
            let after = &strip[offset + selected.len()..];
            (before.chars().next_back().is_none_or(char::is_whitespace)
                && after.chars().next().is_none_or(char::is_whitespace))
            .then_some(offset)
        })
        .unwrap();
    assert_eq!(
        caret_column,
        console::measure_text_width(&strip[..selected_start])
            + console::measure_text_width(selected) / 2,
        "Caret must align under the selected label's display-width midpoint: {raw:?}"
    );
}

fn assert_picker_expansion_style(raw: &str, stem: &str) -> String {
    let line = raw
        .split_once("Would match: ")
        .expect("Missing expansion context")
        .1
        .strip_suffix("\x1b8")
        .expect("Missing context cursor restore");
    let text = console::strip_ansi_codes(line);
    if text.ends_with('…') {
        // Truncation may hide an entire extension, but must never leave an open
        // selection style or a partial escape sequence at the context boundary.
        assert!(!text.contains('\x1b'), "Broken truncated ANSI: {raw:?}");
        if text
            .trim_end_matches('…')
            .split(", ")
            .all(|form| stem.starts_with(form))
        {
            assert_eq!(
                line, text,
                "Truncated stem prefixes must stay unstyled: {raw:?}"
            );
        }
        let green = format!("{}", crossterm::style::SetForegroundColor(Color::Green));
        if let Some(last_green) = line.rfind(&green) {
            let probe = format!("{}", "probe".with(Color::Green).on(Color::Black).bold());
            let reset = probe.split_once("probe").unwrap().1;
            assert!(
                line[last_green..].contains(reset),
                "Unreset affix style: {raw:?}"
            );
        }
    } else {
        assert_eq!(
            line,
            styled_expansion(stem, &text),
            "Only added/changed affix spans may be highlighted: {raw:?}"
        );
    }
    format!("Affix expansion: {text}")
}

fn assert_command_effect(help: &str, key: &str, effect: &str) {
    let description = help
        .lines()
        .filter_map(|line| line.split_once(" - "))
        .find(|(keys, _)| keys.split('/').any(|candidate| candidate.trim() == key))
        .unwrap_or_else(|| panic!("Missing command {key:?}: {help:?}"))
        .1;
    assert_eq!(description, effect, "{help:?}");
}

fn assert_command_help(help: &str, required: &[&str], optional: &[&str]) {
    assert!(
        help.starts_with("\r\n"),
        "Help must start on a new line: {help:?}"
    );
    assert!(help.ends_with("? - show commands\r\n"), "{help:?}");
    let mut commands = Vec::new();
    for line in help.trim_matches(['\r', '\n']).split("\r\n") {
        let (keys, description) = line
            .split_once(" - ")
            .unwrap_or_else(|| panic!("Expected only key - description lines: {help:?}"));
        assert!(
            !description.trim().is_empty()
                && description.chars().count() <= 100
                && !description.chars().any(char::is_control),
            "Expected a brief nonempty side effect: {line:?}"
        );
        // Key grouping and spacing are presentation choices, not part of the
        // contract. Descriptions are deliberately unconstrained except above.
        for key in keys.split('/').map(str::trim) {
            assert!(
                required.contains(&key) || optional.contains(&key),
                "Unexpected command {key:?} for this context: {help:?}"
            );
            assert!(!commands.contains(&key), "Duplicate command: {help:?}");
            commands.push(key);
        }
    }
    for key in required {
        assert!(commands.contains(key), "Missing command {key:?}: {help:?}");
    }
}

fn selection_list_rows(raw: &str) -> Vec<&str> {
    let start = raw.find("\x1b7").expect("Missing saved cursor position");
    let list = raw[start..]
        .strip_prefix("\x1b7\x1b[2K")
        .expect("Missing initial row clear")
        .strip_suffix("\x1b8")
        .expect("Missing restored cursor position");
    // ANSI stripping alone concatenates rows. Preserve the existing upward
    // redraw boundaries so columns are measured from each row's content origin.
    list.split("\x1b[1A\x1b[2K").collect()
}

fn selection_row_content(row: &str) -> &str {
    row.split_once("\x1b[5G").expect("Missing content column").1
}

fn assert_hint_column(rows: &[&str]) -> usize {
    let text = rows
        .iter()
        .map(|row| console::strip_ansi_codes(selection_row_content(row)))
        .collect::<Vec<_>>();
    let max_width = text
        .iter()
        .map(|row| {
            let content = row
                .rsplit_once("< ")
                .map_or(row.as_ref(), |(content, _)| content.trim_end_matches(' '));
            console::measure_text_width(content)
        })
        .max()
        .expect("Missing selection rows");
    for row in &text {
        assert!(
            !row.contains(['\r', '\n']),
            "Wrapped selection row: {row:?}"
        );
        if let Some((before_hint, _)) = row.rsplit_once("< ") {
            assert_eq!(
                console::measure_text_width(before_hint),
                max_width + 1,
                "Hint not aligned with the widest row (including unhinted suggestions): {rows:?}"
            );
        }
    }
    // Content starts in terminal column five; the separator is one column past
    // the widest content. This column is local to this draw, not other menus.
    5 + max_width + 1
}

fn assert_main_picker_list(raw: &str, word: &str, custom: &str, active_idx: usize) -> usize {
    // Backends can suggest different replacements, but these three base rows
    // must always remain separate and in the same order.
    let rendered = selection_list_rows(raw);
    assert!(rendered.len() >= 3, "{raw:?}");
    let column = assert_hint_column(&rendered);
    let max_width = column - 6;
    let rows = [word.to_owned(), custom.to_owned(), format!("`{word}`")];
    let descriptions = ["learn", "custom input", "wrap backticks"];
    let marker = '»'.with(Color::Green).bold();
    for (index, row) in rows.iter().enumerate() {
        let padding = " ".repeat(max_width - console::measure_text_width(row) + 1);
        let color = if index == 1 {
            Color::Yellow
        } else {
            Color::Blue
        };
        let styled = if index == active_idx {
            row.as_str().with(Color::Green).on(Color::Black).bold()
        } else {
            row.as_str().with(color).on(Color::Black)
        };
        // Exact raw output keeps BOTH the padding and hint outside styling.
        assert_eq!(
            selection_row_content(rendered[index]),
            format!("{styled}{padding}< {}", descriptions[index]),
            "Incorrect row content, padding, or hint styling: {raw:?}"
        );
    }
    for row in &rendered[3..] {
        let content = selection_row_content(row);
        let text = console::strip_ansi_codes(content);
        assert!(!text.contains("< "), "Hint on checker suggestion: {row:?}");
        assert_eq!(
            content,
            format!("{}", text.as_ref().with(Color::Blue).on(Color::Black)),
            "Changed checker suggestion styling: {row:?}"
        );
    }
    assert!(
        rendered[active_idx].contains(&format!("\x1b[3G{marker}\x1b[5G")),
        "Missing separate selected marker: {raw:?}"
    );
    assert_eq!(
        console::strip_ansi_codes(raw).matches('»').count(),
        1,
        "{raw:?}"
    );
    column
}

fn assert_learning_picker_list(raw: &str, rows: &[(String, &str)], active_idx: usize) {
    let rendered = selection_list_rows(raw);
    assert_eq!(rendered.len(), rows.len(), "{raw:?}");
    let column = assert_hint_column(&rendered);
    let longest_hint = rows
        .iter()
        .map(|(_, hint)| console::measure_text_width(hint))
        .max()
        .unwrap();
    // All visible content shares one budget, including shorter-hint rows.
    let available = 160usize.saturating_sub(5 + longest_hint + 3);
    let visible = rows
        .iter()
        .map(|(row, _)| console::truncate_str(row, available, "…"))
        .collect::<Vec<_>>();
    let max_width = visible
        .iter()
        .map(|row| console::measure_text_width(row))
        .max()
        .unwrap();
    assert_eq!(column, 5 + max_width + 1, "{raw:?}");
    let marker = '»'.with(Color::Green).bold();
    for (index, (row, (_, hint))) in visible.iter().zip(rows).enumerate() {
        let selected = index == active_idx;
        let padding = " ".repeat(max_width - console::measure_text_width(row) + 1);
        let styled = if selected {
            row.as_ref().with(Color::Green).on(Color::Black).bold()
        } else {
            row.as_ref().with(Color::Blue).on(Color::Black)
        };
        // The default-color padding and hint must follow the style reset.
        assert_eq!(
            selection_row_content(rendered[index]),
            format!("{styled}{padding}< {hint}"),
            "{raw:?}"
        );
        if selected {
            assert!(
                rendered[index].contains(&format!("\x1b[3G{marker}\x1b[5G{styled}")),
                "{raw:?}"
            );
        }
        assert!(
            4 + console::measure_text_width(&console::strip_ansi_codes(selection_row_content(
                rendered[index]
            ))) < 160,
            "Selection row would wrap: {raw:?}"
        );
    }
    assert_eq!(
        console::strip_ansi_codes(raw).matches('»').count(),
        1,
        "{raw:?}"
    );
}

fn assert_affix_picker_list(
    raw: &str,
    rows: &[(&str, &str, &str)],
    active_idx: usize,
    heading: Option<&str>,
    with_prefix: bool,
) {
    let headings = if with_prefix {
        vec!["prefix", "stem", "suffix"]
    } else {
        vec!["stem", "suffix"]
    };
    let separators = |text: &str| {
        text.match_indices('|')
            .map(|(offset, _)| console::measure_text_width(&text[..offset]))
            .collect::<Vec<_>>()
    };
    let rendered = selection_list_rows(raw);
    assert_eq!(rendered.len(), rows.len(), "{raw:?}");
    let mut aligned = None;
    let mut expected = Vec::new();
    for (row, (prefix, stem, suffix)) in rendered.iter().zip(rows) {
        let text = console::strip_ansi_codes(selection_row_content(row));
        let (content, hint) = text.rsplit_once("< ").expect("Missing affix row hint");
        let content = content.trim_end_matches(' ');
        let cells = content.split('|').map(str::trim).collect::<Vec<_>>();
        let values = if with_prefix {
            vec![*prefix, *stem, *suffix]
        } else {
            vec![*stem, *suffix]
        };
        assert_eq!(cells, values, "Incorrect affix cells: {raw:?}");
        let columns = separators(content);
        if let Some(aligned) = &aligned {
            assert_eq!(&columns, aligned, "Misaligned affix columns: {raw:?}");
        } else {
            aligned = Some(columns);
        }
        let plain = (*prefix, *suffix) == ("-", "-");
        assert_eq!(hint, if plain { "plain" } else { "pattern" }, "{raw:?}");
        if plain {
            assert_eq!((*prefix, *suffix), ("-", "-"), "{raw:?}");
        }
        expected.push((content.to_owned(), if plain { "plain" } else { "pattern" }));
    }
    if let Some(heading) = heading {
        assert_eq!(
            heading.split('|').map(str::trim).collect::<Vec<_>>(),
            headings
        );
        assert_eq!(
            separators(heading),
            aligned.unwrap(),
            "Header must align with the value columns: {heading:?}, {raw:?}"
        );
    }
    // Retain the shared renderer's exact marker, color, padding, hint alignment,
    // and terminal-width checks for these already column-padded contents too.
    assert_learning_picker_list(raw, &expected, active_idx);
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
    let target = fixture.root.join(DEFAULT_DICTIONARY);
    session.assert_startup_learning_warning((!persistent).then_some(target.as_path()));
    session.accept_plain(UNKNOWN);
    // Both the already-queued duplicate and the later document must be skipped
    // without another keystroke; a regression fails on this finite deadline.
    session.finish(0);
    session.assert_prompt_count(1);
    session.assert_startup_learning_warning((!persistent).then_some(target.as_path()));
    assert_eq!(fixture.read("first.md"), repeated);
    assert_eq!(fixture.read("second.md"), UNKNOWN);
    fixture.assert_learning_saved(persistent, "1\nZzyzxqvocab\n");
    fixture.assert_config_unchanged();
    let output = fixture.check(&paths, 0);
    assert!(
        !output.contains("Possible spelling mistake found"),
        "{output}"
    );
    let mut next = fixture.cli("fix", &paths);
    next.finish(0);
    next.assert_prompt_count(0);
}

#[test]
fn acceptance_suppresses_duplicates_saves_and_reloads_on_next_check() {
    for backend in BACKENDS {
        for persistent in [false, true] {
            exercise_acceptance(std::slice::from_ref(backend), persistent);
        }
    }
}

#[cfg(any(
    all(feature = "hunspell", feature = "spellbook"),
    all(feature = "hunspell", feature = "zet"),
    all(feature = "spellbook", feature = "zet")
))]
#[test]
fn mixed_backends_share_acceptance_and_a_single_learning_target() {
    // Both configured and automatic targets must suppress queued diagnostics and
    // reload in every backend, even though no TOML extras name the default.
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
    session.accept_plain(UNKNOWN);
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
    session.accept_plain(UNKNOWN);
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
fn automatic_learning_creates_preferred_target_only_on_commit_and_reloads_all_backends() {
    // Include both individual engines and the mixed-engine session. No backend's
    // configured extras name the default, so fresh recognition requires autoload.
    for backends in BACKENDS
        .iter()
        .map(std::slice::from_ref)
        .chain(std::iter::once(BACKENDS))
    {
        for end in [b"q", b"n", b"\x03"] {
            let source = format!("{UNKNOWN} Zzyzxqroots Thirdwrng");
            let fixture = Fixture::with_suffixes(
                &[
                    ("first.md", &source),
                    ("forms.md", "Zzyzxqvocab Zzyzxqroot Zzyzxqroots"),
                    ("unlearned.md", "Thirdwrng"),
                ],
                backends,
            )
            .without_learning_target();
            let target = fixture.root.join(DEFAULT_DICTIONARY);
            fixture.check(&["forms.md"], 17);
            assert!(!fixture.root.join(".config").exists());

            let mut session = fixture.cli("fix", &["first.md"]);
            session.prompt("1/3", "first.md");
            session.assert_startup_learning_warning(Some(&target));
            assert!(!fixture.root.join(".config").exists());
            session.send(b"a");
            session.learning_prompt(UNKNOWN, false);
            session.send(b"\r");
            session.select_plain_fallback(UNKNOWN);
            session.send(b"\r");
            session.prompt("2/3", "first.md");
            assert!(!fixture.root.join(".config").exists());

            session.send(b"\x1b[H");
            session.main_picker_redraw("Zzyzxqroots", 0);
            session.send(b"\r");
            session.affix_menu(
                &[("-", "Zzyzxqroot", "/S"), ("-", "Zzyzxqroots", "-")],
                0,
                false,
            );
            session.send(b"\r");
            session.learning_confirmation("Zzyzxqroot/S", "Zzyzxqroot, Zzyzxqroots");
            session.send(b"\r");
            session.prompt("3/3", "first.md");
            assert!(!fixture.root.join(".config").exists());
            assert_eq!(fixture.read("lingo.dic"), "0\n");
            assert_eq!(fixture.read("first.md"), source);
            session.send(end);
            let abort = end == b"\x03";
            session.finish(if abort { 130 } else { 0 });
            session.assert_prompt_count(3);
            session.assert_startup_learning_warning(Some(&target));
            assert_eq!(fixture.read("first.md"), source);
            fixture.assert_config_unchanged();
            if abort {
                assert!(!fixture.root.join(".config").exists());
                assert_eq!(fixture.read("lingo.dic"), "0\n");
            } else {
                fixture.assert_learning_saved(false, "2\nZzyzxqroot/S\nZzyzxqvocab\n");
            }
            fixture.check(&["forms.md"], if abort { 17 } else { 0 });
            fixture.check(&["unlearned.md"], 17);
            if !abort {
                let mut next = fixture.cli("fix", &["forms.md"]);
                next.finish(0);
                next.assert_prompt_count(0);
            }
        }
    }
}

#[test]
fn automatic_learning_prefers_existing_preferred_target_then_existing_legacy_target() {
    for (preferred, legacy) in [(true, false), (false, true), (true, true)] {
        let fixture = Fixture::with_backends(
            false,
            &[
                ("first.md", UNKNOWN),
                ("preferred.md", "Zzyzxqpreferred"),
                ("legacy.md", "Zzyzxqlegacy"),
            ],
            BACKENDS,
        );
        for (exists, name, contents) in [
            (preferred, DEFAULT_DICTIONARY, "1\nZzyzxqpreferred\n"),
            (legacy, LEGACY_DICTIONARY, "1\nZzyzxqlegacy\n"),
        ] {
            if exists {
                let path = fixture.root.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, contents).unwrap();
            }
        }
        fixture.check(&["preferred.md"], if preferred { 0 } else { 17 });
        fixture.check(&["legacy.md"], if preferred { 17 } else { 0 });
        if preferred {
            assert_eq!(fixture.read(DEFAULT_DICTIONARY), "1\nZzyzxqpreferred\n");
            assert_eq!(
                fs::read_dir(fixture.root.join(".config/spellcheck"))
                    .unwrap()
                    .count(),
                1
            );
        }
        if legacy {
            assert_eq!(fixture.read(LEGACY_DICTIONARY), "1\nZzyzxqlegacy\n");
        }
        assert_eq!(
            fs::read_dir(fixture.root.join(".config")).unwrap().count(),
            usize::from(preferred) + usize::from(legacy)
        );
        let chosen = if preferred {
            DEFAULT_DICTIONARY
        } else {
            LEGACY_DICTIONARY
        };
        let before = fixture.read(chosen);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.assert_startup_learning_warning(None);
        session.send(b"a");
        session.learning_prompt(UNKNOWN, false);
        session.send(b"\r");
        session.select_plain_fallback(UNKNOWN);
        assert_eq!(fixture.read(chosen), before);
        session.send(b"y");
        session.finish(0);
        session.assert_prompt_count(1);
        session.assert_startup_learning_warning(None);
        let seed = if preferred {
            "Zzyzxqpreferred"
        } else {
            "Zzyzxqlegacy"
        };
        assert_eq!(fixture.read(chosen), format!("2\n{seed}\n{UNKNOWN}\n"));
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        if preferred && legacy {
            assert_eq!(fixture.read(LEGACY_DICTIONARY), "1\nZzyzxqlegacy\n");
        } else if preferred {
            assert!(!fixture.root.join(LEGACY_DICTIONARY).exists());
        } else {
            assert!(!fixture.root.join(".config/spellcheck").exists());
        }
        fixture.assert_config_unchanged();
        fixture.check(&["first.md"], 0);
        let mut next = fixture.cli("fix", &["first.md"]);
        next.finish(0);
        next.assert_prompt_count(0);
    }
}

#[test]
fn automatic_learning_does_not_create_empty_config_on_check_quit_or_cancelled_learning() {
    for stage in ["quit", "skip", "abort", "cancel", "decline", "replacement"] {
        let fixture = Fixture::with_backends(
            false,
            &[("first.md", UNKNOWN), ("known.md", "spellchecker")],
            BACKENDS,
        );
        fixture.check(&["first.md"], 17);
        fixture.check(&["known.md"], 0);
        assert!(!fixture.root.join(".config").exists());
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.assert_startup_learning_warning(Some(&fixture.root.join(DEFAULT_DICTIONARY)));
        match stage {
            "quit" => session.send(b"q"),
            "skip" => session.send(b"n"),
            "abort" => session.send(b"\x03"),
            "replacement" => {
                session.replace_custom("Zzyzxqcorrection");
                session.learning_prompt("Zzyzxqcorrection", true);
                session.send(b"n");
            }
            _ => {
                session.send(b"a");
                session.learning_prompt(UNKNOWN, false);
                if stage == "decline" {
                    session.send(b"\r");
                    session.select_plain_fallback(UNKNOWN);
                }
                session.send(b"n");
                session.prompt("1/1", "first.md");
                session.send(b"q");
            }
        }
        session.finish(match stage {
            "abort" => 130,
            "replacement" => 17,
            _ => 0,
        });
        session.assert_startup_learning_warning(Some(&fixture.root.join(DEFAULT_DICTIONARY)));
        assert!(!fixture.root.join(".config").exists());
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        assert_eq!(
            fixture.read("first.md"),
            if stage == "replacement" {
                "Zzyzxqcorrection"
            } else {
                UNKNOWN
            }
        );
        fixture.assert_config_unchanged();
    }
}

#[test]
fn explicitly_configured_writable_learning_target_has_no_warning_or_fallback() {
    for existing_default in [false, true] {
        let fixture = Fixture::with_backends(true, &[("first.md", UNKNOWN)], BACKENDS);
        if existing_default {
            let target = fixture.root.join(DEFAULT_DICTIONARY);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, "1\nZzyzxqpreferred\n").unwrap();
        }
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.assert_startup_learning_warning(None);
        session.send(b"a");
        session.learning_prompt(UNKNOWN, false);
        session.send(b"\r");
        session.select_plain_fallback(UNKNOWN);
        session.send(b"y");
        session.finish(0);
        session.assert_startup_learning_warning(None);
        assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqvocab\n");
        if existing_default {
            assert_eq!(fixture.read(DEFAULT_DICTIONARY), "1\nZzyzxqpreferred\n");
            assert_eq!(
                fs::read_dir(fixture.root.join(".config/spellcheck"))
                    .unwrap()
                    .count(),
                1
            );
        } else {
            assert!(!fixture.root.join(".config").exists());
        }
        assert!(!fixture.root.join(LEGACY_DICTIONARY).exists());
        fixture.assert_config_unchanged();
        fixture.check(&["first.md"], 0);
    }
}

#[test]
fn explicitly_configured_unregistered_target_is_an_error_even_when_a_default_exists() {
    for existing_default in [false, true] {
        let mut fixture = Fixture::with_backends(true, &[("first.md", UNKNOWN)], BACKENDS);
        fixture.config = fixture.config.replace(
            "learning_dictionary = \"lingo.dic\"",
            "learning_dictionary = \"misspelled.dic\"",
        );
        fs::write(fixture.root.join("spellcheck.toml"), &fixture.config).unwrap();
        if existing_default {
            let target = fixture.root.join(DEFAULT_DICTIONARY);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, "1\nZzyzxqvocab\n").unwrap();
        }
        let mut session = fixture.cli("fix", &["first.md"]);
        session.finish(1);
        session.assert_prompt_count(0);
        let output = session.output();
        assert!(output.contains("misspelled.dic"), "{output}");
        assert!(
            output.contains(
                "must be listed in extra_dictionaries for every enabled spelling backend"
            ),
            "{output}"
        );
        session.assert_startup_learning_warning(None);
        assert!(!fixture.root.join("misspelled.dic").exists());
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        if existing_default {
            assert_eq!(fixture.read(DEFAULT_DICTIONARY), "1\nZzyzxqvocab\n");
            assert_eq!(
                fs::read_dir(fixture.root.join(".config/spellcheck"))
                    .unwrap()
                    .count(),
                1
            );
        } else {
            assert!(!fixture.root.join(".config").exists());
        }
        assert!(!fixture.root.join(LEGACY_DICTIONARY).exists());
        fixture.assert_config_unchanged();
    }
}

#[test]
fn missing_registered_explicit_target_is_created_only_on_nonempty_commit() {
    for backends in BACKENDS
        .iter()
        .map(std::slice::from_ref)
        .chain(std::iter::once(BACKENDS))
    {
        for stage in ["apply", "quit", "abort", "cancel", "decline", "empty"] {
            let source = format!("{UNKNOWN} Thirdwrng");
            let mut fixture = Fixture::with_backends(true, &[("first.md", &source)], backends);
            fixture.config = fixture
                .config
                .replace("\"lingo.dic\"", "\"learned/vocabulary.dic\"");
            fs::write(fixture.root.join("spellcheck.toml"), &fixture.config).unwrap();
            let target = fixture.root.join("learned/vocabulary.dic");
            fixture.check(&["first.md"], 17);
            assert!(!fixture.root.join("learned").exists());

            let mut session = fixture.cli("fix", &["first.md"]);
            session.prompt("1/2", "first.md");
            session.assert_startup_learning_warning(Some(&target));
            assert!(!fixture.root.join("learned").exists());
            match stage {
                "empty" => session.send(b"q"),
                "cancel" | "decline" => {
                    session.send(b"a");
                    session.learning_prompt(UNKNOWN, false);
                    if stage == "decline" {
                        session.send(b"\r");
                        session.select_plain_fallback(UNKNOWN);
                    }
                    session.send(b"n");
                    session.prompt("1/2", "first.md");
                    session.send(b"q");
                }
                _ => {
                    session.accept_plain(UNKNOWN);
                    session.prompt("2/2", "first.md");
                    assert!(!fixture.root.join("learned").exists());
                    assert_eq!(fixture.read("first.md"), source);
                    session.send(match stage {
                        "apply" => b"n",
                        "quit" => b"q",
                        "abort" => b"\x03",
                        _ => unreachable!(),
                    });
                }
            }
            session.finish(if stage == "abort" { 130 } else { 0 });
            session.assert_startup_learning_warning(Some(&target));
            assert_eq!(fixture.read("first.md"), source);
            assert_eq!(fixture.read("lingo.dic"), "0\n");
            assert!(!fixture.root.join(".config").exists());
            fixture.assert_config_unchanged();
            if matches!(stage, "apply" | "quit") {
                assert_eq!(fixture.read("learned/vocabulary.dic"), "1\nZzyzxqvocab\n");
                fs::write(fixture.root.join("known.md"), UNKNOWN).unwrap();
                fixture.check(&["known.md"], 0);
                let mut next = fixture.cli("fix", &["known.md"]);
                next.finish(0);
                next.assert_prompt_count(0);
                next.assert_startup_learning_warning(None);
            } else {
                // No dictionary, parent directory, or sidecar lock on abort or
                // when learning was cancelled or never confirmed.
                assert!(!fixture.root.join("learned").exists());
            }
        }
    }
}

#[test]
fn invalid_learning_targets_fail_before_any_interactive_prompt() {
    for configured in [false, true] {
        for kind in ["readonly", "directory", "dangling symlink"] {
            let fixture = Fixture::with_backends(configured, &[("first.md", UNKNOWN)], BACKENDS);
            let target = fixture.root.join(if configured {
                "lingo.dic"
            } else {
                DEFAULT_DICTIONARY
            });
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            if configured {
                fs::remove_file(&target).unwrap();
            }
            match kind {
                "readonly" => {
                    fs::write(&target, "0\n").unwrap();
                    fs::set_permissions(&target, fs::Permissions::from_mode(0o444)).unwrap();
                }
                "directory" => fs::create_dir(&target).unwrap(),
                "dangling symlink" => symlink("missing.dic", &target).unwrap(),
                _ => unreachable!(),
            }

            let mut session = fixture.cli("fix", &["first.md"]);
            session.finish(1);
            session.assert_prompt_count(0);
            session.assert_startup_learning_warning(None);
            let output = session.output();
            // Automatic discovery may return the underlying filesystem error
            // directly for a dangling symlink, without path context.
            if configured || kind != "dangling symlink" {
                assert!(output.contains(&target.display().to_string()), "{output}");
            }
            for prompt in [
                "Learn \"",
                "Learn replacement \"",
                AFFIX_PROMPT,
                EDIT_STEM_PROMPT,
            ] {
                assert!(!output.contains(prompt), "{output}");
            }
            assert_eq!(fixture.read("first.md"), UNKNOWN);
            assert!(!PathBuf::from(format!("{}.cargo-spellcheck.lock", target.display())).exists());
            match kind {
                "readonly" => {
                    assert!(output.contains("writable"), "{output}");
                    assert_eq!(fs::read_to_string(&target).unwrap(), "0\n");
                    assert!(fs::metadata(&target).unwrap().permissions().readonly());
                }
                "directory" => {
                    assert!(output.contains("regular file"), "{output}");
                    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
                }
                "dangling symlink" => {
                    assert_eq!(fs::read_link(&target).unwrap(), Path::new("missing.dic"));
                    assert!(!target.parent().unwrap().join("missing.dic").exists());
                }
                _ => unreachable!(),
            }
            fixture.assert_config_unchanged();
        }
    }
}

#[test]
fn learn_row_goes_directly_to_affixes_and_suppresses_duplicates() {
    for backend in BACKENDS {
        for persistent in [false, true] {
            let repeated = format!("{UNKNOWN} {UNKNOWN}");
            let fixture = Fixture::with_backends(
                persistent,
                &[("first.md", &repeated), ("second.md", UNKNOWN)],
                std::slice::from_ref(backend),
            );
            let paths = ["first.md", "second.md"];
            let mut session = fixture.cli("fix", &paths);
            session.prompt("1/2", "first.md");
            session.send(b"\x1b[H");
            session.main_picker_redraw(UNKNOWN, 0);
            session.send(b"\r");
            session.affix_menu(&[("-", UNKNOWN, "-")], 0, true);
            assert!(!session.output().contains("Learn \""));
            assert!(!session.output().contains("Learn replacement \""));
            assert!(!session.output().contains(WORD_PROMPT));
            assert!(!session.output().contains(CONFIRM_PROMPT));
            assert_eq!(fixture.read("first.md"), repeated);
            assert_eq!(fixture.read("lingo.dic"), "0\n");

            session.send(b"\r");
            session.learning_confirmation(UNKNOWN, UNKNOWN);
            assert_eq!(fixture.read("lingo.dic"), "0\n");
            session.send(b"y");
            // No input is supplied for the queued duplicate or the later source.
            session.finish(0);
            session.assert_prompt_count(1);
            assert_eq!(session.output().matches(AFFIX_PROMPT).count(), 1);
            assert!(!session.output().contains(SUFFIX_PROMPT));
            assert!(!session.output().contains(EDIT_STEM_PROMPT));
            assert!(!session.output().contains(STEM_CHOICE_PROMPT));
            assert_eq!(fixture.read("first.md"), repeated);
            assert_eq!(fixture.read("second.md"), UNKNOWN);
            fixture.assert_learning_saved(persistent, "1\nZzyzxqvocab\n");
            fixture.assert_config_unchanged();
            fixture.check(&paths, 0);
        }
    }
}

#[test]
fn enter_through_learning_row_automatic_plain_and_confirmation_records_entry() {
    let fixture = Fixture::with_suffixes(&[("first.md", UNKNOWN)], &BACKENDS[..1]);
    let mut session = fixture.cli("fix", &["first.md"]);
    session.prompt("1/1", "first.md");
    session.send(b"\x1b[H");
    session.main_picker_redraw(UNKNOWN, 0);
    session.send(b"\r");
    session.affix_menu(&[("-", UNKNOWN, "-")], 0, false);
    assert!(!session.output().contains(SUFFIX_PROMPT));
    session.send(b"\r");
    session.learning_confirmation(UNKNOWN, UNKNOWN);
    assert!(!session.output().contains(EDIT_STEM_PROMPT));
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    session.send(b"\r");
    session.finish(0);
    assert!(
        !session.output().contains("Abort"),
        "Enter must never abort"
    );
    session.assert_prompt_count(1);
    assert!(!session.output().contains("Learn \""));
    assert!(!session.output().contains(WORD_PROMPT));
    assert!(!session.output().contains(STEM_CHOICE_PROMPT));
    assert_eq!(fixture.read("first.md"), UNKNOWN);
    assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqvocab\n");
    fixture.assert_config_unchanged();
    fixture.check(&["first.md"], 0);
}

#[test]
fn cancelling_verbatim_entry_returns_to_main_and_backticks_do_not_learn() {
    for cancel in [b"n".as_slice(), b"\x1b".as_slice()] {
        let fixture = Fixture::new(true, &[("first.md", UNKNOWN), ("original.md", UNKNOWN)]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"\x1b[H");
        session.main_picker_redraw(UNKNOWN, 0);
        session.send(b"\r");
        session.affix_menu(&[("-", UNKNOWN, "-")], 0, true);
        session.send(cancel);
        session.prompt("1/1", "first.md");
        assert!(!session.output().contains("Learn \""));
        assert!(!session.output().contains(WORD_PROMPT));
        assert_eq!(fixture.read("first.md"), UNKNOWN);
        assert_eq!(fixture.read("lingo.dic"), "0\n");

        // Home selects verbatim; the first Up enters custom editing, where a
        // second Up must still navigate to the separate backtick-wrapping row.
        session.send(b"\x1b[H");
        session.main_picker_redraw(UNKNOWN, 0);
        session.send(b"\x1b[A");
        session.main_picker_redraw(UNKNOWN, 1);
        session.expect_custom_cursor(0);
        session.send(b"\x1b[A");
        // Leaving custom input first restores its saved cursor, then redraws.
        session.expect("\x1b8");
        session.main_picker_redraw(UNKNOWN, 2);
        session.send(b"\r");
        session.finish(17);
        session.assert_prompt_count(2);
        assert_eq!(session.output().matches(AFFIX_PROMPT).count(), 1);
        assert!(!session.output().contains(SUFFIX_PROMPT));
        assert!(!session.output().contains("Learn replacement \""));
        assert!(!session.output().contains(CONFIRM_PROMPT));
        assert_eq!(fixture.read("first.md"), format!("`{UNKNOWN}`"));
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        fixture.assert_config_unchanged();
        fixture.check(&["original.md"], 17);
    }
}

#[test]
fn declining_verbatim_confirmation_returns_to_main_without_learning() {
    for decline in [b"n".as_slice(), b"\x1b".as_slice()] {
        let fixture = Fixture::new(true, &[("first.md", UNKNOWN)]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"\x1b[H");
        session.main_picker_redraw(UNKNOWN, 0);
        session.send(b"\r");
        session.affix_menu(&[("-", UNKNOWN, "-")], 0, true);
        session.send(b"\r");
        session.learning_confirmation(UNKNOWN, UNKNOWN);
        assert!(!session.output().contains(EDIT_STEM_PROMPT));
        session.send(decline);
        session.prompt("1/1", "first.md");
        assert!(!session.output().contains("Learn \""));
        assert!(!session.output().contains(WORD_PROMPT));
        session.send(b"q");
        session.finish(0);
        session.assert_prompt_count(2);
        assert_eq!(fixture.read("first.md"), UNKNOWN);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        fixture.assert_config_unchanged();
    }
}

#[test]
fn custom_replacement_edits_source_without_learning_either_word() {
    for skip in [b"n", b"\x1b"] {
        let fixture = Fixture::new(true, &[("first.md", "Spelchekr")]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.replace_custom(UNKNOWN);
        session.learning_prompt(UNKNOWN, true);
        session.send(skip);
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
}

#[test]
fn custom_cursor_stays_on_its_row_while_editing() {
    for source in [UNKNOWN, "Spelchekr"] {
        let fixture = Fixture::new(true, &[("first.md", source)]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"e");
        session.expect_custom_cursor(0);
        for (input, offset) in [
            (b"c".as_slice(), 1),
            (b"a".as_slice(), 2),
            (b"t".as_slice(), 3),
            (b"\x1b[D".as_slice(), 2),
            (b"\x7f".as_slice(), 1),
            (b"a".as_slice(), 2),
            (b"\x1b[C".as_slice(), 3),
        ] {
            session.send(input);
            session.expect_custom_cursor(offset);
        }
        session.send(b"\r");
        session.finish(17);
        assert_eq!(fixture.read("first.md"), "cat");
        assert_eq!(fixture.read("lingo.dic"), "0\n");
    }
}

#[test]
fn long_custom_input_realigns_hints_without_losing_cursor_edits_or_help_state() {
    const CUSTOM: &str = "Zzyzxqlongcustomreplacement";
    for source in [UNKNOWN, "Spelchekr"] {
        let fixture = Fixture::new(true, &[("first.md", source)]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"\x1b[H");
        let initial_column = session.main_picker_redraw(source, 0);
        session.send(b"e");
        session.custom_picker_redraw(source, "...", 0);
        let mut column = initial_column;
        for (offset, byte) in CUSTOM.bytes().enumerate() {
            session.send(&[byte]);
            column = session.custom_picker_redraw(source, &CUSTOM[..offset + 1], offset + 1);
        }
        assert!(
            column > initial_column,
            "Long custom input did not widen the hint column"
        );
        for offset in [CUSTOM.len() - 1, CUSTOM.len() - 2] {
            session.send(b"\x1b[D");
            assert_eq!(session.custom_picker_redraw(source, CUSTOM, offset), column);
        }
        session.command_help(
            &[
                "Left",
                "Right",
                "Up",
                "Down",
                "Backspace",
                "Enter",
                "Esc",
                "Ctrl-C",
                "?",
            ],
            &["text", "characters"],
        );
        session.help_prompt(&format!("(1/1) {PROMPT}"));
        assert_eq!(
            session.custom_picker_redraw(source, CUSTOM, CUSTOM.len() - 2),
            column
        );
        // Insert and remove inside the long field: help must retain the actual
        // editing offset, not merely draw the cursor at its former location.
        let mut edited = CUSTOM.to_owned();
        edited.insert(CUSTOM.len() - 2, 'x');
        session.send(b"x");
        assert_eq!(
            session.custom_picker_redraw(source, &edited, CUSTOM.len() - 1),
            column + 1
        );
        session.send(b"\x7f");
        assert_eq!(
            session.custom_picker_redraw(source, CUSTOM, CUSTOM.len() - 2),
            column
        );
        for offset in [CUSTOM.len() - 1, CUSTOM.len()] {
            session.send(b"\x1b[C");
            session.custom_picker_redraw(source, CUSTOM, offset);
        }
        assert_eq!(fixture.read("first.md"), source);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"\r");
        session.learning_prompt(CUSTOM, true);
        session.send(b"n");
        session.finish(17);
        session.assert_prompt_count(2);
        assert_eq!(fixture.read("first.md"), CUSTOM);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        fixture.assert_config_unchanged();
    }
}

#[test]
fn help_preserves_custom_input_and_every_learning_stage_without_recording_entries() {
    const WORD: &str = "Zzyzxqroots";
    const INITIAL: &str = "Zzyzxqrots";
    const SOURCE: &str = "Zzyzxqtypo Zzyzxqroots";
    let entries = ["Zzyzxqroot/S", "Zzyzxqroots/S"];
    let fixture = Fixture::with_suffixes(&[("first.md", SOURCE)], &BACKENDS[..1]);
    let mut session = fixture.cli("fix", &["first.md"]);
    session.prompt("1/2", "first.md");
    session.send(b"\x1b[H");
    session.main_picker_redraw("Zzyzxqtypo", 0);
    session.main_help("1/2", "Zzyzxqtypo", 0);
    assert_eq!(fixture.read("first.md"), SOURCE);
    assert_eq!(fixture.read("lingo.dic"), "0\n");

    session.send(b"e");
    session.expect_custom_cursor(0);
    for (offset, byte) in INITIAL.bytes().enumerate() {
        session.send(&[byte]);
        session.expect_custom_cursor(offset + 1);
    }
    for offset in [INITIAL.len() - 1, INITIAL.len() - 2] {
        session.send(b"\x1b[D");
        session.expect_custom_cursor(offset);
    }
    session.command_help(
        &[
            "Left",
            "Right",
            "Up",
            "Down",
            "Backspace",
            "Enter",
            "Esc",
            "Ctrl-C",
            "?",
        ],
        &["text", "characters"],
    );
    session.help_prompt(&format!("(1/2) {PROMPT}"));
    session.custom_picker_redraw("Zzyzxqtypo", INITIAL, INITIAL.len() - 2);
    // Inserting the missing 'o' in the middle proves the cursor was preserved,
    // rather than merely restored visually while insertion moved to the end.
    session.send(b"o");
    session.expect_custom_cursor(WORD.len() - 2);
    for offset in [WORD.len() - 1, WORD.len()] {
        session.send(b"\x1b[C");
        session.expect_custom_cursor(offset);
    }
    session.send(b"\r");
    session.learning_prompt(WORD, true);
    session.command_help(&["Enter", "p", "e", "n", "Esc", "Ctrl-C", "?"], &[]);
    session.help_prompt(&learning_question(WORD, true));

    session.send(b"e");
    session.expect(&format!("{WORD_PROMPT}{WORD}"));
    session.command_help(EDITOR_COMMANDS, &["text", "characters"]);
    session.help_prompt(&format!("{WORD_PROMPT}{WORD}"));
    session.send(b"\r");
    assert_eq!(session.plain_fallback_menu(WORD), ["S"]);
    session.send(b"\x1b[A");
    session.suffix_redraw(WORD, 1);
    session.command_help(PICKER_COMMANDS, &[]);
    session.help_prompt(SUFFIX_PROMPT);
    assert_eq!(session.suffix_redraw(WORD, 1), ["S"]);

    session.send(b"\r");
    session.stem_menu(&entries);
    session.send(b"\x1b[A");
    let expansion = session.stem_redraw(&entries, 1);
    session.command_help(PICKER_COMMANDS, &[]);
    session.help_prompt(STEM_CHOICE_PROMPT);
    assert_eq!(session.stem_redraw(&entries, 1), expansion);
    session.send(b"\r");
    session.stem_editor(WORD);
    session.command_help(EDITOR_COMMANDS, &["text", "characters"]);
    session.help_prompt(&format!("{EDIT_STEM_PROMPT}{WORD}"));
    session.send(b"\r");
    session.learning_confirmation("Zzyzxqroots/S", "Zzyzxqroots, Zzyzxqrootss");
    session.confirmation_help();
    assert_eq!(fixture.read("first.md"), SOURCE);
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    session.send(b"n");
    // Help must not have learned either the plain word or its flagged entry:
    // the queued duplicate still needs its own resolution after declining.
    session.prompt("2/2", "first.md");
    assert_eq!(fixture.read("first.md"), SOURCE);
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    session.send(b"q");
    session.finish(17);
    session.assert_prompt_count(4);
    assert_eq!(fixture.read("first.md"), format!("{WORD} {WORD}"));
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    fixture.assert_config_unchanged();
}

#[test]
fn main_help_does_not_advance_findings_or_documents() {
    let fixture = Fixture::with_suffixes(
        &[("first.md", UNKNOWN), ("second.md", "Thirdwrng")],
        &BACKENDS[..1],
    );
    let mut session = fixture.cli("fix", &["first.md", "second.md"]);
    session.prompt("1/1", "first.md");
    session.send(b"\x1b[H");
    session.main_picker_redraw(UNKNOWN, 0);
    session.main_help("1/1", UNKNOWN, 0);
    assert!(!session.output().contains("second.md"));
    session.send(b"a");
    session.learning_prompt(UNKNOWN, false);
    session.command_help(&["Enter", "p", "e", "n", "Esc", "Ctrl-C", "?"], &[]);
    session.help_prompt(&learning_question(UNKNOWN, false));
    session.send(b"n");
    session.expect(PROMPT);
    session.expect("\x1b8");
    session.send(b"n");
    session.prompt("1/1", "second.md");
    session.send(b"\x1b[H");
    session.main_picker_redraw("Thirdwrng", 0);
    session.main_help("1/1", "Thirdwrng", 0);
    session.send(b"q");
    session.finish(0);
    session.assert_prompt_count(5);
    assert_eq!(fixture.read("first.md"), UNKNOWN);
    assert_eq!(fixture.read("second.md"), "Thirdwrng");
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    fixture.assert_config_unchanged();
}

#[test]
fn known_custom_replacement_has_no_learning_followup() {
    for backend in BACKENDS {
        let fixture = Fixture::with_backends(
            true,
            &[("first.md", "Spelchekr"), ("known.md", "spellchecker")],
            std::slice::from_ref(backend),
        );
        fixture.check(&["known.md"], 0);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.replace_custom("spellchecker");
        session.finish(17);
        session.assert_prompt_count(1);
        assert!(!session.output().contains("Learn replacement "));
        assert_eq!(fixture.read("first.md"), "spellchecker");
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        fixture.assert_config_unchanged();
    }
}

#[test]
fn plain_replacement_learning_is_deferred_and_records_only_the_correction() {
    for backend in BACKENDS {
        let source = "Spelchekr Thirdwrng";
        let fixture = Fixture::with_backends(
            true,
            &[("first.md", source), ("remaining.md", "Thirdwrng")],
            std::slice::from_ref(backend),
        );
        let mut session = fixture.cli("fix", &["first.md", "remaining.md"]);
        session.prompt("1/2", "first.md");
        session.replace_custom(UNKNOWN);
        session.learning_prompt(UNKNOWN, true);
        session.send(b"\r");
        session.select_plain_fallback(UNKNOWN);
        assert_eq!(fixture.read("first.md"), source);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"y");
        session.prompt("2/2", "first.md");
        assert_eq!(fixture.read("first.md"), source);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"q");
        session.finish(17);
        session.assert_prompt_count(2);
        assert!(!session.output().contains("remaining.md"));
        assert_eq!(fixture.read("first.md"), "Zzyzxqvocab Thirdwrng");
        assert_eq!(fixture.read("remaining.md"), "Thirdwrng");
        assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqvocab\n");
        fixture.assert_config_unchanged();
        // Fresh processes must recognize the correction, but not the typo.
        fs::write(fixture.root.join("corrected.md"), UNKNOWN).unwrap();
        fs::write(fixture.root.join("original.md"), "Spelchekr").unwrap();
        fixture.check(&["corrected.md"], 0);
        fixture.check(&["original.md"], 17);
    }
}

#[test]
fn plain_fallback_defers_writes_until_quit_or_apply_and_discards_them_on_abort() {
    for end in [b"q", b"n", b"\x03"] {
        let fixture = Fixture::new(
            true,
            &[
                ("first.md", MISTAKES),
                ("stem.md", UNKNOWN),
                ("plural.md", "Zzyzxqvocabs"),
            ],
        );
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/3", "first.md");
        session.send(b"a");
        session.learning_prompt(UNKNOWN, false);
        session.edit_learning_word(UNKNOWN, UNKNOWN);
        session.select_plain_fallback(UNKNOWN);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"y");
        session.prompt("2/3", "first.md");
        session.replace_custom("spellchecker");
        session.prompt("3/3", "first.md");
        assert_eq!(fixture.read("first.md"), MISTAKES);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(end);
        let abort = end == b"\x03";
        session.finish(if abort { 130 } else { 17 });
        session.assert_prompt_count(3);
        assert_eq!(
            fixture.read("first.md"),
            if abort {
                MISTAKES
            } else {
                "Zzyzxqvocab spellchecker Thirdwrng"
            }
        );
        assert_eq!(
            fixture.read("lingo.dic"),
            if abort { "0\n" } else { "1\nZzyzxqvocab\n" }
        );
        fixture.assert_config_unchanged();
        fixture.check(&["stem.md"], if abort { 17 } else { 0 });
        fixture.check(&["plural.md"], 17);
    }
}

#[test]
fn suffix_menu_navigation_selects_real_rows_and_home_returns_to_plain() {
    let fixture = Fixture::with_suffixes(
        &[("first.md", UNKNOWN), ("plural.md", "Zzyzxqvocabs")],
        &BACKENDS[..1],
    );
    let mut session = fixture.cli("fix", &["first.md"]);
    session.prompt("1/1", "first.md");
    session.send(b"a");
    session.learning_prompt(UNKNOWN, false);
    session.edit_learning_word(UNKNOWN, UNKNOWN);
    assert_eq!(session.plain_fallback_menu(UNKNOWN), ["S"]);
    for (key, index) in [
        (b"\x1b[B", 1),
        (b"\x1b[A", 0),
        (b"\x1b[A", 1),
        (b"\x1b[B", 0),
        (b"\x1b[F", 1),
        (b"\x1b[H", 0),
    ] {
        session.send(key);
        session.suffix_redraw(UNKNOWN, index);
    }
    assert_eq!(session.output().matches(SUFFIX_PROMPT).count(), 1);
    session.send(b"\r");
    session.edit_stem(UNKNOWN, UNKNOWN);
    session.learning_confirmation(UNKNOWN, UNKNOWN);
    assert!(!session.output().contains(STEM_CHOICE_PROMPT));
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    session.send(b"y");
    session.finish(0);
    session.assert_prompt_count(1);
    assert_eq!(fixture.read("first.md"), UNKNOWN);
    assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqvocab\n");
    fixture.assert_config_unchanged();
    fixture.check(&["first.md"], 0);
    fixture.check(&["plural.md"], 17);
}

#[test]
fn plain_fallback_replacement_learning_records_only_the_correction_not_flags_or_typo() {
    let source = "Spelchekr Thirdwrng";
    let fixture = Fixture::new(
        true,
        &[
            ("first.md", source),
            ("stem.md", UNKNOWN),
            ("plural.md", "Zzyzxqvocabs"),
        ],
    );
    let mut session = fixture.cli("fix", &["first.md"]);
    session.prompt("1/2", "first.md");
    session.replace_custom(UNKNOWN);
    session.learning_prompt(UNKNOWN, true);
    session.edit_learning_word(UNKNOWN, UNKNOWN);
    session.select_plain_fallback(UNKNOWN);
    session.send(b"y");
    session.prompt("2/2", "first.md");
    assert_eq!(fixture.read("first.md"), source);
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    session.send(b"q");
    session.finish(17);
    session.assert_prompt_count(2);
    assert_eq!(fixture.read("first.md"), "Zzyzxqvocab Thirdwrng");
    assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqvocab\n");
    fixture.assert_config_unchanged();
    fixture.check(&["stem.md"], 0);
    fixture.check(&["plural.md"], 17);
    fs::write(fixture.root.join("original.md"), "Spelchekr").unwrap();
    fixture.check(&["original.md"], 17);
}

#[test]
fn inferred_suffix_defaults_to_shortened_stem_without_manual_pickers() {
    const WORD: &str = "Zzyzxqroots";
    const STEM: &str = "Zzyzxqroot";
    for backend in BACKENDS {
        for replacement in [false, true] {
            let original = if replacement { "Zzyzxqtypo" } else { WORD };
            let source = format!("{original} {WORD}");
            let fixture = Fixture::with_suffixes(
                &[("first.md", &source), ("later.md", WORD), ("stem.md", STEM)],
                std::slice::from_ref(backend),
            );
            let paths = ["first.md", "later.md"];
            let mut session = fixture.cli("fix", &paths);
            session.prompt("1/2", "first.md");
            if replacement {
                session.replace_custom(WORD);
                session.learning_prompt(WORD, true);
                session.send(b"p");
            } else {
                session.send(b"\x1b[H");
                session.main_picker_redraw(WORD, 0);
                session.send(b"\r");
            }
            assert_eq!(
                session.affix_menu(&[("-", STEM, "/S"), ("-", WORD, "-")], 0, false),
                "Affix expansion: Zzyzxqroot, Zzyzxqroots"
            );
            assert!(!session.output().contains(EDIT_STEM_PROMPT));
            assert!(!session.output().contains(CONFIRM_PROMPT));
            session.send(b"\r");
            session.learning_confirmation("Zzyzxqroot/S", "Zzyzxqroot, Zzyzxqroots");
            assert!(!session.output().contains(EDIT_STEM_PROMPT));
            assert!(!session.output().contains(SUFFIX_PROMPT));
            assert!(!session.output().contains(STEM_CHOICE_PROMPT));
            assert_eq!(fixture.read("first.md"), source);
            assert_eq!(fixture.read("lingo.dic"), "0\n");
            session.send(b"\r");
            session.finish(if replacement { 17 } else { 0 });
            session.assert_prompt_count(1);
            assert_eq!(session.output().matches(AFFIX_PROMPT).count(), 1);
            assert_eq!(fixture.read("first.md"), format!("{WORD} {WORD}"));
            assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqroot/S\n");
            fixture.assert_config_unchanged();
            fixture.check(&paths, 0);
            fixture.check(&["stem.md"], 0);
        }
    }
}

#[test]
fn cycling_all_suffix_bundles_keeps_the_stem_and_refreshes_edited_expansions() {
    const WORD: &str = "Zzyzxqroots";
    const STEM: &str = "Zzyzxqroot";
    const EDITED: &str = "Zzyzxqverb";
    const SOURCE: &str = "Zzyzxqroots Zzyzxqverbed Thirdwrng";
    for backend in BACKENDS {
        let fixture = Fixture::with_affixes(
            &[
                ("first.md", SOURCE),
                ("forms.md", "Zzyzxqverb Zzyzxqverbed"),
                ("original.md", WORD),
                ("plural.md", "Zzyzxqverbs"),
            ],
            std::slice::from_ref(backend),
            "SET UTF-8\nSFX S Y 1\nSFX S 0 s .\nSFX D Y 1\nSFX D 0 ed .\n",
        );
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/3", "first.md");
        session.send(b"\x1b[H");
        session.main_picker_redraw(WORD, 0);
        session.send(b"\r");
        let mut rows = [("-", STEM, "/S"), ("-", WORD, "-")];
        session.affix_menu(&rows, 0, false);
        let initial_strip = session.affix_strip();
        assert_eq!(
            initial_strip.0.split_whitespace().collect::<Vec<_>>(),
            ["-", "/D", "/S"]
        );
        session.command_help(AFFIX_COMMANDS, &[]);
        session.help_prompt(AFFIX_PROMPT);
        session.affix_redraw(&rows, 0, true, false);
        assert_eq!(session.affix_strip(), initial_strip);

        // D cannot generate the original plural, but must still be reachable
        // from inferred S without replacing the selected stem or adding a row.
        session.send(b"\x1b[D");
        rows[0].2 = "/D";
        assert_eq!(
            session.affix_redraw(&rows, 0, false, false),
            "Affix expansion: Zzyzxqroot, Zzyzxqrooted"
        );
        let edited_strip = session.affix_strip();
        assert_ne!(edited_strip.0, initial_strip.0, "Left must shift the strip");
        assert_eq!(edited_strip.1, initial_strip.1, "Caret must stay fixed");
        session.send(b"e");
        session.edit_stem(STEM, EDITED);
        rows[0].1 = EDITED;
        assert_eq!(
            session.affix_menu(&rows, 0, false),
            "Affix expansion: Zzyzxqverb, Zzyzxqverbed"
        );
        assert_eq!(session.affix_strip(), edited_strip);
        assert!(!session.output().contains(CONFIRM_PROMPT));

        // Cancelling an edit must preserve the previously accepted row and flags.
        session.send(b"e");
        session.stem_editor(EDITED);
        session.send(b"x");
        session.expect(&format!("{EDIT_STEM_PROMPT}{EDITED}x"));
        session.send(b"\x1b");
        session.affix_menu(&rows, 0, false);
        assert_eq!(session.affix_strip(), edited_strip);
        // Alphabetical strip order is plain, D, S, with wraparound in both directions.
        for (key, suffix, forms) in [
            (b"\x1b[C".as_slice(), "/S", "Zzyzxqverb, Zzyzxqverbs"),
            (
                b"\x1b[C".as_slice(),
                "-",
                "Zzyzxqverb (plain word; no affixes)",
            ),
            (b"\x1b[C".as_slice(), "/D", "Zzyzxqverb, Zzyzxqverbed"),
            (
                b"\x1b[D".as_slice(),
                "-",
                "Zzyzxqverb (plain word; no affixes)",
            ),
            (b"\x1b[D".as_slice(), "/S", "Zzyzxqverb, Zzyzxqverbs"),
            (b"\x1b[D".as_slice(), "/D", "Zzyzxqverb, Zzyzxqverbed"),
        ] {
            let before = session.affix_strip();
            session.send(key);
            rows[0].2 = suffix;
            assert_eq!(
                session.affix_redraw(&rows, 0, false, false),
                format!("Affix expansion: {forms}")
            );
            let after = session.affix_strip();
            assert_ne!(
                after.0, before.0,
                "Left/Right must shift the strip, including wrap"
            );
            assert_eq!(after.1, before.1, "Caret must stay fixed during cycling");
            assert_eq!(fixture.read("first.md"), SOURCE);
            assert_eq!(fixture.read("lingo.dic"), "0\n");
        }
        assert!(!session.output().contains(CONFIRM_PROMPT));
        session.send(b"\r");
        session.learning_confirmation("Zzyzxqverb/D", "Zzyzxqverb, Zzyzxqverbed");
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"y");
        // The newly expanded form is suppressed in the queued diagnostics, but
        // the confirmed entry is not written until the session is applied.
        session.prompt("3/3", "first.md");
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"q");
        session.finish(0);
        session.assert_prompt_count(2);
        assert!(!session.output().contains(SUFFIX_PROMPT));
        assert!(!session.output().contains(STEM_CHOICE_PROMPT));
        assert_eq!(fixture.read("first.md"), SOURCE);
        assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqverb/D\n");
        fixture.assert_config_unchanged();
        fixture.check(&["forms.md"], 0);
        fixture.check(&["original.md"], 17);
        fixture.check(&["plural.md"], 17);
    }
}

#[test]
fn cycling_catalog_exposes_prefixes_and_only_cross_product_pairs() {
    const WORD: &str = "Zzyzxqroots";
    const STEM: &str = "Zzyzxqroot";
    const FORMS: &str = "Zzyzxqroot, Zzyzxqroots, unZzyzxqroot, unZzyzxqroots";
    for backend in BACKENDS {
        let fixture = Fixture::with_affixes(
            &[
                ("first.md", WORD),
                ("forms.md", "Zzyzxqroot Zzyzxqroots unZzyzxqroot unZzyzxqroots"),
            ],
            std::slice::from_ref(backend),
            "SET UTF-8\nPFX U Y 1\nPFX U 0 un .\nPFX R N 1\nPFX R 0 re .\nSFX S Y 1\nSFX S 0 s .\nSFX G N 1\nSFX G 0 ing .\nSFX D Y 1\nSFX D 0 ed .\n",
        );
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"\x1b[H");
        session.main_picker_redraw(WORD, 0);
        session.send(b"\r");
        let mut rows = [("-", STEM, "/S"), ("-", WORD, "-")];
        // Three columns are required even though no inferred row has a prefix.
        session.affix_menu(&rows, 0, true);
        assert_eq!(
            session
                .affix_strip()
                .0
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["-", "/D", "/G", "/R", "/S", "/U", "/UD", "/US"]
        );
        // Cycling the explicit plain row must also keep its own surface stem,
        // rather than substitute the shorter stem from the inferred row.
        session.send(b"\x1b[A");
        session.affix_redraw(&rows, 1, false, true);
        session.send(b"\x1b[C");
        rows[1].2 = "/D";
        assert_eq!(
            session.affix_redraw(&rows, 1, false, true),
            "Affix expansion: Zzyzxqroots, Zzyzxqrootsed"
        );
        session.send(b"\x1b[D");
        rows[1].2 = "-";
        session.affix_redraw(&rows, 1, false, true);
        session.send(b"\x1b[H");
        session.affix_redraw(&rows, 0, false, true);
        // Global alphabetical bundle order ignores declaration order and prefix
        // grouping. Neither N prefix R nor N suffix G may participate in a pair.
        for (prefix, suffix, forms) in [
            ("/U", "-", "Zzyzxqroot, unZzyzxqroot"),
            (
                "/U",
                "/D",
                "Zzyzxqroot, Zzyzxqrooted, unZzyzxqroot, unZzyzxqrooted",
            ),
            ("/U", "/S", FORMS),
            ("-", "-", "Zzyzxqroot (plain word; no affixes)"),
            ("-", "/D", "Zzyzxqroot, Zzyzxqrooted"),
            ("-", "/G", "Zzyzxqroot, Zzyzxqrooting"),
            ("/R", "-", "Zzyzxqroot, reZzyzxqroot"),
            ("-", "/S", "Zzyzxqroot, Zzyzxqroots"),
        ] {
            session.send(b"\x1b[C");
            rows[0] = (prefix, STEM, suffix);
            assert_eq!(
                session.affix_redraw(&rows, 0, false, true),
                format!("Affix expansion: {forms}")
            );
            assert_eq!(fixture.read("lingo.dic"), "0\n");
        }
        for (prefix, suffix) in [
            ("/R", "-"),
            ("-", "/G"),
            ("-", "/D"),
            ("-", "-"),
            ("/U", "/S"),
        ] {
            session.send(b"\x1b[D");
            rows[0] = (prefix, STEM, suffix);
            session.affix_redraw(&rows, 0, false, true);
        }
        assert_eq!(session.output().matches(AFFIX_PROMPT).count(), 1);
        assert!(!session.output().contains(EDIT_STEM_PROMPT));
        session.send(b"y");
        session.learning_confirmation("Zzyzxqroot/US", FORMS);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"\r");
        session.finish(0);
        assert_eq!(fixture.read("first.md"), WORD);
        assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqroot/US\n");
        fixture.assert_config_unchanged();
        fixture.check(&["forms.md"], 0);
    }
}

#[test]
fn automatic_affix_strip_clips_whole_labels_and_wraps_under_a_fixed_caret() {
    const WORD: &str = "Zzyzxqroots";
    let mut affixes = "SET UTF-8\nPFX U Y 1\nPFX U 0 un .\n".to_owned();
    let mut choices = vec!["-".to_owned(), "/U".to_owned()];
    for flag in 'A'..='T' {
        let suffix = if flag == 'S' {
            "s".to_owned()
        } else {
            format!("zz{flag}")
        };
        affixes.push_str(&format!("SFX {flag} Y 1\nSFX {flag} 0 {suffix} .\n"));
        choices.push(format!("/{flag}"));
        choices.push(format!("/U{flag}"));
    }
    choices.sort_by(|left, right| {
        left.strip_prefix('/')
            .unwrap_or("")
            .cmp(right.strip_prefix('/').unwrap_or(""))
    });
    for backend in BACKENDS {
        let fixture = Fixture::with_affixes(
            &[("first.md", WORD)],
            std::slice::from_ref(backend),
            &affixes,
        );
        for columns in [40, 160] {
            let mut session = fixture.cli_with_columns("fix", &["first.md"], columns);
            session.prompt("1/1", "first.md");
            session.send(b"a");
            session.learning_prompt(WORD, false);
            session.send(b"p");
            session.expect(AFFIX_PROMPT);
            let (_, context) = session.picker_redraw(AFFIX_PROMPT);
            // Alphabetical order does not replace the initial best inferred S.
            assert_affix_strip(&context, "/S", columns);
            let initial = session.affix_strip();
            assert!(initial.0.contains(".."), "{initial:?}");
            let mut previous = initial;
            for (key, selected) in [
                (b"\x1b[D".as_slice(), "/R"),
                (b"\x1b[C".as_slice(), "/S"),
                (b"\x1b[A".as_slice(), "-"),
                (b"\x1b[D".as_slice(), "/UT"),
                (b"\x1b[C".as_slice(), "-"),
                (b"\x1b[C".as_slice(), "/A"),
            ] {
                session.send(key);
                let (_, context) = session.picker_redraw(AFFIX_PROMPT);
                assert_affix_strip(&context, selected, columns);
                let current = session.affix_strip();
                assert_ne!(current.0, previous.0, "Selection must shift the strip");
                assert_eq!(current.1, previous.1, "Caret must remain fixed");
                let tokens = current.0.split_whitespace().collect::<Vec<_>>();
                let visible = tokens
                    .iter()
                    .copied()
                    .filter(|token| *token != "..")
                    .collect::<Vec<_>>();
                let start = choices
                    .iter()
                    .position(|choice| choice == visible[0])
                    .unwrap();
                let end = start + visible.len();
                assert_eq!(
                    visible,
                    choices[start..end],
                    "Only edge labels may be hidden"
                );
                assert_eq!(tokens.first() == Some(&".."), start > 0);
                assert_eq!(tokens.last() == Some(&".."), end < choices.len());
                previous = current;
            }
            session.send(b"n");
            session.prompt("1/1", "first.md");
            session.send(b"q");
            session.finish(0);
            assert_eq!(fixture.read("first.md"), WORD);
            assert_eq!(fixture.read("lingo.dic"), "0\n");
            fixture.assert_config_unchanged();
        }
    }
}

#[test]
fn manual_picker_e_edits_the_selected_entry_without_an_extra_candidate_picker() {
    const WORD: &str = "Zzyzxqroots";
    for stage in ["plain", "suffix", "stem"] {
        let fixture = Fixture::with_suffixes(&[("first.md", WORD)], &BACKENDS[..1]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"a");
        session.learning_prompt(WORD, false);
        session.send(b"p");
        session.plain_fallback_menu(WORD);
        if stage != "plain" {
            session.send(b"\x1b[A");
            session.suffix_redraw(WORD, 1);
        }
        let initial = if stage == "stem" {
            session.send(b"\r");
            session.stem_menu(&["Zzyzxqroot/S", "Zzyzxqroots/S"]);
            "Zzyzxqroot"
        } else {
            WORD
        };
        session.command_help(PICKER_COMMANDS, &[]);
        session.help_prompt(if stage == "stem" {
            STEM_CHOICE_PROMPT
        } else {
            SUFFIX_PROMPT
        });
        if stage == "stem" {
            session.stem_redraw(&["Zzyzxqroot/S", "Zzyzxqroots/S"], 0);
        } else {
            session.suffix_redraw(WORD, usize::from(stage == "suffix"));
        }
        session.send(b"e");
        session.edit_stem(initial, "Zzyzxqmanual");
        if stage == "plain" {
            session.learning_confirmation("Zzyzxqmanual", "Zzyzxqmanual");
        } else {
            session.learning_confirmation("Zzyzxqmanual/S", "Zzyzxqmanual, Zzyzxqmanuals");
        }
        assert_eq!(
            session.output().matches(STEM_CHOICE_PROMPT).count(),
            if stage == "stem" { 2 } else { 0 }
        );
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"y");
        session.finish(0);
        assert_eq!(fixture.read("first.md"), WORD);
        assert_eq!(
            fixture.read("lingo.dic"),
            if stage == "plain" {
                "1\nZzyzxqmanual\n"
            } else {
                "1\nZzyzxqmanual/S\n"
            }
        );
        fixture.assert_config_unchanged();
    }
}

#[test]
fn inferred_prefix_suffix_pair_preserves_selection_help_flags_and_native_reload() {
    const WORD: &str = "unZzyzxqrooted";
    const STEM: &str = "Zzyzxqroot";
    const FORMS: &str = "Zzyzxqroot, Zzyzxqrooted, unZzyzxqroot, unZzyzxqrooted";
    let rows = [
        ("/U", STEM, "/D"),
        ("/U", "Zzyzxqrooted", "-"),
        ("-", "unZzyzxqroot", "/D"),
        ("-", WORD, "-"),
    ];
    for backend in BACKENDS {
        let fixture = Fixture::with_affixes(
            &[
                ("first.md", WORD),
                (
                    "forms.md",
                    "Zzyzxqroot Zzyzxqrooted unZzyzxqroot unZzyzxqrooted",
                ),
            ],
            std::slice::from_ref(backend),
            "SET UTF-8\nPFX U Y 1\nPFX U 0 un .\nSFX D Y 1\nSFX D 0 ed .\n",
        );
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"\x1b[H");
        session.main_picker_redraw(WORD, 0);
        session.send(b"\r");
        assert_eq!(
            session.affix_menu(&rows, 0, true),
            format!("Affix expansion: {FORMS}")
        );
        assert!(!session.output().contains("Learn \""));
        assert!(!session.output().contains(WORD_PROMPT));
        assert!(!session.output().contains(EDIT_STEM_PROMPT));
        assert_eq!(fixture.read("lingo.dic"), "0\n");

        for (key, index) in [
            (b"\x1b[B".as_slice(), 3),
            (b"\x1b[A".as_slice(), 0),
            (b"\x1b[F".as_slice(), 3),
            (b"\x1b[H".as_slice(), 0),
            (b"\x1b[A".as_slice(), 1),
        ] {
            session.send(key);
            session.affix_redraw(&rows, index, false, true);
        }
        let help = session.command_help(AFFIX_COMMANDS, &[]);
        assert!(help.contains("edit stem"), "{help:?}");
        assert!(help.contains("choose affixes"), "{help:?}");
        assert!(help.contains("cancel learning"), "{help:?}");
        session.help_prompt(AFFIX_PROMPT);
        assert_eq!(
            session.affix_redraw(&rows, 1, true, true),
            "Affix expansion: Zzyzxqrooted, unZzyzxqrooted"
        );
        // e edits the selected row; Esc must restore it unchanged, not the
        // default pair or the manual suffix/stem candidate menus.
        session.send(b"e");
        session.stem_editor("Zzyzxqrooted");
        session.send(b"\x1b");
        session.affix_menu(&rows, 1, true);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"\x1b[H");
        session.affix_redraw(&rows, 0, false, true);
        session.send(b"e");
        session.stem_editor(STEM);
        session.command_help(EDITOR_COMMANDS, &["text", "characters"]);
        session.help_prompt(&format!("{EDIT_STEM_PROMPT}{STEM}"));
        session.send(b"\r");
        session.affix_menu(&rows, 0, true);
        assert!(!session.output().contains(CONFIRM_PROMPT));
        session.send(b"y");
        session.learning_confirmation("Zzyzxqroot/UD", FORMS);
        // Re-editing from confirmation must retain both flags, too.
        session.send(b"e");
        session.edit_stem(STEM, STEM);
        session.learning_confirmation("Zzyzxqroot/UD", FORMS);
        session.confirmation_help();
        assert_eq!(fixture.read("first.md"), WORD);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        assert!(!session.output().contains(SUFFIX_PROMPT));
        assert!(!session.output().contains(STEM_CHOICE_PROMPT));
        session.send(b"\r");
        session.finish(0);
        session.assert_prompt_count(1);
        assert_eq!(fixture.read("first.md"), WORD);
        assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqroot/UD\n");
        fixture.assert_config_unchanged();
        // A new process checks every form through the native backend, rather
        // than relying on the interactive session's accepted-forms overlay.
        fixture.check(&["forms.md"], 0);
    }
}

#[test]
fn inferred_prefix_only_uses_three_columns_and_highlights_the_added_prefix() {
    const WORD: &str = "unZzyzxqroot";
    const STEM: &str = "Zzyzxqroot";
    for backend in BACKENDS {
        let fixture = Fixture::with_affixes(
            &[("first.md", WORD), ("forms.md", "Zzyzxqroot unZzyzxqroot")],
            std::slice::from_ref(backend),
            "SET UTF-8\nPFX U Y 1\nPFX U 0 un .\n",
        );
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"\x1b[H");
        session.main_picker_redraw(WORD, 0);
        session.send(b"\r");
        assert_eq!(
            session.affix_menu(&[("/U", STEM, "-"), ("-", WORD, "-")], 0, true),
            "Affix expansion: Zzyzxqroot, unZzyzxqroot"
        );
        session.send(b"\r");
        session.learning_confirmation("Zzyzxqroot/U", "Zzyzxqroot, unZzyzxqroot");
        assert!(!session.output().contains(EDIT_STEM_PROMPT));
        assert!(!session.output().contains(SUFFIX_PROMPT));
        assert!(!session.output().contains(STEM_CHOICE_PROMPT));
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"\r");
        session.finish(0);
        assert_eq!(fixture.read("first.md"), WORD);
        assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqroot/U\n");
        fixture.assert_config_unchanged();
        fixture.check(&["forms.md"], 0);
    }
}

#[test]
fn real_suffix_learning_suggests_shortened_stem_persists_flags_and_reloads_expansion() {
    const WORD: &str = "Zzyzxqroots";
    const STEM: &str = "Zzyzxqroot";
    let entries = ["Zzyzxqroot/S", "Zzyzxqroots/S"];
    for backend in BACKENDS {
        for replacement in [false, true] {
            let original = if replacement { "Zzyzxqtypo" } else { WORD };
            let source = format!("{original} {WORD}");
            let fixture = Fixture::with_suffixes(
                &[
                    ("first.md", &source),
                    ("later.md", WORD),
                    ("typo.md", "Zzyzxqtypo"),
                ],
                std::slice::from_ref(backend),
            );
            let paths = ["first.md", "later.md"];
            let mut session = fixture.cli("fix", &paths);
            session.prompt("1/2", "first.md");
            if replacement {
                session.replace_custom(WORD);
                session.learning_prompt(WORD, true);
                session.send(b"\r");
            } else {
                session.send(b"\x1b[H");
                session.main_picker_redraw(WORD, 0);
                session.send(b"\r");
            }
            assert_eq!(session.plain_fallback_menu(WORD), ["S"]);
            session.send(b"\x1b[A");
            session.suffix_redraw(WORD, 1);
            session.send(b"\r");
            assert_eq!(
                session.stem_menu(&entries),
                "Affix expansion: Zzyzxqroot, Zzyzxqroots"
            );
            // The shortened root is first; the unchanged surface remains last.
            session.send(b"\x1b[A");
            assert_eq!(
                session.stem_redraw(&entries, 1),
                "Affix expansion: Zzyzxqroots, Zzyzxqrootss"
            );
            session.send(b"\x1b[B");
            session.stem_redraw(&entries, 0);
            session.send(b"\r");
            session.edit_stem(STEM, STEM);
            session.learning_confirmation("Zzyzxqroot/S", "Zzyzxqroot, Zzyzxqroots");
            assert_eq!(fixture.read("first.md"), source);
            assert_eq!(fixture.read("lingo.dic"), "0\n");
            session.send(b"\r");
            // Full expansions suppress both queued plurals and later sources.
            session.finish(if replacement { 17 } else { 0 });
            session.assert_prompt_count(1);
            assert_eq!(fixture.read("first.md"), format!("{WORD} {WORD}"));
            assert_eq!(fixture.read("later.md"), WORD);
            assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqroot/S\n");
            fixture.assert_config_unchanged();
            fixture.check(&paths, 0);
            fs::write(fixture.root.join("stem.md"), STEM).unwrap();
            fixture.check(&["stem.md"], 0);
            fixture.check(&["typo.md"], 17);
        }
    }
}

#[test]
fn picker_context_is_one_truncated_row_but_confirmation_shows_the_full_expansion() {
    let stem = format!("Zzyzxq{}", "a".repeat(90));
    let word = format!("{stem}s");
    let entry = format!("{stem}/S");
    let original_entry = format!("{word}/S");
    let fixture = Fixture::with_suffixes(&[("first.md", &word)], &BACKENDS[..1]);
    let mut session = fixture.cli("fix", &["first.md"]);
    session.prompt("1/1", "first.md");
    session.send(b"\x1b[H");
    session.main_picker_redraw(&word, 0);
    session.send(b"\r");
    session.plain_fallback_menu(&word);
    session.send(b"\x1b[A");
    session.suffix_redraw(&word, 1);
    session.send(b"\r");
    let context = session.stem_menu(&[&entry, &original_entry]);
    assert!(context.ends_with('…'), "{context}");
    session.send(b"\r");
    session.edit_stem(&stem, &stem);
    session.learning_confirmation(&entry, &format!("{stem}, {word}"));
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    session.send(b"y");
    session.finish(0);
    assert_eq!(fixture.read("first.md"), word);
    assert_eq!(fixture.read("lingo.dic"), format!("1\n{entry}\n"));
    fixture.assert_config_unchanged();
    fixture.check(&["first.md"], 0);
}

#[test]
fn flagged_stem_can_be_edited_again_and_only_the_confirmed_entry_is_saved() {
    for replacement in [false, true] {
        let original = if replacement {
            "Zzyzxqtypo"
        } else {
            "Zzyzxqroots"
        };
        let fixture = Fixture::with_suffixes(
            &[("first.md", original), ("plural.md", "Zzyzxqediteds")],
            &BACKENDS[..1],
        );
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        if replacement {
            session.replace_custom("Zzyzxqroots");
            session.learning_prompt("Zzyzxqroots", true);
        } else {
            session.send(b"a");
            session.learning_prompt(original, false);
        }
        session.edit_learning_word("Zzyzxqroots", "Zzyzxqroots");
        session.plain_fallback_menu("Zzyzxqroots");
        session.send(b"\x1b[A");
        session.suffix_redraw("Zzyzxqroots", 1);
        session.send(b"\r");
        session.stem_menu(&["Zzyzxqroot/S", "Zzyzxqroots/S"]);
        session.send(b"\r");
        session.edit_stem("Zzyzxqroot", "Zzyzxqbase");
        session.learning_confirmation("Zzyzxqbase/S", "Zzyzxqbase, Zzyzxqbases");
        // Editing from confirmation retains the selected suffix, not a literal /S
        // in the plain-word editor, and requires a fresh full-expansion preview.
        session.send(b"e");
        session.edit_stem("Zzyzxqbase", "Zzyzxqedited");
        session.learning_confirmation("Zzyzxqedited/S", "Zzyzxqedited, Zzyzxqediteds");
        assert_eq!(fixture.read("first.md"), original);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"y");
        session.finish(if replacement { 17 } else { 0 });
        assert_eq!(fixture.read("first.md"), "Zzyzxqroots");
        assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqedited/S\n");
        fixture.assert_config_unchanged();
        fixture.check(&["plural.md"], 0);
        fixture.check(&["first.md"], 17);
    }
}

#[test]
fn flagged_picker_backtracking_and_suffix_cancellation_preserve_replacement_only() {
    for cancel in [b"n".as_slice(), b"\x1b".as_slice()] {
        let fixture = Fixture::with_suffixes(&[("first.md", "Zzyzxqtypo")], &BACKENDS[..1]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.replace_custom("Zzyzxqroots");
        session.learning_prompt("Zzyzxqroots", true);
        session.send(b"p");
        session.plain_fallback_menu("Zzyzxqroots");
        session.send(b"\x1b[A");
        session.suffix_redraw("Zzyzxqroots", 1);
        session.send(b"\r");
        let entries = ["Zzyzxqroot/S", "Zzyzxqroots/S"];
        session.stem_menu(&entries);
        session.send(b"\r");
        session.stem_editor("Zzyzxqroot");
        session.send(b"\x1b");
        session.stem_menu(&entries);
        session.send(cancel);
        // Returning from stems may retain the suffix selection. Home must select
        // plain regardless, and cancelling here skips learning, not the edit.
        session.expect(SUFFIX_PROMPT);
        session.picker_redraw(SUFFIX_PROMPT);
        session.send(b"\x1b[H");
        session.suffix_redraw("Zzyzxqroots", 0);
        session.send(cancel);
        session.finish(17);
        session.assert_prompt_count(1);
        assert_eq!(fixture.read("first.md"), "Zzyzxqroots");
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        fixture.assert_config_unchanged();
        fixture.check(&["first.md"], 17);
    }
}

#[test]
fn ctrl_c_in_flagged_pickers_discards_prior_learning_and_source_edits() {
    for stage in ["stems", "stem-editor", "confirmation"] {
        let source = "Zzyzxqvocab Zzyzxqtypo";
        let fixture = Fixture::with_suffixes(&[("first.md", source)], &BACKENDS[..1]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/2", "first.md");
        session.accept_plain(UNKNOWN);
        session.prompt("2/2", "first.md");
        session.replace_custom("Zzyzxqroots");
        session.learning_prompt("Zzyzxqroots", true);
        session.send(b"\r");
        session.plain_fallback_menu("Zzyzxqroots");
        session.send(b"\x1b[A");
        session.suffix_redraw("Zzyzxqroots", 1);
        session.send(b"\r");
        session.stem_menu(&["Zzyzxqroot/S", "Zzyzxqroots/S"]);
        if stage != "stems" {
            session.send(b"\r");
            if stage == "stem-editor" {
                session.stem_editor("Zzyzxqroot");
            } else {
                session.edit_stem("Zzyzxqroot", "Zzyzxqroot");
                session.learning_confirmation("Zzyzxqroot/S", "Zzyzxqroot, Zzyzxqroots");
            }
        }
        assert_eq!(fixture.read("first.md"), source);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"\x03");
        session.finish(130);
        session.assert_prompt_count(2);
        assert_eq!(fixture.read("first.md"), source);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        fixture.assert_config_unchanged();
    }
}

#[test]
fn builtin_suffix_options_still_allow_explicit_plain_learning() {
    for backend in BACKENDS {
        for persistent in [false, true] {
            // Plain learning remains an explicit choice alongside real builtin suffixes.
            let fixture = Fixture::with_backends(
                persistent,
                &[("first.md", UNKNOWN), ("plural.md", "Zzyzxqvocabs")],
                std::slice::from_ref(backend),
            );
            let mut session = fixture.cli("fix", &["first.md"]);
            session.prompt("1/1", "first.md");
            session.send(b"a");
            session.learning_prompt(UNKNOWN, false);
            session.edit_learning_word(UNKNOWN, UNKNOWN);
            let flags = session.plain_fallback_menu(UNKNOWN);
            assert!(flags.iter().any(|flag| flag == "S"), "{flags:?}");
            session.send(b"\r");
            session.edit_stem(UNKNOWN, UNKNOWN);
            session.learning_confirmation(UNKNOWN, UNKNOWN);
            assert_eq!(fixture.read("first.md"), UNKNOWN);
            assert_eq!(fixture.read("lingo.dic"), "0\n");
            session.send(b"y");
            session.finish(0);
            session.assert_prompt_count(1);
            assert_eq!(fixture.read("first.md"), UNKNOWN);
            fixture.assert_learning_saved(persistent, "1\nZzyzxqvocab\n");
            fixture.assert_config_unchanged();
            fixture.check(&["first.md"], 0);
            fixture.check(&["plural.md"], 17);
        }
    }
}

#[test]
fn replacement_learning_uses_session_overlay_for_plain_words_but_not_unlearned_plurals() {
    for backend in BACKENDS {
        for (use_editor, second_replacement) in
            [(false, UNKNOWN), (true, UNKNOWN), (true, "Zzyzxqvocabs")]
        {
            let source = "Spelchekr Thirdwrng";
            let fixture = Fixture::with_backends(
                true,
                &[("first.md", source)],
                std::slice::from_ref(backend),
            );
            let mut session = fixture.cli("fix", &["first.md"]);
            session.prompt("1/2", "first.md");
            session.replace_custom(UNKNOWN);
            session.learning_prompt(UNKNOWN, true);
            if use_editor {
                session.edit_learning_word(UNKNOWN, UNKNOWN);
            } else {
                session.send(b"p");
            }
            session.select_plain_fallback(UNKNOWN);
            session.send(b"y");
            session.prompt("2/2", "first.md");
            // Neither the source nor the dictionary has been applied: only the
            // session overlay can recognize the learned stem, never its plural.
            assert_eq!(fixture.read("first.md"), source);
            assert_eq!(fixture.read("lingo.dic"), "0\n");
            session.replace_custom(second_replacement);
            let is_plural = second_replacement != UNKNOWN;
            if is_plural {
                // The unlearned plural still needs an optional followup. Skipping
                // it must not automatically add it to the pending dictionary.
                session.learning_prompt(second_replacement, true);
                session.send(b"n");
            }
            // A redundant prompt for the stem would block this finite deadline.
            session.finish(17);
            session.assert_prompt_count(2);
            assert_eq!(
                session.output().matches("Learn replacement ").count(),
                if is_plural { 2 } else { 1 }
            );
            assert_eq!(
                fixture.read("first.md"),
                format!("{UNKNOWN} {second_replacement}")
            );
            assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqvocab\n");
            fixture.assert_config_unchanged();
            fixture.check(&["first.md"], if is_plural { 17 } else { 0 });
        }
    }
}

#[test]
fn learning_word_editor_can_preedit_an_arbitrary_plain_stem_without_learning_its_plural() {
    let stem = "Zzyzxqbase";
    let fixture = Fixture::new(
        true,
        &[
            ("first.md", UNKNOWN),
            ("stem.md", stem),
            ("plural.md", "Zzyzxqbases"),
        ],
    );
    let mut session = fixture.cli("fix", &["first.md"]);
    session.prompt("1/1", "first.md");
    session.send(b"a");
    session.learning_prompt(UNKNOWN, false);
    session.edit_learning_word(UNKNOWN, stem);
    session.select_plain_fallback(stem);
    assert_eq!(fixture.read("first.md"), UNKNOWN);
    assert_eq!(fixture.read("lingo.dic"), "0\n");
    session.send(b"y");
    // No source patch was selected, so fix succeeds even though the unchanged
    // original remains a spelling mistake on the subsequent check.
    session.finish(0);
    session.assert_prompt_count(1);
    assert_eq!(fixture.read("first.md"), UNKNOWN);
    assert_eq!(fixture.read("lingo.dic"), "1\nZzyzxqbase\n");
    fixture.assert_config_unchanged();
    fixture.check(&["stem.md"], 0);
    fixture.check(&["plural.md"], 17);
    fixture.check(&["first.md"], 17);
}

#[test]
fn learning_editor_handles_unicode_and_plain_entry_requires_explicit_confirmation() {
    for confirm in [b"\r", b"n", b"\x1b", b"y"] {
        let fixture = Fixture::new(true, &[("first.md", UNKNOWN)]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"a");
        session.learning_prompt(UNKNOWN, false);
        session.send(b"e");
        session.expect(&format!("{WORD_PROMPT}{UNKNOWN}"));
        session.send("naïve🦀".as_bytes());
        session.expect(&format!("{WORD_PROMPT}{UNKNOWN}naïve🦀"));
        session.send(b"\x7f");
        session.expect(&format!("{WORD_PROMPT}{UNKNOWN}naïve"));
        session.send(b"\x15");
        session.expect(WORD_PROMPT);
        session.send(UNKNOWN.as_bytes());
        session.expect(&format!("{WORD_PROMPT}{UNKNOWN}"));
        session.send(b"\r");
        session.select_plain_fallback(UNKNOWN);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(confirm);
        let yes = confirm == b"y" || confirm == b"\r";
        if !yes {
            session.prompt("1/1", "first.md");
            session.send(b"q");
        }
        session.finish(0);
        session.assert_prompt_count(if yes { 1 } else { 2 });
        assert_eq!(fixture.read("first.md"), UNKNOWN);
        assert_eq!(
            fixture.read("lingo.dic"),
            if yes { "1\nZzyzxqvocab\n" } else { "0\n" }
        );
        fixture.assert_config_unchanged();
    }
}

#[test]
fn cancelling_acceptance_or_backing_out_of_the_editor_does_not_learn() {
    for stage in [
        "followup-n",
        "followup-esc",
        "word",
        "affixes-n",
        "affixes-esc",
        "automatic-plain-stem",
        "suffix-n",
        "suffix-esc",
        "plain-stem",
    ] {
        let fixture = Fixture::new(true, &[("first.md", UNKNOWN)]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/1", "first.md");
        session.send(b"a");
        session.learning_prompt(UNKNOWN, false);
        match stage {
            "followup-n" => session.send(b"n"),
            "followup-esc" => session.send(b"\x1b"),
            "affixes-n" | "affixes-esc" | "automatic-plain-stem" => {
                session.send(b"\r");
                let rows = [("-", UNKNOWN, "-")];
                session.affix_menu(&rows, 0, true);
                if stage == "automatic-plain-stem" {
                    session.send(b"e");
                    session.stem_editor(UNKNOWN);
                    session.send(b"\x1b");
                    session.affix_menu(&rows, 0, true);
                }
                session.send(if stage == "affixes-n" { b"n" } else { b"\x1b" });
            }
            _ => {
                if stage == "word" {
                    session.send(b"e");
                    session.expect(&format!("{WORD_PROMPT}{UNKNOWN}"));
                    session.send(b"\x1b");
                } else {
                    session.send(b"\r");
                    session.plain_fallback_menu(UNKNOWN);
                    if stage == "plain-stem" {
                        session.send(b"\r");
                        session.stem_editor(UNKNOWN);
                        session.send(b"\x1b");
                        session.plain_fallback_menu(UNKNOWN);
                    }
                    session.send(if stage == "suffix-n" { b"n" } else { b"\x1b" });
                }
            }
        }
        session.prompt("1/1", "first.md");
        session.send(b"q");
        session.finish(0);
        session.assert_prompt_count(2);
        assert_eq!(fixture.read("first.md"), UNKNOWN);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        fixture.assert_config_unchanged();
    }
}

#[test]
fn ctrl_c_at_each_learning_stage_discards_pending_source_edits_and_plain_entries() {
    for stage in [
        "replacement-followup",
        "followup",
        "word",
        "affixes",
        "automatic-stem-editor",
        "automatic-confirmation",
        "suffix",
        "stem-editor",
        "confirmation",
    ] {
        let fixture = Fixture::new(true, &[("first.md", MISTAKES)]);
        let mut session = fixture.cli("fix", &["first.md"]);
        session.prompt("1/3", "first.md");
        session.accept_plain(UNKNOWN);
        session.prompt("2/3", "first.md");
        session.replace_custom("Zzyzxqcorrection");
        session.learning_prompt("Zzyzxqcorrection", true);
        if stage != "replacement-followup" {
            session.send(b"n");
            session.prompt("3/3", "first.md");
            session.send(b"a");
            session.learning_prompt("Thirdwrng", false);
            match stage {
                "word" => {
                    session.send(b"e");
                    session.expect(&format!("{WORD_PROMPT}Thirdwrng"));
                }
                "affixes" | "automatic-stem-editor" | "automatic-confirmation" => {
                    session.send(b"\r");
                    session.affix_menu(&[("-", "Thirdwrng", "-")], 0, true);
                    if stage == "automatic-stem-editor" {
                        session.send(b"e");
                        session.stem_editor("Thirdwrng");
                    } else if stage == "automatic-confirmation" {
                        session.send(b"\r");
                        session.learning_confirmation("Thirdwrng", "Thirdwrng");
                    }
                }
                "suffix" | "stem-editor" | "confirmation" => {
                    session.edit_learning_word("Thirdwrng", "Thirdwrng");
                    if stage == "confirmation" {
                        session.select_plain_fallback("Thirdwrng");
                    } else {
                        session.plain_fallback_menu("Thirdwrng");
                        if stage == "stem-editor" {
                            session.send(b"\r");
                            session.stem_editor("Thirdwrng");
                        }
                    }
                }
                _ => {}
            }
        }
        assert_eq!(fixture.read("first.md"), MISTAKES);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        session.send(b"\x03");
        session.finish(130);
        session.assert_prompt_count(if stage == "replacement-followup" {
            2
        } else {
            3
        });
        assert_eq!(fixture.read("first.md"), MISTAKES);
        assert_eq!(fixture.read("lingo.dic"), "0\n");
        fixture.assert_config_unchanged();
    }
}
