//! PP6's Konami sound sequencer (docs/pp6-sound-driver.md), ported from the
//! reference model docs/pp6-sound-driver/model.py. Structs keep the IWRAM
//! byte layouts so offsets match the disassembly. This file: requests,
//! track start/stop and the 250 Hz tick; psg.rs and pcm.rs hold the tracks.
use crate::apu::Apu;
use crate::mem::Mem;
use crate::rom::Rom;
use std::rc::Rc;

pub const BGM_TABLE: u32 = 0x08135c0c;
pub const SE_TABLE: u32 = 0x0813a568;
pub const PERIODS: u32 = 0x080fbab4;
pub const PERIOD_FRAC: u32 = 0x080fbbb4;
pub const NOISE: u32 = 0x080fbc34;
pub const LFO2: u32 = 0x080fbcb4;
pub const WAVES: u32 = 0x080fbd34;
pub const WAVE_VOL: u32 = 0x0813d36c;
pub const VIB: u32 = 0x0813d376;
pub const PAN: u32 = 0x0813d386;
pub const DRUM_STREAMS: u32 = 0x0855e574;
pub const PCM_DRUMS: u32 = 0x0855e318;
pub const SAMPLE_BANKS: u32 = 0x0855e2a0;
pub const TRANSPOSE: u32 = 0x0855e2a8;
pub const SAMPLE_BASE: u32 = 0x08722da8;
pub const SCALE: u32 = 0x08554408;
pub const FREQS: u32 = 0x085544bc;

/// Fixed PSG slot fields: SOUNDCNT_L enable mask, volume/duty and frequency
/// registers (IO offsets; 0 = the control track's dummy registers).
const PSG_SLOTS: [(i64, i64, i64); 6] = [
    (0x1100, 0x62, 0x64),
    (0x2200, 0x68, 0x6c),
    (0x4400, 0x72, 0x74),
    (0, 0, 0),
    (0x1100, 0x62, 0x64),
    (0x8800, 0x78, 0x7c),
];

pub struct Request {
    pub id: i64,
    pub prio: i64,
    pub cat: i64,
    pub param: i64,
}

pub struct Driver {
    pub rom: Rc<Rom>,
    pub g: Mem,
    pub psg: Vec<Mem>,
    pub pcm: Vec<Mem>,
    /// m4a SoundChannels (0x40 bytes each); m4a.rs mixes them.
    pub ch: Vec<Mem>,
    pub apu: Apu,
    pub cursor: u32,
    /// The slot being processed and whether it is a PCM slot.
    pub cs: usize,
    pub on_pcm: bool,
    pending: Vec<(bool, i64)>,
}

impl Driver {
    pub fn new(rom: Rc<Rom>) -> Driver {
        let mut psg = vec![Mem::new(0x84); 6];
        for (i, &(mask, vol, freq)) in PSG_SLOTS.iter().enumerate() {
            psg[i].w16(0, mask);
            psg[i].w32(
                4,
                if vol != 0 {
                    0x0400_0000 + vol
                } else {
                    0x0300_0160
                },
            );
            psg[i].w32(
                8,
                if freq != 0 {
                    0x0400_0000 + freq
                } else {
                    0x0300_0160
                },
            );
            psg[i].w8(0x15, i as i64);
        }
        let mut pcm = vec![Mem::new(0x80); 7];
        for (i, slot) in pcm.iter_mut().enumerate() {
            slot.w8(3, i as i64);
        }
        let mut d = Driver {
            rom,
            g: Mem::new(0x58),
            psg,
            pcm,
            ch: vec![Mem::new(0x40); 7],
            apu: Apu::new(),
            cursor: 0,
            cs: 0,
            on_pcm: false,
            pending: Vec::new(),
        };
        d.g.w16(0x3e, 0x77);
        d.iowrite(0x80, 0x77);
        d
    }

    /// Queue a BGM (`se = false`) or SE id; the next phase-0 tick starts it.
    pub fn request(&mut self, se: bool, id: i64) {
        self.pending.push((se, id));
    }

