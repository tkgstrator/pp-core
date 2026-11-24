//! Native PP4/PP6 GBA sound extraction.
pub(crate) mod midi;
mod sequence;
pub(crate) use sequence::TrackFormat;
mod sound_blob;
pub use sound_blob::pp6_sound_blob;
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
    /// Render only this channel of each sequence in the reference WAV (and
    /// the percussion it triggers); for comparing one part with hardware.
    pub solo_channel: Option<usize>,
}
impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            render_tracks: true,
            selected_ids: None,
            bgm_seconds: 90.0,
            se_seconds: 20.0,
            solo_channel: None,
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
pub(crate) fn sample_params(game: Game, pp4_v0: bool) -> (usize, usize, usize, usize) {
    if pp4_v0 {
        return (0x6f0794, 0x7fb968, 155, 0x512f64);
    }
    match game {
        Game::Pp4 => (0x6f101c, 0x7fc1f0, 155, 0x513730),
        Game::Pp6 => (0x722da8, 0x7f53f8, 205, 0x55df6c),
    }
}
type SequenceTable = (usize, u32, usize, usize, usize, usize);
type PercussionTable = (usize, usize, usize);

#[derive(Clone, Copy)]
pub(crate) struct Playback {
    pub id: &'static str,
    pub psg_periods: usize,
    pub pcm_rates: usize,
    pub transpose: usize,
    pub sample_bases: &'static [usize],
    pub sample_count: usize,
}

impl Playback {
    pub(crate) const fn pp5(psg_periods: usize, pcm_rates: usize, transpose: usize) -> Self {
        Self {
            id: "pp5",
            psg_periods,
            pcm_rates,
            transpose,
            sample_bases: &[0, 51],
            sample_count: 156,
        }
    }
}

fn original_playback(game: Game, pp4_v0: bool) -> Playback {
    let shift = if pp4_v0 { 0x7cc } else { 0 };
    match game {
        Game::Pp4 => Playback {
            id: "pp4",
            psg_periods: 0x0ceb1c - shift,
            pcm_rates: 0x50cbd8 - shift,
            transpose: 0x5139a8 - shift,
            sample_bases: &[0, 45, 46],
            sample_count: 155,
        },
        Game::Pp6 => Playback {
            id: "pp6",
            psg_periods: 0x0fbab4,
            pcm_rates: 0x554408,
            transpose: 0x55e2a8,
            sample_bases: &[0, 56],
            sample_count: 205,
        },
    }
}

pub(crate) struct NativeAudioLayout {
    pub driver: Game,
    pub playback: Option<Playback>,
    pub track_format: sequence::TrackFormat,
    pub bgm: SequenceTable,
    pub se: SequenceTable,
    pub percussion: PercussionTable,
}

impl NativeAudioLayout {
    fn table(&self, category: &str) -> SequenceTable {
        if category == "bgm" { self.bgm } else { self.se }
    }
}

fn seq_params(game: Game, pp4_v0: bool, cat: &str) -> SequenceTable {
    let relocation = if pp4_v0 { 0x7cc } else { 0 };
    match (game, cat) {
        (Game::Pp4, "bgm") => (
            0x512010 - relocation,
            0x40,
            164,
            32,
            0x0cef6a - relocation,
            0x113d83 - relocation,
        ),
        (Game::Pp4, _) => (
            0x116f2c - relocation,
            0x200,
            190,
            8,
            0x113d83 - relocation,
            0x116f2c - relocation,
        ),
        (Game::Pp6, "bgm") => (0x135f0c, 0x40, 153, 12, 0x0fbeb5, 0x135f0c),
        (Game::Pp6, _) => (0x13bd68, 0x200, 435, 12, 0x136638, 0x13bd68),
    }
}
fn perc_params(game: Game, pp4_v0: bool) -> (usize, usize, usize) {
    let relocation = if pp4_v0 { 0x7cc } else { 0 };
    match game {
        Game::Pp4 => (
            0x513c60 - relocation,
            0x11751c - relocation,
            0x117650 - relocation,
        ),
        Game::Pp6 => (0x55e574, 0x13d1cc, 0x13d30c),
    }
}

