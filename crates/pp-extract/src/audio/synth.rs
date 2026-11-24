//! Direct-ROM reference synthesis. ROM pitch tables drive every oscillator.
use super::{
    DRIVER_UPDATE_HZ, Kind, Playback, Sample, le16, le32,
    sequence::{Decoded, Event},
    sha,
};
use anyhow::{Context, Result, ensure};
use std::fs;
use std::path::Path;

pub(super) struct WavInfo {
    pub sha256: String,
    pub frames: usize,
    pub peak: i32,
    pub notes: usize,
}
pub(super) struct RenderSettings {
    pub rate: u32,
    pub stereo: bool,
    pub update_rate: f64,
}
pub(super) struct WavOutput<'a> {
    pub path: &'a Path,
    pub rate: u32,
    pub stereo: bool,
}
pub(super) fn transpose(rom: &[u8], playback: Playback, bank: usize, local: usize) -> Result<i32> {
    if bank != 0 {
        return Ok(0);
    }
    let (base, count) = (playback.transpose, playback.sample_bases[1]);
    ensure!(local < count, "PCM bank-0 index out of range: {local}");
    Ok(le16(rom, base + local * 2)? as i16 as i32)
}
#[derive(Debug)]
pub(super) struct PcmPitchOutsideTable(i32);

impl std::fmt::Display for PcmPitchOutsideTable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "PCM pitch outside ROM rate table: {:#x}", self.0)
    }
}

impl std::error::Error for PcmPitchOutsideTable {}

