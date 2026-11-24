//! Raw Nintendo DS cartridge files, indexed by their Nitro FAT IDs.

mod animation;
mod cells;
mod compression;
mod dialogue;
pub use dialogue::DialogueGame;
pub(crate) use dialogue::{export_dialogue, find_dialogue};
mod font;
mod graphics;
mod messages;
mod overlay;
mod stream;

use anyhow::{Context, Result, ensure};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::path::Path;

const GAMES: &[(&[u8; 4], &str, DialogueGame)] = &[
    (b"APKJ", "pp_koushien", DialogueGame::PpKoushien),
    (b"APXJ", "pp_ds_koushien", DialogueGame::PpDsKoushien),
    (b"AP8J", "pp8", DialogueGame::Pp8),
    (b"APIJ", "pp9", DialogueGame::Pp9),
    (b"YPJJ", "pp10", DialogueGame::Pp10),
    (b"CXIJ", "pp11", DialogueGame::Pp11),
    (b"VPTJ", "pp12", DialogueGame::Pp12),
    (b"VPLJ", "pp13", DialogueGame::Pp13),
    (b"VP4J", "pp14", DialogueGame::Pp14),
];

// SHA-1 identities from the DS catalog snapshot in docs/rom-revisions.md.
pub(crate) const CATALOG: &[(&[u8; 4], u8, usize, &str)] = &[
    (
        b"APXJ",
        0,
        67108864,
        "67ce30e66b263204aad82cfa15dccfdc47321c32",
    ),
    (
        b"APXJ",
        1,
        67108864,
        "ec19244d3b2f37488484c47bf1e2185054520188",
    ),
    (
        b"APKJ",
        0,
        33554432,
        "955fb91d872274d54c7f0b45e9eac1197dbdb679",
    ),
    (
        b"YPJJ",
        0,
        67108864,
        "e8873752c143e9c1c028b089e80ee905380344f9",
    ),
    (
        b"CXIJ",
        0,
        67108864,
        "5dd3c68686db4b108ccc70999ca9ace05f82cea4",
    ),
    (
        b"VPTJ",
        0,
        134217728,
        "4327f56f23729a59d44bd170e0ed53a2bb4667dd",
    ),
    (
        b"VPTJ",
        1,
        134217728,
        "fb7239d05b85676522af977213601c206c837a0d",
    ),
    (
        b"VPLJ",
        0,
        134217728,
        "8660495da41b8172db5925213a0651ffdf9d253f",
    ),
    (
        b"VP4J",
        0,
        134217728,
        "636216f91b449aa9b132cf20ed2fa48dedb93f86",
    ),
    (
        b"AP8J",
        0,
        33554432,
        "8dbd2336191c1ddeb653620efb2e58c965cc43eb",
    ),
    (
        b"APIJ",
        0,
        67108864,
        "9d37ea0bbd71bda9db9bd641460061b0ce9e3aeb",
    ),
];

pub(crate) fn profile_id(code: &[u8]) -> Option<&'static str> {
    GAMES
        .iter()
        .find(|(candidate, _, _)| candidate.as_slice() == code)
        .map(|(_, id, _)| *id)
}

fn catalog_revision(code: &[u8], size: usize, sha1: &str) -> Option<u8> {
    CATALOG.iter().find_map(|(game, revision, length, hash)| {
        (game.as_slice() == code && *length == size && *hash == sha1).then_some(*revision)
    })
}

fn le_word(bytes: &[u8]) -> usize {
    u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize
}

fn file_ranges(rom: &[u8]) -> Result<Vec<std::ops::Range<usize>>> {
    let header = rom.get(..0x160).context("incomplete Nintendo DS header")?;
    // DS header fields 0x48 and 0x4c give the cartridge FAT position and size.
    let offset = le_word(&header[0x48..]);
    let size = le_word(&header[0x4c..]);
    ensure!(
        offset >= header.len() && size != 0 && size.is_multiple_of(8) && size / 8 <= 65536,
        "invalid Nitro FAT location or size"
    );
    let end = offset
        .checked_add(size)
        .context("Nitro FAT range overflow")?;
    let table = rom.get(offset..end).context("Nitro FAT outside ROM")?;
    allocation_ranges(table, rom.len(), header.len())
}

fn allocation_ranges(
    table: &[u8],
    size: usize,
    min_start: usize,
) -> Result<Vec<std::ops::Range<usize>>> {
    let mut total_size = 0usize;
    table
        .chunks_exact(8)
        .enumerate()
        .map(|(id, entry)| {
            let start = le_word(entry);
            let end = le_word(&entry[4..]);
            ensure!(
                start <= end && end <= size && (start == end || start >= min_start),
                "Nitro FAT file {id} has an invalid ROM range"
            );
            total_size = total_size
                .checked_add(end - start)
                .context("Nitro FAT byte count overflow")?;
            ensure!(total_size <= size, "Nitro FAT files exceed the ROM size");
            Ok(start..end)
        })
        .collect()
}

fn le_half(bytes: &[u8]) -> usize {
    u16::from_le_bytes(bytes[..2].try_into().unwrap()) as usize
}

fn escape_name(out: &mut String, bytes: &[u8]) -> Result<()> {
    for &byte in bytes {
        match byte {
            b' '..=b'~' if !matches!(byte, b'%' | b'/' | b'\\') => out.push(char::from(byte)),
            _ => write!(out, "%{byte:02X}")?,
        }
    }
    Ok(())
}

fn file_names(rom: &[u8], files: usize) -> Result<Vec<Option<String>>> {
    let offset = le_word(&rom[0x40..]);
    let size = le_word(&rom[0x44..]);
    if size == 0 {
        return Ok(vec![None; files]);
    }
    let end = offset
        .checked_add(size)
        .context("Nitro FNT range overflow")?;
    let table = rom.get(offset..end).context("Nitro FNT outside ROM")?;
    names_from_table(table, files)
}

fn names_from_table(table: &[u8], files: usize) -> Result<Vec<Option<String>>> {
    let mut names = vec![None; files];
    let root = table.get(..8).context("missing Nitro FNT root")?;
    let count = le_half(&root[6..]);
    ensure!(
        (1..=4096).contains(&count) && count * 8 <= table.len(),
        "invalid Nitro FNT directory count"
    );
    let mut visited = vec![false; count];
    let mut queue = std::collections::VecDeque::from([(0, String::new())]);
    visited[0] = true;
    while let Some((directory, prefix)) = queue.pop_front() {
        let head = &table[directory * 8..directory * 8 + 8];
        let mut cursor = le_word(head);
        let mut file = le_half(&head[4..]);
        ensure!(
            cursor >= count * 8,
            "Nitro FNT name list overlaps directory table"
        );
        loop {
            let flags = *table
                .get(cursor)
                .context("unterminated Nitro FNT name list")?;
            cursor += 1;
            if flags == 0 {
                break;
            }
            let length = usize::from(flags & 0x7f);
            ensure!(length > 0, "empty Nitro FNT name");
            let bytes = table
                .get(cursor..cursor + length)
                .context("truncated Nitro FNT name")?;
            cursor += length;
            let mut path = prefix.clone();
            escape_name(&mut path, bytes)?;
            ensure!(path.len() <= 4096, "Nitro FNT path is too long");
            if flags & 0x80 != 0 {
                let child = le_half(
                    table
                        .get(cursor..cursor + 2)
                        .context("truncated Nitro FNT directory ID")?,
                );
                cursor += 2;
                let id = child
                    .checked_sub(0xf000)
                    .filter(|&id| id < count)
                    .context("invalid Nitro FNT directory ID")?;
                ensure!(!visited[id], "repeated Nitro FNT directory ID");
                ensure!(
                    le_half(&table[id * 8 + 6..]) == 0xf000 + directory,
                    "incorrect Nitro FNT parent ID"
                );
                visited[id] = true;
                path.push('/');
                queue.push_back((id, path));
            } else {
                let entry = names
                    .get_mut(file)
                    .context("Nitro FNT file ID outside FAT")?;
                ensure!(entry.is_none(), "repeated Nitro FNT file ID");
                *entry = Some(path);
                file += 1;
            }
        }
    }
    ensure!(
        visited.into_iter().all(|seen| seen),
        "unreachable Nitro FNT directory"
    );
    Ok(names)
}

