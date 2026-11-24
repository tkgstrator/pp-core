//! PCM tracks driving m4a SoundChannels: 0x08011f9c onwards.
use crate::driver::{Driver, PAN, PCM_DRUMS, SAMPLE_BANKS, SAMPLE_BASE, TRANSPOSE};
use crate::mem::{s8, s16};

impl Driver {
    pub(crate) fn pcm_tick(&mut self) {
        self.g.w8(0x3b, 0);
        self.g.w16(0x3c, 0);
        self.pcm_fades();
        let g32 = self.g.u8(0x32);
        let t = self.t();
        let add = t.u8(0xe) + if t.u8(0x13) == 1 { g32 } else { 0 };
        let ph = (t.u16(0xc) + add) & 0xffff;
        t.w16(0xc, ph);
        if ph <= 0xff {
            self.pcm_freq_update();
            return;
        }
        t.w16(0xc, ph - 0x100);
        t.w8(0xf, t.u8(0xf) - 1);
        if t.u8(0xf) != 0 {
            self.pcm_step();
        } else {
            self.cursor = t.u32(8) as u32;
            self.pcm_interp();
            if self.pcm[self.cs].u16(0) == 0 {
                return;
            }
            let c = self.cursor as i64;
            self.t().w32(8, c);
        }
        let v = self.tr().u8(0x25);
        self.pcm_volume(v);
    }

    /// Request-specific fades (0x08012038..0x080120fc); idle in normal play.
    fn pcm_fades(&mut self) {
        let t = self.t();
        if t.u16(0x68) != 0 {
            t.w16(0x68, t.u16(0x68) - 1);
            if t.u16(0x68) == 0 {
                t.w16(0x68, t.u16(0x66));
                if t.u16(0x6a) != 0 {
                    t.w16(0x6a, t.u16(0x6a) - 1)
                }
            }
        }
        if t.u16(0x70) != 0 {
            t.w16(0x70, t.u16(0x70) - 1);
            if t.u16(0x70) == 0 {
                t.w16(0x70, t.u16(0x6e));
                if t.u16(0x72) != 0 {
                    t.w16(0x72, t.u16(0x72) - 1);
                    t.w16(0x74, t.u16(0x74) + 1);
                } else {
                    t.w16(0x70, 0);
                }
            }
        }
        if t.u16(0x78) != 0 {
            t.w16(0x78, t.u16(0x78) - 1);
            if t.u16(0x78) == 0 {
                t.w16(0x78, t.u16(0x76));
                let lim = if t.u16(0) == 0x6a { 10 } else { 6 };
                if t.u16(0x7a) >= lim {
                    t.w16(0x7a, t.u16(0x7a) - 1)
                } else {
                    t.w16(0x78, 0)
                }
            }
        }
        if t.u8(0x7d) != 0 {
            t.w8(0x7d, t.u8(0x7d) - 1);
            if t.u8(0x7d) == 0 {
                t.w8(0x7d, t.u8(0x7c));
                if t.u8(0x7e) != 0 {
                    t.w8(0x7e, t.u8(0x7e) - 1);
                    t.w8(0x7f, t.u8(0x7f) + 1);
                } else {
                    t.w8(0x7d, 0);
                }
            }
        }
    }

    fn pcm_step(&mut self) {
        let cs = self.cs;
        let t = self.t();
        if t.u8(0x64) != 0 {
            return;
        }
        if t.u8(0x1b) != 0 {
            t.w8(0x1b, t.u8(0x1b) - 1);
            if t.u8(0x1b) == 0 {
                t.w16(0x28, t.u16(0x28) + (s8(t.u8(0x24)) << 8));
                self.g.w8(0x3b, 1);
            }
        }
        let t = self.t();
        if t.u8(0x2d) != 0 {
            t.w8(0x2d, t.u8(0x2d) - 1);
            if t.u8(0x2d) == 0 {
                let rel = t.u8(0x1f);
                let c = &mut self.ch[cs];
                c.w8(7, rel);
                c.w8(0, c.u8(0) | 0x40);
            }
        }
        self.pcm_freq_update();
    }

    fn pcm_freq_update(&mut self) {
        let t = self.t();
        let mut f = t.u16(0x28);
        if t.u8(0x1b) != 0 {
            t.w16(0x22, t.u16(0x22) + t.u16(0x20));
            f = (f + t.u16(0x22)) & 0xffff;
            self.g.w8(0x3b, 1);
        }
        if self.tr().u8(0x1c) != 0 {
            self.vibrato([0x2e, 0x30, 0x34, 0x36, 0x37, 0x38, 0x1c, 0x32]);
            f = (f + self.tr().u16(0x32)) & 0xffff;
        }
        if self.tr().u8(0x1d) != 0 {
            self.lfo2([0x3e, 0x1d, 0x39, 0x3c, 0x3a]);
            f = (f + self.tr().u16(0x3a)) & 0xffff;
        }
        if self.g.u8(0x3b) != 0 {
            self.set_freq(f);
        }
    }

