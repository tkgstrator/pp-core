//! GB PSG on the GBA (square 1/2, wave, noise) as mGBA models it: register
//! writes take effect at once, the 512 Hz frame sequencer clocks length,
//! sweep and envelopes, and `output` point-samples the four DACs.
//! All timing is in GBA CPU cycles (16.78 MHz; the PSG runs at a quarter).

/// Readable bits of the registers the driver reads back (mGBA).
fn read_mask(o: usize) -> u16 {
    match o {
        0x62 | 0x68 => 0xffc0,
        0x72 => 0xe000,
        0x78 => 0xff00,
        0x64 | 0x6c | 0x74 | 0x7c => 0x4000,
        0x70 => 0x00e0,
        _ => 0xffff,
    }
}

const DUTY: [u8; 4] = [0b0000_0001, 0b1000_0001, 0b1000_0111, 0b0111_1110];

#[derive(Default, Clone)]
struct Envelope {
    vol: u8,
    up: bool,
    period: u8,
    timer: u8,
}
impl Envelope {
    fn load(&mut self, reg: u8) {
        self.vol = reg >> 4;
        self.up = reg & 8 != 0;
        self.period = reg & 7;
        self.timer = self.period;
    }
    fn clock(&mut self) {
        if self.period == 0 {
            return;
        }
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.timer = self.period;
            if self.up && self.vol < 15 {
                self.vol += 1;
            } else if !self.up && self.vol > 0 {
                self.vol -= 1;
            }
        }
    }
}

#[derive(Default, Clone)]
struct Channel {
    on: bool,
    env: Envelope,
    env_reg: u8,
    freq: u32,
    counter: i64,
    pos: u32,
    length: u32,
    length_on: bool,
}
impl Channel {
    /// Advance the frequency timer; returns the number of steps taken.
    fn run(&mut self, cycles: i64, period: i64) -> u32 {
        self.counter -= cycles;
        if self.counter > 0 {
            return 0;
        }
        let steps = 1 + (-self.counter) / period;
        self.counter += steps * period;
        steps as u32
    }
    fn clock_length(&mut self) {
        if self.length_on && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.on = false;
            }
        }
    }
}

pub struct Apu {
    regs: [u16; 0x50],
    wave_ram: [[u8; 16]; 2],
    sq: [Channel; 2],
    wave: Channel,
    noise: Channel,
    lfsr: u16,
    /// LFSR steps and high outputs since the last sample (mGBA averages noise).
    noise_steps: u32,
    noise_high: u32,
    sweep_timer: u8,
    sweep_freq: u32,
    step: u32,
}

impl Apu {
    pub fn new() -> Apu {
        let mut apu = Apu {
            regs: [0; 0x50],
            wave_ram: [[0; 16]; 2],
            sq: Default::default(),
            wave: Channel::default(),
            noise: Channel::default(),
            lfsr: 0x7fff,
            noise_steps: 0,
            noise_high: 0,
            sweep_timer: 0,
            sweep_freq: 0,
            step: 0,
        };
        apu.regs[0x82 / 2] = 0x210d; // PP6: PSG 50%, FIFO A right, FIFO B left
        apu.regs[0x84 / 2] = 0x80;
        apu.regs[0x88 / 2] = 0x200;
        apu
    }
    fn reg(&self, o: usize) -> u16 {
        self.regs[o / 2]
    }
    pub fn read(&self, o: usize) -> i64 {
        (self.reg(o) & read_mask(o)) as i64
    }

