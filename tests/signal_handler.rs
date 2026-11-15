#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use nix::sys::signal::{kill, pthread_sigmask, SigSet, SigmaskHow, Signal};
use nix::unistd::Pid;

use cargo_spellcheck::{signal_handler, TinHat};

const SCENARIO_ENV: &str = "CARGO_SPELLCHECK_SIGNAL_TEST_SCENARIO";
const EVENT_PREFIX: &str = "spellcheck-signal-test:";
const CHILD_TIMEOUT: Duration = Duration::from_secs(10);
const QUIET_PERIOD: Duration = Duration::from_millis(200);

struct Subprocess {
    child: Child,
    events: mpsc::Receiver<String>,
    deadline: Instant,
}

impl Subprocess {
    fn spawn(scenario: &str) -> Self {
        let deadline = Instant::now() + CHILD_TIMEOUT;
        let mut child = Command::new(std::env::current_exe().expect("locate test executable"))
            .args([
                "--exact",
                "signal_handler_subprocess",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(SCENARIO_ENV, scenario)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn signal-handler test subprocess");
        let stdout = child.stdout.take().expect("child stdout");
        let (tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.expect("read subprocess stdout");
                // libtest may put its test-name prefix on the same line.
                if let Some((_, event)) = line.split_once(EVENT_PREFIX) {
                    if tx.send(event.to_owned()).is_err() {
                        break;
                    }
                }
            }
        });
        let mut subprocess = Self {
            child,
            events,
            deadline,
        };
        subprocess.expect_event("ready");
        subprocess
    }

    fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    fn expect_event(&mut self, expected: &str) {
        let event = self.events.recv_timeout(self.remaining());
        assert_eq!(
            event.as_deref(),
            Ok(expected),
            "subprocess did not report {expected:?} before its deadline"
        );
    }

    fn expect_quiet(&mut self) {
        assert!(self.remaining() >= QUIET_PERIOD, "subprocess timed out");
        assert_eq!(
            self.events.recv_timeout(QUIET_PERIOD),
            Err(mpsc::RecvTimeoutError::Timeout),
            "callback ran while an existing TinHat was still active"
        );
    }

    fn command(&mut self, command: &str) {
        let stdin = self.child.stdin.as_mut().expect("child stdin");
        writeln!(stdin, "{command}").expect("write subprocess command");
        stdin.flush().expect("flush subprocess command");
    }

    fn signal(&self, signal: Signal) {
        kill(Pid::from_raw(self.child.id() as i32), signal).expect("signal subprocess");
    }

    fn expect_exit_130(&mut self) {
        loop {
            if let Some(status) = self.child.try_wait().expect("poll subprocess exit") {
                assert_eq!(status.code(), Some(130), "unexpected exit status: {status}");
                return;
            }
            let remaining = self.remaining();
            assert!(
                !remaining.is_zero(),
                "subprocess did not exit before its deadline"
            );
            // Polling only bounds process lifetime; scenario synchronization uses pipes.
            std::thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }
}

impl Drop for Subprocess {
    fn drop(&mut self) {
        // Also reap a hung child when an assertion or protocol operation fails.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn signal_waits_for_existing_guards() {
    let mut child = Subprocess::spawn("guards");
    child.signal(Signal::SIGQUIT);
    child.expect_quiet();

    child.command("drop-first");
    child.expect_event("one-guard-remains");
    child.expect_quiet();

    child.command("complete");
    child.expect_event("callback-entered");
    child.expect_event("callback-complete");
    child.expect_exit_130();
}

#[test]
fn callback_can_request_nested_tinhats() {
    let mut child = Subprocess::spawn("reentry");
    child.signal(Signal::SIGQUIT);
    child.expect_event("callback-entered");
    child.expect_event("callback-complete");
    child.expect_exit_130();
}

#[test]
fn signal_exits_with_code_130() {
    let mut child = Subprocess::spawn("exit");
    child.signal(Signal::SIGINT);
    child.expect_event("callback-entered");
    child.expect_event("callback-complete");
    child.expect_exit_130();
}

#[test]
fn repeated_signals_during_callback_exit_normally() {
    let mut child = Subprocess::spawn("repeated");
    child.signal(Signal::SIGQUIT);
    child.expect_event("callback-entered");

    // The callback cannot return until the parent releases it, so every signal
    // here is sent during shutdown rather than racing process termination.
    for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGQUIT] {
        child.signal(signal);
    }
    child.command("finish-callback");
    child.expect_event("callback-complete");
    child.expect_exit_130();
}

fn emit(event: &str) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{EVENT_PREFIX}{event}").expect("write subprocess event");
    stdout.flush().expect("flush subprocess event");
}

fn expect_command(expected: &str) {
    let mut command = String::new();
    std::io::stdin()
        .read_line(&mut command)
        .expect("read parent command");
    assert_eq!(command.trim_end(), expected, "unexpected parent command");
}

#[test]
fn signal_handler_subprocess() {
    let Ok(scenario) = std::env::var(SCENARIO_ENV) else {
        // The normal harness runs this test too, but only subprocesses install
        // a process-global signal handler or exit the process.
        return;
    };
    assert!(matches!(
        scenario.as_str(),
        "guards" | "reentry" | "exit" | "repeated"
    ));

    let mut signals = SigSet::empty();
    for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGQUIT] {
        signals.add(signal);
    }
    pthread_sigmask(SigmaskHow::SIG_UNBLOCK, Some(&signals), None)
        .expect("unblock test signals before spawning the handler");

    let guards = (scenario == "guards").then(|| (TinHat::on(), TinHat::on()));
    let completed = Arc::new(AtomicBool::new(false));
    let callback_completed = Arc::clone(&completed);
    let (finish_tx, finish_rx) = mpsc::channel();
    let check_guards = scenario == "guards";
    let check_reentry = scenario == "reentry";
    let repeated = scenario == "repeated";
    signal_handler(move || {
        emit("callback-entered");
        if check_guards {
            assert!(
                callback_completed.load(Ordering::Acquire),
                "callback ran before protected work completed"
            );
        }
        if check_reentry {
            let outer = TinHat::on();
            let inner = TinHat::on();
            drop(inner);
            drop(outer);
        }
        if repeated {
            finish_rx.recv().expect("parent releases callback");
        }
        emit("callback-complete");
    });
    emit("ready");

    if let Some((first, last)) = guards {
        expect_command("drop-first");
        drop(first);
        emit("one-guard-remains");
        expect_command("complete");
        completed.store(true, Ordering::Release);
        drop(last);
    } else if repeated {
        expect_command("finish-callback");
        finish_tx.send(()).expect("release callback");
    }

    // Returning would let libtest exit successfully without exercising shutdown.
    // The parent kills and reaps this process if the handler fails to exit it.
    loop {
        std::thread::park();
    }
}
