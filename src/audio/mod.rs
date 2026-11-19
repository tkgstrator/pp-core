//! Native PP4/PP6 GBA sound extraction.
mod midi;
mod sequence;
mod soundfont;
mod synth;

use crate::Game;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

pub const DRIVER_UPDATE_HZ: f64 = 16_777_216.0 / (64.0 * 1049.0 * 4.0);

#[derive(Clone, Debug)]
pub struct AudioOptions {
    pub render_tracks: bool,
    pub selected_ids: Option<BTreeSet<u32>>,
    pub bgm_seconds: f64,
    pub se_seconds: f64,
}
impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            render_tracks: true,
            selected_ids: None,
            bgm_seconds: 90.0,
            se_seconds: 20.0,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Sample {
    pub offset: usize,
    pub next: usize,
    pub data_offset: usize,
    pub sample_type: u32,
    pub pitch_q10: u32,
    pub rate: u32,
    pub loop_start: usize,
    pub length: usize,
    pub trailer_hex: String,
}
impl Sample {
    pub fn looped(&self) -> bool {
        self.sample_type & 0x4000_0000 != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Pcm,
    Square,
    Wave,
    Control,
    Noise,
}
impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pcm => "pcm",
            Self::Square => "square",
            Self::Wave => "wave",
            Self::Control => "control",
            Self::Noise => "noise",
        }
    }
    fn mode(self) -> &'static str {
        if self == Self::Pcm { "pcm" } else { "psg" }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Sequence {
    pub id: u32,
    pub descriptor: usize,
    pub width: usize,
    pub flags: Option<u32>,
    pub priority: Option<u32>,
    pub aux_hex: Option<String>,
    pub pointer_offset: Option<usize>,
    pub starts: Vec<usize>,
    pub kinds: Vec<Kind>,
    pub psg_types: Vec<Option<u8>>,
}

#[derive(Clone, Debug)]
pub(super) struct Percussion {
    pub index: usize,
    pub table_entry: usize,
    pub start: usize,
    pub end: usize,
}