    pub fn write(&mut self, o: usize, v: u16) {
        if (0x90..0xa0).contains(&o) {
            // The CPU reaches the bank the channel is not playing.
            let bank = 1 - ((self.reg(0x70) >> 6) & 1) as usize;
            self.wave_ram[bank][o - 0x90..o - 0x90 + 2].copy_from_slice(&v.to_le_bytes());
            return;
        }
        self.regs[o / 2] = v;
        match o {
            0x62 | 0x68 => {
                let c = &mut self.sq[(o == 0x68) as usize];
                c.length = 64 - (v & 63) as u32;
                c.env_reg = (v >> 8) as u8;
                if c.env_reg & 0xf8 == 0 {
                    c.on = false;
                }
            }
            0x64 | 0x6c => {
                let i = (o == 0x6c) as usize;
                let c = &mut self.sq[i];
                c.freq = (v & 0x7ff) as u32;
                c.length_on = v & 0x4000 != 0;
                if v & 0x8000 != 0 {
                    c.on = c.env_reg & 0xf8 != 0;
                    if c.length == 0 {
                        c.length = 64;
                    }
                    c.env.load(c.env_reg);
                    c.counter = (2048 - c.freq as i64) * 16;
                    // The duty position runs on across triggers (mGBA, DMG).
                    if i == 0 {
                        self.sweep_freq = c.freq;
                        self.sweep_timer = ((self.reg(0x60) >> 4) & 7) as u8;
                    }
                }
            }
            0x70 => {
                if v & 0x80 == 0 {
                    self.wave.on = false;
                }
            }
            0x72 => self.wave.length = 256 - (v & 0xff) as u32,
            0x74 => {
                let c = &mut self.wave;
                c.freq = (v & 0x7ff) as u32;
                c.length_on = v & 0x4000 != 0;
                if v & 0x8000 != 0 {
                    c.on = self.regs[0x70 / 2] & 0x80 != 0;
                    if c.length == 0 {
                        c.length = 256;
                    }
                    c.pos = 0;
                    c.counter = (2048 - c.freq as i64) * 8;
                }
            }
            0x78 => {
                let c = &mut self.noise;
                c.length = 64 - (v & 63) as u32;
                c.env_reg = (v >> 8) as u8;
                if c.env_reg & 0xf8 == 0 {
                    c.on = false;
                }
            }
            0x7c => {
                let period = self.noise_period();
                let c = &mut self.noise;
                c.length_on = v & 0x4000 != 0;
                if v & 0x8000 != 0 {
                    c.on = c.env_reg & 0xf8 != 0;
                    if c.length == 0 {
                        c.length = 64;
                    }
                    c.env.load(c.env_reg);
                    c.counter = period;
                    self.lfsr = 0x7fff;
                }
            }
            _ => {}
        }
    }

    fn noise_period(&self) -> i64 {
        let r = self.reg(0x7c);
        let div = if r & 7 == 0 { 8 } else { 16 * (r & 7) as i64 };
        (div << ((r >> 4) & 15)) * 4
    }

    /// Run the channel timers for `cycles` CPU cycles.
    pub fn advance(&mut self, cycles: i64) {
        if cycles <= 0 {
            return;
        }
        for c in self.sq.iter_mut().filter(|c| c.on) {
            let steps = c.run(cycles, (2048 - c.freq as i64) * 16);
            c.pos = (c.pos + steps) & 7;
        }
        let w = &mut self.wave;
        let steps = w.run(cycles, (2048 - w.freq as i64) * 8);
        let len = if self.regs[0x70 / 2] & 0x20 != 0 {
            64
        } else {
            32
        };
        w.pos = (w.pos + steps) % len;
        let period = self.noise_period();
        let steps = self.noise.run(cycles, period);
        let narrow = self.reg(0x7c) & 8 != 0;
        for _ in 0..steps.min(0x8000) {
            let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            if narrow {
                self.lfsr = (self.lfsr & !0x40) | (bit << 6);
            }
            self.noise_steps += 1;
            self.noise_high += (self.lfsr & 1 == 0) as u32;
        }
    }

