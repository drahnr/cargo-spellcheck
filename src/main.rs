use log::warn;

use cargo_spellcheck::{run, errors::Result, action};

#[allow(missing_docs)]
fn main() -> Result<()> {
    let _ = color_eyre::install()?;
    let res = run();
    // no matter what, restore the terminal
    if let Err(e) = action::interactive::ScopedRaw::restore_terminal() {
        warn!("Failed to restore terminal: {}", e);
    }
    let val = res?.as_u8();
    if val != 0 {
        std::process::exit(val as i32)
    }
    Ok(())
}