pub(crate) type SampleBank = (usize, usize, usize, usize);

fn scan_samples(rom: &[u8], bank: SampleBank) -> Result<Vec<Sample>> {
    ensure!(rom.len() == 8 * 1024 * 1024, "Expected 8 MiB ROM");
    let (start, end, expected, table) = bank;
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

// The indexed banks used by the verified GBA releases store consecutive PCM
// records and an independent table of offsets relative to the first record.
pub(crate) fn discover_sample_banks(rom: &[u8]) -> Vec<SampleBank> {
    const MIN_RECORDS: usize = 32;

    let next = |position: usize| {
        let header = rom.get(position..)?.get(..16)?;
        let fields = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
        let (kind, pitch, loop_start, size) = (fields(0), fields(4), fields(8), fields(12));
        if !matches!(kind, 0 | 0x4000_0000)
            || pitch % 1024 != 0
            || !(1000..=96000).contains(&(pitch / 1024))
            || loop_start >= size
            || (kind == 0 && loop_start != 0)
        {
            return None;
        }
        let end = (position.checked_add(20)?.checked_add(size as usize)?) & !3;
        (end <= rom.len()).then_some(end)
    };
    let mut starts = HashMap::<[usize; 3], Vec<usize>>::new();
    for start in (0..rom.len()).step_by(4) {
        if let Some((a, b, c)) = next(start)
            .and_then(|a| next(a).map(|b| (a, b)))
            .and_then(|(a, b)| next(b).map(|c| (a, b, c)))
        {
            starts
                .entry([a - start, b - start, c - start])
                .or_default()
                .push(start);
        }
    }
    let mut banks = Vec::new();
    for table in (0..rom.len().saturating_sub(15)).step_by(4) {
        let offsets = |index| le32(rom, table + index * 4).ok().map(|n| n as usize);
        if offsets(0) != Some(0) {
            continue;
        }
        let prefix = [
            offsets(1).unwrap(),
            offsets(2).unwrap(),
            offsets(3).unwrap(),
        ];
        let Some(matches) = starts.get(&prefix) else {
            continue;
        };
        for &start in matches {
            let mut count = 0;
            let mut end = start;
            while let Some(position) = next(end).filter(|_| offsets(count) == Some(end - start)) {
                end = position;
                count += 1;
            }
            let bank = (start, end, count, table);
            if count >= MIN_RECORDS && scan_samples(rom, bank).is_ok() {
                banks.push(bank);
            }
        }
    }
    banks.sort_unstable();
    banks.dedup();
    banks
}

// The two PP5 Rev 0 variants relocate their PCM metadata and PSG/sequence
// region independently. Use the discovered PCM index and a PSG table
// fingerprint as separate anchors, then parse every candidate track.
pub(crate) fn discover_pp5_audio(rom: &[u8], bank: SampleBank) -> Option<NativeAudioLayout> {
    const PCM_INDEX: usize = 0x4e7558;
    const PSG: usize = 0xf22dc;
    const PSG_HASH: &str = "e3c1e26635c898dbb170d9afb59c30549e57df40874082518ad89ea8dba313e1";
    const PCM_HASH: &str = "768b2cc5a30c7b0fe0fd6e4dc271c4d6d4fb1a787d16931b3e97a3a792f57d80";
    const TRANSPOSE_HASH: &str = "57bd765386e510267691026ac8b7470470e563a4fd56a7fb83f97ea57fc7abbb";
    if bank.2 != 156 {
        return None;
    }
    let pcm = |known| {
        bank.3
            .checked_add_signed(known as isize - PCM_INDEX as isize)
    };
    let pcm_rates = pcm(0x4e0394)?;
    let transpose = pcm(0x4e77d0)?;
    let bgm_table = pcm(0x4e6394)?;
    let percussion_table = pcm(0x4e7a94)?;
    if sha(rom.get(pcm_rates..pcm_rates.checked_add(0x1b4)?)?) != PCM_HASH
        || sha(rom.get(transpose..transpose.checked_add(51)?)?) != TRANSPOSE_HASH
        || bgm_table.checked_add(117 * 32)? > rom.len()
        || percussion_table.checked_add(64)? > rom.len()
    {
        return None;
    }
    let mut matches = rom
        .windows(0x400)
        .step_by(4)
        .enumerate()
        .filter_map(|(index, data)| {
            (data.starts_with(&[0x20, 0, 0x95, 0, 0xfd, 0, 0x5a, 1]) && sha(data) == PSG_HASH)
                .then_some(index * 4)
        });
    let psg_periods = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let psg = |known| psg_periods + (known - PSG);
    if psg(0x13ad18) > rom.len() || psg(0x13a354) + 274 * 8 > rom.len() {
        return None;
    }
    let layout = NativeAudioLayout {
        driver: Game::Pp4,
        playback: Some(Playback::pp5(psg_periods, pcm_rates, transpose)),
        track_format: TrackFormat::Legacy,
        bgm: (bgm_table, 0x40, 117, 32, psg(0xf26dd), psg(0x13749f)),
        se: (psg(0x13a354), 0x200, 274, 8, psg(0x13749f), psg(0x13a354)),
        percussion: (percussion_table, psg(0x13abe4), psg(0x13ad18)),
    };
    scan_sequences(rom, layout.driver, &layout).ok()?;
    scan_percussion(rom, layout.percussion).ok()?;
    Some(layout)
}

type SequenceScan = (BTreeMap<String, Vec<Sequence>>, HashMap<usize, usize>);

fn scan_sequences(rom: &[u8], game: Game, layout: &NativeAudioLayout) -> Result<SequenceScan> {
    let mut result = BTreeMap::new();
    let mut ends = HashMap::new();
    for cat in ["bgm", "se"] {
        let (table, first, count, width, region_start, region_end) = layout.table(cat);
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
                    } else if width == 8 {
                        (
                            u32::from(le16(rom, descriptor)?),
                            Some(u32::from(le16(rom, descriptor + 2)?)),
                            le32(rom, descriptor + 4)?,
                            None,
                        )
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
            sequence::parse_linear(&rom[start..end], start, modes[&start], layout.track_format)?;
            ends.insert(start, end);
        }
        result.insert(cat.to_string(), seqs);
    }
    Ok((result, ends))
}

