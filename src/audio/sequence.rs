use super::{Game, Kind, Percussion};
use anyhow::{Result, bail, ensure};

#[derive(Clone, Debug)]
pub(super) struct Token {
    pub end: usize,
}
pub(super) fn operand_count(opcode: u8, kind: Kind) -> usize {
    if opcode < 0x80 {
        return 3
            - ((opcode & 0x10 != 0) as usize)
            - ((opcode & 0x20 != 0) as usize)
            - ((opcode & 0x40 != 0) as usize);
    }
    if (0xf1..=0xf6).contains(&opcode) {
        return 3;
    }
    if opcode == 0xf0 {
        return if kind == Kind::Pcm { 2 } else { 3 };
    }
    if (0xe0..=0xe3).contains(&opcode) {
        return 2;
    }
    if (0xd0..=0xd6).contains(&opcode) || (0xd8..=0xdc).contains(&opcode) {
        return 1;
    }
    0
}
pub(super) fn parse_linear(data: &[u8], offset: usize, kind: Kind) -> Result<Vec<Token>> {
    let mut cursor = 0;
    let mut result = Vec::new();
    while cursor < data.len() {
        let at = cursor;
        let opcode = data[cursor];
        cursor += 1;
        let count = operand_count(opcode, kind);
        ensure!(
            cursor + count <= data.len(),
            "Truncated sound opcode {opcode:#x} at {:#x}",
            offset + at
        );
        cursor += count;
        result.push(Token {
            end: offset + cursor,
        });
        if opcode == 0xc0 {
            return Ok(result);
        }
    }
    bail!("Missing sound bytecode terminator at {offset:#x}")
}