    // ---- IO (offsets from 0x04000000) ----
    pub fn ioread(&self, a: i64) -> i64 {
        let o = a - 0x0400_0000;
        if !(0..0xa0).contains(&o) {
            0
        } else {
            self.apu.read(o as usize)
        }
    }
    /// Writes outside IO (the control track's dummy registers) are dropped.
    pub fn iowrite(&mut self, a: i64, v: i64) {
        let o = a - 0x0400_0000;
        if (0..0xa0).contains(&o) {
            self.apu.write(o as usize, (v & 0xffff) as u16);
        }
    }
    pub fn byte(&mut self) -> i64 {
        let v = self.rom.r8(self.cursor) as i64;
        self.cursor += 1;
        v
    }
    pub fn r8(&self, a: u32) -> i64 {
        self.rom.r8(a) as i64
    }
    pub fn r16(&self, a: u32) -> i64 {
        self.rom.r16(a) as i64
    }
    pub fn r32(&self, a: u32) -> i64 {
        self.rom.r32(a) as i64
    }
    /// The current track struct.
    pub fn t(&mut self) -> &mut Mem {
        if self.on_pcm {
            &mut self.pcm[self.cs]
        } else {
            &mut self.psg[self.cs]
        }
    }
    pub fn tr(&self) -> &Mem {
        if self.on_pcm {
            &self.pcm[self.cs]
        } else {
            &self.psg[self.cs]
        }
    }

    // ================= start / stop =================
    fn start(&mut self, se: bool, mut id: i64) {
        if !se && id == 0x7d {
            id = 0x7c; // 0x0800f438 replays 0x7d as 0x7c
        }
        let e = (if se { SE_TABLE } else { BGM_TABLE }) + id as u32 * 12;
        let flags = self.r32(e);
        let prio = self.r16(e + 4) & 0xff;
        let (lo, hi) = (flags & 0xff, flags & 0x7f00);
        let n = (lo & 0x3f).count_ones() + hi.count_ones();
        if flags == 0 || n == 0 || n > 12 {
            return;
        }
        let list = self.r32(e + 8) as u32;
        let ptrs: Vec<i64> = if n > 1 {
            (0..n).map(|k| self.r32(list + 4 * k)).collect()
        } else {
            vec![list as i64]
        };
        if !se {
            self.stop_bgm();
        }
        let req = Request {
            id,
            prio,
            cat: if se { 2 } else { 1 },
            param: 0,
        };
        let (mut k, mut mask) = (0, 0);
        for (bit, slot, en) in [
            (1, 0, 0x1100),
            (2, 1, 0x2200),
            (4, 2, 0x4400),
            (0x10, 3, 0x8800),
            (8, 5, 0x8800),
        ] {
            if lo & bit != 0 {
                let slot = if se && bit == 1 { 4 } else { slot };
                if bit == 1 {
                    self.iowrite(0x4000060, 8);
                }
                self.psg_init(slot, &req, ptrs[k]);
                k += 1;
                mask |= en;
            }
        }
        if mask != 0 {
            let v = self.g.u16(0x3e) | mask;
            self.g.w16(0x3e, v);
            self.iowrite(0x4000080, v);
        }
        for i in 0..7 {
            if hi & (0x100 << i) != 0 {
                self.pcm_init(i, &req, ptrs[k]);
                k += 1;
            }
        }
    }

    /// 0x0800fdcc: reset the PSG master and stop every BGM track.
    pub fn stop_bgm(&mut self) {
        self.iowrite(0x4000080, 0);
        self.g.w16(0x3e, 0x77);
        self.iowrite(0x4000080, 0x77);
        self.g.w8(0x2d, 0);
        for s in 0..6 {
            if self.psg[s].u8(0x10) == 1 {
                self.psg_kill(s);
            }
        }
        for s in 0..7 {
            if self.pcm[s].u8(0x13) == 1 {
                self.pcm_kill(s);
            }
        }
    }
    /// Stop everything (BGM and SE).
    pub fn stop_all(&mut self) {
        for s in 0..6 {
            if self.psg[s].u16(0x12) != 0 {
                self.psg_kill(s);
            }
        }
        for s in 0..7 {
            if self.pcm[s].u16(0) != 0 {
                self.pcm_kill(s);
            }
        }
        self.pending.clear();
    }

