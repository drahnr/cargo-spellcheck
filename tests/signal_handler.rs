#![cfg(not(target_os = "windows"))]

use std::convert::TryInto;
use std::sync::atomic::Ordering;

use cargo_spellcheck::{signal_handler, WRITE_IN_PROGRESS};
use fork::{Fork, daemon};

fn main() {
    println!("Signal handler check");
    let _signalthread = std::thread::spawn(signal_handler);
    use signal_hook::consts::signal::SIGINT;
    let _ = env_logger::Builder::new()
        .filter_level(log::LevelFilter::Trace)
        .is_test(true)
        .try_init();

    if let Ok(Fork::Parent(child)) = daemon(true, false) {
        WRITE_IN_PROGRESS.fetch_add(1, Ordering::Release);
        println!("[parent] Send signal handler to child");
        unsafe {
            syscalls::syscall2(
                syscalls::SYS_kill,
                child.try_into().unwrap(),
                SIGINT.try_into().unwrap(),
            )
            .expect("[parent] Sending signal works.");
        }
        assert_eq!(1, 1);
        WRITE_IN_PROGRESS.fetch_sub(1, Ordering::Release);
    }
    std::thread::sleep(std::time::Duration::from_secs(5));
    unreachable!("[child] Signal handler exits before panic.");
}