    fn set_freq(&mut self, f: i64) {
        let x = (f + 0x1800) & 0xffff;
        let wav = self.ch[self.cs].u32(0x24) as u32;
        let freq = self.midi_key_to_freq(wav, (x >> 8) & 0x7f, x & 0xff);
        self.ch[self.cs].w32(0x20, freq);
    }

    fn pcm_volume(&mut self, v: i64) {
        let t = self.tr();
        let p = t.u16(0x10);
        let mut out = [0i64; 2];
        for (k, idx) in [p, 20 - p].into_iter().enumerate() {
            let mut x = (v * self.r8(PAN + idx as u32)) >> 8;
            if t.u16(0x68) != 0 {
                x = (x * t.u16(0x6a)) >> 4;
                if t.u16(0x6a) == 0 {
                    let slot = t.u8(3) as usize;
                    self.pcm_kill(slot);
                    return;
                }
            }
            if t.u16(0x70) != 0 {
                x = (x * t.u16(0x74)) >> 4
            }
            if t.u16(0x7a) != 0 {
                x = (x * t.u16(0x7a)) >> 4
            }
            if t.u8(0x7e) != 0 {
                x = (x * t.u8(0x7f)) >> 4
            }
            out[k] = x & 0xff;
        }
        let c = &mut self.ch[self.cs];
        c.w8(3, out[0]);
        c.w8(2, out[1]);
    }

    fn sample(&mut self, local: i64) {
        self.t().w8(0x14, local);
        let bank = self.tr().u8(0x15);
        let table = self.r32(SAMPLE_BANKS + 4 * bank as u32) as u32;
        let wav = self.r32(table + 4 * local as u32) + SAMPLE_BASE as i64;
        self.ch[self.cs].w32(0x24, wav);
        let tr = if bank == 0 {
            self.r16(TRANSPOSE + 2 * (local & 0xff) as u32)
        } else {
            0
        };
        self.t().w16(0x2a, tr);
    }

    fn pcm_interp(&mut self) {
        let cs = self.cs;
        self.t().w8(0x1b, 0);
        let op = loop {
            let op = self.byte();
            if op < 0x80 {
                break op;
            }
            self.pcm_op(op);
            if matches!(op, 0xd9 | 0xc0 | 0xe3) || self.pcm[cs].u16(0) == 0 {
                return;
            }
        };
        let t = self.t();
        let retrig = if t.u8(0x64) != 0 {
            t.w8(0x64, 0);
            1
        } else {
            t.u8(0x12)
        };
        if op & 0x10 == 0 {
            let b = self.byte();
            self.t().w8(0x12, b)
        }
        let t = self.t();
        t.w8(0x2d, t.u8(0x12));
        if op & 0x20 == 0 {
            let b = self.byte();
            self.t().w8(0x2c, b)
        }
        let t = self.t();
        t.w8(0xf, t.u8(0x2c));
        if op & 0x40 == 0 {
            let b = self.byte();
            self.t().w8(0x26, b)
        }
        let t = self.t();
        let mut v = ((op & 15) * 8 + s16(t.u16(0x4a)) + s16(t.u16(0x4c))) * t.u8(0x1a);
        if v < 0 {
            v = 0;
            t.w8(0x64, 1);
        } else if v > 0x7fff {
            v = 0x7fff;
        }
        t.w8(0x25, v >> 7);
        if t.u8(0x26) > 0x4f {
            let i = (t.u8(0x26) - 0x50) as u32;
            let (local, tr) = (self.r16(PCM_DRUMS + 4 * i), self.r16(PCM_DRUMS + 4 * i + 2));
            let bank0 = self.r32(SAMPLE_BANKS) as u32;
            let wav = self.r32(bank0 + 4 * (local & 0xff) as u32) + SAMPLE_BASE as i64;
            let t = self.t();
            t.w8(0x14, local);
            t.w16(0x2a, tr);
            t.w8(0x26, 0x18);
            self.ch[cs].w32(0x24, wav);
        } else if t.u8(0x1c) != 0 {
            t.w16(0x30, t.u8(0x2e));
            t.w16(0x34, 0);
            t.w8(0x37, 0);
            t.w16(0x32, 0);
        }
        let t = self.t();
        let f = (s8(t.u8(0x26)) << 8)
            + t.u16(0x2a)
            + t.u16(0x16)
            + t.u16(0x18)
            + t.u16(0x4e)
            + t.u16(0x50);
        t.w16(0x28, f);
        let attack = t.u8(0x1e);
        self.set_freq(f & 0xffff);
        if retrig != 0 {
            let c = &mut self.ch[cs];
            c.w8(4, attack);
            c.w8(5, 0xff);
            c.w8(6, 0xff);
            c.w8(7, 0xff);
            c.w8(0, 0x80);
        }
    }