pub(super) fn pcm_rate(
    rom: &[u8],
    playback: Playback,
    s: &Sample,
    pitch_fixed: i32,
) -> Result<u32> {
    let (lookup, mantissas) = (playback.pcm_rates, playback.pcm_rates + 0xb4);
    let signed = pitch_fixed as u16 as i16 as i32;
    let adjusted = signed + 0x1800;
    let mut semitone = adjusted >> 8;
    let fraction = adjusted & 255;
    if playback.id == "pp7" {
        // The PP7 track pitch update at 0x17bfc adds 0x1800 and
        // masks the integer pitch to seven bits before the rate lookup.
        semitone &= 0x7f;
    } else if !(0..127).contains(&semitone) {
        return Err(PcmPitchOutsideTable(pitch_fixed).into());
    }
    let ratio = |i: usize| -> Result<u64> {
        let packed = *rom.get(lookup + i).context("PCM ratio table outside ROM")?;
        let mantissa = le32(rom, mantissas + (packed as usize & 15) * 4)? as u64;
        Ok(mantissa >> (packed >> 4))
    };
    let low = ratio(semitone as usize)?;
    let high = ratio(semitone as usize + 1)?;
    let interpolated =
        (low as i64 + ((high as i64 - low as i64) * fraction as i64).div_euclid(256)) as u64;
    Ok(((s.pitch_q10 as u64 * interpolated) >> 32) as u32)
}
pub(super) fn psg_period(rom: &[u8], playback: Playback, pitch_fixed: u16) -> Result<u16> {
    let (base, frac) = (playback.psg_periods, playback.psg_periods + 0x100);
    let index = ((pitch_fixed >> 8) & 127) as usize;
    let integer = le16(rom, base + index * 2)?;
    let correction = (rom[frac + index] as u16 * (pitch_fixed & 255)) >> 8;
    Ok((integer + correction) & 0x7ff)
}
pub(super) fn noise_register(rom: &[u8], playback: Playback, pitch_fixed: u16) -> u8 {
    rom[(playback.psg_periods + 0x180) + ((pitch_fixed >> 8) & 127) as usize]
}
pub(super) fn noise_cycle(narrow: bool) -> Vec<f32> {
    let mut state = 0x7fffu16;
    let mut values = Vec::with_capacity(if narrow { 127 } else { 32767 });
    for _ in 0..values.capacity() {
        values.push(if state & 1 != 0 { -1.0 } else { 1.0 });
        let feedback = (state ^ (state >> 1)) & 1;
        state = (state >> 1) | (feedback << 14);
        if narrow {
            state = (state & !0x40) | (feedback << 6);
        }
    }
    values
}
fn sample_position(frame: usize, sample_rate: u32, update_rate: f64) -> usize {
    (frame as f64 * sample_rate as f64 / update_rate).round_ties_even() as usize
}
pub(super) fn render_pcm(
    rom: &[u8],
    playback: Playback,
    channels: &[Decoded],
    kinds: &[Kind],
    samples: &[Sample],
    settings: RenderSettings,
) -> Result<(Vec<i16>, usize, i32)> {
    let RenderSettings {
        rate,
        stereo,
        update_rate,
    } = settings;
    ensure!(
        update_rate > 0.0 && rate >= 8000,
        "Invalid reference synthesizer rate"
    );
    ensure!(channels.len() == kinds.len(), "Invalid sound channel kinds");
    let end_frame = channels.iter().map(|d| d.end_frame).max().unwrap_or(0);
    let frames = sample_position(end_frame, rate, update_rate).max(1);
    let stride = if stereo { 2 } else { 1 };
    let mut mix = vec![0f32; frames * stride];
    let mut notes = 0usize;
    let noise_wide = noise_cycle(false);
    let noise_narrow = noise_cycle(true);
    for (decoded, &kind) in channels.iter().zip(kinds) {
        if kind == Kind::Control {
            continue;
        }
        let mut active: Option<&Event> = None;
        let mut release: Option<&Event> = None;
        let mut spans = Vec::<(&Event, Option<&Event>, usize)>::new();
        for event in &decoded.events {
            if event.note {
                if let Some(note) = active {
                    spans.push((note, release, event.frame));
                }
                active = Some(event);
                release = None;
            } else if active.is_some() {
                release = Some(event);
            }
        }
        if let Some(note) = active {
            spans.push((note, release, decoded.end_frame));
        }
        for (note, off_event, next_frame) in spans {
            let start = sample_position(note.frame, rate, update_rate).min(frames);
            let off_frame = off_event.map(|e| e.frame).unwrap_or(next_frame);
            let off = sample_position(off_frame, rate, update_rate).min(frames);
            let release_period = if kind == Kind::Square {
                off_event.map(|e| e.release_period).unwrap_or(0)
            } else {
                0
            };
            let end_frame = if release_period > 0 {
                next_frame
            } else {
                off_frame
            };
            let end = sample_position(end_frame, rate, update_rate).min(frames);
            if end <= start {
                continue;
            }
            let mut source: Option<Vec<f32>> = None;
            let mut play_rate = 0u32;
            if kind == Kind::Pcm {
                let s = samples
                    .get(note.sample_index)
                    .context("Unresolved PCM sample")?;
                let data = &rom[s.data_offset..s.data_offset + s.length];
                source = Some(data.iter().map(|b| (*b as i8 as f32) / 128.0).collect());
                play_rate = pcm_rate(
                    rom,
                    playback,
                    s,
                    note.pitch_fixed as i32
                        + transpose(rom, playback, note.sample_bank, note.sample_local)?,
                )?;
            }
            let period = if kind == Kind::Square || kind == Kind::Wave {
                psg_period(rom, playback, note.pitch_fixed)?
            } else {
                0
            };
            let wave = if kind == Kind::Wave {
                let base = (playback.psg_periods + 0x280) + note.wave_index as usize * 16;
                let bytes = rom.get(base..base + 16).context("Wave slot outside ROM")?;
                let mut vals = [0f32; 32];
                for (i, &b) in bytes.iter().enumerate() {
                    vals[i * 2] = (b >> 4) as f32;
                    vals[i * 2 + 1] = (b & 15) as f32;
                }
                for val in &mut vals {
                    *val = (*val - 7.5) / 7.5;
                }
                Some(vals)
            } else {
                None
            };
            let register = if kind == Kind::Noise {
                noise_register(rom, playback, note.pitch_fixed)
            } else {
                0
            };
            let divisor = ((2 * (register & 7)) as u32).max(1);
            let clock = 524288.0 / (divisor as f64 * (1u32 << (register >> 4)) as f64);
            let noise = if register & 8 != 0 {
                &noise_narrow
            } else {
                &noise_wide
            };
            let square_frequency = if kind == Kind::Square {
                131072.0 / (2048 - period as u32) as f64
            } else {
                0.0
            };
            let wave_frequency = if kind == Kind::Wave {
                65536.0 / (2048 - period as u32) as f64
            } else {
                0.0
            };
            let duty = [0.125, 0.25, 0.5, 0.75][note.duty as usize];
            let pcm_pan = note.pan.min(20) as f32;
            let left_gain = if kind == Kind::Pcm {
                (20.0 - pcm_pan) / 20.0
            } else {
                1.0
            };
            let right_gain = if kind == Kind::Pcm {
                pcm_pan / 20.0
            } else {
                1.0
            };
            for i in start..end {
                let p = (i - start) as f64;
                let mut val = match kind {
                    Kind::Pcm => {
                        let s = &samples[note.sample_index];
                        let bytes = source.as_ref().unwrap();
                        let mut ix = (p * play_rate as f64 / rate as f64) as usize;
                        if ix >= bytes.len() {
                            if s.looped() {
                                ix = s.loop_start
                                    + (ix - bytes.len()) % (bytes.len() - s.loop_start);
                            } else {
                                continue;
                            }
                        }
                        bytes[ix] * 0.22 * (note.volume as f32 / 255.0)
                    }
                    Kind::Wave => {
                        let ix = ((p * wave_frequency * 32.0 / rate as f64) as usize) % 32;
                        wave.as_ref().unwrap()[ix] * 0.10 * (note.volume as f32 / 255.0)
                    }
                    Kind::Noise => {
                        let ix = ((p * clock / rate as f64) as usize) % noise.len();
                        noise[ix] * 0.08 * (note.volume as f32 / 255.0)
                    }
                    Kind::Square => {
                        let phase = (p * square_frequency / rate as f64).fract();
                        (if phase < duty { 0.10f32 } else { -0.10f32 })
                            * (note.volume as f32 / 255.0)
                    }
                    Kind::Control => 0.0,
                };
                if kind == Kind::Square && release_period > 0 && i >= off {
                    let initial = note.volume / 17;
                    if initial > 0 {
                        let elapsed = (i - off) as f64 / rate as f64;
                        let steps = (elapsed * 64.0 / release_period as f64).floor();
                        val *= ((initial as f64 - steps).max(0.0) / initial as f64) as f32;
                    }
                }
                let at = i * stride;
                mix[at] += val * left_gain;
                if stereo {
                    mix[at + 1] += val * right_gain;
                }
            }
            notes += 1;
        }
    }
    let pcm: Vec<i16> = mix
        .into_iter()
        .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect();
    let peak = pcm.iter().map(|x| (*x as i32).abs()).max().unwrap_or(0);
    Ok((pcm, notes, peak))
}
pub(super) fn write_wav(path: &Path, pcm: &[i16], rate: u32, stereo: bool) -> Result<String> {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: if stereo { 2 } else { 1 },
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for &s in pcm {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    Ok(sha(&fs::read(path)?))
}
pub(super) fn synthesize(
    rom: &[u8],
    playback: Playback,
    channels: &[Decoded],
    kinds: &[Kind],
    drums: &[Decoded],
    samples: &[Sample],
    output: WavOutput<'_>,
) -> Result<WavInfo> {
    let WavOutput { path, rate, stereo } = output;
    let mut all = channels.to_vec();
    all.extend_from_slice(drums);
    let mut all_kinds = kinds.to_vec();
    all_kinds.extend(std::iter::repeat_n(Kind::Noise, drums.len()));
    let (pcm, notes, peak) = render_pcm(
        rom,
        playback,
        &all,
        &all_kinds,
        samples,
        RenderSettings {
            rate,
            stereo,
            update_rate: DRIVER_UPDATE_HZ,
        },
    )?;
    let sha256 = write_wav(path, &pcm, rate, stereo)?;
    Ok(WavInfo {
        sha256,
        frames: pcm.len() / if stereo { 2 } else { 1 },
        peak,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{Game, original_playback};
    use super::*;
    #[test]
    fn rom_psg_periods_match_verified_driver_trace() {
        for (name, game) in [("pp4", Game::Pp4), ("pp6", Game::Pp6)] {
            let Some(path) = crate::test_assets::rom(name) else {
                continue;
            };
            let verified = crate::verify_rom(&path).unwrap();
            assert_eq!(verified.game, game);
            let rom = verified.bytes;
            let playback = original_playback(game, false);
            assert_eq!(psg_period(&rom, playback, 36 << 8).unwrap(), 0x706);
            assert_eq!(psg_period(&rom, playback, (36 << 8) | 0x80).unwrap(), 0x70d);
            assert_eq!(noise_register(&rom, playback, 31 << 8), 0x52);
        }
    }
}
