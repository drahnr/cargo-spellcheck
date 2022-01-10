#![cfg(target_os = "linux")]

use std::sync::atomic::Ordering;

use nix::sys::signal::*;
use nix::sys::wait::*;
use nix::unistd::Pid;
use nix::unistd::{fork, ForkResult};

use cargo_spellcheck::{signal_handler, WRITE_IN_PROGRESS};

#[test]
fn signal_handler_works() -> Result<(), Box<dyn std::error::Error + 'static>> {
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    println!("Signal handler check");

    const QUIT: Signal = Signal::SIGQUIT;

    let sigs = {
        let mut sigs = SigSet::empty();
        sigs.add(QUIT);
        sigs
    };

    // best effort unblock
    let _ = sigprocmask(SigmaskHow::SIG_UNBLOCK, Some(&sigs), None);
    let _ = pthread_sigmask(SigmaskHow::SIG_UNBLOCK, Some(&sigs), None);

    if let Ok(ForkResult::Parent { child, .. }) = unsafe { fork() } {
        println!("[parent] Wait for child");

        loop {
            let options = WaitPidFlag::WNOHANG;
            match nix::sys::wait::waitpid(child, Some(options)) {
                Ok(WaitStatus::StillAlive) => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    continue;
                }
                Ok(WaitStatus::Signaled(_pid, signal, _core_dump)) => {
                    assert_eq!(signal, QUIT);
                    unreachable!("Should exit via exit. qed")
                }
                Ok(WaitStatus::Exited(_pid, _exit_code)) => {
                    return Ok(());
                }
                Ok(ws) => unreachable!("Unexpected wait status: {:?}", ws),
                Err(errno) => {
                    unreachable!("Did not expect an error: {:?}", errno);
                }
            }
        }
    } else {
        signal_handler();

        // signal while in a lock
        dbg!(WRITE_IN_PROGRESS.load(Ordering::Acquire));

        WRITE_IN_PROGRESS.fetch_add(1, Ordering::Release);
        println!("[child] Raise signal");

        kill(Pid::this(), QUIT).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(1));

        WRITE_IN_PROGRESS.fetch_sub(1, Ordering::Release);
        dbg!(WRITE_IN_PROGRESS.load(Ordering::Acquire));

        std::thread::sleep(std::time::Duration::from_secs(10_000));
        unreachable!("[child] Signal handler exits before panic.");
    }
}