fn narc_block<'a>(remaining: &mut &'a [u8], tag: &[u8; 4]) -> Result<&'a [u8]> {
    let header = remaining.get(..8).context("truncated NARC block header")?;
    ensure!(&header[..4] == tag, "unexpected NARC block");
    let length = le_word(&header[4..]);
    let block = remaining
        .get(8..length)
        .context("invalid NARC block size")?;
    *remaining = &remaining[length..];
    Ok(block)
}

struct Archive<'a> {
    contents: &'a [u8],
    ranges: Vec<std::ops::Range<usize>>,
    names: Vec<Option<String>>,
}

fn narc_entries(archive: &[u8]) -> Result<Archive<'_>> {
    let header = archive.get(..16).context("truncated NARC header")?;
    ensure!(
        matches!(&header[4..8], [0xff, 0xfe, 1, 0] | [0xfe, 0xff, 0, 1]),
        "unsupported NARC byte order or version"
    );
    let size = le_word(&header[8..]);
    let header_size = le_half(&header[12..]);
    ensure!(
        header_size >= 16 && le_half(&header[14..]) == 3,
        "invalid NARC header"
    );
    let mut blocks = archive
        .get(header_size..size)
        .context("invalid NARC archive size")?;
    let allocation = narc_block(&mut blocks, b"BTAF")?;
    let names = narc_block(&mut blocks, b"BTNF")?;
    let contents = narc_block(&mut blocks, b"GMIF")?;
    ensure!(blocks.is_empty(), "unexpected data inside NARC");
    let count = le_half(allocation.get(..2).context("missing NARC file count")?);
    let table = allocation
        .get(4..4 + count * 8)
        .context("truncated NARC FAT")?;
    let ranges = allocation_ranges(table, contents.len(), 0)?;
    let names = if names.is_empty() || names.get(..4).is_some_and(|prefix| le_word(prefix) < 8) {
        vec![None; count]
    } else {
        names_from_table(names, count)?
    };
    Ok(Archive {
        contents,
        ranges,
        names,
    })
}

fn banner_icon(bitmap: &[u8], palette: &[u8], flip_x: bool, flip_y: bool) -> image::RgbaImage {
    image::RgbaImage::from_fn(32, 32, |x, y| {
        let x = if flip_x { 31 - x } else { x } as usize;
        let y = if flip_y { 31 - y } else { y } as usize;
        let tile = (y / 8) * 4 + x / 8;
        let byte = bitmap[tile * 32 + (y % 8) * 4 + (x % 8) / 2];
        let color = usize::from((byte >> (4 * (x % 2))) & 0x0f);
        let entry = le_half(&palette[2 * color..]) as u16;
        image::Rgba(crate::gba::obj_rgba(entry, color))
    })
}

fn export_banner_animation(banner: &[u8], root: &Path) -> Result<()> {
    let mut index = String::from("step\tduration_vblanks\tbitmap\tpalette\tflip_x\tflip_y\tpath\n");
    for (step, command) in banner[0x2340..0x23c0]
        .chunks_exact(2)
        .map(le_half)
        .enumerate()
    {
        let duration = command & 0xff;
        if duration == 0 {
            break;
        }
        let bitmap = (command >> 8) & 7;
        let palette = (command >> 11) & 7;
        let flip_x = command & 0x4000 != 0;
        let flip_y = command & 0x8000 != 0;
        let pixels = 0x1240 + bitmap * 0x200;
        let colors = 0x2240 + palette * 0x20;
        let icon = banner_icon(
            &banner[pixels..pixels + 0x200],
            &banner[colors..colors + 0x20],
            flip_x,
            flip_y,
        );
        let path = format!("frames/{step:02}.png");
        crate::gba::save_png(&icon, &root.join(&path))?;
        writeln!(
            index,
            "{step}\t{duration}\t{bitmap}\t{palette}\t{flip_x}\t{flip_y}\t{path}"
        )?;
    }
    std::fs::write(root.join("animation.tsv"), index)?;
    Ok(())
}

fn export_banner(rom: &[u8], root: &Path) -> Result<()> {
    let offset = le_word(&rom[0x68..]);
    if offset == 0 {
        return Ok(());
    }
    let data = rom
        .get(offset..)
        .filter(|data| data.len() >= 2)
        .context("DS banner outside ROM")?;
    let version = le_half(data);
    let (size, languages) = match version {
        1 => (0x840, 6),
        2 => (0x940, 7),
        3 => (0xa40, 8),
        0x103 => (0x23c0, 8),
        _ => anyhow::bail!("unknown DS banner version {version:#06x}"),
    };
    let banner = data.get(..size).context("truncated DS banner")?;
    let icon = banner_icon(&banner[0x20..0x220], &banner[0x220..0x240], false, false);
    let mut titles = serde_json::Map::new();
    for (language, field) in ["ja", "en", "fr", "de", "it", "es", "zh", "ko"]
        .into_iter()
        .take(languages)
        .zip(banner[0x240..].chunks_exact(0x100))
    {
        let units: Vec<_> = field
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .take_while(|&unit| unit != 0)
            .collect();
        titles.insert(language.into(), String::from_utf16_lossy(&units).into());
    }
    let out = root.join("banner");
    crate::gba::save_png(&icon, &out.join("icon.png"))?;
    std::fs::write(out.join("banner.bin"), banner)?;
    std::fs::write(
        out.join("titles.json"),
        serde_json::to_string_pretty(&titles)?,
    )?;
    if version == 0x103 {
        export_banner_animation(banner, &out)?;
    }
    Ok(())
}

pub(crate) fn render_background(
    screen: &Path,
    tiles: &Path,
    palette: &Path,
    out: &Path,
) -> Result<()> {
    graphics::export_background(screen, tiles, palette, out)
}

// In Nintendo SDK ARM9 images the module settings contain the RAM address
// immediately after the compressed static region, or zero for a raw binary.
fn expand_arm9(bytes: &[u8], ram_address: usize) -> Result<Option<Vec<u8>>> {
    const MARKER: &[u8] = &[0x21, 0x06, 0xc0, 0xde, 0xde, 0xc0, 0x06, 0x21];
    let mut settings = bytes[..bytes.len().min(0x8000)]
        .windows(MARKER.len())
        .enumerate()
        .skip(0x1c)
        .step_by(4)
        .filter_map(|(offset, marker)| {
            (marker == MARKER).then(|| (offset, le_word(&bytes[offset - 8..])))
        });
    let Some((marker, compressed_end)) = settings.next() else {
        return Ok(None);
    };
    ensure!(settings.next().is_none(), "ambiguous ARM9 module settings");
    if compressed_end == 0 {
        return Ok(None);
    }
    let end = compressed_end
        .checked_sub(ram_address)
        .context("ARM9 BLZ end precedes RAM base")?;
    ensure!(
        end >= marker + MARKER.len() + 8,
        "ARM9 BLZ end overlaps module settings"
    );
    let packed = bytes
        .get(..end)
        .context("ARM9 BLZ end outside executable")?;
    let span = le_word(&packed[end - 8..]) & 0xff_ffff;
    ensure!(
        span <= end - marker - MARKER.len(),
        "ARM9 BLZ range overlaps module settings"
    );
    let mut decoded = overlay::decompress(packed, None)?;
    decoded.extend_from_slice(&bytes[end..]);
    Ok(Some(decoded))
}