    pub fn psg_kill(&mut self, s: usize) {
        let t = &mut self.psg[s];
        t.w16(0x12, 0);
        t.w8(0x14, 0);
        t.w16(0x76, 0);
        t.w16(0x74, 0);
        t.w8(0x10, 0);
        self.silence(s);
    }
    /// 0x0800feb8: silence the slot's channel (the control track's is noise).
    pub fn silence(&mut self, s: usize) {
        let s = if self.psg[s].u16(0) == 0 { 5 } else { s };
        let (vol, freq, mask) = (self.psg[s].u32(4), self.psg[s].u32(8), self.psg[s].u16(0));
        self.iowrite(vol, 0);
        if mask & 0xb00 != 0 {
            self.iowrite(freq, 0x8000);
        }
    }
    pub fn pcm_kill(&mut self, s: usize) {
        let t = &mut self.pcm[s];
        t.w16(0, 0);
        t.w8(2, 0);
        t.w16(0x68, 0);
        t.w16(0x6a, 0);
        t.w16(0x6c, 0);
        t.w8(0x13, 0);
        let c = &mut self.ch[s];
        c.w8(7, 0);
        c.w8(0, c.u8(0) | 0x40);
    }

    pub fn psg_init(&mut self, s: usize, q: &Request, ptr: i64) {
        let t = &mut self.psg[s];
        t.w16(0x12, q.id);
        t.w8(0x14, q.prio);
        t.w8(0x16, 0);
        t.w32(0x18, ptr);
        t.w8(0x10, q.cat);
        t.w16(0x1c, 0x80);
        t.w8(0x1e, 0x80);
        t.w8(0x1f, 1);
        t.w32(0x78, 0);
        let mask = t.u16(0);
        t.w16(0x20, mask);
        t.w8(0x22, q.param);
        t.w16(0x26, 0);
        t.w16(0x28, 0);
        for o in 0x2a..0x30 {
            t.w8(o, 0);
        }
        t.w8(0x23, 0);
        t.w8(0x5b, 0);
        t.w8(0x5a, 0);
        t.w16(0x5e, 0);
        t.w16(0x5c, 0);
    }
    fn pcm_init(&mut self, s: usize, q: &Request, ptr: i64) {
        let t = &mut self.pcm[s];
        t.w16(0, q.id);
        t.w8(2, q.prio);
        t.w8(4, 0);
        t.w32(8, ptr);
        t.w8(0x13, q.cat);
        t.w16(0xc, 0x80);
        t.w8(0xe, 0x80);
        t.w8(0xf, 1);
        t.w16(0x10, 10);
        t.w8(0x12, 1);
        t.w16(0x16, 0);
        t.w16(0x18, 0);
        t.w8(0x1a, 0xff);
        for o in [0x1b, 0x1c, 0x1d, 0x1f, 0x14, 0x15] {
            t.w8(o, 0);
        }
        t.w8(0x1e, 0xff);
        for o in [
            0x68, 0x6a, 0x72, 0x74, 0x6c, 0x76, 0x78, 0x7a, 0x4c, 0x4a, 0x50, 0x4e,
        ] {
            t.w16(o, 0);
        }
    }

    /// Whether any track is still running.
    pub fn active(&self) -> bool {
        self.psg.iter().any(|t| t.u16(0x12) != 0) || self.pcm.iter().any(|t| t.u16(0) != 0)
    }

    // ================= timer 2 interrupt (0x080103f4) =================
    pub fn tick(&mut self) {
        let phase = self.g.u32(4) & 3;
        match phase {
            0 => {
                for (se, id) in std::mem::take(&mut self.pending) {
                    self.start(se, id);
                }
                (0..3).for_each(|s| self.pcm_slot(s));
            }
            1 => (3..7).for_each(|s| self.pcm_slot(s)),
            2 => (0..3).for_each(|s| self.psg_slot(s)),
            _ => (3..6).for_each(|s| self.psg_slot(s)),
        }
        let c = self.g.u32(4) + 1;
        self.g.w32(4, c);
    }
    fn psg_slot(&mut self, s: usize) {
        if self.psg[s].u16(0x12) == 0 {
            return;
        }
        let delay = self.psg[s].u8(0x16);
        if delay != 0 {
            self.psg[s].w8(0x16, delay - 1);
            return;
        }
        self.cs = s;
        self.on_pcm = false;
        self.psg_tick();
    }
    fn pcm_slot(&mut self, s: usize) {
        if self.pcm[s].u16(0) == 0 {
            return;
        }
        let delay = self.pcm[s].u8(4);
        if delay != 0 {
            self.pcm[s].w8(4, delay - 1);
            return;
        }
        self.cs = s;
        self.on_pcm = true;
        self.pcm_tick();
    }
}
