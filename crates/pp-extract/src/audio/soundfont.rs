//! SoundFont 2 bank for native program changes in extracted MIDI.
use super::{
    DRIVER_UPDATE_HZ, Kind, Percussion, Playback, Sample, TrackFormat,
    sequence::{Decoded, decode},
    sha, synth,
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[derive(Default)]
pub(super) struct PsgKeys {
    pub square: BTreeMap<u8, BTreeSet<u8>>,
    pub wave: BTreeMap<u8, BTreeSet<u8>>,
    pub noise: BTreeSet<u8>,
}
impl PsgKeys {
    pub fn note_keys(&mut self, d: &Decoded, kind: Kind) {
        for e in &d.events {
            if !e.note {
                continue;
            }
            match kind {
                Kind::Square => {
                    self.square.entry(e.duty).or_default().insert(e.pitch);
                }
                Kind::Wave => {
                    self.wave.entry(e.wave_index).or_default().insert(e.pitch);
                }
                Kind::Noise => {
                    self.noise.insert(e.pitch);
                }
                _ => {}
            }
        }
    }
}
fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len() + 9);
    result.extend(tag);
    result.extend((data.len() as u32).to_le_bytes());
    result.extend(data);
    if data.len() & 1 != 0 {
        result.push(0);
    }
    result
}
fn name(value: &str) -> [u8; 20] {
    let mut result = [0u8; 20];
    let bytes = value.as_bytes();
    let n = bytes.len().min(20);
    result[..n].copy_from_slice(&bytes[..n]);
    result
}
fn info_string(value: &str) -> Vec<u8> {
    let mut v = value.as_bytes().to_vec();
    v.push(0);
    if v.len() & 1 != 0 {
        v.push(0);
    }
    v
}
fn generator(op: u16, amount: i32) -> [u8; 4] {
    let mut v = [0u8; 4];
    v[..2].copy_from_slice(&op.to_le_bytes());
    v[2..].copy_from_slice(&(amount as u16).to_le_bytes());
    v
}
fn put16(v: &mut Vec<u8>, n: u16) {
    v.extend(n.to_le_bytes());
}
fn put32(v: &mut Vec<u8>, n: u32) {
    v.extend(n.to_le_bytes());
}
#[derive(Clone)]
struct Zone {
    sample: usize,
    low: u8,
    high: u8,
    looped: bool,
    coarse: i32,
    fine: i32,
}
struct Preset {
    name: String,
    bank: u16,
    program: u16,
    zones: Vec<Zone>,
}
struct Builder {
    samples: Vec<u8>,
    sample_headers: Vec<u8>,
    presets: Vec<Preset>,
}
impl Builder {
    fn new() -> Self {
        Self {
            samples: Vec::new(),
            sample_headers: Vec::new(),
            presets: Vec::new(),
        }
    }
    fn add_sample(
        &mut self,
        label: &str,
        mut pcm: Vec<i16>,
        rate: u32,
        root: u8,
        loop_start: Option<usize>,
    ) -> Result<usize> {
        ensure!(rate > 0 && root <= 127, "Invalid SoundFont sample");
        if pcm.len() < 48 {
            pcm.resize(48, 0);
        }
        let frames = pcm.len();
        let start = (self.samples.len() / 2) as u32;
        for &x in &pcm {
            self.samples.extend(x.to_le_bytes());
        }
        let (loop_a, loop_b) = if let Some(loop_at) = loop_start {
            ensure!(loop_at < frames - 8, "Invalid SoundFont loop");
            for &x in &pcm[loop_at..loop_at + 8] {
                self.samples.extend(x.to_le_bytes());
            }
            (start + loop_at as u32, start + frames as u32)
        } else {
            self.samples.extend([0u8; 16]);
            (start + 8, start + frames as u32 - 8)
        };
        let end = (self.samples.len() / 2) as u32;
        self.samples.extend([0u8; 92]);
        let index = self.sample_headers.len() / 46;
        self.sample_headers.extend(name(label));
        for n in [start, end, loop_a, loop_b, rate] {
            put32(&mut self.sample_headers, n);
        }
        self.sample_headers.push(root);
        self.sample_headers.push(0);
        put16(&mut self.sample_headers, 0);
        put16(&mut self.sample_headers, 1);
        Ok(index)
    }
    fn add_preset(&mut self, label: &str, bank: u16, program: u16, zones: Vec<Zone>) -> Result<()> {
        ensure!(
            !zones.is_empty() && bank <= 128 && program <= 127,
            "Invalid SoundFont preset"
        );
        self.presets.push(Preset {
            name: label.to_owned(),
            bank,
            program,
            zones,
        });
        Ok(())
    }
    fn build(&mut self, label: &str) -> Vec<u8> {
        self.presets.sort_by_key(|p| (p.bank, p.program));
        let mut phdr = Vec::new();
        let mut pbag = Vec::new();
        let mut pgen = Vec::new();
        let mut inst = Vec::new();
        let mut ibag = Vec::new();
        let mut igen = Vec::new();
        for preset in &self.presets {
            let instrument = (inst.len() / 22) as u16;
            phdr.extend(name(&preset.name));
            put16(&mut phdr, preset.program);
            put16(&mut phdr, preset.bank);
            put16(&mut phdr, (pbag.len() / 4) as u16);
            put32(&mut phdr, 0);
            put32(&mut phdr, 0);
            put32(&mut phdr, 0);
            put16(&mut pbag, (pgen.len() / 4) as u16);
            put16(&mut pbag, 0);
            pgen.extend(generator(41, instrument as i32));
            inst.extend(name(&preset.name));
            put16(&mut inst, (ibag.len() / 4) as u16);
            for zone in &preset.zones {
                put16(&mut ibag, (igen.len() / 4) as u16);
                put16(&mut ibag, 0);
                igen.extend(generator(43, zone.low as i32 | ((zone.high as i32) << 8)));
                if zone.coarse != 0 {
                    igen.extend(generator(51, zone.coarse));
                }
                if zone.fine != 0 {
                    igen.extend(generator(52, zone.fine));
                }
                igen.extend(generator(54, if zone.looped { 1 } else { 0 }));
                if preset.bank == 128 {
                    igen.extend(generator(38, 1200));
                }
                igen.extend(generator(53, zone.sample as i32));
            }
        }
        phdr.extend(name("EOP"));
        put16(&mut phdr, 0);
        put16(&mut phdr, 0);
        put16(&mut phdr, (pbag.len() / 4) as u16);
        put32(&mut phdr, 0);
        put32(&mut phdr, 0);
        put32(&mut phdr, 0);
        put16(&mut pbag, (pgen.len() / 4) as u16);
        put16(&mut pbag, 0);
        pgen.extend(generator(0, 0));
        inst.extend(name("EOI"));
        put16(&mut inst, (ibag.len() / 4) as u16);
        put16(&mut ibag, (igen.len() / 4) as u16);
        put16(&mut ibag, 0);
        igen.extend(generator(0, 0));
        let end = (self.samples.len() / 2) as u32;
        self.sample_headers.extend(name("EOS"));
        for n in [end, end, end, end, 44100] {
            put32(&mut self.sample_headers, n);
        }
        self.sample_headers.extend([0, 0]);
        put16(&mut self.sample_headers, 0);
        put16(&mut self.sample_headers, 1);
        let mut info = chunk(b"ifil", &[2, 0, 1, 0]);
        info.extend(chunk(b"isng", &info_string("EMU8000")));
        info.extend(chunk(b"INAM", &info_string(label)));
        info.extend(chunk(
            b"ICMT",
            &info_string("Extracted from verified local ROM; envelopes approximate"),
        ));
        let mut pdta = Vec::new();
        for (tag, data) in [
            (b"phdr", &phdr),
            (b"pbag", &pbag),
            (b"pmod", &vec![0; 10]),
            (b"pgen", &pgen),
            (b"inst", &inst),
            (b"ibag", &ibag),
            (b"imod", &vec![0; 10]),
            (b"igen", &igen),
            (b"shdr", &self.sample_headers),
        ] {
            pdta.extend(chunk(tag, data));
        }
        let mut body = chunk(b"LIST", &[b"INFO".as_slice(), info.as_slice()].concat());
        body.extend(chunk(
            b"LIST",
            &[b"sdta".as_slice(), chunk(b"smpl", &self.samples).as_slice()].concat(),
        ));
        body.extend(chunk(
            b"LIST",
            &[b"pdta".as_slice(), pdta.as_slice()].concat(),
        ));
        chunk(b"RIFF", &[b"sfbk".as_slice(), body.as_slice()].concat())
    }
}
fn zone(sample: usize, key: u8, looped: bool) -> Zone {
    Zone {
        sample,
        low: key,
        high: key,
        looped,
        coarse: 0,
        fine: 0,
    }
}
fn square(duty: u8) -> Vec<i16> {
    let positive = [8, 16, 32, 48][duty as usize];
    (0..64)
        .map(|i| if i < positive { 8192 } else { -8192 })
        .collect()
}
fn wave(rom: &[u8], playback: Playback, slot: u8) -> Result<Vec<i16>> {
    let base = (playback.psg_periods + 0x280) + slot as usize * 16;
    let raw = rom
        .get(base..base + 16)
        .ok_or_else(|| anyhow::anyhow!("Wave slot outside ROM at {base:#x}"))?;
    let mut v = Vec::new();
    for &byte in raw {
        for nibble in [byte >> 4, byte & 15] {
            v.push(((nibble as f64 - 7.5) * 1024.0).round_ties_even() as i16);
        }
    }
    let copy = v.clone();
    v.extend(copy);
    Ok(v)
}
fn noise(rom: &[u8], playback: Playback, source_pitch: u16) -> Vec<i16> {
    let register = synth::noise_register(rom, playback, source_pitch);
    let divisor = ((register & 7) as u32 * 2).max(1);
    let clock = 524288.0 / (divisor as f64 * (1u32 << (register >> 4)) as f64);
    let cycle = synth::noise_cycle(register & 8 != 0);
    (0..8192)
        .map(|i| {
            let ix = ((i as f64 * clock / 32768.0) as usize) % cycle.len();
            if cycle[ix] > 0.0 { 8192 } else { -8192 }
        })
        .collect()
}

