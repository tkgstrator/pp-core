//! PSG tracks (square 1/2, wave, control/drums, noise): 0x08010f18 onwards.
use crate::driver::{Driver, NOISE, PERIOD_FRAC, PERIODS, WAVE_VOL};
use crate::mem::s8;

impl Driver {
    pub(crate) fn psg_tick(&mut self) {
        self.g.w8(0x3b, 0);
        self.g.w16(0x3c, 0);
        let sq1_busy = self.tr().u8(0x15) == 0 && self.psg[4].u16(0x12) != 0;
        self.g.w8(0x3a, sq1_busy as i64);
        let g32 = self.g.u8(0x32);
        let t = self.t();
        let add = t.u8(0x1e) + if t.u8(0x10) == 1 { g32 } else { 0 };
        let ph = (t.u16(0x1c) + add) & 0xffff;
        t.w16(0x1c, ph);
        if ph <= 0xff {
            self.wave_release();
            self.psg_freq_update();
            return;
        }
        t.w16(0x1c, ph - 0x100);
        t.w8(0x1f, t.u8(0x1f) - 1);
        if t.u8(0x1f) != 0 {
            self.psg_step();
            return;
        }
        self.cursor = t.u32(0x18) as u32;
        self.psg_interp();
        let c = self.cursor as i64;
        if self.psg[self.cs].u16(0x12) != 0 {
            self.t().w32(0x18, c);
        }
    }

    fn psg_step(&mut self) {
        if self.tr().u8(0x70) != 0 {
            return;
        }
        let t = self.t();
        if t.u8(0x2b) != 0 {
            t.w8(0x2b, t.u8(0x2b) - 1);
            if t.u8(0x2b) == 0 {
                t.w16(0x38, t.u16(0x38) + (s8(t.u8(0x36)) << 8));
                self.g.w8(0x3b, 1);
            }
        }
        let sq1_busy = self.g.u8(0x3a) != 0;
        let t = self.t();
        if t.u8(0x3c) != 0 {
            t.w8(0x3c, t.u8(0x3c) - 1);
            if t.u8(0x3c) == 0 {
                if t.u8(0x15) == 2 {
                    if t.u8(0x2f) == 0 && t.u8(0x31) == 0 {
                        t.w8(0x70, 1);
                        self.soundcnt(0);
                        return;
                    }
                    t.w8(0x31, t.u8(0x2f));
                    self.wave_release();
                } else {
                    if sq1_busy {
                        return;
                    }
                    let (a, rel, vol) = (t.u32(4), t.u8(0x2f), t.u8(0x30));
                    let v = if rel != 0 {
                        (vol << 12) | (rel << 8) | self.ioread(a)
                    } else {
                        self.ioread(a) & 0xc0
                    };
                    self.iowrite(a, v);
                    self.g.w16(0x3c, 0x8000);
                }
            }
        }
        self.psg_freq_update();
    }

    fn wave_release(&mut self) {
        let t = self.t();
        if t.u8(0x15) != 2 || t.u8(0x70) != 0 || t.u8(0x31) == 0 {
            return;
        }
        t.w8(0x31, t.u8(0x31) - 1);
        if t.u8(0x31) != 0 || t.u8(0x30) == 0 {
            return;
        }
        t.w8(0x30, t.u8(0x30) - 1);
        let v = t.u8(0x30);
        self.set_vol(v);
        let t = self.t();
        if t.u8(0x30) != 0 {
            t.w8(0x31, t.u8(0x2f))
        } else {
            t.w8(0x70, 1)
        }
    }

    pub(crate) fn set_vol(&mut self, v: i64) {
        let (kind, a, env, pan) = {
            let t = self.tr();
            (t.u8(0x15), t.u32(4), t.u8(0x2e), t.u16(0x20))
        };
        if kind == 2 {
            let v = s8(v);
            let v = if !(0..=4).contains(&v) { 4 } else { v };
            let reg = self.r16(WAVE_VOL + 2 * v as u32);
            self.iowrite(a, reg);
            self.soundcnt(pan);
            return;
        }
        let mut x = self.ioread(a) & 0xc0;
        x |= if env & 0x40 != 0 {
            env << 8
        } else {
            (v & 0xff) << 12
        };
        self.iowrite(a, x);
        self.g.w16(0x3c, 0x8000);
    }

