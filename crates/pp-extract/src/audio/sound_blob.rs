//! The ROM ranges PP6's sound driver reads, packed for the pp-sound player
//! (crates/pp-sound). Format: `PP6S`, u32 chunk count, then per chunk a u32
//! GBA address, a u32 length and the bytes (padded to 4). Everything is
//! little-endian; the player reads ROM by GBA address.
use super::{Game, le32, original_layout, sample_params, scan_percussion, scan_sequences};
use anyhow::{Result, ensure};

/// Driver tables, with the byte length each one occupies (docs/pp6-sound-driver.md).
const TABLES: &[(usize, usize)] = &[
    (0x0fbab4, 0x100), // PERIODS u16[128]
    (0x0fbbb4, 0x80),  // PERIOD_FRAC
    (0x0fbc34, 0x80),  // NOISE
    (0x0fbcb4, 0x81),  // LFO2 (u16 window)
    (0x0fbd34, 0x181), // WAVES, 16 bytes each, up to the first BGM track
    (0x13d36c, 0x2f),  // WAVE_VOL, VIB, PAN
    (0x554408, 0xb4),  // MidiKeyToFreq scale
    (0x5544bc, 0x40),  // MidiKeyToFreq frequencies
    (0x55e2a0, 0x8),   // sample bank pointers
    (0x55e318, 0x5c),  // PCM drums (overlaps the fade-speed table)
    (0x55e574, 0x40),  // percussion stream pointers
];

pub fn pp6_sound_blob(rom: &[u8]) -> Result<Vec<u8>> {
    let layout = original_layout(Game::Pp6, false);
    let (sequences, ends) = scan_sequences(rom, Game::Pp6, &layout)?;
    let mut ranges: Vec<(usize, usize)> = TABLES.iter().map(|&(a, n)| (a, a + n)).collect();
    for cat in ["bgm", "se"] {
        // The descriptors of every BGM / SE id (the driver indexes from id 0).
        let (table, _, count, width, _, _) = layout.table(cat);
        ranges.push((table, table + count * width));
        for seq in &sequences[cat] {
            if let Some(list) = seq.pointer_offset {
                ranges.push((list, list + seq.starts.len() * 4));
            }
            for &start in &seq.starts {
                ranges.push((start, ends[&start]));
            }
        }
    }
    for p in scan_percussion(rom, layout.percussion)? {
        ranges.push((p.start, p.end));
    }
    // Sample index tables (bank 0 and 1) and the transposition table.
    let playback = layout.playback.expect("PP6 playback tables");
    let (sample_start, sample_end, sample_count, _) = sample_params(Game::Pp6, false);
    for bank in 0..2 {
        let table = le32(rom, 0x55e2a0 + bank * 4)? as usize - 0x0800_0000;
        let count = if bank == 0 { playback.sample_bases[1] } else { sample_count - playback.sample_bases[1] };
        ranges.push((table, table + count * 4));
    }
    ranges.push((playback.transpose, playback.transpose + playback.sample_bases[1] * 2));
    ranges.push((sample_start, sample_end));

    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        ensure!(start < end && end <= rom.len(), "Invalid sound range {start:#x}..{end:#x}");
        match merged.last_mut() {
            // Join ranges a few bytes apart: one header costs more than the gap.
            Some(last) if start <= last.1 + 64 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let mut out = b"PP6S".to_vec();
    out.extend((merged.len() as u32).to_le_bytes());
    for (start, end) in merged {
        out.extend((0x0800_0000 + start as u32).to_le_bytes());
        out.extend(((end - start) as u32).to_le_bytes());
        out.extend(&rom[start..end]);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Ok(out)
}
