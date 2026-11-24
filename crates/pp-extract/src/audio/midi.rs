use super::{
    DRIVER_UPDATE_HZ, Kind,
    sequence::{Decoded, Event, StopReason},
};
use anyhow::{Result, ensure};
use std::fs;
use std::path::Path;

pub(super) struct MidiInfo {
    pub notes: usize,
    pub tracks: usize,
    pub sha256: String,
}
fn bend_displacement(event: &Event, kind: Kind) -> f64 {
    let offset = match kind {
        Kind::Wave => 24.0,
        Kind::Square => 36.0,
        _ => 0.0,
    };
    event.pitch_fixed as i16 as f64 / 256.0 + offset - f64::from(event.pitch)
}

fn bend_range(track: &Decoded, kind: Kind) -> Result<u8> {
    let mut range = 12.0_f64;
    if !matches!(kind, Kind::Noise | Kind::Control) {
        for event in track.events.iter().filter(|event| event.note) {
            let displacement = bend_displacement(event, kind);
            // MIDI bend has an inclusive negative edge and an exclusive positive edge.
            range = range
                .max((-displacement).ceil())
                .max(displacement.floor() + 1.0);
        }
    }
    ensure!(range <= 127.0, "Native pitch outside MIDI bend range");
    Ok(range as u8)
}

fn vlq(mut value: u64) -> Vec<u8> {
    let mut bytes = vec![(value & 127) as u8];
    value >>= 7;
    while value > 0 {
        bytes.insert(0, 0x80 | ((value & 127) as u8));
        value >>= 7;
    }
    bytes
}
pub(crate) fn meta(kind: u8, value: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xff, kind];
    bytes.extend(vlq(value.len() as u64));
    bytes.extend(value);
    bytes
}
fn track(events: Vec<(u64, i32, Vec<u8>)>) -> Vec<u8> {
    track_until(events, 0)
}

pub(crate) fn track_until(mut events: Vec<(u64, i32, Vec<u8>)>, end_tick: u64) -> Vec<u8> {
    events.sort_by_key(|e| (e.0, e.1));
    let mut body = Vec::new();
    let mut previous = 0u64;
    for (tick, _, data) in events {
        body.extend(vlq(tick - previous));
        body.extend(data);
        previous = tick;
    }
    body.extend(vlq(end_tick.saturating_sub(previous)));
    body.extend([0xff, 0x2f, 0]);
    let mut result = b"MTrk".to_vec();
    result.extend((body.len() as u32).to_be_bytes());
    result.extend(body);
    result
}
fn tick(frame: usize) -> u64 {
    ((frame as f64 * 960.0 / DRIVER_UPDATE_HZ).round_ties_even()) as u64
}