pub(super) fn le16(rom: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        rom.get(at..at + 2)
            .context("ROM u16 outside range")?
            .try_into()?,
    ))
}
pub(super) fn le32(rom: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        rom.get(at..at + 4)
            .context("ROM u32 outside range")?
            .try_into()?,
    ))
}
fn sha(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
fn hex(value: usize) -> String {
    format!("{value:#x}")
}
fn sample_params(game: Game) -> (usize, usize, usize, usize) {
    match game {
        Game::Pp4 => (0x6f101c, 0x7fc1f0, 155, 0x513730),
        Game::Pp6 => (0x722da8, 0x7f53f8, 205, 0x55df6c),
    }
}
fn seq_params(game: Game, cat: &str) -> (usize, u32, usize, usize, usize, usize) {
    match (game, cat) {
        (Game::Pp4, "bgm") => (0x512010, 0x40, 164, 32, 0x0cef6a, 0x113d83),
        (Game::Pp4, _) => (0x116f2c, 0x200, 190, 8, 0x113d83, 0x116f2c),
        (Game::Pp6, "bgm") => (0x135f0c, 0x40, 153, 12, 0x0fbeb5, 0x135f0c),
        (Game::Pp6, _) => (0x13bd68, 0x200, 435, 12, 0x136638, 0x13bd68),
    }
}
fn perc_params(game: Game) -> (usize, usize, usize) {
    match game {
        Game::Pp4 => (0x513c60, 0x11751c, 0x117650),
        Game::Pp6 => (0x55e574, 0x13d1cc, 0x13d30c),
    }
}

fn scan_samples(rom: &[u8], game: Game) -> Result<Vec<Sample>> {
    ensure!(rom.len() == 8 * 1024 * 1024, "Expected 8 MiB ROM");
    let (start, end, expected, table) = sample_params(game);
    let mut pos = start;
    let mut result = Vec::new();
    while pos < end {
        ensure!(pos + 16 <= end, "Truncated PCM header at {pos:#x}");
        let sample_type = le32(rom, pos)?;
        let pitch = le32(rom, pos + 4)?;
        let loop_start = le32(rom, pos + 8)? as usize;
        let length = le32(rom, pos + 12)? as usize;
        ensure!(
            sample_type == 0 || sample_type == 0x4000_0000,
            "Invalid PCM type at {pos:#x}"
        );
        ensure!(
            pitch % 1024 == 0 && (1000..=96000).contains(&(pitch / 1024)),
            "Invalid PCM frequency at {pos:#x}"
        );
        ensure!(
            length > 0 && pos + 16 + length < end,
            "Invalid PCM length at {pos:#x}"
        );
        ensure!(
            loop_start < length && (sample_type != 0 || loop_start == 0),
            "Invalid PCM loop at {pos:#x}"
        );
        let data_offset = pos + 16;
        let data_end = data_offset + length;
        let next = (data_end + 4) & !3;
        ensure!(next <= end, "PCM length exceeds bank at {pos:#x}");
        let trailer_hex = rom[data_end..next]
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect();
        result.push(Sample {
            offset: pos,
            next,
            data_offset,
            sample_type,
            pitch_q10: pitch,
            rate: pitch / 1024,
            loop_start,
            length,
            trailer_hex,
        });
        pos = next;
        ensure!(result.len() <= expected, "Too many PCM records");
    }
    ensure!(
        pos == end && result.len() == expected,
        "PCM bank boundary/count mismatch"
    );
    for (i, sample) in result.iter().enumerate() {
        ensure!(
            le32(rom, table + i * 4)? as usize == sample.offset - start,
            "sample index mismatch at {i}"
        );
    }
    Ok(result)
}

type SequenceScan = (BTreeMap<String, Vec<Sequence>>, HashMap<usize, usize>);

fn scan_sequences(rom: &[u8], game: Game) -> Result<SequenceScan> {
    let mut result = BTreeMap::new();
    let mut ends = HashMap::new();
    for cat in ["bgm", "se"] {
        let (table, first, count, width, region_start, region_end) = seq_params(game, cat);
        let mut seqs = Vec::new();
        let mut starts = Vec::new();
        let mut modes = HashMap::new();
        for i in 0..count {
            let descriptor = table + i * width;
            let (flags, priority, aux_hex, pointer_offset, pointers, psg_types) =
                if game == Game::Pp4 && cat == "bgm" {
                    let pointers = (0..8)
                        .map(|n| le32(rom, descriptor + n * 4))
                        .collect::<Result<Vec<_>>>()?;
                    (
                        None,
                        None,
                        None,
                        None,
                        pointers,
                        vec![Some(0), Some(1), Some(2), Some(3), None, None, None, None],
                    )
                } else {
                    let (flags, priority, pointer, aux_hex) = if game == Game::Pp4 {
                        let f = le16(rom, descriptor)? as u32;
                        let aux = rom[descriptor + 2..descriptor + 4]
                            .iter()
                            .map(|v| format!("{v:02x}"))
                            .collect();
                        (f, None, le32(rom, descriptor + 4)?, Some(aux))
                    } else {
                        (
                            le32(rom, descriptor)?,
                            Some(le32(rom, descriptor + 4)?),
                            le32(rom, descriptor + 8)?,
                            None,
                        )
                    };
                    let channel_count = if game == Game::Pp4 {
                        ((flags >> 4) & 15).count_ones() as usize
                    } else {
                        ((flags & 0x3f).count_ones() + ((flags >> 8) & 0x7f).count_ones()) as usize
                    };
                    let offset = pointer
                        .checked_sub(0x0800_0000)
                        .context("invalid sound pointer")?
                        as usize;
                    ensure!(
                        (1..=11).contains(&channel_count) && offset < rom.len(),
                        "Invalid descriptor {descriptor:#x}"
                    );
                    let pointers = if channel_count == 1 {
                        vec![pointer]
                    } else {
                        (0..channel_count)
                            .map(|n| le32(rom, offset + n * 4))
                            .collect::<Result<Vec<_>>>()?
                    };
                    let types = if game == Game::Pp4 {
                        [(0x80, Some(4)), (0x40, Some(5)), (0x20, None), (0x10, None)]
                            .iter()
                            .filter(|(bit, _)| flags & bit != 0)
                            .map(|(_, ty)| *ty)
                            .collect()
                    } else {
                        let mut v: Vec<Option<u8>> = [
                            (0, Some(if cat == "bgm" { 0 } else { 4 })),
                            (1, Some(1)),
                            (2, Some(2)),
                            (4, Some(3)),
                            (3, Some(5)),
                        ]
                        .iter()
                        .filter(|(bit, _)| flags & (1 << bit) != 0)
                        .map(|(_, ty)| *ty)
                        .collect();
                        v.extend((8..15).filter(|bit| flags & (1 << bit) != 0).map(|_| None));
                        v
                    };
                    (
                        Some(flags),
                        priority,
                        aux_hex,
                        if channel_count > 1 {
                            Some(offset)
                        } else {
                            None
                        },
                        pointers,
                        types,
                    )
                };
            ensure!(
                psg_types.len() == pointers.len(),
                "Invalid channel mask at {descriptor:#x}"
            );
            let kinds: Vec<Kind> = psg_types
                .iter()
                .map(|ty| match ty {
                    None => Kind::Pcm,
                    Some(2) => Kind::Wave,
                    Some(3) => Kind::Control,
                    Some(5) => Kind::Noise,
                    _ => Kind::Square,
                })
                .collect();
            let offsets: Vec<usize> = pointers
                .iter()
                .map(|p| {
                    p.checked_sub(0x0800_0000)
                        .map(|x| x as usize)
                        .context("Invalid track pointer")
                })
                .collect::<Result<_>>()?;
            ensure!(
                offsets
                    .iter()
                    .all(|o| *o >= region_start && *o < region_end),
                "Invalid track pointer at {descriptor:#x}"
            );
            starts.extend(&offsets);
            for (&at, &kind) in offsets.iter().zip(&kinds) {
                modes.insert(at, kind);
            }
            seqs.push(Sequence {
                id: first + i as u32,
                descriptor,
                width,
                flags,
                priority,
                aux_hex,
                pointer_offset,
                starts: offsets,
                kinds,
                psg_types,
            });
        }
        starts.sort_unstable();
        ensure!(
            starts.first() == Some(&region_start) && starts.windows(2).all(|w| w[0] != w[1]),
            "Invalid track coverage"
        );
        for (i, &start) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(region_end);
            if i + 1 < starts.len() {
                ensure!(
                    rom[end - 1] == 0xc0,
                    "Missing track end marker before {end:#x}"
                );
            }
            sequence::parse_linear(&rom[start..end], start, modes[&start])?;
            ends.insert(start, end);
        }
        result.insert(cat.to_string(), seqs);
    }
    Ok((result, ends))
}