// The offsets and lengths are defined by the NDS and extended DSi headers.
// https://github.com/devkitPro/libnds/blob/master/include/nds/memory.h
fn export_executable(
    rom: &[u8],
    root: &Path,
    cpu: &str,
    header_offset: usize,
    dsi: bool,
) -> Result<Option<serde_json::Value>> {
    let header = rom
        .get(header_offset..header_offset + 16)
        .context("incomplete executable header")?;
    let offset = le_word(header);
    let size = le_word(&header[12..]);
    if size == 0 {
        return Ok(None);
    }
    let minimum = if dsi { 0x200 } else { 0x160 };
    ensure!(offset >= minimum, "executable overlaps ROM header");
    let end = offset
        .checked_add(size)
        .context("executable range overflow")?;
    let bytes = rom.get(offset..end).context("executable outside ROM")?;

    let path = format!("executables/{cpu}.bin");
    std::fs::create_dir_all(root.join("executables"))?;
    std::fs::write(root.join(&path), bytes)?;
    let mut entry = serde_json::json!({
        "cpu": cpu,
        "rom_offset": offset,
        "size": size,
        "ram_address": le_word(&header[8..]),
        "entry_address": (!dsi).then(|| le_word(&header[4..])),
        "path": path,
    });
    if cpu == "arm9" {
        match expand_arm9(bytes, le_word(&header[8..])) {
            Ok(Some(decoded)) => {
                let path = "executables/arm9-decompressed.bin";
                std::fs::write(root.join(path), &decoded)?;
                entry["decompressed_path"] = serde_json::json!(path);
                entry["decompressed_size"] = serde_json::json!(decoded.len());
            }
            Ok(None) => {}
            Err(error) => entry["decompression_error"] = serde_json::json!(format!("{error:#}")),
        }
    }
    Ok(Some(entry))
}

pub(crate) fn detect(rom: &[u8]) -> bool {
    rom.get(0x0c..0x10).and_then(profile_id).is_some() && file_ranges(rom).is_ok()
}

pub(crate) fn extract(path: &Path, out: &Path) -> Result<()> {
    let rom = std::fs::read(path).with_context(|| format!("reading ROM {}", path.display()))?;
    extract_bytes(&rom, out)
}

