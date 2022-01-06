#![cfg(target_os = "linux")]

use std::sync::atomic::Ordering;

use nix::sys::signal::*;
use nix::sys::wait::*;
use nix::unistd::Pid;
use nix::unistd::{fork, ForkResult};

use cargo_spellcheck::{signal_handler, WRITE_IN_PROGRESS};

extern "C" fn bare(x: i32) {
    println!("waiting 4 WIRTE_IN_PROGRES! {}", x);
    let mut i = 0;
    while WRITE_IN_PROGRESS.load(Ordering::Acquire) == 0 {
        std::thread::sleep(std::time::Duration::from_nanos(100));
        i += 1;
        if i > 100_000 {
            break;
        }
        std::hint::spin_loop();
    }
    println!("rdy after {} iterations! {}", i, x);
    signal_hook::low_level::exit(130);
}

#[test]
fn main() {
    println!("Signal handler check");

    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    const QUIT: Signal = Signal::SIGQUIT;

    let sigs = {
        let mut sigs = SigSet::empty();
        sigs.add(QUIT);
        sigs
    };

    sigprocmask(SigmaskHow::SIG_UNBLOCK, Some(&sigs), None).unwrap();
    pthread_sigmask(SigmaskHow::SIG_UNBLOCK, Some(&sigs), None)
        .expect("Must be able to unblock signals");

    if let Ok(ForkResult::Parent { child, .. }) = unsafe { fork() } {
        println!("[parent] Wait for child");

        loop {
            let options = WaitPidFlag::WNOHANG;
            let mut status = 0i32;
            unsafe {
                libc::waitpid(
                    child.as_raw(),
                    &mut status as *mut libc::c_int,
                    options.bits() as libc::c_int,
                )
            };
            if status == -1 {
                panic!("fuck");
            } else if status == 0 {
                // running
            } else if libc::WIFSIGNALED(status) {
                let signal = libc::WTERMSIG(status);
                println!("signaled: {}", signal);
            } else if libc::WIFEXITED(status) {
                let exit_code = dbg!(libc::WEXITSTATUS(status));
                println!("exited: {}", exit_code);
                assert_eq!(exit_code, 130);
                break;
            } else {
                println!("shrug");
            }
            // match dbg!(status) {
            //     Ok(WaitStatus::StillAlive) => {
            //         std::thread::sleep(std::time::Duration::from_millis(200));
            //         continue;
            //     }
            //     Ok(WaitStatus::Signaled(_pid, signal, _core_dump)) => {
            //         assert_eq!(signal, QUIT);
            //     }
            //     Ok(WaitStatus::Exited(_pid, exit_code)) => {
            //         break;
            //     }
            //     Ok(ws) => unreachable!("Unexpected wait status: {:?}", ws),
            //     Err(e) => {
            //         dbg!(e);
            //         break;
            //     },
            // }
        }
    } else {
        let action = SigAction::new(SigHandler::Handler(bare), SaFlags::SA_RESTART, sigs);

        let id = unsafe { sigaction(QUIT, &action).unwrap() };

        dbg!(WRITE_IN_PROGRESS.load(Ordering::Acquire));

        WRITE_IN_PROGRESS.fetch_add(1, Ordering::Release);
        println!("[child] Raise signal");

        // signal_hook::low_level::raise(QUIT as i32).unwrap();
        kill(Pid::this(), QUIT).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(1));

        WRITE_IN_PROGRESS.fetch_sub(1, Ordering::Release);
        dbg!(WRITE_IN_PROGRESS.load(Ordering::Acquire));

        std::thread::sleep(std::time::Duration::from_secs(10_000));
        unreachable!("[child] Signal handler exits before panic.");
    }
}
