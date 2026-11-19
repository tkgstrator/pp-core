use anyhow::{Result, bail};
use clap::Parser;
use ppextractor::cli::{Cli, Only, execute};
use ppextractor::{audio, images, scripts};
use std::path::Path;

fn main() -> Result<()> {
    execute(Cli::parse(), extract_category)
}

fn extract_category(category: Only, rom: &[u8], out: &Path, game: ppextractor::Game) -> Result<()> {
    match category {
        Only::Images => images::extract(rom, out, game)?,
        Only::Fonts => ppextractor::fonts::extract(rom, out, game)?,
        Only::Scripts => scripts::extract(rom, out, game)?,
        Only::Portraits => ppextractor::portraits::extract(rom, out, game)?,
        Only::Audio => audio::extract(rom, out, game)?,
        Only::All => bail!("internal error: all must be expanded before extraction"),
    }
    println!("{}: {}", game.id(), out.display());
    Ok(())
}