pub(crate) fn extract_bytes(rom: &[u8], out: &Path) -> Result<()> {
    let code = rom
        .get(0x0c..0x10)
        .context("incomplete Nintendo DS header")?;
    let game =
        profile_id(code).context("unrecognized Power Pro Kun Pocket Nintendo DS game code")?;
    let ranges = file_ranges(rom)?;
    let names = file_names(rom, ranges.len())?;
    let named_files = names.iter().flatten().count();
    let revision = rom[0x1e];
    let sha256 = format!("{:x}", Sha256::digest(rom));
    let sha1 = format!("{:x}", Sha1::digest(rom));
    let root = out.join(format!("{game}_v{revision}")).join(&sha256);
    write_entries(
        &root,
        rom,
        &ranges,
        names,
        0,
        &mut ExtractionBudget::default(),
    )?;
    if let Err(error) = overlay::export(rom, &ranges, &root) {
        eprintln!("{game}: skipping overlay table: {error:#}");
    }
    if let Err(error) = export_banner(rom, &root) {
        eprintln!("{game}: skipping banner: {error:#}");
    }
    let mut executables = Vec::new();
    for (cpu, offset, dsi) in [
        ("arm9", 0x20, false),
        ("arm7", 0x30, false),
        ("arm9i", 0x1c0, true),
        ("arm7i", 0x1d0, true),
    ] {
        if dsi && rom[0x12] & 2 == 0 {
            continue;
        }
        match export_executable(rom, &root, cpu, offset, dsi) {
            Ok(Some(entry)) => executables.push(entry),
            Ok(None) => {}
            Err(error) => {
                eprintln!("{game}: skipping {cpu} executable: {error:#}");
                executables.push(serde_json::json!({ "cpu": cpu, "error": format!("{error:#}") }));
            }
        }
    }
    let manifest = serde_json::json!({
        "profile": game,
        "game_code": std::str::from_utf8(code)?,
        "header_revision": revision,
        "unit_code": rom[0x12],
        "size": rom.len(),
        "sha1": sha1,
        "sha256": sha256,
        "catalog_revision": catalog_revision(code, rom.len(), &sha1),
        "fat_files": ranges.len(),
        "named_files": named_files,
        "executables": executables,
    });
    std::fs::write(
        root.join("rom.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    println!("{game}: {}", root.display());
    Ok(())
}

fn wave_data(file: &[u8]) -> Result<&[u8]> {
    let header = file
        .get(..24)
        .context("truncated Nintendo DS audio header")?;
    ensure!(
        header[4..8] == [0xff, 0xfe, 0, 1]
            && le_half(&header[12..]) == 16
            && le_half(&header[14..]) == 1
            && &header[16..20] == b"DATA",
        "invalid Nintendo DS audio header"
    );
    let end = le_word(&header[8..]);
    ensure!(
        le_word(&header[20..]) == end.saturating_sub(16),
        "invalid audio DATA size"
    );
    file.get(..end)
        .filter(|data| data.len() >= 24)
        .context("audio DATA outside file")
}

fn write_loop_metadata(path: &Path, start: Option<usize>, frames: usize, rate: u32) -> Result<()> {
    let metadata = serde_json::json!({
        "sample_rate": rate,
        "frames": frames,
        "loop_start_frame": start.filter(|&frame| frame < frames),
        "loop_end_frame_exclusive": frames,
    });
    std::fs::write(
        path.with_extension("loop.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    Ok(())
}

fn write_wave(data: &[u8], path: &Path) -> Result<()> {
    let info = data.get(..12).context("truncated wave information")?;
    let bits_per_sample = match info[0] {
        0 => 8,
        1 | 2 => 16,
        codec => anyhow::bail!("unsupported Nintendo DS wave codec {codec}"),
    };
    let sample_rate = le_half(&info[2..]) as u32;
    ensure!(sample_rate != 0, "invalid wave sample rate");
    let loop_words = le_half(&info[6..]);
    let payload = loop_words
        .checked_add(le_word(&info[8..]))
        .and_then(|words| words.checked_mul(4))
        .context("wave byte count overflow")?;
    let samples = data
        .get(12..)
        .and_then(|data| data.get(..payload))
        .context("wave samples outside file")?;
    ensure!(
        info[0] != 2 || samples.len() >= 4,
        "truncated ADPCM predictor"
    );
    std::fs::create_dir_all(path.parent().context("missing wave output folder")?)?;
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    if info[0] == 2 {
        for sample in adpcm_samples(samples) {
            writer.write_sample(sample)?;
        }
    } else if bits_per_sample == 8 {
        for &sample in samples {
            writer.write_sample(sample as i8)?;
        }
    } else {
        for sample in samples.chunks_exact(2) {
            writer.write_sample(i16::from_le_bytes([sample[0], sample[1]]))?;
        }
    }
    writer.finalize()?;
    if info[1] != 0 {
        // SWAV loop offsets count 32-bit words from the encoded payload,
        // including the ADPCM predictor word if present.
        let frames_from_bytes = |bytes: usize| match info[0] {
            0 => bytes,
            1 => bytes / 2,
            _ => bytes.saturating_sub(4) * 2,
        };
        write_loop_metadata(
            path,
            Some(frames_from_bytes(loop_words * 4)),
            frames_from_bytes(payload),
            sample_rate,
        )?;
    }
    Ok(())
}

// Nintendo DS IMA ADPCM uses a four-byte predictor header, then low nibble first.
fn adpcm_samples(data: &[u8]) -> impl Iterator<Item = i16> + '_ {
    const STEP: [i32; 89] = [
        7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60,
        66, 73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371,
        408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878,
        2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845,
        8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086,
        29794, 32767,
    ];
    const CHANGE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];
    let mut predictor = i32::from(i16::from_le_bytes([data[0], data[1]]));
    let mut index = i32::from((data[2] & 0x7f).min(88));
    data[4..]
        .iter()
        .flat_map(|byte| [byte & 15, byte >> 4])
        .map(move |code| {
            let step = STEP[index as usize];
            let mut magnitude = step >> 3;
            for (mask, shift) in [(1, 2), (2, 1), (4, 0)] {
                if code & mask != 0 {
                    magnitude += step >> shift;
                }
            }
            let delta = if code & 8 == 0 { magnitude } else { -magnitude };
            predictor = (predictor + delta).clamp(-32767, 32767);
            index = (index + CHANGE[usize::from(code & 7)]).clamp(0, 88);
            predictor as i16
        })
}

fn export_wave(file: &[u8], root: &Path, id: usize) -> Result<()> {
    if file.starts_with(b"SWAV") {
        let data = wave_data(file)?;
        write_wave(&data[24..], &root.join("wav").join(format!("{id:04x}.wav")))?;
    } else if file.starts_with(b"SWAR") {
        let data = wave_data(file)?;
        let count = le_word(data.get(0x38..0x3c).context("missing SWAR wave count")?);
        let max_count = data.len().saturating_sub(0x3c) / 4;
        ensure!(count <= max_count, "truncated SWAR offset table");
        let table_end = 0x3c + count * 4;
        let offsets: Vec<_> = data[0x3c..table_end]
            .chunks_exact(4)
            .map(le_word)
            .chain(std::iter::once(data.len()))
            .collect();
        ensure!(
            offsets[0] >= table_end && offsets.windows(2).all(|pair| pair[0] < pair[1]),
            "invalid SWAR wave offsets"
        );
        for (wave, pair) in offsets.windows(2).enumerate() {
            let path = root
                .join("wav")
                .join(format!("{id:04x}"))
                .join(format!("{wave:04x}.wav"));
            if let Err(error) = write_wave(&data[pair[0]..pair[1]], &path) {
                eprintln!("SWAR {id:04x} wave {wave:04x}: {error:#}");
            }
        }
    }
    Ok(())
}

fn sdat_ranges(archive: &[u8]) -> Result<Vec<std::ops::Range<usize>>> {
    let header = archive.get(..0x40).context("truncated SDAT header")?;
    ensure!(
        header[4..8] == [0xff, 0xfe, 0, 1] && le_half(&header[12..]) == 0x40,
        "invalid SDAT header"
    );
    let archive = archive
        .get(..le_word(&header[8..]))
        .context("SDAT size outside file")?;
    let find_block = |at: usize, tag: &[u8; 4]| -> Result<(usize, &[u8])> {
        let offset = le_word(&header[at..]);
        let size = le_word(&header[at + 4..]);
        let end = offset
            .checked_add(size)
            .context("SDAT block size overflow")?;
        ensure!(
            offset >= header.len() && size >= 12,
            "invalid SDAT block range"
        );
        let block = archive
            .get(offset..end)
            .context("SDAT block outside file")?;
        ensure!(
            &block[..4] == tag && le_word(&block[4..]) == size,
            "invalid SDAT block header"
        );
        Ok((offset, block))
    };
    let (fat_start, fat) = find_block(0x20, b"FAT ")?;
    let (file_start, file) = find_block(0x28, b"FILE")?;
    ensure!(
        fat_start + fat.len() <= file_start,
        "overlapping SDAT blocks"
    );
    let count = le_word(&fat[8..]);
    ensure!(count <= (fat.len() - 12) / 16, "truncated SDAT file table");
    let mut total_size = 0usize;
    fat[12..12 + count * 16]
        .chunks_exact(16)
        .enumerate()
        .map(|(id, entry)| {
            let start = le_word(entry);
            let size = le_word(&entry[4..]);
            let end = start
                .checked_add(size)
                .context("SDAT entry size overflow")?;
            ensure!(
                start >= file_start + 12 && end <= file_start + file.len(),
                "SDAT file {id} outside FILE block"
            );
            total_size = total_size
                .checked_add(size)
                .context("SDAT total byte count overflow")?;
            ensure!(
                total_size <= file.len(),
                "SDAT entries exceed FILE block size"
            );
            Ok(start..end)
        })
        .collect()
}

fn sdat_symbol_list(data: &[u8], offset: usize, stride: usize) -> Result<&[u8]> {
    if offset == 0 {
        return Ok(&[]);
    }
    ensure!(offset >= 40, "SDAT symbol list overlaps header");
    let list = data
        .get(offset..)
        .filter(|data| data.len() >= 4)
        .context("SDAT symbol list outside block")?;
    let count = le_word(list);
    ensure!(
        count <= (list.len() - 4) / stride,
        "truncated SDAT symbol list"
    );
    Ok(&list[4..4 + count * stride])
}

fn sdat_symbol_index(archive: &[u8]) -> Result<Option<String>> {
    let offset = le_word(&archive[0x10..]);
    let size = le_word(&archive[0x14..]);
    if offset == 0 && size == 0 {
        return Ok(None);
    }
    let end = offset
        .checked_add(size)
        .context("SDAT SYMB range overflow")?;
    ensure!(
        offset >= 0x40 && size >= 40 && end <= le_word(&archive[8..]),
        "invalid SDAT SYMB range"
    );
    let data = archive.get(offset..end).context("SDAT SYMB outside file")?;
    ensure!(&data[..4] == b"SYMB", "invalid SDAT SYMB signature");
    let declared = le_word(&data[4..]);
    let data = data
        .get(..declared)
        .filter(|data| data.len() >= 40)
        .context("invalid SDAT SYMB size")?;
    let mut budget = 4 * 1024 * 1024;
    let mut records = 262_144;
    let mut index = String::from("kind\tlogical_id\tsubsequence_id\tname\n");
    let mut append = |kind: &str, logical: usize, sub: Option<usize>, name: usize| -> Result<()> {
        ensure!(records > 0, "SDAT symbol record limit exceeded");
        records -= 1;
        if name != 0 {
            let tail = data.get(name..).context("SDAT name outside SYMB")?;
            let limit = tail.len().min(budget);
            let stop = tail[..limit]
                .iter()
                .position(|&byte| byte == 0)
                .context("unterminated SDAT name or symbol limit exceeded")?;
            budget -= stop + 1;
            let sub = sub.map_or_else(String::new, |id| id.to_string());
            write!(index, "{kind}\t{logical}\t{sub}\t")?;
            escape_name(&mut index, &tail[..stop])?;
            index.push('\n');
        }
        Ok(())
    };
    for (part, kind) in [
        "SSEQ",
        "SSAR",
        "SBNK",
        "SWAR",
        "PLAYER",
        "GROUP",
        "STRM_PLAYER",
        "STRM",
    ]
    .into_iter()
    .enumerate()
    {
        let stride = if part == 1 { 8 } else { 4 };
        let list = sdat_symbol_list(data, le_word(&data[8 + part * 4..]), stride)?;
        for (id, record) in list.chunks_exact(stride).enumerate() {
            append(kind, id, None, le_word(record))?;
            if part == 1 {
                let names = sdat_symbol_list(data, le_word(&record[4..]), 4)?;
                for (sub, name) in names.chunks_exact(4).map(le_word).enumerate() {
                    append(kind, id, Some(sub), name)?;
                }
            }
        }
    }
    Ok(Some(index))
}

fn sdat_info_index(archive: &[u8], files: usize) -> Result<Option<String>> {
    // The archive header stores the absolute offset and size of INFO.
    let offset = le_word(&archive[0x18..]);
    let size = le_word(&archive[0x1c..]);
    if offset == 0 && size == 0 {
        return Ok(None);
    }
    let end = offset
        .checked_add(size)
        .context("SDAT INFO size overflow")?;
    let archive_size = le_word(&archive[8..]);
    ensure!(
        offset >= 0x40 && size >= 40 && end <= archive_size,
        "invalid SDAT INFO range"
    );
    let info = archive.get(offset..end).context("SDAT INFO outside file")?;
    ensure!(
        &info[..4] == b"INFO" && le_word(&info[4..]) == size,
        "invalid SDAT INFO header"
    );

    let mut index = String::from(
        "kind\tlogical_id\tfile_id\tpath\tbank_id\twave_archive_0\twave_archive_1\twave_archive_2\twave_archive_3\n",
    );
    for (part, kind, entry_size) in [
        (0, "SSEQ", 10),
        (1, "SSAR", 4),
        (2, "SBNK", 12),
        (3, "SWAR", 4),
        (7, "STRM", 8),
    ] {
        let list = le_word(&info[8 + part * 4..]);
        if list == 0 {
            continue;
        }
        let list = info
            .get(list..)
            .filter(|bytes| bytes.len() >= 4)
            .context("SDAT INFO list outside block")?;
        let count = le_word(list);
        ensure!(count <= (list.len() - 4) / 4, "truncated SDAT INFO list");
        for (id, pos) in list[4..4 + count * 4]
            .chunks_exact(4)
            .map(le_word)
            .enumerate()
        {
            if pos == 0 {
                continue;
            }
            let entry = info
                .get(pos..)
                .and_then(|bytes| bytes.get(..entry_size))
                .context("SDAT INFO record outside block")?;
            let file = le_half(entry);
            ensure!(
                file < files,
                "SDAT {kind} {id} references missing file {file}"
            );
            write!(index, "{kind}\t{id}\t{file}\t{file:04x}.bin")?;
            let optional = |value: usize| {
                if value == usize::from(u16::MAX) {
                    String::new()
                } else {
                    value.to_string()
                }
            };
            if part == 0 {
                write!(index, "\t{}", optional(le_half(&entry[4..])))?;
            } else {
                index.push('\t');
            }
            for slot in 0..4 {
                index.push('\t');
                if part == 2 {
                    index.push_str(&optional(le_half(&entry[4 + slot * 2..])));
                }
            }
            index.push('\n');
        }
    }
    Ok(Some(index))
}

const MAX_ARCHIVE_DEPTH: usize = 4;

struct ExtractionBudget {
    compression_bytes: usize,
    archive_bytes: usize,
    archive_entries: usize,
}

impl Default for ExtractionBudget {
    fn default() -> Self {
        Self {
            compression_bytes: 256 * 1024 * 1024,
            archive_bytes: 512 * 1024 * 1024,
            archive_entries: 200_000,
        }
    }
}

impl ExtractionBudget {
    fn reserve_archive(&mut self, ranges: &[std::ops::Range<usize>]) -> Result<()> {
        let size = ranges
            .iter()
            .try_fold(0usize, |total, range| total.checked_add(range.len()))
            .context("Nintendo DS archive byte count overflow")?;
        ensure!(
            size <= self.archive_bytes && ranges.len() <= self.archive_entries,
            "Nintendo DS archive extraction budget exhausted"
        );
        self.archive_bytes -= size;
        self.archive_entries -= ranges.len();
        Ok(())
    }
}

fn export_archive(
    archive: &[u8],
    root: &Path,
    id: usize,
    depth: usize,
    budget: &mut ExtractionBudget,
) -> Result<()> {
    if !archive.starts_with(b"NARC") && !archive.starts_with(b"SDAT") {
        return Ok(());
    }
    if depth >= MAX_ARCHIVE_DEPTH {
        eprintln!("Nintendo DS file {id:04x}: archive nesting limit reached");
        return Ok(());
    }
    if archive.starts_with(b"NARC") {
        match narc_entries(archive).and_then(|entries| {
            budget.reserve_archive(&entries.ranges)?;
            Ok(entries)
        }) {
            Ok(entries) => write_entries(
                &root.join("narc").join(format!("{id:04x}")),
                entries.contents,
                &entries.ranges,
                entries.names,
                depth + 1,
                budget,
            )?,
            Err(error) => eprintln!("Nintendo DS file {id:04x}: skipping NARC: {error:#}"),
        }
    } else {
        match sdat_ranges(archive).and_then(|files| {
            budget.reserve_archive(&files)?;
            Ok(files)
        }) {
            Ok(files) => {
                let out = root.join("sdat").join(format!("{id:04x}"));
                write_entries(
                    &out,
                    archive,
                    &files,
                    vec![None; files.len()],
                    depth + 1,
                    budget,
                )?;
                match sdat_symbol_index(archive) {
                    Ok(Some(index)) => std::fs::write(out.join("sound-symbols.tsv"), index)?,
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("Nintendo DS file {id:04x}: skipping SDAT SYMB: {error:#}")
                    }
                }
                match sdat_info_index(archive, files.len()) {
                    Ok(Some(index)) => std::fs::write(out.join("sound-info.tsv"), index)?,
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("Nintendo DS file {id:04x}: skipping SDAT INFO: {error:#}")
                    }
                }
            }
            Err(error) => eprintln!("Nintendo DS file {id:04x}: skipping SDAT: {error:#}"),
        }
    }
    Ok(())
}