fn scan_percussion(rom: &[u8], (table, first, end): PercussionTable) -> Result<Vec<Percussion>> {
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
        let tokens = sequence::parse_linear(
            &rom[starts[i]..stop],
            starts[i],
            Kind::Noise,
            sequence::TrackFormat::Legacy,
        )?;
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

pub(crate) fn export_samples(rom: &[u8], root: &Path, bank: SampleBank) -> Result<()> {
    let samples = scan_samples(rom, bank)?;
    let records = write_samples(rom, root, &samples)?;
    let (start, end, count, index_table) = bank;
    let manifest = json!({
        "rom_sha256": sha(rom),
        "bank": {"start": hex(start), "end_exclusive": hex(end), "count": count, "index_table": hex(index_table)},
        "samples": records,
    });
    fs::write(
        root.join("samples-manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

fn write_samples(rom: &[u8], root: &Path, samples: &[Sample]) -> Result<Vec<Value>> {
    fs::create_dir_all(root.join("samples"))?;
    fs::create_dir_all(root.join("native/samples"))?;
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
    Ok(sample_json)
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
    let pp4_v0 = game == Game::Pp4 && rom.get(0xbc) == Some(&0);
    let revision = if pp4_v0 { 0 } else { game.revision() };
    ensure!(
        rom.len() == 8 * 1024 * 1024
            && rom.get(0xac..0xb0) == Some(game.code().as_bytes())
            && rom[0xbc] == revision,
        "Expected verified {} revision {} ROM",
        game.code(),
        revision
    );
    let digest = sha(rom);
    ensure!(
        game.accepts_sha256(&digest),
        "{} SHA-256 mismatch: {digest}",
        game.code()
    );
    let layout = original_layout(game, pp4_v0);
    export_audio(
        rom,
        out,
        game,
        sample_params(game, pp4_v0),
        &layout,
        options,
        digest,
    )
}

fn original_layout(game: Game, pp4_v0: bool) -> NativeAudioLayout {
    NativeAudioLayout {
        driver: game,
        playback: Some(original_playback(game, pp4_v0)),
        track_format: sequence::TrackFormat::Legacy,
        bgm: seq_params(game, pp4_v0, "bgm"),
        se: seq_params(game, pp4_v0, "se"),
        percussion: perc_params(game, pp4_v0),
    }
}

pub(crate) fn export_native_audio(
    rom: &[u8],
    out: &Path,
    samples: SampleBank,
    layout: &NativeAudioLayout,
) -> Result<()> {
    let options = AudioOptions {
        render_tracks: layout.playback.is_some(),
        ..AudioOptions::default()
    };
    export_audio(rom, out, layout.driver, samples, layout, &options, sha(rom))
}

fn pp7_sound_target(kind: Kind, opcode: u8, id: u32) -> Option<(&'static str, usize)> {
    match (kind, opcode, id) {
        (Kind::Pcm, 0xf5, 0x3b8) => Some(("se", 0x3b9)),
        (Kind::Pcm, 0xf6, 0x272) => Some(("bgm", 0x89)),
        (Kind::Pcm, _, _) => None,
        // PSG F5 changes priority. PSG F6 starts SE 0x3b6.
        (_, 0xf6, _) => Some(("se", 0x3b6)),
        _ => None,
    }
}

fn export_audio(
    rom: &[u8],
    out: &Path,
    game: Game,
    bank: SampleBank,
    layout: &NativeAudioLayout,
    options: &AudioOptions,
    digest: String,
) -> Result<()> {
    let samples = scan_samples(rom, bank)?;
    let (sequences, ends) = scan_sequences(rom, game, layout)?;
    let percussion = scan_percussion(rom, layout.percussion)?;
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
    for directory in ["bgm", "se", "native/percussion", "native/bgm", "native/se"] {
        fs::create_dir_all(root.join(directory))?;
    }
    let sample_json = write_samples(rom, root, &samples)?;
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
    let mut bgm_renders = 0;
    let mut se_renders = 0;
    let mut triggers = 0;
    let mut silent = 0;
    for cat in ["bgm", "se"] {
        let mut items = Vec::new();
        let (_, _, _, _, _, region_end) = layout.table(cat);
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
            let mut sound_calls = Vec::new();
            for (channel, &start) in seq.starts.iter().enumerate() {
                let end = ends[&start];
                let data = &rom[start..end];
                let tokens =
                    sequence::parse_linear(data, start, seq.kinds[channel], layout.track_format)?;
                if layout.track_format == TrackFormat::Pp7 {
                    let kind = seq.kinds[channel];
                    let mut command_offset = start;
                    for token in &tokens {
                        if let Some((category, id)) = pp7_sound_target(kind, token.opcode, seq.id) {
                            sound_calls.push(json!({
                                "channel": channel,
                                "command_offset": hex(command_offset),
                                "opcode": hex(usize::from(token.opcode)),
                                "target_category": category,
                                "target_id": hex(id),
                            }));
                        }
                        command_offset = token.end;
                    }
                }
                let rel = format!("{native_prefix}/track_{channel:02}.bin");
                fs::write(root.join(&rel), data)?;
                let term = tokens.last().context("Track had no terminator")?.end;
                track_json.push(json!({"channel":channel,"mode":seq.kinds[channel].mode(),"kind":seq.kinds[channel].as_str(),
                    "start":hex(start),"end_exclusive":hex(end),"boundary_basis":if end==region_end{"declared track region end"}else{"next referenced track"},
                    "linear_token_count":tokens.len(),"first_terminator_end":hex(term),"post_terminator_bytes":end-term,"raw":rel,"sha256":sha(data)}));
            }
            item["tracks"] = json!(track_json);
            let has_sound_calls = !sound_calls.is_empty();
            if has_sound_calls {
                item["static_sound_calls"] = json!(sound_calls);
            }
            if options.render_tracks
                && options
                    .selected_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&seq.id))
            {
                let playback = layout.playback.context("Missing audio playback tables")?;
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
                        playback,
                        layout.track_format,
                    )?);
                }
                let mut kinds = seq.kinds.clone();
                if has_sound_calls {
                    let mut timeline = Vec::new();
                    for (channel, track) in decoded.iter().enumerate() {
                        for &(frame, offset, opcode) in &track.sound_commands {
                            if let Some((category, id)) =
                                pp7_sound_target(seq.kinds[channel], opcode, seq.id)
                            {
                                timeline.push((frame, channel, offset, opcode, category, id));
                            }
                        }
                    }
                    timeline.sort_by_key(|&(frame, channel, ..)| (frame, channel));
                    item["preview_sound_calls"] = json!(
                        timeline
                            .iter()
                            .map(|&(frame, channel, offset, opcode, category, id)| json!({
                                "update": frame,
                                "channel": channel,
                                "command_offset": hex(offset),
                                "opcode": hex(usize::from(opcode)),
                                "target_category": category,
                                "target_id": hex(id),
                            }))
                            .collect::<Vec<_>>()
                    );
                    for (frame, _, _, _, category, id) in timeline {
                        let target = sequences[category]
                            .iter()
                            .find(|target| target.id as usize == id)
                            .with_context(|| {
                                format!("Missing PP7 sound call target {category} {id:#x}")
                            })?;
                        for (channel, &start) in target.starts.iter().enumerate() {
                            let kind = target.kinds[channel];
                            let mut track = sequence::decode(
                                &rom[start..ends[&start]],
                                start,
                                max_frames - frame,
                                kind,
                                target.psg_types[channel],
                                playback,
                                layout.track_format,
                            )?;
                            ensure!(
                                !track.sound_commands.iter().any(|&(_, _, opcode)| {
                                    pp7_sound_target(kind, opcode, target.id).is_some()
                                }),
                                "Nested PP7 sound call in {category} {id:#x}"
                            );
                            track.end_frame += frame;
                            for event in &mut track.events {
                                event.frame += frame;
                            }
                            decoded.push(track);
                            kinds.push(kind);
                        }
                    }
                    item.as_object_mut()
                        .unwrap()
                        .remove("unsupported_playback_opcodes");
                }
                for (d, &kind) in decoded.iter().zip(&kinds) {
                    keysets.note_keys(d, kind);
                }
                let mut drums = Vec::new();
                for (d, &kind) in decoded.iter().zip(&kinds) {
                    if kind == Kind::Control {
                        drums.extend(sequence::expand_percussion(
                            rom,
                            d,
                            &percussion,
                            playback,
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
                    &kinds,
                    &midi_path,
                    &format!(
                        "{} {} {:#x}",
                        playback.id.to_uppercase(),
                        cat.to_uppercase(),
                        seq.id
                    ),
                )?;
                item["midi"] = json!({"path":midi_rel,"notes":midi_info.notes,"tracks":midi_info.tracks,
                    "sha256":midi_info.sha256,"update_rate_hz":DRIVER_UPDATE_HZ,"timebase":"960 ticks per second","native_soundfont":true});
                if cat == "bgm" {
                    bgm_midis += 1;
                } else {
                    se_midis += 1;
                }
                let wav_path = root.join(&wav_rel);
                if let Some(solo) = options.solo_channel {
                    for (channel, track) in decoded.iter_mut().enumerate() {
                        if channel != solo {
                            track.events.clear();
                        }
                    }
                    if kinds.get(solo) != Some(&Kind::Control) {
                        drums.clear();
                    }
                }
                let wav_info = synth::synthesize(
                    rom,
                    playback,
                    &decoded,
                    &kinds,
                    &drums,
                    &samples,
                    synth::WavOutput {
                        path: &wav_path,
                        rate: 32768,
                        stereo: true,
                    },
                )?;
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
                    bgm_renders += 1;
                } else {
                    se_renders += 1;
                }
            } else {
                remove_track_renders(root, &stem)?;
            }
            items.push(item);
        }
        seq_json.insert(cat.to_string(), json!(items));
    }
    let soundfont_info = if options.render_tracks {
        Some(soundfont::write_soundfont(
            rom,
            layout.playback.context("Missing audio playback tables")?,
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
    let (start, end, count, index_table) = bank;
    let mut tables = serde_json::Map::new();
    for cat in ["bgm", "se"] {
        let (table, first, n, width, lo, hi) = layout.table(cat);
        tables.insert(
            cat.to_string(),
            json!({"offset":hex(table),"first_id":hex(first as usize),"entry_count":n,
            "descriptor_bytes":width,"track_region":[hex(lo),hex(hi)]}),
        );
    }
    let manifest = json!({"rom_game_code":String::from_utf8_lossy(&rom[0xac..0xb0]),"rom_sha256":digest,
        "source_range":{"start":hex(start),"end_exclusive":hex(end)},
        "sample_index_table":{"offset":hex(index_table),"base":hex(start),"entry_count":count,"encoding":"u32 little-endian relative offsets"},
        "sample_format":"mono signed 8-bit PCM in ROM; WAV uses unsigned 8-bit PCM",
        "pitch_units":"Q10 sample playback rate at source pitch 36 before per-sample transposition",
        "sequence_status":format!("verified native BGM/SE descriptors and bounded tracks; {}",if layout.track_format == TrackFormat::Pp7 {"reference playback including timed sound calls"} else if options.render_tracks{"direct note transcriptions and reference WAVs"}else{"playback undecoded"}),
        "classification":format!("native BGM/SE bytecode and playable PCM samples; {}",if options.render_tracks{"MIDI with companion ROM SoundFont and approximate WAV synthesis"}else{"no complete track renders"}),
        "counts":{"sample_wavs":samples.len(),"raw_records":samples.len(),"bgm_sequences":sequences["bgm"].len(),"se_sequences":sequences["se"].len(),
            "raw_tracks":ends.len(),"bgm_midis":bgm_midis,"se_midis":se_midis,"bgm_renders":bgm_renders,"se_renders":se_renders,
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

fn remove_track_renders(root: &Path, stem: &str) -> Result<()> {
    for extension in ["mid", "wav"] {
        let p = root.join(format!("{stem}.{extension}"));
        if p.exists() {
            fs::remove_file(p)?;
        }
    }
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

#[cfg(test)]
mod bank_discovery_tests {
    use super::*;
    use crate::rom::{PROFILES, verify_supported_rom};

    #[test]
    fn discovers_each_known_roms_pcm_bank() {
        for profile in PROFILES {
            let Some(path) = crate::test_assets::rom(profile.id) else {
                continue;
            };
            let (verified, rom) = verify_supported_rom(&path).unwrap();
            assert_eq!(verified.id, profile.id);
            let expected = profile
                .samples
                .or_else(|| {
                    profile
                        .game
                        .map(|game| sample_params(game, profile.id == "pp4_v0"))
                })
                .unwrap();
            assert_eq!(discover_sample_banks(&rom), [expected], "{}", profile.id);
        }
    }
    #[test]
    fn relocates_both_pp5_audio_regions_without_fixed_rom_offsets() {
        for id in ["pp5_v0", "pp5_v0_p"] {
            let Some(path) = crate::test_assets::rom(id) else {
                continue;
            };
            let (profile, mut rom) = verify_supported_rom(&path).unwrap();
            let bank = discover_sample_banks(&rom)[0];
            let layout = discover_pp5_audio(&rom, bank).unwrap();
            let known = profile.native_audio.as_ref().unwrap();
            assert_eq!(layout.bgm, known.bgm);
            assert_eq!(layout.se, known.se);
            assert_eq!(layout.percussion, known.percussion);
            let (found, expected) = (layout.playback.unwrap(), known.playback.unwrap());
            assert_eq!(
                (found.psg_periods, found.pcm_rates, found.transpose),
                (expected.psg_periods, expected.pcm_rates, expected.transpose)
            );
            rom[found.pcm_rates + 10] ^= 1;
            assert!(discover_pp5_audio(&rom, bank).is_none());
        }
    }
}
