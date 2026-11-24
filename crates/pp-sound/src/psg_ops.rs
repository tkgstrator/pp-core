//! PSG opcodes (handler table 0x0855e374): timbre, panning, drums, loops.
use crate::driver::{DRUM_STREAMS, Driver, Request, WAVES};
use crate::mem::s8;

impl Driver {
    pub(crate) fn d1(&mut self, b: i64) {
        self.t().w8(0x24, b);
        if self.g.u8(0x3a) != 0 {
            self.t().w8(0x23, b);
            return;
        }
        let (kind, a, f) = {
            let t = self.tr();
            (t.u8(0x15), t.u32(4), t.u32(8))
        };
        match kind {
            0 | 1 | 4 => {
                self.t().w8(0x23, b);
                let v = self.ioread(a) & 0xff3f;
                self.iowrite(a, v);
                let v = self.ioread(a) | b;
                self.iowrite(a, v);
            }
            2 => {
                self.iowrite(0x4000070, 0x40);
                for i in 0..8u32 {
                    let w = self.r16(WAVES + b as u32 * 16 + 2 * i);
                    self.iowrite(0x4000090 + 2 * i as i64, w);
                }
                let v = self.g.u16(0x3e) & 0xbbff;
                self.g.w16(0x3e, v);
                self.iowrite(0x4000080, v);
                self.iowrite(0x4000070, 0x80);
                let v = self.ioread(f) | 0x8000;
                self.iowrite(f, v);
            }
            5 => self.t().w8(0x23, if b != 0 { 8 } else { 0 }),
            _ => {}
        }
    }

    pub(crate) fn pan(&mut self, b: i64) {
        let t = self.t();
        let mask = t.u16(0);
        match b {
            1 => t.w16(0x20, mask & 0x0f00),
            2 => t.w16(0x20, mask & 0xf000),
            3 => t.w16(0x20, mask),
            _ => {}
        }
        let p = self.tr().u16(0x20);
        self.soundcnt(p);
    }

    pub(crate) fn psg_stop(&mut self) {
        let t = self.t();
        t.w16(0x12, 0);
        t.w8(0x14, 0);
        t.w8(0x30, 0);
        let k = t.u8(0x15) as usize;
        if k == 4 || self.g.u8(0x3a) != 0 || (k == 3 && self.psg[5].u16(0x12) != 0) {
            return; // SE square 1 handing back to BGM is not modelled
        }
        self.silence(k);
    }

    pub(crate) fn drum(&mut self, nib: i64) {
        let pitch = self.tr().u8(0x37);
        if self.psg[5].u16(0x12) > 0x3f || pitch < 0x50 {
            return;
        }
        self.t().w8(0x70, 1);
        let v = self.g.u16(0x3e) | 0x8800;
        self.g.w16(0x3e, v);
        self.iowrite(0x4000080, v);
        let ptr = self.r32(DRUM_STREAMS + 4 * (pitch - 0x50) as u32);
        self.psg_init(
            5,
            &Request {
                id: 0xf,
                prio: 0,
                cat: 2,
                param: nib,
            },
            ptr,
        );
    }

    pub(crate) fn psg_op(&mut self, op: i64) {
        const LOOPS: [usize; 4] = [0x50, 0x58, 0x5a, 0x5c];
        match op {
            0x80..=0x83 => self.d1((op - 0x80) << 6),
            0x84..=0x93 => self.d1(op - 0x84),
            0x94 => self.t().w8(0x23, 0),
            0x95 => self.t().w8(0x23, 8),
            0x96..=0xa5 => self.t().w8(0x2a, op - 0x96),
            0xa6..=0xad => self.t().w8(0x2f, op - 0xa6),
            0xae..=0xb0 => {
                let m = self.tr().u16(0);
                let p = [m & 0xf000, m & 0x0f00, m][(op - 0xae) as usize];
                self.t().w16(0x20, p);
                self.soundcnt(p);
            }
            0xc1 | 0xc2 => self.loop_mark((op - 0xc1) as usize, LOOPS, false),
            0xf3 | 0xf4 => self.repeat((op - 0xf3) as usize, LOOPS, 15),
            0xc3 => {
                let c = self.cursor as i64;
                self.t().w32(0x60, c)
            }
            0xc6 => self.cursor = self.tr().u32(0x60) as u32,
            0xc4 => {
                let c = self.cursor as i64;
                let t = self.t();
                t.w8(0x64, 0);
                t.w32(0x68, c)
            }
            0xc5 => self.volta(0x64, 0x68, 0x6c),
            0xe4 => {}
            0xd0 => {
                let b = self.byte();
                self.t().w8(0x1e, b)
            }
            0xd1 => {
                let b = self.byte();
                self.d1(b)
            }
            0xdc => {
                self.byte();
            }
            0xd2 => {
                let b = self.byte();
                self.t().w8(0x2a, b)
            }
            0xd3 => {
                let b = self.byte();
                self.pan(b)
            }
            0xd4 => {
                let b = self.byte();
                self.t().w16(0x26, b << 8)
            }
            0xd5 => {
                let b = self.byte();
                self.t().w16(0x28, s8(b) << 2)
            }
            0xd6 => {
                let b = self.byte();
                self.t().w8(0x2f, b)
            }
            0xda => {
                let b = self.byte();
                self.t().w8(0x2e, b)
            }
            0xe0 => {
                let b = self.byte();
                self.t().w8(0x2a, b);
                let b = self.byte();
                self.d1(b)
            }
            0xf0 => {
                let b = self.byte();
                self.t().w8(0x2a, b);
                let b = self.byte();
                self.d1(b);
                let b = self.byte();
                self.pan(b);
            }
            0xdb => {
                let b = self.byte();
                if self.tr().u16(0) & 0x1100 != 0 {
                    if self.tr().u8(0x15) != 0 {
                        self.iowrite(0x4000060, b);
                    } else {
                        self.g.w16(0x38, b);
                        if self.g.u8(0x3a) == 0 {
                            self.iowrite(0x4000060, b)
                        }
                    }
                }
            }
            0xe1 => {
                let a = self.byte();
                let b = self.byte();
                let t = self.t();
                t.w8(0x2b, a);
                t.w8(0x36, b)
            }
            0xe2 => {
                let a = self.byte();
                let b = self.byte();
                let t = self.t();
                t.w16(0x32, a | b << 8);
                t.w16(0x34, 0)
            }
            0xf1 => self.vib_set([0x3d, 0x2c, 0x44, 0x46]),
            0xd8 => {
                let b = self.byte();
                self.t().w8(0x3d, b)
            }
            0xf2 => {
                let (a, hi, lo) = (self.byte(), self.byte(), self.byte());
                let t = self.t();
                t.w8(0x2d, a);
                t.w16(0x4a, hi << 8 | lo);
                t.w16(0x4c, 0);
                t.w8(0x47, 0);
            }
            0xf5 | 0xf6 => self.cursor += 3,
            0xe3 => {
                let (a, b) = (self.byte(), self.byte());
                let t = self.t();
                t.w8(0x3a, a);
                t.w8(0x1f, a);
                t.w8(0x3b, b);
                t.w8(0x3c, b);
            }
            0xd9 => {
                let b = self.byte();
                let t = self.t();
                t.w8(0x70, 1);
                t.w8(0x3a, b);
                t.w8(0x1f, b);
                if self.g.u8(0x3a) == 0 {
                    let k = self.tr().u8(0x15) as usize;
                    self.silence(k);
                }
            }
            _ => self.psg_stop(),
        }
    }
}
