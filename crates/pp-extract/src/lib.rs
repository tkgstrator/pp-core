pub mod audio;
pub mod cli;
pub mod fonts;
mod gb;
pub mod gba;
pub mod images;
mod nds;
pub mod portraits;
pub mod rom;
pub mod scripts;
mod switch;

#[cfg(test)]
mod test_assets;

pub use rom::{Game, VerifiedRom, verify_rom};
