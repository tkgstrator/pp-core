//! Effects and flow control shared by PSG and PCM tracks: vibrato, LFO2,
//! repeats, voltas, and m4a's MidiKeyToFreq.
use crate::driver::{Driver, FREQS, LFO2, SCALE, VIB};
use crate::mem::{s8, s16};

impl Driver {
    /// Vibrato (0x080116bc / 0x080125f8); offsets: ramp, scale, acc, speed, pos, step, depth, out.
    pub fn vibrato(&mut self, o: [usize; 8]) {
        let [ramp, scale, acc, speed, pos, step, depth, out] = o;
        let counter = self.g.u32(4);
        let t = self.t();
        if counter & 0xc == 0 && t.u8(ramp) != 0 && t.u16(scale) != 0 {
            t.w16(scale, t.u16(scale) + t.u8(ramp));
            if t.u16(scale) > 0xff {
                t.w16(scale, 0);
            }
        }
        t.w16(acc, t.u16(acc) + t.u8(speed));
        if t.u16(acc) <= 0xff {
            return;
        }
        t.w16(acc, t.u16(acc) & 0xff);
        let d = if t.u16(scale) != 0 {
            (t.u8(depth) * t.u16(scale)) >> 8
        } else {
            t.u8(depth)
        };
        t.w8(pos, t.u8(pos) + t.u8(step));
        let p = t.u8(pos);
        let x = self.r8(VIB + (p & 15) as u32) * d;
        self.t().w16(out, if p & 0x10 != 0 { -x } else { x });
        self.g.w8(0x3b, 1);
    }
    /// LFO2 (0x08011640 / 0x080126ac); offsets: acc, speed, index, mask, out.
    pub fn lfo2(&mut self, o: [usize; 5]) {
        let [acc, speed, idx, mask, out] = o;
        let t = self.t();
        t.w16(acc, t.u16(acc) + t.u8(speed));
        if t.u16(acc) <= 0xff {
            return;
        }
        t.w16(acc, t.u16(acc) & 0xff);
        t.w8(idx, (t.u8(idx) + 1) & 0x7f);
        let i = t.u8(idx) as u32;
        let v = (self.r8(LFO2 + i) | self.r8(LFO2 + i + 1) << 8) & self.tr().u16(mask);
        self.t().w16(out, v);
        self.g.w8(0x3b, 1);
    }
    /// c1/c2: mark a repeat; offsets: cursor, count, volume, pitch.
    pub fn loop_mark(&mut self, s: usize, o: [usize; 4], wide: bool) {
        let [cur, cnt, vol, pit] = o;
        let c = self.cursor as i64;
        let t = self.t();
        t.w32(cur + 4 * s, c);
        t.w8(cnt + s, 0);
        if wide {
            t.w16(vol + 2 * s, 0)
        } else {
            t.w8(vol + s, 0)
        }
        t.w16(pit + 2 * s, 0);
    }
    /// f3/f4: repeat with volume and pitch steps.
    pub fn repeat(&mut self, s: usize, o: [usize; 4], bound: i64) {
        let [cur, cnt, vol, pit] = o;
        let n = self.byte();
        if n != 0 {
            let t = self.t();
            t.w8(cnt + s, t.u8(cnt + s) + 1);
            if t.u8(cnt + s) == n {
                if bound == 15 {
                    t.w8(vol + s, 0)
                } else {
                    t.w16(vol + 2 * s, 0)
                }
                t.w16(pit + 2 * s, 0);
                self.cursor += 2;
                return;
            }
        }
        let dv = s8(self.byte());
        let dp = s8(self.byte());
        let t = self.t();
        if dv != 0 {
            if bound == 15 {
                let v = s8(t.u8(vol + s) + dv);
                t.w8(vol + s, v.clamp(-15, 15));
            } else {
                let v = s16(t.u16(vol + 2 * s) + dv);
                t.w16(vol + 2 * s, v.clamp(-255, 255));
            }
        }
        if dp != 0 {
            t.w16(pit + 2 * s, t.u16(pit + 2 * s) + (dp << 3));
        }
        self.cursor = t.u32(cur + 4 * s) as u32;
    }
    /// c5: A B A C sections; offsets: state, start, return.
    pub fn volta(&mut self, st: usize, a: usize, ret: usize) {
        let c = self.cursor as i64;
        let t = self.t();
        match t.u8(st) {
            0 => t.w8(st, 1),
            1 => {
                t.w8(st, 2);
                t.w32(ret, c);
                self.cursor = t.u32(a) as u32;
            }
            2 => {
                t.w8(st, 1);
                self.cursor = t.u32(ret) as u32;
            }
            _ => {}
        }
    }
    /// f1: delayed vibrato setup; offsets: ramp, depth, speed, step.
    pub fn vib_set(&mut self, o: [usize; 4]) {
        let [ramp, depth, speed, step] = o;
        let r = self.byte();
        let b = self.byte();
        let d = self.byte();
        let t = self.t();
        t.w8(ramp, r);
        t.w8(depth, d);
        if d == 0 {
            return;
        }
        let (sp, st) = if b == 0xff {
            (b, 8)
        } else if b & 0x80 != 0 {
            (b, 4)
        } else if b > 0x3f {
            (b << 1, 2)
        } else {
            (b << 2, 1)
        };
        t.w8(speed, sp);
        t.w8(step, st);
    }
    /// m4a MidiKeyToFreq (0x080f83f0).
    pub fn midi_key_to_freq(&self, wav: u32, mut key: i64, mut fine: i64) -> i64 {
        if key > 178 {
            key = 178;
            fine = 255;
        }
        let val = |k: i64| {
            let x = self.r8(SCALE + k as u32);
            (self.r32(FREQS + (x as u32 & 15) * 4) as u64) >> (x >> 4)
        };
        let (v1, v2) = (val(key), val(key + 1));
        let d = v2.wrapping_sub(v1) & 0xffff_ffff;
        let v = (v1 + ((d * ((fine as u64) << 24)) >> 32)) & 0xffff_ffff;
        ((self.r32(wav + 4) as u64 * v) >> 32) as i64
    }
}