fn sequence_byte(stream: &mut &[u8]) -> Result<u8> {
    let (&byte, rest) = stream.split_first().context("truncated SSEQ command")?;
    *stream = rest;
    Ok(byte)
}

fn sequence_varint(stream: &mut &[u8]) -> Result<u32> {
    let mut value = 0;
    for _ in 0..4 {
        let byte = sequence_byte(stream)?;
        value = (value << 7) | u32::from(byte & 127);
        if byte < 128 {
            return Ok(value);
        }
    }
    anyhow::bail!("SSEQ variable-length value exceeds four bytes")
}

fn sequence_target(stream: &mut &[u8], length: usize) -> Result<usize> {
    let offset = le_word(&[
        sequence_byte(stream)?,
        sequence_byte(stream)?,
        sequence_byte(stream)?,
        0,
    ]);
    ensure!(offset < length, "SSEQ target outside sequence");
    Ok(offset)
}

type MidiEvent = (u64, i32, Vec<u8>);

#[derive(Clone, Copy)]
struct TrackStart {
    channel: u8,
    offset: usize,
    tick: u64,
}

fn mark_sequence_loop(events: &mut Vec<MidiEvent>, start: u64, end: u64) -> Result<()> {
    use crate::audio::midi::meta;
    ensure!(start < end, "SSEQ loop does not advance time");
    events.push((start, -1, meta(6, b"loopStart")));
    events.push((end, 1, meta(6, b"loopEnd")));
    Ok(())
}

fn decode_sequence_track(
    code: &[u8],
    start: TrackStart,
    pending: &mut Vec<TrackStart>,
    started: &mut [bool; 16],
    tempos: &mut Vec<MidiEvent>,
    budget: &mut usize,
) -> Result<(Vec<MidiEvent>, u64)> {
    use crate::audio::midi::meta;

    let TrackStart {
        channel,
        offset,
        mut tick,
    } = start;
    let mut stream = &code[offset..];
    let mut first_ticks = std::collections::HashMap::new();
    let mut returns = Vec::new();
    let mut loops = Vec::new();
    let mut note_wait = true;
    let mut transpose = 0i8;
    let mut events = Vec::new();
    loop {
        ensure!(*budget > 0, "SSEQ command limit exceeded");
        *budget -= 1;
        if returns.is_empty() && loops.is_empty() {
            first_ticks.entry(code.len() - stream.len()).or_insert(tick);
        }
        match sequence_byte(&mut stream)? {
            key @ 0..=0x7f => {
                let velocity = sequence_byte(&mut stream)? & 127;
                let duration = u64::from(sequence_varint(&mut stream)?);
                let note = (i16::from(key) + i16::from(transpose)).clamp(0, 127) as u8;
                ensure!(
                    tick + duration <= 0x0fff_ffff,
                    "SSEQ exceeds MIDI tick limit"
                );
                events.push((tick, 0, vec![0x90 | channel, note, velocity]));
                events.push((tick + duration, 0, vec![0x80 | channel, note, 0]));
                if note_wait {
                    tick += duration;
                }
            }
            0x80 => tick += u64::from(sequence_varint(&mut stream)?),
            0x81 => {
                let program = sequence_varint(&mut stream)?;
                ensure!(program < 1 << 21, "SSEQ program exceeds MIDI bank range");
                events.push((tick, 0, vec![0xb0 | channel, 0, (program >> 14) as u8]));
                events.push((
                    tick,
                    0,
                    vec![0xb0 | channel, 32, ((program >> 7) & 127) as u8],
                ));
                events.push((tick, 0, vec![0xc0 | channel, (program & 127) as u8]));
            }
            0x93 => {
                let channel = sequence_byte(&mut stream)?;
                let offset = sequence_target(&mut stream, code.len())?;
                ensure!(
                    channel < 16 && !started[usize::from(channel)],
                    "duplicate or invalid SSEQ track {channel}"
                );
                started[usize::from(channel)] = true;
                pending.push(TrackStart {
                    channel,
                    offset,
                    tick,
                });
            }
            0x94 => {
                let offset = sequence_target(&mut stream, code.len())?;
                if returns.is_empty()
                    && loops.is_empty()
                    && let Some(&start) = first_ticks.get(&offset)
                {
                    mark_sequence_loop(&mut events, start, tick)?;
                    break;
                }
                stream = &code[offset..];
            }
            0x95 => {
                let offset = sequence_target(&mut stream, code.len())?;
                ensure!(returns.len() < 32, "SSEQ call stack limit exceeded");
                returns.push(code.len() - stream.len());
                stream = &code[offset..];
            }
            code @ (0xc0 | 0xc1 | 0xd5) => {
                let controller = match code {
                    0xc0 => 10,
                    0xc1 => 7,
                    _ => 11,
                };
                let value = sequence_byte(&mut stream)?;
                ensure!(value < 128, "invalid SSEQ controller value");
                events.push((tick, 0, vec![0xb0 | channel, controller, value]));
            }
            0xc3 => transpose = sequence_byte(&mut stream)? as i8,
            0xc7 => note_wait = sequence_byte(&mut stream)? != 0,
            0xe1 => {
                let bpm = u32::from(u16::from_le_bytes([
                    sequence_byte(&mut stream)?,
                    sequence_byte(&mut stream)?,
                ]));
                ensure!(bpm >= 4, "SSEQ tempo outside MIDI range");
                tempos.push((tick, 0, meta(0x51, &(60_000_000 / bpm).to_be_bytes()[1..])));
            }
            0xd4 => {
                let count = sequence_byte(&mut stream)?;
                ensure!(loops.len() < 32, "SSEQ loop stack limit exceeded");
                loops.push((code.len() - stream.len(), tick, count));
            }
            0xfc => {
                let (offset, start, remaining) =
                    loops.pop().context("SSEQ loop end without loop start")?;
                match remaining {
                    0 => {
                        ensure!(
                            loops.is_empty() && returns.is_empty(),
                            "nested indefinite SSEQ loop"
                        );
                        mark_sequence_loop(&mut events, start, tick)?;
                        break;
                    }
                    1 => {}
                    _ => {
                        loops.push((offset, start, remaining - 1));
                        stream = &code[offset..];
                    }
                }
            }
            0xfd => {
                let offset = returns.pop().context("SSEQ return without call")?;
                stream = &code[offset..];
            }
            0xfe => {
                sequence_byte(&mut stream)?;
                sequence_byte(&mut stream)?;
            }
            0xff => break,
            code => anyhow::bail!("unsupported SSEQ command {code:#04x}"),
        }
        ensure!(tick <= 0x0fff_ffff, "SSEQ exceeds MIDI tick limit");
    }
    ensure!(
        loops.is_empty() && returns.is_empty(),
        "SSEQ ended with unfinished control flow"
    );
    Ok((events, tick))
}

