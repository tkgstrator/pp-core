//! The GBA sound hardware timeline: timer 2 runs the driver at 249.9 Hz,
//! VBlank runs m4a SoundMain, the 512 Hz frame sequencer clocks the PSG, and
//! the DACs are sampled every 512 cycles (32,768 Hz, like mGBA).
use crate::driver::Driver;
use crate::m4a::{M4a, SAMPLES_PER_FRAME, Segment};
use crate::rom::Rom;
use std::collections::VecDeque;
use std::rc::Rc;

pub const NATIVE_RATE: u32 = 32768;
const SAMPLE_CYCLES: u64 = 512;
const TIMER2_CYCLES: u64 = 64 * 1049;
const VBLANK_CYCLES: u64 = 280_896;
const FIFO_CYCLES: u64 = 798;
const FRAME_SEQ_CYCLES: u64 = 32_768;

pub struct Player {
    pub driver: Driver,
    m4a: M4a,
    now: u64,
    apu_time: u64,
    next_timer: u64,
    next_vblank: u64,
    next_seq: u64,
    segments: VecDeque<(u64, Segment)>,
    /// Cycles between SoundMain mixing a segment and the FIFO DMA playing it.
    pcm_latency: u64,
    /// Output resampling and the analog high-pass (game output only).
    volume: f32,
    phase: f64,
    prev: [f32; 2],
    cur: [f32; 2],
    dc: [f32; 2],
    pub out: Vec<f32>,
}

impl Player {
    pub fn new(rom: Rc<Rom>) -> Player {
        Player {
            driver: Driver::new(rom),
            m4a: M4a::default(),
            now: 0,
            apu_time: 0,
            next_timer: TIMER2_CYCLES,
            next_vblank: VBLANK_CYCLES,
            next_seq: FRAME_SEQ_CYCLES,
            segments: VecDeque::new(),
            pcm_latency: VBLANK_CYCLES,
            volume: 1.0,
            phase: 1.0,
            prev: [0.0; 2],
            cur: [0.0; 2],
            dc: [0.0; 2],
            out: Vec::new(),
        }
    }
    /// Line the hardware clocks up with a recording: the first timer-2 tick
    /// and VBlank (in cycles from now), the driver's tick counter, and the
    /// PCM latency in cycles.
    pub fn set_timing(&mut self, timer: u64, counter: u32, vblank: u64, latency: u64) {
        self.next_timer = self.now + timer;
        self.next_vblank = self.now + vblank;
        self.driver.g.w32(4, counter as i64);
        self.pcm_latency = latency;
    }
    pub fn play_bgm(&mut self, id: u32) {
        self.driver.request(false, id as i64);
    }
    pub fn play_se(&mut self, id: u32) {
        self.driver.request(true, id as i64);
    }
    pub fn stop(&mut self) {
        self.sync_apu(self.now);
        self.driver.stop_all();
    }
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }
    pub fn active(&self) -> bool {
        self.driver.active()
    }

    fn sync_apu(&mut self, t: u64) {
        self.driver.apu.advance((t - self.apu_time) as i64);
        self.apu_time = t;
    }

    /// The next 32,768 Hz sample as (left, right) 10-bit DAC levels.
    pub fn next_dac(&mut self) -> (i32, i32) {
        let t = self.now;
        loop {
            let next = self.next_timer.min(self.next_vblank).min(self.next_seq);
            if next > t {
                break;
            }
            self.sync_apu(next);
            if next == self.next_seq {
                self.driver.apu.frame_step();
                self.next_seq += FRAME_SEQ_CYCLES;
            } else if next == self.next_timer {
                self.driver.tick();
                self.next_timer += TIMER2_CYCLES;
            } else {
                let rom = self.driver.rom.clone();
                let seg = self.m4a.sound_main(&rom, &mut self.driver.ch);
                self.segments.push_back((next + self.pcm_latency, seg));
                self.next_vblank += VBLANK_CYCLES;
            }
        }
        self.sync_apu(t);
        while self.segments.len() > 1 && self.segments[1].0 <= t {
            self.segments.pop_front();
        }
        let (fa, fb) = match self.segments.front() {
            Some((start, (r, l))) if *start <= t => {
                let i = (((t - start) / FIFO_CYCLES) as usize).min(SAMPLES_PER_FRAME - 1);
                (r[i] as i32, l[i] as i32)
            }
            _ => (0, 0),
        };
        let (mut left, mut right) = self.driver.apu.output();
        let (ga, al, ar, gb, bl, br) = self.driver.apu.fifo_routing();
        left += if al { fa * ga } else { 0 } + if bl { fb * gb } else { 0 };
        right += if ar { fa * ga } else { 0 } + if br { fb * gb } else { 0 };
        let bias = self.driver.apu.bias();
        let clip = |v: i32| (v + bias).clamp(0, 0x3ff) - bias;
        self.now += SAMPLE_CYCLES;
        (clip(left), clip(right))
    }

    /// Render `frames` interleaved stereo f32 samples at `rate` into `out`.
    pub fn render(&mut self, frames: usize, rate: u32) {
        self.out.clear();
        let step = NATIVE_RATE as f64 / rate as f64;
        // About 5 Hz: the console's output capacitor removes the PSG's DC.
        let hp = 1.0 - (2.0 * std::f32::consts::PI * 5.0 / NATIVE_RATE as f32);
        for _ in 0..frames {
            while self.phase >= 1.0 {
                self.phase -= 1.0;
                self.prev = self.cur;
                let (l, r) = self.next_dac();
                for (k, v) in [l, r].into_iter().enumerate() {
                    let x = v as f32 / 512.0;
                    self.dc[k] = self.dc[k] * hp + x * (1.0 - hp);
                    self.cur[k] = x - self.dc[k];
                }
            }
            let f = self.phase as f32;
            for k in 0..2 {
                self.out
                    .push((self.prev[k] + (self.cur[k] - self.prev[k]) * f) * self.volume);
            }
            self.phase += step;
        }
    }
}
