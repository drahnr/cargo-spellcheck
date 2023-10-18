//! Tinhat
//!
//! Makes sure the cosmic signals don't meddle with IO that's in progress.
//!
//! ```
//! # use cargo_spellcheck::TinHat;
//! let th = TinHat::on();
//! // do IO
//! drop(th);
//! ```

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

#[cfg(not(target_os = "windows"))]
use signal_hook::{
    consts::signal::{SIGINT, SIGQUIT, SIGTERM},
    iterator,
};

/// Global atomic to block signal processing while a file write is currently in
/// progress.
static WRITE_IN_PROGRESS: AtomicU16 = AtomicU16::new(0);
/// Delay if the signal handler is currently running.
static SIGNAL_HANDLER_AT_WORK: AtomicBool = AtomicBool::new(false);

/// Handle incoming signals.
///
/// Only relevant for *-nix platforms.
#[cfg(not(target_os = "windows"))]
pub fn signal_handler<F>(fx: F)
where
    F: FnOnce() -> () + Send + 'static,
{
    let mut signals =
        iterator::Signals::new(&[SIGTERM, SIGINT, SIGQUIT]).expect("Failed to create Signals");

    std::thread::spawn(move || {
        for s in signals.forever() {
            match s {
                SIGTERM | SIGINT | SIGQUIT => {
                    SIGNAL_HANDLER_AT_WORK.store(true, Ordering::SeqCst);
                    // Wait for potential writing to disk to be finished.
                    while WRITE_IN_PROGRESS.load(Ordering::Acquire) > 0 {
                        std::hint::spin_loop();
                        std::thread::yield_now();
                    }
                    fx();
                    signal_hook::low_level::exit(130);
                }
                sig => log::warn!("Received unhandled signal {sig}, ignoring"),
            }
        }
    });
}

/// Blocks (UNIX) signals.
pub struct TinHat;

impl TinHat {
    /// Put the tin hat on, and only allow signals being processed once it's
    /// dropped.
    pub fn on() -> Self {
        // If there is a signal handler in progress, block.
        while SIGNAL_HANDLER_AT_WORK.load(Ordering::Acquire) {
            std::hint::spin_loop();
            std::thread::yield_now();
        }
        let _ = WRITE_IN_PROGRESS.fetch_add(1, Ordering::Release);
        Self
    }
}

impl Drop for TinHat {
    fn drop(&mut self) {
        let _ = WRITE_IN_PROGRESS.fetch_sub(1, Ordering::Release);
    }
}