fn sequence_midi(code: &[u8], offset: usize, budget: &mut usize) -> Result<Vec<u8>> {
    use crate::audio::midi::track_until;

    ensure!(offset < code.len(), "SSEQ start outside sequence data");
    let mut pending = vec![TrackStart {
        channel: 0,
        offset,
        tick: 0,
    }];
    let mut started = [false; 16];
    started[0] = true;
    let mut tempos = Vec::new();
    let mut tracks = Vec::new();
    while let Some(start) = pending.get(tracks.len()).copied() {
        tracks.push(decode_sequence_track(
            code,
            start,
            &mut pending,
            &mut started,
            &mut tempos,
            budget,
        )?);
    }
    let mut midi = b"MThd\0\0\0\x06".to_vec();
    if tracks.len() == 1 {
        midi.extend([0, 0, 0, 1, 0, 48]);
        let (mut events, tick) = tracks.pop().unwrap();
        // Tempo messages precede channel events at the same tick.
        tempos.append(&mut events);
        midi.extend(track_until(tempos, tick));
    } else {
        midi.extend([0, 1, 0, tracks.len() as u8 + 1, 0, 48]);
        midi.extend(track_until(tempos, 0));
        for (events, tick) in tracks {
            midi.extend(track_until(events, tick));
        }
    }
    Ok(midi)
}

fn export_sequence(file: &[u8], root: &Path, id: usize) -> Result<()> {
    let data = wave_data(file)?;
    let offset = le_word(data.get(24..28).context("missing SSEQ data offset")?);
    let code = data
        .get(offset..)
        .filter(|_| offset >= 28)
        .context("invalid SSEQ data offset")?;
    let midi = sequence_midi(code, 0, &mut 250_000)?;
    let out = root.join("midi");
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join(format!("{id:04x}.mid")), midi)?;
    Ok(())
}

fn export_sequence_archive(file: &[u8], root: &Path, id: usize) -> Result<()> {
    let data = wave_data(file)?;
    let header = data.get(24..32).context("missing SSAR DATA fields")?;
    let offset = le_word(header);
    let count = le_word(&header[4..]);
    ensure!(
        offset >= 32 && count <= (offset - 32) / 12,
        "invalid SSAR entry table"
    );
    let code = data
        .get(offset..)
        .context("SSAR data offset outside file")?;
    let out = root.join("midi").join(format!("{id:04x}"));
    std::fs::create_dir_all(&out)?;
    let mut index =
        String::from("sequence_id\tevent_offset\tbank_id\tvolume\tplayer_id\tmidi\tstatus\n");
    let mut archive_budget: usize = 2_000_000;
    for (entry, record) in data[32..32 + count * 12].chunks_exact(12).enumerate() {
        let start = le_word(record);
        let bank = le_half(&record[4..]);
        let volume = record[6];
        let player = record[9];
        if start == u32::MAX as usize {
            writeln!(index, "{entry}\t\t{bank}\t{volume}\t{player}\t\tempty")?;
            continue;
        }
        write!(index, "{entry}\t{start:08x}\t{bank}\t{volume}\t{player}\t")?;
        let cap = archive_budget.min(250_000);
        let mut budget = cap;
        let preview = sequence_midi(code, start, &mut budget);
        archive_budget -= cap - budget;
        match preview {
            Ok(bytes) => {
                let name = format!("{entry:04x}.mid");
                std::fs::write(out.join(&name), bytes)?;
                writeln!(index, "{name}\tconverted")?;
            }
            Err(error) => {
                eprintln!("SSAR {id:04x} sequence {entry:04x}: {error:#}");
                writeln!(index, "\tunavailable")?;
            }
        }
    }
    std::fs::write(out.join("sequences.tsv"), index)?;
    Ok(())
}

fn bank_regions(kind: u8, data: &[u8]) -> Result<Vec<(u8, u8, usize, &[u8])>> {
    if matches!(kind, 1..=3) {
        let note = data
            .get(..10)
            .context("truncated single-note SBNK instrument")?;
        return Ok(vec![(0, 127, usize::from(kind), note)]);
    }
    let (header_size, ranges) = match kind {
        16 => {
            let header = data.get(..2).context("missing SBNK pitch range")?;
            ensure!(
                header[0] <= header[1] && header[1] <= 127,
                "invalid SBNK pitch range"
            );
            (
                2,
                (header[0]..=header[1])
                    .map(|key| (key, key))
                    .collect::<Vec<_>>(),
            )
        }
        17 => {
            let ends = data.get(..8).context("missing SBNK region bounds")?;
            let mut ranges = Vec::new();
            let mut low = 0;
            for (i, &high) in ends.iter().enumerate() {
                if i > 0 && high == 0 {
                    break;
                }
                ensure!(high >= low && high <= 127, "invalid SBNK region bounds");
                ranges.push((low, high));
                low = high + 1;
            }
            (8, ranges)
        }
        _ => anyhow::bail!("unsupported SBNK instrument type {kind}"),
    };
    let notes = data
        .get(header_size..header_size + ranges.len() * 12)
        .context("truncated SBNK note records")?;
    Ok(ranges
        .into_iter()
        .zip(notes.chunks_exact(12))
        .map(|((low, high), note)| (low, high, le_half(note), &note[2..]))
        .collect())
}