pub(super) fn write_soundfont(
    rom: &[u8],
    playback: Playback,
    records: &[Sample],
    percussion: &[Percussion],
    keys: &PsgKeys,
    path: &Path,
) -> Result<Value> {
    let mut builder = Builder::new();
    for (index, s) in records.iter().enumerate() {
        let raw = &rom[s.data_offset..s.data_offset + s.length];
        let pcm: Vec<i16> = raw.iter().map(|b| (*b as i8 as i16) << 8).collect();
        let sample = builder.add_sample(
            &format!("PCM {index:03}"),
            pcm,
            s.rate,
            36,
            if s.looped() { Some(s.loop_start) } else { None },
        )?;
        let trans = if index < playback.sample_bases[1] {
            synth::transpose(rom, playback, 0, index)?
        } else {
            0
        };
        let coarse = trans.div_euclid(256);
        let fine = ((trans - coarse * 256) as f64 * 100.0 / 256.0).round_ties_even() as i32;
        builder.add_preset(
            &format!("PCM {index:03}"),
            (index / 128) as u16,
            (index % 128) as u16,
            vec![Zone {
                sample,
                low: 0,
                high: 127,
                looped: s.looped(),
                coarse,
                fine,
            }],
        )?;
    }
    for duty in 0..4u8 {
        let mut zones = Vec::new();
        let mut note_keys = keys.square.get(&duty).cloned().unwrap_or_default();
        note_keys.insert(36);
        for key in note_keys {
            let source = ((key as i32 - 36) << 8) as u16;
            let period = synth::psg_period(rom, playback, source)?;
            let rate = (64.0 * 131072.0 / (2048 - period as u32) as f64).round_ties_even() as u32;
            let sample = builder.add_sample(
                &format!("SQ{duty} K{key:03}"),
                square(duty),
                rate,
                key,
                Some(0),
            )?;
            zones.push(zone(sample, key, true));
        }
        builder.add_preset(&format!("Square duty {duty}"), 2, duty as u16, zones)?;
    }
    for (&slot, note_keys) in &keys.wave {
        let mut zones = Vec::new();
        let payload = wave(rom, playback, slot)?;
        for &key in note_keys {
            let source = ((key as i32 - 24) << 8) as u16;
            let period = synth::psg_period(rom, playback, source)?;
            let rate = (32.0 * 65536.0 / (2048 - period as u32) as f64).round_ties_even() as u32;
            let sample = builder.add_sample(
                &format!("W{slot:02} K{key:03}"),
                payload.clone(),
                rate,
                key,
                Some(0),
            )?;
            zones.push(zone(sample, key, true));
        }
        builder.add_preset(&format!("Wave slot {slot}"), 3, slot as u16, zones)?;
    }
    let mut noise_keys = keys.noise.clone();
    noise_keys.insert(36);
    let mut zones = Vec::new();
    for key in noise_keys {
        let sample = builder.add_sample(
            &format!("NOISE K{key:03}"),
            noise(rom, playback, ((key as i32 - 36) << 8) as u16),
            32768,
            key,
            Some(0),
        )?;
        zones.push(zone(sample, key, true));
    }
    builder.add_preset("PSG noise", 4, 0, zones)?;
    let mut drum_zones = Vec::new();
    for p in percussion {
        let decoded = decode(
            &rom[p.start..p.end],
            p.start,
            (20.0 * DRIVER_UPDATE_HZ).round_ties_even() as usize,
            Kind::Noise,
            Some(5),
            playback,
            TrackFormat::Legacy,
        )?;
        let (pcm, _, _) = synth::render_pcm(
            rom,
            playback,
            &[decoded],
            &[Kind::Noise],
            &[],
            synth::RenderSettings {
                rate: 32768,
                stereo: false,
                update_rate: DRIVER_UPDATE_HZ,
            },
        )?;
        let key = (0x50 + p.index) as u8;
        let sample = builder.add_sample(&format!("DRUM {:02}", p.index), pcm, 32768, key, None)?;
        drum_zones.push(zone(sample, key, false));
    }
    builder.add_preset("ROM percussion", 128, 0, drum_zones)?;
    let payload = builder.build(&format!("{} ROM sound bank", playback.id.to_uppercase()));
    fs::write(path, &payload)?;
    let square_count: usize = keys.square.values().map(|v| v.len()).sum();
    let wave_count: usize = keys.wave.values().map(|v| v.len()).sum();
    Ok(
        json!({"path":path.file_name().unwrap().to_string_lossy(),"sha256":sha(&payload),"bytes":payload.len(),
        "presets":builder.presets.len(),"samples":builder.sample_headers.len()/46-1,
        "wave_slots":keys.wave.keys().collect::<Vec<_>>(),
        "psg_key_count":{"square":square_count,"wave":wave_count,"noise":keys.noise.len()},
        "bank_map":{"pcm":"index // 128; program index % 128","square":2,"wave":3,"noise":4,"percussion":128}}),
    )
}
