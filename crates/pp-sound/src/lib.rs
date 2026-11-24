//! PP6's sound driver for the browser: the Konami sequencer, the GB PSG and
//! the m4a PCM mixer, fed by the ROM ranges pp-extract packs into a blob.
//! See docs/pp6-sound-driver.md.
mod apu;
mod driver;
mod effects;
mod ffi;
mod m4a;
mod mem;
mod pcm;
mod player;
mod psg;
mod psg_ops;
mod rom;

pub use player::{NATIVE_RATE, Player};
pub use rom::Rom;