    fn pcm_op(&mut self, op: i64) {
        const LOOPS: [usize; 4] = [0x40, 0x48, 0x4a, 0x4e];
        let cs = self.cs;
        match op {
            0xc1 | 0xc2 => self.loop_mark((op - 0xc1) as usize, LOOPS, true),
            0xf3 | 0xf4 => self.repeat((op - 0xf3) as usize, LOOPS, 255),
            0xc3 => {
                let c = self.cursor as i64;
                self.t().w32(0x54, c)
            }
            0xc6 => self.cursor = self.tr().u32(0x54) as u32,
            0xc4 => {
                let c = self.cursor as i64;
                let t = self.t();
                t.w8(0x58, 0);
                t.w32(0x5c, c)
            }
            0xc5 => self.volta(0x58, 0x5c, 0x60),
            0xe4 | 0xf5 | 0xf6 => {}
            0xd0 => {
                let b = self.byte();
                self.t().w8(0xe, b)
            }
            0xdc => {
                let b = self.byte();
                self.t().w8(0x15, b)
            }
            0xd1 => {
                let b = self.byte();
                self.sample(b)
            }
            0x80..=0x83 => self.sample(0),
            0x84..=0x93 => self.sample(5),
            0xd2 => {
                let b = self.byte();
                self.t().w8(0x1a, b)
            }
            0x96..=0xa5 => self.t().w8(0x1a, (15 - (op - 0x96)) << 4),
            0xd3 => {
                let b = self.byte();
                self.t().w16(0x10, b)
            }
            0xae => self.t().w16(0x10, 0),
            0xaf => self.t().w16(0x10, 20),
            0xb0 => self.t().w16(0x10, 10),
            0xd4 => {
                let b = self.byte();
                self.t().w16(0x16, b << 8)
            }
            0xd5 => {
                let b = self.byte();
                self.t().w16(0x18, s8(b) << 2)
            }
            0xd6 => {
                let b = self.byte();
                self.t().w8(0x1f, if b != 0 { b + 0xdf } else { 0 })
            }
            0xa6..=0xad => self.t().w8(0x1f, op + 0x5a),
            0xda => {
                let b = self.byte();
                self.t()
                    .w8(0x1e, if b > 0x77 { ((b << 1) | 1) & 0xff } else { b });
            }
            0xe0 => {
                let b = self.byte();
                self.t().w8(0x1a, b);
                let b = self.byte();
                self.sample(b)
            }
            0xf0 => {
                let b = self.byte();
                self.t().w8(0x1a, b);
                let b = self.byte();
                self.sample(b);
                let b = self.byte();
                self.t().w16(0x10, b);
            }
            0xe1 => {
                let (a, b) = (self.byte(), self.byte());
                let t = self.t();
                t.w8(0x1b, a);
                t.w8(0x24, b)
            }
            0xe2 => {
                let (a, b) = (self.byte(), self.byte());
                let t = self.t();
                t.w16(0x20, a | b << 8);
                t.w16(0x22, 0)
            }
            0xf1 => self.vib_set([0x2e, 0x1c, 0x36, 0x38]),
            0xd8 => {
                let b = self.byte();
                self.t().w8(0x2e, b)
            }
            0xf2 => {
                let (a, hi, lo) = (self.byte(), self.byte(), self.byte());
                let t = self.t();
                t.w8(0x1d, a);
                t.w16(0x3c, hi << 8 | lo);
                t.w16(0x3e, 0);
                t.w8(0x39, 0);
            }
            0xe3 => {
                let (a, b) = (self.byte(), self.byte());
                let t = self.t();
                t.w8(0x2c, a);
                t.w8(0xf, a);
                t.w8(0x12, b);
                t.w8(0x2d, b);
            }
            0xd9 => {
                let b = self.byte();
                let t = self.t();
                t.w8(0x64, 1);
                t.w8(0x2c, b);
                t.w8(0xf, b);
                let c = &mut self.ch[cs];
                c.w8(7, 0);
                c.w8(0, c.u8(0) | 0x40);
            }
            _ => self.pcm_kill(cs),
        }
    }
}