pub(super) fn write_midi(
    channels: &[Decoded],
    kinds: &[Kind],
    path: &Path,
    title: &str,
) -> Result<MidiInfo> {
    ensure!(
        channels.len() <= 15 && channels.len() == kinds.len(),
        "Invalid MIDI channel count"
    );
    let mut conductor = vec![
        (0, 0, meta(3, title.as_bytes())),
        (0, 1, meta(0x51, &1_000_000u32.to_be_bytes()[1..])),
        (0, 2, meta(1, b"ROM timbres via companion SoundFont")),
    ];
    // One song-wide loop for players (loopStart/loopEnd markers): every
    // looping channel repeats with the same period, but PCM and PSG channels
    // may enter their loop a few frames apart, so take the most common
    // period and the earliest start among the channels that use it.
    let loops: Vec<(usize, usize)> = channels
        .iter()
        .filter(|d| d.stop_reason == StopReason::Loop)
        .filter_map(|d| d.loop_frame.map(|start| (start, d.end_frame - start)))
        .collect();
    let period = loops
        .iter()
        .map(|&(_, p)| p)
        .max_by_key(|&p| loops.iter().filter(|&&(_, q)| q == p).count());
    if let Some(period) = period {
        let start = loops.iter().filter(|&&(_, p)| p == period).map(|&(s, _)| s).min().unwrap_or(0);
        conductor.push((tick(start), 3, meta(0x06, b"loopStart")));
        conductor.push((tick(start + period), 4, meta(0x06, b"loopEnd")));
    }
    let mut tracks = vec![track(conductor)];
    let mut total_notes = 0;
    for (index, (decoded, &kind)) in channels.iter().zip(kinds).enumerate() {
        let percussion = kind == Kind::Control;
        let bend_range = bend_range(decoded, kind)?;
        let midi_channel = if percussion {
            9
        } else if index < 9 {
            index as u8
        } else {
            index as u8 + 1
        };
        let name = if percussion {
            format!("Native percussion trigger {}", index + 1)
        } else {
            format!("Voice {}", index + 1)
        };
        let mut stream = vec![(0, 0, meta(3, name.as_bytes()))];
        if !percussion {
            for (i, (controller, value)) in [
                (101, 0),
                (100, 0),
                (6, bend_range),
                (38, 0),
                (101, 127),
                (100, 127),
            ]
            .iter()
            .enumerate()
            {
                stream.push((
                    0,
                    i as i32 - 10,
                    vec![0xb0 | midi_channel, *controller, *value],
                ));
            }
        }
        let mut preset = None;
        let mut bend = None;
        let mut pan = None;
        for event in &decoded.events {
            let at = tick(event.frame);
            let pitch = if percussion {
                event.source_pitch
            } else {
                event.pitch
            };
            if !event.note {
                stream.push((at, 1, vec![0x80 | midi_channel, pitch, 0]));
                continue;
            }
            let new_preset = match kind {
                Kind::Pcm => (
                    (event.sample_index / 128) as u8,
                    (event.sample_index % 128) as u8,
                ),
                Kind::Square => (2, event.duty),
                Kind::Wave => (3, event.wave_index),
                Kind::Noise => (4, 0),
                Kind::Control => (128, 0),
            };
            if preset != Some(new_preset) {
                let (bank, program) = new_preset;
                if !percussion {
                    stream.push((at, -3, vec![0xb0 | midi_channel, 0, bank]));
                    stream.push((at, -2, vec![0xb0 | midi_channel, 32, bank]));
                    stream.push((at, -1, vec![0xc0 | midi_channel, program]));
                } else {
                    stream.push((at, -1, vec![0xc0 | midi_channel, program]));
                }
                preset = Some(new_preset);
            }
            if !percussion && kind != Kind::Noise {
                let displacement = bend_displacement(event, kind);
                let value = (8192.0
                    + (displacement * 8192.0 / f64::from(bend_range)).round_ties_even())
                .clamp(0.0, 16383.0) as u16;
                if bend != Some(value) {
                    stream.push((
                        at,
                        0,
                        vec![0xe0 | midi_channel, (value & 127) as u8, (value >> 7) as u8],
                    ));
                    bend = Some(value);
                }
            }
            if kind == Kind::Pcm {
                ensure!(event.pan <= 20, "PCM pan outside native range");
                let value = (event.pan as f64 * 127.0 / 20.0).round_ties_even() as u8;
                if pan != Some(value) {
                    stream.push((at, 1, vec![0xb0 | midi_channel, 10, value]));
                    pan = Some(value);
                }
            }
            stream.push((at, 2, vec![0x90 | midi_channel, pitch, event.velocity]));
            total_notes += 1;
        }
        match decoded.stop_reason {
            StopReason::Loop => stream.push((tick(decoded.end_frame), 3, meta(0x01, b"LOOP"))),
            StopReason::DurationCap => {
                stream.push((tick(decoded.end_frame), 3, meta(0x01, b"TRUNCATED")))
            }
            StopReason::Terminator => {}
        }
        tracks.push(track(stream));
    }
    let mut payload = b"MThd".to_vec();
    payload.extend(6u32.to_be_bytes());
    payload.extend(1u16.to_be_bytes());
    payload.extend((tracks.len() as u16).to_be_bytes());
    payload.extend(960u16.to_be_bytes());
    for t in tracks {
        payload.extend(t);
    }
    fs::write(path, &payload)?;
    Ok(MidiInfo {
        notes: total_notes,
        tracks: channels.len(),
        sha256: super::sha(&payload),
    })
}

/// A one-second middle-C audition of a ROM PCM preset in the companion bank.
pub(super) fn write_sample_preview(index: usize, path: &Path) -> Result<()> {
    let name = format!("PCM {index:03}");
    let title = track(vec![
        (0, 0, meta(3, name.as_bytes())),
        (0, 1, meta(0x51, &1_000_000u32.to_be_bytes()[1..])),
    ]);
    let voice = track(vec![
        (0, 0, meta(3, b"PCM sample audition")),
        (0, 1, vec![0xb0, 0, (index / 128) as u8]),
        (0, 2, vec![0xb0, 32, (index / 128) as u8]),
        (0, 3, vec![0xc0, (index % 128) as u8]),
        (0, 4, vec![0x90, 36, 100]),
        (960, 0, vec![0x80, 36, 0]),
    ]);
    let mut bytes = b"MThd".to_vec();
    bytes.extend(6u32.to_be_bytes());
    bytes.extend(1u16.to_be_bytes());
    bytes.extend(2u16.to_be_bytes());
    bytes.extend(960u16.to_be_bytes());
    bytes.extend(title);
    bytes.extend(voice);
    fs::write(path, bytes)?;
    Ok(())
}
