use std::convert::TryInto;
use std::sync::atomic::Ordering;

use cargo_spellcheck::{signal_handler, WRITE_IN_PROGRESS};

#[cfg(not(target_os = "windows"))]
fn main() {
    let _signalthread = std::thread::spawn(signal_handler);
    use signal_hook::consts::signal::SIGINT;
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    WRITE_IN_PROGRESS.fetch_add(1, Ordering::Release);
    let pid = std::process::id();
    unsafe {
        syscalls::syscall2(
            syscalls::SYS_kill,
            pid.try_into().unwrap(),
            SIGINT.try_into().unwrap(),
        )
        .expect("Sending signal works.");
    }
    assert_eq!(1, 1);
    WRITE_IN_PROGRESS.fetch_sub(1, Ordering::Release);
    panic!("Signal handler exits before panic.");
}
