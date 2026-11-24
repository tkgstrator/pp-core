//! Write the pp-sound data blob from a verified PP6 ROM:
//!   cargo run -p pp-extract --example pp6_sound_blob -- ROM OUT.bin
use anyhow::{Context, Result, ensure};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    ensure!(args.len() == 3, "usage: pp6_sound_blob ROM OUT.bin");
    let rom = ppextractor::rom::verify_rom(std::path::Path::new(&args[1]))?;
    ensure!(rom.game == ppextractor::Game::Pp6, "Expected a PP6 ROM");
    let blob = ppextractor::audio::pp6_sound_blob(&rom.bytes)?;
    std::fs::write(&args[2], &blob).with_context(|| format!("write {}", args[2]))?;
    println!("{} bytes", blob.len());
    Ok(())
}