fn scan_percussion(rom: &[u8], game: Game) -> Result<Vec<Percussion>> {
    let (table, first, end) = perc_params(game);
    let mut starts = Vec::new();
    for i in 0..16 {
        starts.push(
            le32(rom, table + i * 4)?
                .checked_sub(0x0800_0000)
                .context("Invalid percussion pointer")? as usize,
        );
    }
    ensure!(
        starts[0] == first && starts.windows(2).all(|w| w[0] < w[1]) && starts[15] < end,
        "Invalid percussion pointer table"
    );
    let mut result = Vec::new();
    for i in 0..16 {
        let stop = starts.get(i + 1).copied().unwrap_or(end);
        let tokens = sequence::parse_linear(&rom[starts[i]..stop], starts[i], Kind::Noise)?;
        ensure!(
            tokens.last().map(|x| x.end) == Some(stop),
            "Invalid percussion stream boundary"
        );
        result.push(Percussion {
            index: i,
            table_entry: table + i * 4,
            start: starts[i],
            end: stop,
        });
    }
    Ok(result)
}

pub fn extract(rom: &[u8], out: &Path, game: Game) -> Result<()> {
    extract_with_options(rom, out, game, &AudioOptions::default())
}

pub fn extract_with_options(
    rom: &[u8],
    out: &Path,
    game: Game,
    options: &AudioOptions,
) -> Result<()> {
    ensure!(
        options.bgm_seconds > 0.0 && options.se_seconds > 0.0,
        "Audio duration caps must be positive"
    );
    ensure!(
        rom.len() == 8 * 1024 * 1024
            && rom.get(0xac..0xb0) == Some(game.code().as_bytes())
            && rom[0xbc] == game.revision(),
        "Expected verified {} revision {} ROM",
        game.code(),
        game.revision()
    );
    let digest = sha(rom);
    ensure!(
        digest == game.expected_sha256(),
        "{} SHA-256 mismatch: {digest}",
        game.code()
    );
    let samples = scan_samples(rom, game)?;
    let (sequences, ends) = scan_sequences(rom, game)?;
    let percussion = scan_percussion(rom, game)?;
    if let Some(ids) = &options.selected_ids {
        for id in ids {
            ensure!(
                sequences
                    .values()
                    .flat_map(|v| v.iter())
                    .any(|s| s.id == *id),
                "Unknown audio ID {id:#x}"
            );
        }
    }
    let root = out;
    for directory in [
        "samples",
        "bgm",
        "se",
        "native/samples",
        "native/percussion",
        "native/bgm",
        "native/se",
    ] {
        fs::create_dir_all(root.join(directory))?;
    }
    let mut sample_json = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        let name = format!("{:06x}", s.offset);
        let raw_rel = format!("native/samples/{name}.bin");
        let wav_rel = format!("samples/{name}.wav");
        let midi_rel = format!("samples/{name}.mid");
        let raw = &rom[s.offset..s.next];
        let data = &rom[s.data_offset..s.data_offset + s.length];
        fs::write(root.join(&raw_rel), raw)?;
        write_sample_wav(&root.join(&wav_rel), data, s.rate)?;
        midi::write_sample_preview(i, &root.join(&midi_rel))?;
        sample_json.push(json!({"index":i,"header_offset":hex(s.offset),"data_offset":hex(s.data_offset),"end_exclusive":hex(s.next),
            "raw_record":raw_rel,"raw_record_sha256":sha(raw),"pcm_sha256":sha(data),"wav":wav_rel,"midi":midi_rel,
            "sample_type":hex(s.sample_type as usize),"pitch_q10":hex(s.pitch_q10 as usize),"sample_rate_hz":s.rate,
            "loop_enabled":s.looped(),"loop_start":s.loop_start,"length":s.length,"trailer_hex":s.trailer_hex}));
    }
    let mut perc_json = Vec::new();
    for p in &percussion {
        let rel = format!("native/percussion/{:02x}.bin", p.index);
        let data = &rom[p.start..p.end];
        fs::write(root.join(&rel), data)?;
        perc_json.push(json!({"index":p.index,"native_pitch":hex(0x50+p.index),"table_entry_offset":hex(p.table_entry),"stream_offset":hex(p.start),
            "end_exclusive":hex(p.end),"raw":rel,"sha256":sha(data),"mode":"PSG type 5 noise subtrack"}));
    }
    let mut seq_json = serde_json::Map::new();
    let mut keysets = soundfont::PsgKeys::default();
    let mut bgm_midis = 0;
    let mut se_midis = 0;
    let mut triggers = 0;
    let mut silent = 0;
    for cat in ["bgm", "se"] {
        let mut items = Vec::new();
        let (_, _, _, _, _, region_end) = seq_params(game, cat);
        for seq in &sequences[cat] {
            let stem = format!("{cat}/{:03x}", seq.id);
            let native_prefix = format!("native/{stem}");
            let native_folder = root.join(&native_prefix);
            fs::create_dir_all(&native_folder)?;
            let descriptor = &rom[seq.descriptor..seq.descriptor + seq.width];
            fs::write(native_folder.join("descriptor.bin"), descriptor)?;
            let mut item = json!({"id":hex(seq.id as usize),"descriptor_offset":hex(seq.descriptor),
                "descriptor_raw":format!("{native_prefix}/descriptor.bin"),"descriptor_sha256":sha(descriptor),
                "flags":seq.flags.map(|x|hex(x as usize)),"priority":seq.priority.map(|x|hex(x as usize)),
                "aux_bytes_hex":seq.aux_hex,"channel_count":seq.starts.len(),"tracks":[]});
            if let Some(pointer) = seq.pointer_offset {
                let data = &rom[pointer..pointer + seq.starts.len() * 4];
                fs::write(native_folder.join("pointers.bin"), data)?;
                item["pointer_list"] = json!({"offset":hex(pointer),"raw":format!("{native_prefix}/pointers.bin"),"sha256":sha(data)});
            }
            let mut track_json = Vec::new();
            for (channel, &start) in seq.starts.iter().enumerate() {
                let end = ends[&start];
                let data = &rom[start..end];
                let tokens = sequence::parse_linear(data, start, seq.kinds[channel])?;
                let rel = format!("{native_prefix}/track_{channel:02}.bin");
                fs::write(root.join(&rel), data)?;
                let term = tokens.last().unwrap().end;
                track_json.push(json!({"channel":channel,"mode":seq.kinds[channel].mode(),"kind":seq.kinds[channel].as_str(),
                    "start":hex(start),"end_exclusive":hex(end),"boundary_basis":if end==region_end{"declared track region end"}else{"next referenced track"},
                    "linear_token_count":tokens.len(),"first_terminator_end":hex(term),"post_terminator_bytes":end-term,"raw":rel,"sha256":sha(data)}));
            }
            item["tracks"] = json!(track_json);
            if options.render_tracks
                && options
                    .selected_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&seq.id))
            {
                let cap = if cat == "bgm" {
                    options.bgm_seconds
                } else {
                    options.se_seconds
                };
                let max_frames = (cap * DRIVER_UPDATE_HZ).ceil() as usize;
                let mut decoded = Vec::new();
                for (channel, &start) in seq.starts.iter().enumerate() {
                    decoded.push(sequence::decode(
                        &rom[start..ends[&start]],
                        start,
                        max_frames,
                        seq.kinds[channel],
                        seq.psg_types[channel],
                        game,
                    )?);
                }
                for (d, &kind) in decoded.iter().zip(&seq.kinds) {
                    keysets.note_keys(d, kind);
                }
                let mut drums = Vec::new();
                for (d, &kind) in decoded.iter().zip(&seq.kinds) {
                    if kind == Kind::Control {
                        drums.extend(sequence::expand_percussion(
                            rom,
                            d,
                            &percussion,
                            game,
                            max_frames,
                        )?);
                    }
                }
                let drum_notes = drums
                    .iter()
                    .flat_map(|d| &d.events)
                    .filter(|e| e.is_note())
                    .count();
                triggers += drums.len();
                let midi_rel = format!("{stem}.mid");
                let wav_rel = format!("{stem}.wav");
                let midi_path = root.join(&midi_rel);
                let midi_info = midi::write_midi(
                    &decoded,
                    &seq.kinds,
                    &midi_path,
                    &format!(
                        "{} {} {:#x}",
                        game.id().to_uppercase(),
                        cat.to_uppercase(),
                        seq.id
                    ),
                )?;
                let wav_path = root.join(&wav_rel);
                let wav_info = synth::synthesize(
                    rom,
                    game,
                    &decoded,
                    &seq.kinds,
                    &drums,
                    &samples,
                    synth::WavOutput {
                        path: &wav_path,
                        rate: 32768,
                        stereo: true,
                    },
                )?;
                item["midi"] = json!({"path":midi_rel,"notes":midi_info.notes,"tracks":midi_info.tracks,
                    "sha256":midi_info.sha256,"update_rate_hz":DRIVER_UPDATE_HZ,"timebase":"960 ticks per second","native_soundfont":true});
                item["wav"] = json!({"path":wav_rel,"sha256":wav_info.sha256,"sample_rate_hz":32768,
                    "channels":2,"sample_frames":wav_info.frames,"duration_seconds":wav_info.frames as f64/32768.0,
                    "peak":wav_info.peak,"notes":wav_info.notes,
                    "method":"direct-ROM PCM, PSG pitch/noise tables and wave RAM; linear PCM pan, PSG duplicated; envelope and mix approximated"});
                item["percussion"] = json!({"triggers":drums.len(),"expanded_notes":drum_notes,"midi_mapping":"native pitch 0x50..0x5f on channel 10",
                    "wav_source":"ROM PSG type-5 subtracks; parent trigger nibble subtracted from child note volume; noise timbre approximated"});
                item["content_status"] = json!(if wav_info.notes == 0 {
                    "no_decoded_notes"
                } else if wav_info.peak > 0 {
                    "rendered_notes"
                } else {
                    "silent_waveform"
                });
                item["decode"] = json!({"duration_cap_seconds":cap,"loop_policy":"one pass per channel, stop at first unconditional loop",
                    "channel_stop_reasons":decoded.iter().map(|d|d.stop_reason.as_str()).collect::<Vec<_>>(),
                    "channel_end_updates":decoded.iter().map(|d|d.end_frame).collect::<Vec<_>>()});
                if wav_info.peak == 0 {
                    silent += 1;
                }
                if cat == "bgm" {
                    bgm_midis += 1;
                } else {
                    se_midis += 1;
                }
            } else {
                for extension in ["mid", "wav"] {
                    let p = root.join(format!("{stem}.{extension}"));
                    if p.exists() {
                        fs::remove_file(p)?;
                    }
                }
            }
            items.push(item);
        }
        seq_json.insert(cat.to_string(), json!(items));
    }
    let soundfont_info = if options.render_tracks {
        Some(soundfont::write_soundfont(
            rom,
            game,
            &samples,
            &percussion,
            &keysets,
            &root.join("rom.sf2"),
        )?)
    } else {
        let p = root.join("rom.sf2");
        if p.exists() {
            fs::remove_file(p)?;
        }
        None
    };
    let (start, end, count, index_table) = sample_params(game);
    let mut tables = serde_json::Map::new();
    for cat in ["bgm", "se"] {
        let (table, first, n, width, lo, hi) = seq_params(game, cat);
        tables.insert(
            cat.to_string(),
            json!({"offset":hex(table),"first_id":hex(first as usize),"entry_count":n,
            "descriptor_bytes":width,"track_region":[hex(lo),hex(hi)]}),
        );
    }
    let manifest = json!({"rom_game_code":game.code(),"rom_sha256":digest,
        "source_range":{"start":hex(start),"end_exclusive":hex(end)},
        "sample_index_table":{"offset":hex(index_table),"base":hex(start),"entry_count":count,"encoding":"u32 little-endian relative offsets"},
        "sample_format":"mono signed 8-bit PCM in ROM; WAV uses unsigned 8-bit PCM",
        "pitch_units":"Q10 sample playback rate at source pitch 36 before per-sample transposition",
        "sequence_status":format!("verified native BGM/SE descriptors and bounded tracks; {}",if options.render_tracks{"direct note transcriptions and reference WAVs"}else{"playback undecoded"}),
        "classification":format!("native BGM/SE bytecode and playable PCM samples; {}",if options.render_tracks{"MIDI with companion ROM SoundFont and approximate WAV synthesis"}else{"no complete track renders"}),
        "counts":{"sample_wavs":samples.len(),"raw_records":samples.len(),"bgm_sequences":sequences["bgm"].len(),"se_sequences":sequences["se"].len(),
            "raw_tracks":ends.len(),"bgm_midis":bgm_midis,"se_midis":se_midis,"bgm_renders":bgm_midis,"se_renders":se_midis,
            "percussion_subtracks":percussion.len(),"percussion_triggers":triggers,"silent_renders":silent},
        "conversion_policy":{"driver_update_hz":DRIVER_UPDATE_HZ,"timer2":"16777216 / (64 * 1049 * 4) channel updates per second",
            "bgm_cap_seconds":options.bgm_seconds,"se_cap_seconds":options.se_seconds,
            "midi":"native note/gate/wait timing, per-note ROM SoundFont bank/program, and PCM CC10 pan (0..20 mapped to 0..127); GS bank select",
            "wav":"direct ROM PCM and pitch tables; measured linear PCM pan; square A6..AD release timing; PSG routing, wave/noise envelope and mixer approximate",
            "soundfont":"ROM PCM with loop/root transpose, keyed square/wave/noise and rendered percussion; envelope, pan, gain and PSG fractional pitch approximate",
            "percussion":"native type-3 trigger 0x50..0x5f selects one of 16 ROM PSG noise subtracks; parent trigger volume nibble subtracts from child note volume in reference WAV",
            "loops":"first pass of unconditional loops; channels reaching cap are truncated"},
        "sequence_tables":Value::Object(tables),"sequences":Value::Object(seq_json),"percussion_subtracks":perc_json,
        "soundfont":soundfont_info,"samples":sample_json});
    let payload = serde_json::to_string_pretty(&manifest)? + "\n";
    fs::write(root.join("manifest.json"), payload)?;
    Ok(())
}

fn write_sample_wav(path: &Path, data: &[u8], rate: u32) -> Result<()> {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 8,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for &byte in data {
        writer.write_sample(byte as i8)?;
    }
    writer.finalize()?;
    Ok(())
}