fn export_sound_bank(file: &[u8], root: &Path, id: usize) -> Result<()> {
    let data = wave_data(file)?;
    let count = le_word(
        data.get(0x38..0x3c)
            .context("missing SBNK instrument count")?,
    );
    ensure!(
        count <= (data.len() - 0x3c) / 4,
        "truncated SBNK instrument table"
    );
    let table_end = 0x3c + count * 4;
    let mut index = String::from(concat!(
        "program\tinstrument_type\toffset\tlow_key\thigh_key\tnote_type\t",
        "wave_id_or_duty\twave_archive_slot\troot_key\tattack\tdecay\tsustain\trelease\tpan\n"
    ));
    for (program, entry) in data[0x3c..table_end].chunks_exact(4).enumerate() {
        let kind = entry[0];
        if kind == 0 {
            continue;
        }
        let offset = le_half(&entry[1..]);
        let regions = data
            .get(offset..)
            .filter(|_| offset >= table_end)
            .context("SBNK instrument outside data")
            .and_then(|data| bank_regions(kind, data));
        match regions {
            Ok(regions) => {
                for (low, high, note_type, note) in regions {
                    write!(
                        index,
                        "{program}\t{kind}\t{offset:08x}\t{low}\t{high}\t{note_type}\t{}\t{}",
                        le_half(note),
                        le_half(&note[2..])
                    )?;
                    for &value in &note[4..] {
                        write!(index, "\t{value}")?;
                    }
                    index.push('\n');
                }
            }
            Err(error) => eprintln!("SBNK {id:04x} program {program}: {error:#}"),
        }
    }
    let out = root.join("banks");
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join(format!("{id:04x}.tsv")), index)?;
    Ok(())
}