    /// One 512 Hz frame-sequencer step.
    pub fn frame_step(&mut self) {
        let step = self.step;
        self.step = (step + 1) & 7;
        if step.is_multiple_of(2) {
            self.sq.iter_mut().for_each(Channel::clock_length);
            self.wave.clock_length();
            self.noise.clock_length();
        }
        if step == 2 || step == 6 {
            self.clock_sweep();
        }
        if step == 7 {
            self.sq.iter_mut().for_each(|c| c.env.clock());
            self.noise.env.clock();
        }
    }

    fn clock_sweep(&mut self) {
        let r = self.reg(0x60);
        let (shift, negate, time) = (r & 7, r & 8 != 0, ((r >> 4) & 7) as u8);
        if time == 0 || !self.sq[0].on {
            return;
        }
        self.sweep_timer = self.sweep_timer.saturating_sub(1);
        if self.sweep_timer > 0 {
            return;
        }
        self.sweep_timer = time;
        let delta = self.sweep_freq >> shift;
        let next = if negate {
            self.sweep_freq.wrapping_sub(delta)
        } else {
            self.sweep_freq + delta
        };
        if next > 0x7ff {
            self.sq[0].on = false;
        } else if shift != 0 {
            self.sweep_freq = next;
            self.sq[0].freq = next;
        }
    }

    /// (left, right) PSG level in 10-bit DAC units, after SOUNDCNT_L/H.
    /// Call once per output sample, after `advance`.
    pub fn output(&mut self) -> (i32, i32) {
        let mut ch = [0i32; 4];
        for (i, c) in self.sq.iter().enumerate() {
            let duty = ((self.reg(if i == 0 { 0x62 } else { 0x68 }) >> 6) & 3) as usize;
            if c.on && DUTY[duty] >> (7 - c.pos) & 1 != 0 {
                ch[i] = c.env.vol as i32;
            }
        }
        let w = &self.wave;
        let nr30 = self.reg(0x70);
        if w.on && nr30 & 0x80 != 0 {
            let bank = if nr30 & 0x20 != 0 {
                (w.pos / 32) as usize ^ ((nr30 >> 6) & 1) as usize
            } else {
                ((nr30 >> 6) & 1) as usize
            };
            let byte = self.wave_ram[bank][(w.pos as usize % 32) / 2];
            let nibble = (if w.pos.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 15
            }) as i32;
            let r = self.reg(0x72);
            ch[2] = if r & 0x8000 != 0 {
                nibble * 3 / 4
            } else {
                [0, nibble, nibble >> 1, nibble >> 2][((r >> 13) & 3) as usize]
            };
        }
        if self.noise.on {
            let vol = self.noise.env.vol as i32;
            ch[3] = if self.lfsr & 1 == 0 { vol } else { 0 };
        }
        self.noise_steps = 0;
        self.noise_high = 0;
        let l = self.reg(0x80);
        let side = |enable: u16, master: u16| -> i32 {
            let sum: i32 = (0..4).filter(|i| enable >> i & 1 != 0).map(|i| ch[i]).sum();
            sum * (master as i32 + 1)
        };
        let (mut left, mut right) = (side(l >> 12, (l >> 4) & 7), side(l >> 8, l & 7));
        match self.reg(0x82) & 3 {
            0 => {
                left >>= 1;
                right >>= 1;
            }
            2 | 3 => {
                left <<= 1;
                right <<= 1;
            }
            _ => {}
        }
        (left, right)
    }

    /// FIFO A/B routing from SOUNDCNT_H: (A gain, A left, A right, B gain, B left, B right).
    pub fn fifo_routing(&self) -> (i32, bool, bool, i32, bool, bool) {
        let h = self.reg(0x82);
        let gain = |bit: u16| if h & bit != 0 { 4 } else { 2 };
        (
            gain(4),
            h & 0x200 != 0,
            h & 0x100 != 0,
            gain(8),
            h & 0x2000 != 0,
            h & 0x1000 != 0,
        )
    }
    pub fn bias(&self) -> i32 {
        (self.reg(0x88) & 0x3fe) as i32
    }
}
