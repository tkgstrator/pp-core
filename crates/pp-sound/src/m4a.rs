//! Nintendo m4a SoundMain as PP6 runs it (docs/pp6-sound-driver.md §7):
//! once per VBlank, step each SoundChannel's envelope and mix 352 samples
//! at 21,024 Hz into 8-bit wrapping buffers (no reverb), resampling with
//! linear interpolation (23-bit fraction, divFreq 399).
use crate::mem::Mem;
use crate::rom::Rom;

pub const SAMPLES_PER_FRAME: usize = 352;
const MASTER_VOLUME: u32 = 14;
const DIV_FREQ: u64 = 399;

#[derive(Default, Clone)]
struct Voice {
    wav: u32,
    pos: i64,
    fw: u64,
    count: i64,
}

#[derive(Default)]
pub struct M4a {
    voices: [Voice; 7],
}

/// One VBlank's mix: (right = FIFO A, left = FIFO B).
pub type Segment = ([i8; SAMPLES_PER_FRAME], [i8; SAMPLES_PER_FRAME]);

impl M4a {
    pub fn sound_main(&mut self, rom: &Rom, channels: &mut [Mem]) -> Segment {
        let mut right = [0i8; SAMPLES_PER_FRAME];
        let mut left = [0i8; SAMPLES_PER_FRAME];
        for (c, v) in channels.iter_mut().zip(&mut self.voices) {
            let mut st = c.u8(0) as u32;
            if st & 0xc7 == 0 {
                continue;
            }
            let mut env = c.u8(9) as u32;
            let wav = c.u32(0x24) as u32;
            if st & 0x80 != 0 {
                if st & 0x40 != 0 {
                    c.w8(0, 0);
                    continue;
                }
                st = 3 | if rom.r8(wav + 3) & 0xc0 != 0 { 0x10 } else { 0 };
                *v = Voice {
                    wav,
                    pos: 0,
                    fw: 0,
                    count: rom.r32(wav + 12) as i64,
                };
                env = 0;
                env += c.u8(4) as u32;
                if env >= 0xff {
                    env = 0xff;
                    st -= 1;
                }
            } else if st & 4 != 0 {
                let len = c.u8(0xd) - 1;
                c.w8(0xd, len);
                if len <= 0 {
                    c.w8(0, 0);
                    continue;
                }
            } else if st & 0x40 != 0 {
                env = (env * c.u8(7) as u32) >> 8;
                let echo = c.u8(0xc) as u32;
                if env <= echo {
                    if echo == 0 {
                        c.w8(0, 0);
                        continue;
                    }
                    st |= 4;
                    env = echo;
                }
            } else if st & 3 == 2 {
                env = (env * c.u8(5) as u32) >> 8;
                let sustain = c.u8(6) as u32;
                if env <= sustain {
                    env = sustain;
                    if sustain == 0 {
                        c.w8(0, 0);
                        continue;
                    }
                    st -= 1;
                }
            } else if st & 3 == 3 {
                env += c.u8(4) as u32;
                if env >= 0xff {
                    env = 0xff;
                    st -= 1;
                }
            }
            c.w8(0, st as i64);
            c.w8(9, env as i64);
            let vol = ((MASTER_VOLUME + 1) * env) >> 4;
            let env_r = ((c.u8(2) as u32 * vol) >> 8) as i32;
            let env_l = ((c.u8(3) as u32 * vol) >> 8) as i32;
            c.w8(0xa, env_r as i64);
            c.w8(0xb, env_l as i64);
            if !mix(
                rom,
                v,
                c.u32(0x20) as u64,
                st & 0x10 != 0,
                env_r,
                env_l,
                &mut right,
                &mut left,
            ) {
                c.w8(0, 0);
            }
        }
        (right, left)
    }
}

/// Mix one voice; false once a one-shot sample has ended.
#[allow(clippy::too_many_arguments)]
fn mix(
    rom: &Rom,
    v: &mut Voice,
    freq: u64,
    looped: bool,
    env_r: i32,
    env_l: i32,
    right: &mut [i8],
    left: &mut [i8],
) -> bool {
    let data = rom.bytes(v.wav + 0x10);
    let size = rom.r32(v.wav + 12) as i64;
    let loop_start = rom.r32(v.wav + 8) as i64;
    let at = |i: i64| data.get(i as usize).map_or(0, |&b| b as i8 as i64);
    let step = (DIV_FREQ * freq) & 0xffff_ffff;
    for n in 0..SAMPLES_PER_FRAME {
        let (s0, s1) = (at(v.pos), at(v.pos + 1));
        let s = (s0 + ((v.fw as i64 * (s1 - s0)) >> 23)) as i32;
        right[n] = right[n].wrapping_add(((env_r * s) >> 8) as i8);
        left[n] = left[n].wrapping_add(((env_l * s) >> 8) as i8);
        v.fw += step;
        let adv = (v.fw >> 23) as i64;
        v.fw &= 0x7f_ffff;
        if adv == 0 {
            continue;
        }
        v.pos += adv;
        v.count -= adv;
        while v.count <= 0 {
            if !looped || size <= loop_start {
                return false;
            }
            v.count += size - loop_start;
            v.pos -= size - loop_start;
        }
    }
    true
}