fn write_entries(
    root: &Path,
    data: &[u8],
    ranges: &[std::ops::Range<usize>],
    names: Vec<Option<String>>,
    depth: usize,
    budget: &mut ExtractionBudget,
) -> Result<()> {
    let mut index = String::from(
        "file_id\tstart_offset\tend_offset_exclusive\tpath\trom_path\tdecompressed_path\n",
    );
    let mut screens = graphics::ScreenSets::default();
    std::fs::create_dir_all(root)?;
    for (id, (range, original)) in ranges.iter().zip(&names).enumerate() {
        let name = format!("{id:04x}.bin");
        let body = &data[range.clone()];
        std::fs::write(root.join(&name), body)?;
        let mut expanded_path = String::new();
        let maximum = budget.compression_bytes.min(64 * 1024 * 1024);
        let expanded = if compression::candidate(body, original.as_deref()) {
            match compression::decompress(body, maximum) {
                Ok(bytes) => Some(bytes),
                Err(error) => {
                    eprintln!("Nintendo DS file {id:04x}: {error:#}");
                    None
                }
            }
        } else {
            compression::probe(body, maximum)
        };
        if let Some(bytes) = &expanded {
            budget.compression_bytes -= bytes.len();
            let folder = root.join("decompressed");
            std::fs::create_dir_all(&folder)?;
            std::fs::write(folder.join(&name), bytes)?;
            expanded_path = format!("decompressed/{name}");
        }
        let body = expanded.as_deref().unwrap_or(body);
        if let Some(original) = original {
            screens.add(original, body, id, expanded.is_some());
        }
        export_archive(body, root, id, depth, budget)?;
        let preview = if body.starts_with(b"SSEQ") {
            export_sequence(body, root, id)
        } else if body.starts_with(b"SSAR") {
            export_sequence_archive(body, root, id)
        } else if body.starts_with(b"STRM") {
            stream::export(body, root, id)
        } else if body.starts_with(b"SBNK") {
            export_sound_bank(body, root, id)
        } else if body.starts_with(b"RNAN") {
            animation::export(body, root, id)
        } else if body.starts_with(b"RECN") {
            cells::export(body, root, id)
        } else if body.starts_with(b"RTFN") {
            font::export(body, root, id)
        } else if body.starts_with(b"MESGbmg1") {
            messages::export(body, root, id)
        } else if body.starts_with(b"RGCN") || body.starts_with(b"RLCN") {
            graphics::export(body, root, id)
        } else {
            export_wave(body, root, id)
        };
        if let Err(error) = preview {
            eprintln!("Nintendo DS file {id:04x}: {error:#}");
        }
        writeln!(
            index,
            "{id}\t{:08x}\t{:08x}\t{name}\t{}\t{expanded_path}",
            range.start,
            range.end,
            original.as_deref().unwrap_or_default()
        )?;
    }
    std::fs::write(root.join("index.tsv"), index)?;
    screens.render(root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_member_narc(member: &[u8]) -> Vec<u8> {
        let mut archive = b"NARC\xff\xfe\x01\x00".to_vec();
        archive.extend_from_slice(&((52 + member.len()) as u32).to_le_bytes());
        archive.extend_from_slice(&[16, 0, 3, 0]);
        archive.extend_from_slice(b"BTAF\x14\0\0\0\x01\0\0\0\0\0\0\0");
        archive.extend_from_slice(&(member.len() as u32).to_le_bytes());
        archive.extend_from_slice(b"BTNF\x08\0\0\0GMIF");
        archive.extend_from_slice(&((8 + member.len()) as u32).to_le_bytes());
        archive.extend_from_slice(member);
        archive
    }

    #[test]
    fn reports_swav_adpcm_loop_in_decoded_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wave.wav");
        let mut bytes = vec![0; 12 + 16];
        bytes[0] = 2;
        bytes[1] = 1;
        bytes[2..4].copy_from_slice(&8000u16.to_le_bytes());
        bytes[6] = 2;
        bytes[8] = 2;
        write_wave(&bytes, &path).unwrap();
        let loop_info: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.with_extension("loop.json")).unwrap())
                .unwrap();
        assert_eq!(loop_info["loop_start_frame"], 8);
        assert_eq!(loop_info["loop_end_frame_exclusive"], 24);
        assert_eq!(hound::WavReader::open(path).unwrap().duration(), 24);
    }

    #[test]
    fn extracts_nested_compressed_archive_and_stops_at_depth_limit() {
        let packed = b"\x10\x03\0\0\0\x01\x02\xfa";
        let archive = single_member_narc(&single_member_narc(packed));
        let out = tempfile::tempdir().unwrap();
        export_archive(&archive, out.path(), 0, 0, &mut ExtractionBudget::default()).unwrap();
        let nested = out.path().join("narc/0000/narc/0000");
        assert_eq!(std::fs::read(nested.join("0000.bin")).unwrap(), packed);
        assert_eq!(
            std::fs::read(nested.join("decompressed/0000.bin")).unwrap(),
            b"\x01\x02\xfa"
        );

        let capped = tempfile::tempdir().unwrap();
        export_archive(
            &archive,
            capped.path(),
            0,
            MAX_ARCHIVE_DEPTH - 1,
            &mut ExtractionBudget::default(),
        )
        .unwrap();
        assert!(capped.path().join("narc/0000/0000.bin").exists());
        assert!(!capped.path().join("narc/0000/narc").exists());
    }

    #[test]
    fn expands_arm9_static_region_and_preserves_trailing_data() {
        let base = 0x0200_0000usize;
        let mut bytes = vec![0; 36];
        bytes[28..].copy_from_slice(&[0x21, 0x06, 0xc0, 0xde, 0xde, 0xc0, 0x06, 0x21]);
        assert!(expand_arm9(&bytes, base).unwrap().is_none());
        bytes.extend([0, 0xf0, b'A', b'A', b'A', 0x10, 14, 0, 0, 8, 7, 0, 0, 0]);
        let end = bytes.len();
        bytes[20..24].copy_from_slice(&((base + end) as u32).to_le_bytes());
        bytes.extend(b"tail");
        let expanded = expand_arm9(&bytes, base).unwrap().unwrap();
        assert_eq!(&expanded[..36], &bytes[..36]);
        assert_eq!(&expanded[36..57], &[b'A'; 21]);
        assert_eq!(&expanded[57..], b"tail");
        bytes[20..24].copy_from_slice(&((base + 0x1000) as u32).to_le_bytes());
        assert!(expand_arm9(&bytes, base).is_err());
    }

    #[test]
    fn verifies_available_retail_fat_files_through_extract_command() {
        use clap::Parser;

        let Some(assets) = crate::test_assets::root() else {
            return;
        };
        let mut checked = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(assets.join("roms")).unwrap() {
            let path = entry.unwrap().path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("nds"))
            {
                continue;
            }
            let rom = std::fs::read(&path).unwrap();
            let Some(profile) = rom.get(0x0c..0x10).and_then(profile_id) else {
                continue;
            };
            let ranges = file_ranges(&rom).unwrap();
            let revision = rom[0x1e];
            let digest = format!("{:x}", Sha256::digest(&rom));
            if !checked.insert(digest.clone()) {
                continue;
            }
            let out = tempfile::tempdir().unwrap();
            let args = [
                std::ffi::OsStr::new("ppextractor"),
                std::ffi::OsStr::new("extract"),
                path.as_os_str(),
                std::ffi::OsStr::new("--out"),
                out.path().as_os_str(),
            ];
            crate::cli::execute(crate::cli::Cli::parse_from(args), |_, _, _, _| {
                panic!("DS cartridge routed to the GBA extractor")
            })
            .unwrap();
            let root = out
                .path()
                .join(format!("{profile}_v{revision}"))
                .join(&digest);
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join("rom.json")).unwrap()).unwrap();
            assert_eq!(manifest["fat_files"], ranges.len());
            assert_eq!(manifest["sha256"], digest);
            for (id, range) in ranges.into_iter().enumerate() {
                assert_eq!(
                    std::fs::read(root.join(format!("{id:04x}.bin"))).unwrap(),
                    &rom[range],
                    "{profile} revision {revision}, FAT entry {id}"
                );
            }
        }
        eprintln!("verified {} local DS ROM dumps", checked.len());
    }

    #[test]
    fn records_unknown_dump_identity_and_extracted_fat_count() {
        let hash = "fb7239d05b85676522af977213601c206c837a0d";
        assert_eq!(catalog_revision(b"VPTJ", 134217728, hash), Some(1));
        assert_eq!(catalog_revision(b"VP4J", 134217728, hash), None);
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("fixture.nds");
        let output = temp.path().join("out");
        let mut rom = vec![0; 0x240];
        rom[0x0c..0x10].copy_from_slice(b"VPTJ");
        rom[0x1e] = 1;
        for (offset, word) in [(0x48, 0x180u32), (0x4c, 8), (0x180, 0x200), (0x184, 0x203)] {
            rom[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        rom[0x200..0x203].copy_from_slice(b"raw");
        rom[0x12] = 2;
        for (offset, word) in [
            (0x20, 0x220u32),
            (0x24, 0x0200_0004),
            (0x28, 0x0200_0000),
            (0x2c, 3),
            (0x30, 0x23f),
            (0x3c, 4),
            (0x1c0, 0x230),
            (0x1c8, 0x0240_0000),
            (0x1cc, 3),
        ] {
            rom[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        rom[0x220..0x223].copy_from_slice(b"ntr");
        rom[0x230..0x233].copy_from_slice(b"twl");
        std::fs::write(&source, &rom).unwrap();
        extract(&source, &output).unwrap();
        let sha256 = format!("{:x}", Sha256::digest(&rom));
        let root = output.join("pp12_v1").join(&sha256);
        assert_eq!(std::fs::read(root.join("0000.bin")).unwrap(), b"raw");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("rom.json")).unwrap()).unwrap();
        assert_eq!(manifest["sha256"], sha256);
        assert_eq!(manifest["header_revision"], 1);
        assert_eq!(manifest["fat_files"], 1);
        assert!(manifest["catalog_revision"].is_null());
        assert_eq!(
            std::fs::read(root.join("executables/arm9.bin")).unwrap(),
            b"ntr"
        );
        assert_eq!(
            std::fs::read(root.join("executables/arm9i.bin")).unwrap(),
            b"twl"
        );
        assert_eq!(manifest["executables"][0]["entry_address"], 0x0200_0004);
        assert_eq!(
            manifest["executables"][1]["error"],
            "executable outside ROM"
        );
        assert!(manifest["executables"][2]["entry_address"].is_null());
    }

    #[test]
    fn reads_sdat_symbols_with_sparse_archive_subsequences() {
        let mut archive = vec![0; 64 + 90];
        archive[8..12].copy_from_slice(&154u32.to_le_bytes());
        archive[0x10] = 64;
        archive[0x14] = 90;
        let symbols = &mut archive[64..];
        symbols[..4].copy_from_slice(b"SYMB");
        for (at, value) in [
            (4, 90u32),
            (8, 40),
            (12, 48),
            (40, 1),
            (44, 72),
            (48, 1),
            (52, 77),
            (56, 60),
            (60, 2),
            (68, 84),
        ] {
            symbols[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        symbols[72..77].copy_from_slice(b"song\0");
        symbols[77..84].copy_from_slice(b"archive");
        symbols[83] = 0;
        symbols[84..90].copy_from_slice(b"fx%\t!\0");
        assert_eq!(
            sdat_symbol_index(&archive)
                .unwrap()
                .unwrap()
                .lines()
                .skip(1)
                .collect::<Vec<_>>(),
            [
                "SSEQ\t0\t\tsong",
                "SSAR\t0\t\tarchiv",
                "SSAR\t0\t1\tfx%25%09!"
            ]
        );
        archive[153] = b'x';
        assert!(sdat_symbol_index(&archive).is_err());
    }

    #[test]
    fn extracts_dsi_banner_titles_and_icon_tile_order() {
        let root = tempfile::tempdir().unwrap();
        let mut rom = vec![0; 0x400 + 0x23c0];
        rom[0x68..0x6c].copy_from_slice(&0x400u32.to_le_bytes());
        let banner = &mut rom[0x400..];
        banner[..2].copy_from_slice(&0x103u16.to_le_bytes());
        // Second tile starts at x=8. The low nibble is the left pixel.
        banner[0x40] = 0x21;
        banner[0x222..0x224].copy_from_slice(&0x001fu16.to_le_bytes());
        banner[0x224..0x226].copy_from_slice(&0x03e0u16.to_le_bytes());
        for (slot, title) in [(0, "パワポケ"), (7, "한국어")] {
            let at = 0x240 + slot * 0x100;
            for (i, unit) in title.encode_utf16().enumerate() {
                banner[at + i * 2..at + i * 2 + 2].copy_from_slice(&unit.to_le_bytes());
            }
        }

        banner[0x1240 + 0x200] = 1;
        banner[0x2240 + 2 * 0x20 + 2..0x2240 + 2 * 0x20 + 4]
            .copy_from_slice(&0x7c00u16.to_le_bytes());
        banner[0x2340..0x2342].copy_from_slice(&0xd103u16.to_le_bytes());
        // A zero-duration step terminates the animation even with nonzero bitmap bits.
        banner[0x2342..0x2344].copy_from_slice(&0x0100u16.to_le_bytes());
        banner[0x2344..0x2346].copy_from_slice(&0x0001u16.to_le_bytes());

        export_banner(&rom, root.path()).unwrap();
        let out = root.path().join("banner");
        let icon = image::open(out.join("icon.png")).unwrap().into_rgba8();
        assert_eq!(icon.dimensions(), (32, 32));
        assert_eq!(icon.get_pixel(7, 0).0, [0, 0, 0, 0]);
        assert_eq!(icon.get_pixel(8, 0).0, [255, 0, 0, 255]);
        assert_eq!(icon.get_pixel(9, 0).0, [0, 255, 0, 255]);
        let frame = image::open(out.join("frames/00.png")).unwrap().into_rgba8();
        assert_eq!(frame.get_pixel(31, 31).0, [0, 0, 255, 255]);
        assert_eq!(frame.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert!(!out.join("frames/01.png").exists());
        let animation = std::fs::read_to_string(out.join("animation.tsv")).unwrap();
        assert_eq!(
            animation.lines().nth(1),
            Some("0\t3\t1\t2\ttrue\ttrue\tframes/00.png")
        );
        let titles: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("titles.json")).unwrap()).unwrap();
        assert_eq!(titles["ja"], "パワポケ");
        assert_eq!(titles["ko"], "한국어");
        assert_eq!(std::fs::read(out.join("banner.bin")).unwrap(), rom[0x400..]);
    }
}