#[derive(Clone, Debug)]
pub(super) struct Event {
    pub frame: usize,
    pub pitch: u8,
    pub source_pitch: u8,
    pub note: bool,
    pub pitch_fixed: u16,
    pub volume: u8,
    pub velocity: u8,
    pub volume_index: u8,
    pub sample_bank: usize,
    pub sample_local: usize,
    pub sample_index: usize,
    pub pan: u8,
    pub duty: u8,
    pub wave_index: u8,
    pub release_period: u8,
}
impl Event {
    pub fn is_note(&self) -> bool {
        self.note
    }
    fn off(frame: usize, pitch: u8, source_pitch: u8, release_period: u8) -> Self {
        Self {
            frame,
            pitch,
            source_pitch,
            note: false,
            pitch_fixed: 0,
            volume: 0,
            velocity: 0,
            volume_index: 0,
            sample_bank: 0,
            sample_local: 0,
            sample_index: 0,
            pan: 10,
            duty: 0,
            wave_index: 0,
            release_period,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StopReason {
    Terminator,
    Loop,
    DurationCap,
}
impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Terminator => "terminator",
            Self::Loop => "loop",
            Self::DurationCap => "duration_cap",
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Decoded {
    pub events: Vec<Event>,
    pub stop_reason: StopReason,
    pub end_frame: usize,
}
impl Decoded {
    fn done(events: Vec<Event>, stop_reason: StopReason, end_frame: usize) -> Self {
        Self {
            events,
            stop_reason,
            end_frame,
        }
    }
}
fn signed8(x: u8) -> i32 {
    x as i8 as i32
}
fn note_off(
    events: &mut Vec<Event>,
    active: &mut Option<(u8, u8)>,
    frame: usize,
    kind: Kind,
    psg_type: Option<u8>,
    release: u8,
) {
    if let Some((pitch, source)) = active.take() {
        let r = if kind == Kind::Square && matches!(psg_type, Some(0 | 1 | 4)) {
            release
        } else {
            0
        };
        events.push(Event::off(frame, pitch, source, r));
    }
}

pub(super) fn decode(
    data: &[u8],
    source_offset: usize,
    max_frames: usize,
    kind: Kind,
    psg_type: Option<u8>,
    game: Game,
) -> Result<Decoded> {
    ensure!(max_frames > 0, "max_frames must be positive");
    let mut cursor = 0usize;
    let mut phase = 0x80u32;
    let mut tempo = 0x80u32;
    let mut wait = 1u16;
    let mut gate = 0u16;
    let mut gate_cache = if kind == Kind::Pcm { 1u8 } else { 0 };
    let mut wait_cache = 0u8;
    let mut pitch_cache = 0u8;
    let mut sample_bank = 0usize;
    let mut sample_local = 0usize;
    let mut timbre_byte = 0u8;
    let mut pitch_base = 0i32;
    let mut pitch_bend = 0i32;
    let mut psg_volume_offset = 0i32;
    let mut square_release = 0u8;
    let mut pcm_master_volume = 255i32;
    let mut pcm_pan = 10u8;
    let mut loop_pitch = [0i32; 2];
    let mut loop_voice = [0i32; 2];
    let mut loop_cursor = [None; 2];
    let mut loop_iteration = [0u32; 2];
    let mut unconditional_cursor = None;
    let mut conditional_cursor = None;
    let mut conditional_return = None;
    let mut conditional_state = 0u8;
    let mut active = None;
    let mut events = Vec::new();
    let mut commands = 0usize;
    for frame in 0..max_frames {
        phase += tempo;
        if phase <= 255 {
            continue;
        }
        phase -= 256;
        wait = (wait.wrapping_sub(1)) & 255;
        if gate > 0 {
            gate -= 1;
            if gate == 0 {
                note_off(
                    &mut events,
                    &mut active,
                    frame,
                    kind,
                    psg_type,
                    square_release,
                );
            }
        }
        if wait > 0 {
            continue;
        }
        loop {
            commands += 1;
            ensure!(
                commands <= 1_000_000 && cursor < data.len(),
                "Sound bytecode cursor escaped track at {:#x}",
                source_offset + cursor
            );
            let opcode = data[cursor];
            let start = cursor;
            cursor += 1;
            let count = operand_count(opcode, kind);
            ensure!(
                cursor + count <= data.len(),
                "Truncated sound opcode at {:#x}",
                source_offset + start
            );
            let operands = &data[cursor..cursor + count];
            cursor += count;
            if opcode < 0x80 {
                let mut arg = 0;
                if opcode & 0x10 == 0 {
                    gate_cache = operands[arg];
                    arg += 1;
                }
                if opcode & 0x20 == 0 {
                    wait_cache = operands[arg];
                    arg += 1;
                }
                if opcode & 0x40 == 0 {
                    pitch_cache = operands[arg];
                }
                note_off(
                    &mut events,
                    &mut active,
                    frame,
                    kind,
                    psg_type,
                    square_release,
                );
                let pitch_fixed = ((signed8(pitch_cache) << 8)
                    + pitch_base
                    + pitch_bend
                    + loop_pitch.iter().sum::<i32>()) as u16;
                let signed_pitch = pitch_fixed as i16 as i32;
                let pitch_offset = if kind != Kind::Pcm {
                    if psg_type == Some(2) { 24 } else { 36 }
                } else {
                    0
                };
                let pitch = (signed_pitch.div_euclid(256) + pitch_offset).clamp(0, 127) as u8;
                let (volume, velocity) = if kind == Kind::Pcm {
                    let base = ((opcode & 15) as i32 * 8 + loop_voice.iter().sum::<i32>()).max(0);
                    let volume = ((base * pcm_master_volume) >> 7).min(255);
                    (
                        volume as u8,
                        ((volume as f64 / 2.0).round_ties_even() as i32).clamp(1, 127) as u8,
                    )
                } else {
                    let base = ((opcode & 15) as i32 - psg_volume_offset
                        + loop_voice.iter().sum::<i32>())
                    .clamp(0, 15);
                    ((base * 17) as u8, (base * 8).clamp(1, 127) as u8)
                };
                let sample_index = if kind == Kind::Pcm {
                    let banks: &[usize] = match game {
                        Game::Pp4 => &[0, 45, 46],
                        Game::Pp6 => &[0, 56],
                    };
                    ensure!(
                        sample_bank < banks.len(),
                        "Unknown PCM sample bank at {:#x}",
                        source_offset + start
                    );
                    let index = banks[sample_bank] + sample_local;
                    ensure!(
                        index < if game == Game::Pp4 { 155 } else { 205 },
                        "PCM sample index out of range at {:#x}",
                        source_offset + start
                    );
                    index
                } else {
                    0
                };
                events.push(Event {
                    frame,
                    pitch,
                    source_pitch: pitch_cache,
                    note: true,
                    pitch_fixed,
                    volume,
                    velocity,
                    volume_index: opcode & 15,
                    sample_bank,
                    sample_local,
                    sample_index,
                    pan: pcm_pan,
                    duty: timbre_byte >> 6,
                    wave_index: if psg_type == Some(2) { timbre_byte } else { 0 },
                    release_period: 0,
                });
                active = Some((pitch, pitch_cache));
                gate = gate_cache as u16;
                wait = if wait_cache == 0 {
                    256
                } else {
                    wait_cache as u16
                };
                break;
            }
            if opcode == 0xc0 {
                note_off(
                    &mut events,
                    &mut active,
                    frame,
                    kind,
                    psg_type,
                    square_release,
                );
                return Ok(Decoded::done(events, StopReason::Terminator, frame));
            }
            match opcode {
                0xc1 | 0xc2 => {
                    let slot = (opcode - 0xc1) as usize;
                    loop_cursor[slot] = Some(cursor);
                    loop_iteration[slot] = 0;
                    loop_pitch[slot] = 0;
                    loop_voice[slot] = 0;
                }
                0xc3 => unconditional_cursor = Some(cursor),
                0xc4 => {
                    conditional_cursor = Some(cursor);
                    conditional_state = 0;
                }
                0xc5 => {
                    if conditional_state == 0 {
                        conditional_state = 1;
                    } else if conditional_state == 1 {
                        conditional_state = 2;
                        conditional_return = Some(cursor);
                        cursor = conditional_cursor
                            .ok_or_else(|| anyhow::anyhow!("Uninitialized conditional loop"))?;
                    } else {
                        conditional_state = 1;
                        cursor = conditional_return
                            .ok_or_else(|| anyhow::anyhow!("Uninitialized conditional return"))?;
                    }
                }
                0xc6 => {
                    let _target = unconditional_cursor.ok_or_else(|| {
                        anyhow::anyhow!("Uninitialized sound loop at {:#x}", source_offset + start)
                    })?;
                    note_off(
                        &mut events,
                        &mut active,
                        frame,
                        kind,
                        psg_type,
                        square_release,
                    );
                    return Ok(Decoded::done(events, StopReason::Loop, frame));
                }
                0xd0 => tempo = operands[0] as u32,
                0xd1 => {
                    if kind == Kind::Pcm {
                        sample_local = operands[0] as usize
                    } else {
                        timbre_byte = operands[0];
                    }
                }
                0xd2 => {
                    if kind == Kind::Pcm {
                        pcm_master_volume = operands[0] as i32
                    } else {
                        psg_volume_offset = operands[0] as i32;
                    }
                }
                0xd3 if kind == Kind::Pcm => pcm_pan = operands[0].min(20),
                0xdc if kind == Kind::Pcm => sample_bank = operands[0] as usize,
                0xe0 | 0xf0 if kind == Kind::Pcm => sample_local = operands[1] as usize,
                0xd4 => pitch_base = (operands[0] as i32) << 8,
                0xd5 => pitch_bend = signed8(operands[0]) << 2,
                0xd9 => {
                    note_off(
                        &mut events,
                        &mut active,
                        frame,
                        kind,
                        psg_type,
                        square_release,
                    );
                    wait_cache = operands[0];
                    wait = if wait_cache == 0 {
                        256
                    } else {
                        wait_cache as u16
                    };
                    break;
                }
                0xe3 => {
                    wait_cache = operands[0];
                    gate_cache = operands[1];
                    wait = if wait_cache == 0 {
                        256
                    } else {
                        wait_cache as u16
                    };
                    gate = gate_cache as u16;
                    break;
                }
                0xf3 | 0xf4 => {
                    let slot = (opcode - 0xf3) as usize;
                    let target = loop_cursor[slot].ok_or_else(|| {
                        anyhow::anyhow!(
                            "Uninitialized sound repeat at {:#x}",
                            source_offset + start
                        )
                    })?;
                    let repeats = operands[0] as u32;
                    loop_iteration[slot] += 1;
                    if repeats != 0 && loop_iteration[slot] == repeats {
                        loop_iteration[slot] = 0;
                        loop_pitch[slot] = 0;
                        loop_voice[slot] = 0;
                    } else if repeats == 0 {
                        note_off(
                            &mut events,
                            &mut active,
                            frame,
                            kind,
                            psg_type,
                            square_release,
                        );
                        return Ok(Decoded::done(events, StopReason::Loop, frame));
                    } else {
                        let bound = if kind == Kind::Pcm { 255 } else { 15 };
                        loop_voice[slot] =
                            (loop_voice[slot] + signed8(operands[1])).clamp(-bound, bound);
                        loop_pitch[slot] =
                            (loop_pitch[slot] + (signed8(operands[2]) << 3)) & 0xffff;
                        cursor = target;
                    }
                }
                _ => {
                    if kind != Kind::Pcm && (0x80..=0x83).contains(&opcode) {
                        timbre_byte = (opcode - 0x80) << 6;
                    } else if kind != Kind::Pcm && (0x84..=0x93).contains(&opcode) {
                        timbre_byte = opcode - 0x84;
                    } else if kind != Kind::Pcm && (0x96..=0xa5).contains(&opcode) {
                        psg_volume_offset = (opcode - 0x96) as i32;
                    } else if kind == Kind::Square
                        && matches!(psg_type, Some(0 | 1 | 4))
                        && (0xa6..=0xad).contains(&opcode)
                    {
                        square_release = opcode - 0xa6;
                    } else if kind == Kind::Pcm && (0x80..=0x83).contains(&opcode) {
                        sample_local = 0;
                    } else if kind == Kind::Pcm && (0x84..=0x93).contains(&opcode) {
                        sample_local = 5;
                    }
                }
            }
        }
    }
    note_off(
        &mut events,
        &mut active,
        max_frames,
        kind,
        psg_type,
        square_release,
    );
    Ok(Decoded::done(events, StopReason::DurationCap, max_frames))
}

pub(super) fn expand_percussion(
    rom: &[u8],
    control: &Decoded,
    bank: &[Percussion],
    game: Game,
    max_frames: usize,
) -> Result<Vec<Decoded>> {
    let mut templates = std::collections::HashMap::<usize, Decoded>::new();
    let mut result = Vec::new();
    for trigger in &control.events {
        if !trigger.note || trigger.source_pitch < 0x50 {
            continue;
        }
        let index = (trigger.source_pitch - 0x50) as usize;
        ensure!(
            index < bank.len(),
            "Percussion index outside ROM table: {index}"
        );
        if trigger.frame >= max_frames {
            continue;
        }
        let template = if let Some(value) = templates.get(&index) {
            value
        } else {
            let p = &bank[index];
            let decoded = decode(
                &rom[p.start..p.end],
                p.start,
                max_frames,
                Kind::Noise,
                Some(5),
                game,
            )?;
            templates.insert(index, decoded);
            &templates[&index]
        };
        let end_frame = max_frames.min(trigger.frame + template.end_frame);
        let mut events = Vec::new();
        let mut active = None;
        for event in &template.events {
            let frame = trigger.frame + event.frame;
            if frame > max_frames {
                break;
            }
            let mut shifted = event.clone();
            shifted.frame = frame;
            if event.note {
                let nibble = ((event.volume as usize / 17)
                    .saturating_sub(trigger.volume_index as usize))
                    as u8;
                shifted.volume = nibble * 17;
                shifted.velocity = (nibble as u16 * 8).clamp(1, 127) as u8;
                active = Some(shifted.pitch);
            } else {
                active = None;
            }
            events.push(shifted);
        }
        if let Some(pitch) = active
            && end_frame == max_frames
        {
            events.push(Event::off(end_frame, pitch, pitch, 0));
        }
        result.push(Decoded {
            events,
            end_frame,
            stop_reason: if end_frame == max_frames {
                StopReason::DurationCap
            } else {
                template.stop_reason
            },
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};
    #[test]
    fn note_cache_timing_and_pitch() {
        let data = [
            0xd0, 0x80, 0x09, 0x01, 0x02, 0x18, 0x39, 0x19, 0xd9, 0x03, 0xc0,
        ];
        let d = decode(&data, 0x123400, 100, Kind::Square, Some(0), Game::Pp4).unwrap();
        let notes: Vec<_> = d
            .events
            .iter()
            .filter(|e| e.note)
            .map(|e| (e.frame, e.pitch))
            .collect();
        assert_eq!(notes, vec![(0, 60), (4, 61)]);
    }
    #[test]
    fn parent_percussion_nibble_reduces_child_noise_volume() {
        for (file, game, sound_id) in [("pp4", Game::Pp4, 0x40), ("pp6", Game::Pp6, 0x7b)] {
            let path = format!("/Users/devonly/Developer/biccame-pocket/assets/roms/{file}.gba");
            if !Path::new(&path).exists() {
                continue;
            }
            let rom = fs::read(path).unwrap();
            let (sequences, ends) = super::super::scan_sequences(&rom, game).unwrap();
            let bank = super::super::scan_percussion(&rom, game).unwrap();
            let song = sequences["bgm"].iter().find(|s| s.id == sound_id).unwrap();
            let channel = song.kinds.iter().position(|k| *k == Kind::Control).unwrap();
            let start = song.starts[channel];
            let control = decode(
                &rom[start..ends[&start]],
                start,
                100,
                Kind::Control,
                Some(3),
                game,
            )
            .unwrap();
            let drums = expand_percussion(&rom, &control, &bank, game, 100).unwrap();
            let first: Vec<_> = drums[0]
                .events
                .iter()
                .filter(|e| e.note)
                .take(2)
                .map(|e| e.volume / 17)
                .collect();
            assert_eq!(first, [2, 1], "{file} percussion volume");
        }
    }
}
