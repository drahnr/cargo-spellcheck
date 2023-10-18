use cargo_spellcheck::{action, errors::Result, run, Args};

#[allow(missing_docs)]
fn main() -> Result<()> {
    let _ = color_eyre::install()?;
    let args = Args::parse(std::env::args()).unwrap_or_else(|e| e.exit());
    let res = run(args);
    // no matter what, restore the terminal
    if let Err(e) = action::interactive::ScopedRaw::restore_terminal() {
        log::warn!("Failed to restore terminal: {e}");
    }
    let val = res?.as_u8();
    if val != 0 {
        std::process::exit(val as i32)
    }
    Ok(())
}