    pub(crate) fn soundcnt(&mut self, x: i64) {
        if self.g.u8(0x3a) != 0 {
            return;
        }
        let v = (self.g.u16(0x3e) & !self.tr().u16(0)) | x;
        self.g.w16(0x3e, v);
        self.iowrite(0x4000080, v);
    }

    fn freq_write(&mut self, f: i64) {
        let (frac, semi) = (f & 0xff, ((f >> 8) & 0x7f) as u32);
        let (kind, vol_reg, freq_reg, duty) = {
            let t = self.tr();
            (t.u8(0x15), t.u32(4), t.u32(8), t.u8(0x23))
        };
        let trigger = self.g.u16(0x3c);
        if kind == 5 {
            let v = self.r8(NOISE + semi) | duty | trigger;
            self.iowrite(freq_reg, v);
            return;
        }
        let frac = if frac != 0 {
            (self.r8(PERIOD_FRAC + semi) * frac) >> 8
        } else {
            0
        };
        let period = (self.r16(PERIODS + 2 * semi) + frac) & 0x7ff;
        self.iowrite(freq_reg, period | trigger);
        let v = self.ioread(vol_reg) | duty;
        self.iowrite(vol_reg, v);
    }

    fn psg_freq_update(&mut self) {
        let t = self.t();
        let mut f = t.u16(0x38);
        if t.u8(0x2b) != 0 {
            t.w16(0x34, t.u16(0x34) + t.u16(0x32));
            f = (f + t.u16(0x34)) & 0xffff;
            self.g.w8(0x3b, 1);
        }
        if self.tr().u8(0x2c) != 0 {
            self.vibrato([0x3d, 0x3e, 0x42, 0x44, 0x45, 0x46, 0x2c, 0x40]);
            f = (f + self.tr().u16(0x40)) & 0xffff;
        }
        if self.tr().u8(0x2d) != 0 {
            self.lfo2([0x4c, 0x2d, 0x47, 0x4a, 0x48]);
            f = (f + self.tr().u16(0x48)) & 0xffff;
        }
        if self.g.u8(0x3a) == 0 && (self.g.u8(0x3b) != 0 || self.g.u16(0x3c) != 0) {
            self.freq_write(f);
        }
    }

    pub(crate) fn psg_interp(&mut self) {
        self.t().w8(0x2b, 0);
        let op = loop {
            let op = self.byte();
            if op < 0x80 {
                break op;
            }
            self.psg_op(op);
            if matches!(op, 0xd9 | 0xc0 | 0xe3) || self.tr().u16(0x12) == 0 {
                return;
            }
        };
        self.t().w8(0x31, 0);
        self.t().w8(0x70, 0);
        if op & 0x10 == 0 {
            let b = self.byte();
            self.t().w8(0x3b, b);
        }
        let t = self.t();
        t.w8(0x3c, t.u8(0x3b));
        if op & 0x20 == 0 {
            let b = self.byte();
            self.t().w8(0x3a, b);
        }
        let t = self.t();
        t.w8(0x1f, t.u8(0x3a));
        if op & 0x40 == 0 {
            let b = self.byte();
            self.t().w8(0x37, b);
        }
        let nib = op & 15;
        if self.tr().u8(0x15) == 3 {
            self.drum(nib);
            return;
        }
        let t = self.t();
        let mut v = s8(nib - t.u8(0x2a) - t.u8(0x22));
        v = s8(v + t.u8(0x5a));
        v = s8(v + t.u8(0x5b));
        if v < 0 {
            v = 0;
            t.w8(0x70, 1);
        } else if v > 15 {
            v = 15;
        }
        t.w8(0x30, v);
        if self.g.u8(0x3a) == 0 {
            self.set_vol(v);
        }
        let t = self.t();
        if t.u8(0x2c) != 0 {
            t.w16(0x3e, t.u8(0x3d));
            t.w16(0x42, 0);
            t.w8(0x45, 0);
            t.w16(0x40, 0);
        }
        let f = (s8(t.u8(0x37)) << 8) + t.u16(0x26) + t.u16(0x28) + t.u16(0x5c) + t.u16(0x5e);
        t.w16(0x38, f);
        if self.g.u8(0x3a) == 0 {
            self.freq_write(f & 0xffff);
        }
    }
}
