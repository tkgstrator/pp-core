//! Dialogue font extraction from verified PP4 and PP6 cartridges.
//!
//! The atlas keeps every decoded slot, including the raw PP4 palette values.
//! Both CFF and TrueType fonts trace ink runs as square outlines at 64 units per pixel.

use anyhow::{Context, Result, bail, ensure};
use image::{Rgba, RgbaImage};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::Game;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FontFormat {
    Pp4,
    Pp5,
    Pp6,
}

const ROM_BASE: usize = 0x0800_0000;
const PP4_TABLE: usize = 0x50d218;
const PP6_TABLE: usize = 0x554a80;
const SCALE: i32 = 64;
const PP6_VERIFIED: &[usize] = &[
    0, 1, 2, 3, 4, 0x100, 0xc96, 0x1005, 0x1051, 0x105b, 0x107d, 0x107e,
];

#[derive(Serialize)]
struct Slot {
    index: String,
    page: usize,
    cell_xy: [usize; 2],
    bitmap_size: [usize; 2],
    unicode: Vec<String>,
    mapping_confidence: &'static str,
}

fn word(rom: &[u8], at: usize) -> Result<usize> {
    let bytes = rom
        .get(at..at + 4)
        .context("glyph page table outside ROM")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
}

fn source(rom: &[u8], table: usize, page: usize) -> Result<usize> {
    let pointer = word(rom, table + 4 * page)?;
    let source = pointer
        .checked_sub(ROM_BASE)
        .context("glyph page pointer below ROM base")?;
    ensure!(
        source < rom.len(),
        "glyph page pointer outside ROM: {pointer:#x}"
    );
    Ok(source)
}

fn supported(index: usize) -> Result<(usize, usize)> {
    let (page, slot) = (index / 64, index % 64);
    ensure!(
        page < 55 || (64..67).contains(&page),
        "unsupported font page {page}"
    );
    Ok((page, slot))
}

fn pp4_pixels(rom: &[u8], table: usize, index: usize) -> Result<Vec<Vec<u8>>> {
    let (page, slot) = supported(index)?;
    let start = source(rom, table, page)? + ((slot & 0x38) << 7) + ((slot & 7) << 6);
    let mut rows = vec![vec![0; 16]; 16];
    for (y, row) in rows.iter_mut().enumerate() {
        for (x, pixel) in row.iter_mut().enumerate() {
            let at = start + (y / 8) * 512 + (x / 8) * 32 + (y % 8) * 4 + (x % 8) / 2;
            let byte = rom
                .get(at)
                .with_context(|| format!("PP4 glyph {index:#x} outside ROM"))?;
            *pixel = (byte >> (4 * (x % 2))) & 15;
        }
    }
    Ok(rows)
}

fn compact_pixels(
    rom: &[u8],
    table: usize,
    index: usize,
    format: FontFormat,
) -> Result<Vec<Vec<u8>>> {
    let (page, slot) = supported(index)?;
    let source = source(rom, table, page)?;
    let base = if slot & 8 != 0 {
        200 + (slot & 0x30) * 36
    } else {
        (slot & 0x38) * 36
    } + (slot & 4) * 24
        + [0, 17, 48, 65][slot & 3];
    let offsets = [0, if slot & 1 != 0 { 15 } else { 1 }, 16];
    let width = if page >= 64 { 8 } else { 12 };
    let mut rows = vec![vec![0; width]; 12];
    for (y, row) in rows.iter_mut().enumerate() {
        let split = if slot & 8 != 0 { 4 } else { 8 };
        let step = if y < split {
            2 * y
        } else {
            (if slot & 8 != 0 { 0xb8 } else { 0xc0 }) + 2 * (y - split)
        };
        for (x, pixel) in row.iter_mut().enumerate() {
            let group = base + offsets[x / 4] + step;
            let (at, shift, mask) = if format == FontFormat::Pp5 {
                (source + group * 2 + (x % 4) / 2, 4 * (x % 2), 15)
            } else {
                (source + group, 2 * (x % 4), 3)
            };
            let byte = rom
                .get(at)
                .with_context(|| format!("Glyph {index:#x} outside ROM"))?;
            *pixel = (byte >> shift) & mask;
        }
    }
    Ok(rows)
}

pub(crate) fn discover_tables(rom: &[u8]) -> Vec<(FontFormat, usize)> {
    const SLOTS: usize = 67;
    const FORMATS: [(FontFormat, usize); 3] = [
        (FontFormat::Pp4, 0x2000),
        (FontFormat::Pp5, 0x1200),
        (FontFormat::Pp6, 0x0900),
    ];
    let mut tables = Vec::new();
    for (word_index, table) in rom.windows(4 * SLOTS).step_by(4).enumerate() {
        let read = |page| word(table, page * 4).unwrap();
        let first = read(0);
        let Some(start) = first.checked_sub(ROM_BASE) else {
            continue;
        };
        let Some((format, stride)) = FORMATS
            .into_iter()
            .find(|&(_, stride)| first.checked_add(stride) == Some(read(1)))
        else {
            continue;
        };
        if start.saturating_add(58 * stride) > rom.len()
            || (55..64).any(|page| read(page) != first)
            || (2..55).any(|page| read(page) != first + page * stride)
            || (64..67).any(|page| read(page) != first + (page - 9) * stride)
        {
            continue;
        }
        let offset = word_index * 4;
        if format == FontFormat::Pp6
            && (0..55 * 64).chain(64 * 64..67 * 64).any(|glyph| {
                !compact_pixels(rom, offset, glyph, format)
                    .is_ok_and(|pixels| pixels.iter().flatten().all(|&ink| ink < 3))
            })
        {
            continue;
        }
        tables.push((format, offset));
    }
    tables
}

fn mappings(game: Game) -> BTreeMap<char, i32> {
    // The canonical table is shared with the dialogue script decoder.  Its
    // ASCII convenience names for the broad symbol cells are restored below.
    let mut mapping = BTreeMap::new();
    for line in include_str!("scripts/glyphs.txt").lines() {
        let (number, character) = line.split_once('\t').expect("canonical glyph row");
        let index = number.parse::<usize>().expect("canonical glyph index");
        let ch = character.chars().next().expect("canonical glyph character");
        if !(0xa1..0x100).contains(&index) {
            mapping.insert(ch, index as i32);
        } else if index < 0xc5 {
            mapping.insert(char::from_u32(ch as u32 + 0xfee0).unwrap(), index as i32);
        }
    }
    for (ordinal, ch) in "♂♀◎○★＠→←↑↓。、…「」＋ー？！『』～♥×・／（）％￥㎞㎏"
        .chars()
        .enumerate()
    {
        mapping.insert(ch, (0xc5 + ordinal) as i32);
    }
    for (start, chars) in [
        (0x1051, "0123456789"),
        (0x105b, "ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
        (0x107e, "abcdefghijklmnopqrstuvwxyz"),
    ] {
        for (offset, ch) in chars.chars().enumerate() {
            mapping.insert(ch, start + offset as i32);
        }
    }
    for (ascii, wide) in [
        ('@', '＠'),
        ('+', '＋'),
        ('?', '？'),
        ('!', '！'),
        ('(', '（'),
        (')', '）'),
        ('%', '％'),
        ('~', '～'),
    ] {
        mapping.insert(ascii, mapping[&wide]);
    }
    mapping.insert('¥', mapping[&'￥']);
    mapping.insert('/', 0x107d);
    mapping.insert('ｶ', 0x1005);
    if game == Game::Pp4 {
        for (ch, index) in [('屁', 0xd02), ('璧', 0xd44), ('祟', 0xd54)] {
            mapping.insert(ch, index);
        }
    }
    mapping.insert(' ', -1);
    mapping.insert('　', -2);
    mapping
}

fn palette(game: Game) -> Map<String, Value> {
    let mut palette = Map::new();
    match game {
        Game::Pp4 => {
            for ink in 1..=15 {
                palette.insert(ink.to_string(), json!([ink * 16, 0, 255 - ink * 16, 255]));
            }
            palette.insert("0".into(), json!([255, 255, 255, 0]));
            palette.insert("2".into(), json!([255, 255, 255, 255]));
        }
        Game::Pp6 => {
            palette.insert("0".into(), json!([255, 255, 255, 0]));
            palette.insert("1".into(), json!([255, 255, 255, 128]));
            palette.insert("2".into(), json!([255, 255, 255, 255]));
        }
    }
    palette
}

fn rgba(game: Game, ink: u8) -> Result<Rgba<u8>> {
    let values = match game {
        Game::Pp4 if ink == 0 => [255, 255, 255, 0],
        Game::Pp4 if ink == 2 => [255, 255, 255, 255],
        Game::Pp4 if ink <= 15 => [ink * 16, 0, 255 - ink * 16, 255],
        Game::Pp6 if ink == 0 => [255, 255, 255, 0],
        Game::Pp6 if ink == 1 => [255, 255, 255, 128],
        Game::Pp6 if ink == 2 => [255, 255, 255, 255],
        _ => bail!("unexpected {:?} ink value {ink}", game),
    };
    Ok(Rgba(values))
}

/// Write `glyphs.png`, `glyphs.json`, and the CFF `dialogue.otf` into `out`.
pub fn extract(rom: &[u8], out: &Path, game: Game) -> Result<()> {
    let digest = format!("{:x}", Sha256::digest(rom));
    let pp4_v0 = game == Game::Pp4 && digest == crate::rom::PP4_V0_SHA256;
    ensure!(
        game.accepts_sha256(&digest),
        "Expected verified Japanese {} ROM",
        game.code()
    );
    let table = match game {
        Game::Pp4 if pp4_v0 => 0x50ca4c,
        Game::Pp4 => PP4_TABLE,
        Game::Pp6 => PP6_TABLE,
    };
    let format = match game {
        Game::Pp4 => FontFormat::Pp4,
        Game::Pp6 => FontFormat::Pp6,
    };
    export(rom, out, game.id(), format, table)
}

pub(crate) fn export(
    rom: &[u8],
    out: &Path,
    id: &str,
    format: FontFormat,
    table: usize,
) -> Result<()> {
    let game = match format {
        FontFormat::Pp4 => Game::Pp4,
        FontFormat::Pp5 | FontFormat::Pp6 => Game::Pp6,
    };
    // The 4-bit layouts include diagnostic palette indices in non-text cells.
    let ink_game = if format == FontFormat::Pp5 {
        Game::Pp4
    } else {
        game
    };
    let inherited = id != game.id();
    std::fs::create_dir_all(out)?;
    let mapping = mappings(game);
    let mut by_index: BTreeMap<i32, Vec<char>> = BTreeMap::new();
    for (&ch, &index) in &mapping {
        by_index.entry(index).or_default().push(ch);
    }
    let cell = if game == Game::Pp4 { 16 } else { 12 };
    let mut atlas = RgbaImage::new((cell * 64) as u32, (cell * 58) as u32);
    let mut slots = Vec::with_capacity(3712);
    let mut outlines = BTreeMap::new();
    for (slot, index) in (0..55 * 64).chain(64 * 64..67 * 64).enumerate() {
        let pixels = match game {
            Game::Pp4 => pp4_pixels(rom, table, index)?,
            Game::Pp6 => compact_pixels(rom, table, index, format)?,
        };
        let (x0, y0) = ((slot % 64) * cell, (slot / 64) * cell);
        for (y, row) in pixels.iter().enumerate() {
            for (x, &ink) in row.iter().enumerate() {
                atlas.put_pixel((x0 + x) as u32, (y0 + y) as u32, rgba(ink_game, ink)?);
            }
        }
        if by_index.contains_key(&(index as i32)) {
            outlines.insert(
                index as i32,
                pixels
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|&v| {
                                if ink_game == Game::Pp4 {
                                    v == 2
                                } else {
                                    v != 0
                                }
                            })
                            .collect()
                    })
                    .collect(),
            );
        }
        let unicode: Vec<String> = by_index
            .get(&(index as i32))
            .map(|chars| {
                chars
                    .iter()
                    .map(|ch| format!("U+{:04X}", *ch as u32))
                    .collect()
            })
            .unwrap_or_default();
        let confidence = if unicode.is_empty() {
            "unmapped"
        } else if inherited {
            "inherited_assumption"
        } else if game == Game::Pp4 {
            "pp4_research"
        } else if PP6_VERIFIED.contains(&index) {
            "sample_verified"
        } else {
            "inherited_assumption"
        };
        slots.push(Slot {
            index: format!("{index:#x}"),
            page: index / 64,
            cell_xy: [x0, y0],
            bitmap_size: [pixels[0].len(), pixels.len()],
            unicode,
            mapping_confidence: confidence,
        });
    }
    crate::gba::save_png(&atlas, &out.join("glyphs.png"))?;
    let mapped_indices = slots.iter().filter(|s| !s.unicode.is_empty()).count();
    let manifest = json!({
        "game": id,
        "bitmap": "glyphs.png",
        "cell_size": [cell, cell],
        "ink_value_to_rgba": palette(ink_game),
        "otf_ink_values": if ink_game == Game::Pp4 { vec![2] } else { vec![1, 2] },
        "slots": slots.len(),
        "mapped_indices": mapped_indices,
        "mappings": mapping.len(),
        "virtual_glyphs": { "U+0020": { "advance_px": 4 }, "U+3000": { "advance_px": 12 } },
        "fallback_pages_omitted": (55..64).collect::<Vec<_>>(),
        "mapping_note": if inherited {
            "Unicode assignments are inherited from the matching font format and have not been verified for this title."
        } else if game == Game::Pp4 {
            "PP4 uses the researched Unicode table and script narrow/extra assignments."
        } else {
            "PP6 uses PP4 Unicode assignments as assumptions except sample_verified indices; unmapped slots have no Unicode assignment."
        },
        "glyphs": slots,
    });
    std::fs::write(
        out.join("glyphs.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    let font = build_otf(game, &mapping, &outlines)?;
    std::fs::write(out.join("dialogue.otf"), font)?;
    let font = build_ttf(game, &mapping, &outlines)?;
    std::fs::write(out.join("dialogue.ttf"), font)?;
    Ok(())
}

fn be16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn be32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn i16be(out: &mut Vec<u8>, value: i16) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn i32be(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn cff_int(out: &mut Vec<u8>, value: i32) {
    match value {
        -107..=107 => out.push((value + 139) as u8),
        108..=1131 => {
            let n = value - 108;
            out.extend_from_slice(&[(247 + n / 256) as u8, (n % 256) as u8]);
        }
        -1131..=-108 => {
            let n = -value - 108;
            out.extend_from_slice(&[(251 + n / 256) as u8, (n % 256) as u8]);
        }
        -32768..=32767 => {
            out.push(28);
            i16be(out, value as i16);
        }
        _ => {
            out.push(29);
            i32be(out, value);
        }
    }
}

fn cff_dict_int(out: &mut Vec<u8>, value: i32) {
    // Fixed-size encoding keeps Top DICT offsets stable while it is rebuilt.
    out.push(29);
    i32be(out, value);
}

fn cff_real(out: &mut Vec<u8>, decimal: &str) {
    out.push(30);
    let mut nibbles = Vec::new();
    for byte in decimal.bytes() {
        nibbles.push(match byte {
            b'.' => 0xa,
            b'-' => 0xe,
            b'0'..=b'9' => byte - b'0',
            _ => unreachable!(),
        });
    }
    nibbles.push(0xf);
    if nibbles.len() % 2 != 0 {
        nibbles.push(0xf);
    }
    for pair in nibbles.as_chunks::<2>().0 {
        out.push((pair[0] << 4) | pair[1]);
    }
}

fn cff_index(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    be16(&mut out, objects.len() as u16);
    if objects.is_empty() {
        return out;
    }
    let max_offset = objects.iter().map(Vec::len).sum::<usize>() + 1;
    let off_size = if max_offset <= 0xff {
        1
    } else if max_offset <= 0xffff {
        2
    } else if max_offset <= 0xff_ffff {
        3
    } else {
        4
    };
    out.push(off_size);
    let mut offset = 1usize;
    for object in objects {
        let bytes = (offset as u32).to_be_bytes();
        out.extend_from_slice(&bytes[4 - off_size as usize..]);
        offset += object.len();
    }
    let bytes = (offset as u32).to_be_bytes();
    out.extend_from_slice(&bytes[4 - off_size as usize..]);
    for object in objects {
        out.extend_from_slice(object);
    }
    out
}

fn charstring(width: i32, pixels: Option<&Vec<Vec<bool>>>, baseline: i32) -> Vec<u8> {
    let mut out = Vec::new();
    cff_int(&mut out, width);
    let mut position = (0, 0);
    if let Some(pixels) = pixels {
        for (y, row) in pixels.iter().enumerate() {
            let mut x = 0;
            while x < row.len() {
                if !row[x] {
                    x += 1;
                    continue;
                }
                let begin = x;
                while x < row.len() && row[x] {
                    x += 1;
                }
                let left = begin as i32 * SCALE;
                let bottom = (baseline - y as i32 - 1) * SCALE;
                cff_int(&mut out, left - position.0);
                cff_int(&mut out, bottom - position.1);
                out.push(21); // rmoveto; closes the preceding contour
                for delta in [
                    (x as i32 * SCALE - left, 0),
                    (0, SCALE),
                    (left - x as i32 * SCALE, 0),
                    (0, -SCALE),
                ] {
                    cff_int(&mut out, delta.0);
                    cff_int(&mut out, delta.1);
                }
                out.push(5); // rlineto
                position = (left, bottom);
            }
        }
    }
    out.push(14); // endchar
    out
}

fn top_dict(
    charset: usize,
    charstrings: usize,
    family_sid: i32,
    full_sid: i32,
    weight_sid: i32,
    upem: i32,
    bbox: [i32; 4],
) -> Vec<u8> {
    let mut out = Vec::new();
    cff_dict_int(&mut out, full_sid);
    out.push(2); // FullName
    cff_dict_int(&mut out, family_sid);
    out.push(3); // FamilyName
    cff_dict_int(&mut out, weight_sid);
    out.push(4); // Weight
    for value in bbox {
        cff_dict_int(&mut out, value);
    }
    out.push(5); // FontBBox
    let factor = if upem == 1024 {
        "0.0009765625"
    } else {
        "0.0013020833"
    };
    cff_real(&mut out, factor);
    for _ in 0..2 {
        cff_dict_int(&mut out, 0);
    }
    cff_real(&mut out, factor);
    for _ in 0..2 {
        cff_dict_int(&mut out, 0);
    }
    out.extend_from_slice(&[12, 7]); // FontMatrix
    cff_dict_int(&mut out, charset as i32);
    out.push(15);
    cff_dict_int(&mut out, charstrings as i32);
    out.push(17);
    cff_dict_int(&mut out, 0); // empty Private DICT size
    cff_dict_int(&mut out, 0); // offset is unused when size is zero
    out.push(18);
    out
}

fn cff_table(
    family: &str,
    names: &[String],
    widths: &[u16],
    outlines: &[Option<&Vec<Vec<bool>>>],
    baseline: i32,
    upem: i32,
    bbox: [i32; 4],
) -> Vec<u8> {
    let postscript = family.replace(' ', "");
    let name = cff_index(&[postscript.into_bytes()]);
    let mut strings = Vec::new();
    for glyph in names.iter().skip(1) {
        strings.push(glyph.as_bytes().to_vec());
    }
    let full_sid = 391 + strings.len() as i32;
    strings.push(family.as_bytes().to_vec());
    let family_sid = full_sid;
    let weight_sid = full_sid + 1;
    strings.push(b"Regular".to_vec());
    let string_index = cff_index(&strings);
    let subrs = cff_index(&[]);
    let mut charset = vec![0];
    for index in 1..names.len() {
        be16(&mut charset, (390 + index) as u16);
    }
    let strings_data = names
        .iter()
        .enumerate()
        .map(|(i, _)| charstring(widths[i] as i32, outlines[i], baseline))
        .collect::<Vec<_>>();
    let charstrings = cff_index(&strings_data);
    let header = [1, 0, 4, 4];
    let placeholder = cff_index(&[top_dict(0, 0, family_sid, full_sid, weight_sid, upem, bbox)]);
    let charset_offset =
        header.len() + name.len() + placeholder.len() + string_index.len() + subrs.len();
    let charstrings_offset = charset_offset + charset.len();
    let top = cff_index(&[top_dict(
        charset_offset,
        charstrings_offset,
        family_sid,
        full_sid,
        weight_sid,
        upem,
        bbox,
    )]);
    debug_assert_eq!(top.len(), placeholder.len());
    let mut out = header.to_vec();
    for part in [&name, &top, &string_index, &subrs, &charset, &charstrings] {
        out.extend_from_slice(part);
    }
    out
}

fn cmap_table(cmap: &BTreeMap<u32, u16>) -> Vec<u8> {
    // Every mapped character is in the BMP; format 4 is supported by both
    // modern renderers and older Windows OpenType clients.
    let count = cmap.len() + 1;
    let mut sub = Vec::new();
    be16(&mut sub, 4);
    be16(&mut sub, (16 + count * 8) as u16);
    be16(&mut sub, 0);
    be16(&mut sub, (count * 2) as u16);
    let power = 1usize << (usize::BITS - 1 - count.leading_zeros());
    be16(&mut sub, (power * 2) as u16);
    be16(&mut sub, power.trailing_zeros() as u16);
    be16(&mut sub, ((count - power) * 2) as u16);
    for &codepoint in cmap.keys() {
        be16(&mut sub, codepoint as u16);
    }
    be16(&mut sub, 0xffff);
    be16(&mut sub, 0);
    for &codepoint in cmap.keys() {
        be16(&mut sub, codepoint as u16);
    }
    be16(&mut sub, 0xffff);
    for (&codepoint, &glyph) in cmap {
        be16(&mut sub, glyph.wrapping_sub(codepoint as u16));
    }
    be16(&mut sub, 1); // sentinel 0xffff maps to .notdef
    for _ in 0..count {
        be16(&mut sub, 0);
    }
    let mut out = Vec::new();
    be16(&mut out, 0);
    be16(&mut out, 2);
    for (platform, encoding) in [(0, 3), (3, 1)] {
        be16(&mut out, platform);
        be16(&mut out, encoding);
        be32(&mut out, 20);
    }
    out.extend_from_slice(&sub);
    out
}

fn name_table(family: &str) -> Vec<u8> {
    let values = [
        (1u16, family.to_string()),
        (2, "Regular".to_string()),
        (3, format!("{family} ROM Reference")),
        (4, family.to_string()),
        (5, "Version 1.000".to_string()),
        (6, family.replace(' ', "")),
    ];
    let mut out = Vec::new();
    be16(&mut out, 0);
    be16(&mut out, values.len() as u16);
    be16(&mut out, (6 + values.len() * 12) as u16);
    let mut payload = Vec::new();
    for (id, value) in values {
        be16(&mut out, 3);
        be16(&mut out, 1);
        be16(&mut out, 0x409);
        be16(&mut out, id);
        let encoded = value
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        be16(&mut out, encoded.len() as u16);
        be16(&mut out, payload.len() as u16);
        payload.extend_from_slice(&encoded);
    }
    out.extend_from_slice(&payload);
    out
}

fn head_table(upem: i16, bbox: [i16; 4], long_loca: bool) -> Vec<u8> {
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be32(&mut out, 0x0001_0000);
    be32(&mut out, 0);
    be32(&mut out, 0x5f0f_3cf5);
    be16(&mut out, 0);
    be16(&mut out, upem as u16);
    for _ in 0..2 {
        be32(&mut out, 0);
        be32(&mut out, 2_082_844_800);
    } // 1970-01-01 in Mac epoch
    for value in bbox {
        i16be(&mut out, value);
    }
    be16(&mut out, 0);
    be16(&mut out, 8);
    i16be(&mut out, 2);
    i16be(&mut out, if long_loca { 1 } else { 0 });
    i16be(&mut out, 0);
    out
}

fn hhea_table(ascent: i16, descent: i16, widths: &[u16], bbox: [i16; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    i16be(&mut out, ascent);
    i16be(&mut out, descent);
    i16be(&mut out, 0);
    be16(&mut out, *widths.iter().max().unwrap());
    i16be(&mut out, 0);
    i16be(&mut out, 0); // all mapped ink fits its advance
    i16be(&mut out, bbox[2]);
    i16be(&mut out, 1);
    i16be(&mut out, 0);
    i16be(&mut out, 0);
    for _ in 0..4 {
        i16be(&mut out, 0);
    }
    i16be(&mut out, 0);
    be16(&mut out, widths.len() as u16);
    out
}

fn os2_table(ascent: i16, descent: i16, first: u16, last: u16, width: i16) -> Vec<u8> {
    let mut out = Vec::new();
    be16(&mut out, 4);
    i16be(&mut out, width);
    be16(&mut out, 400);
    be16(&mut out, 5);
    be16(&mut out, 0);
    for _ in 0..11 {
        i16be(&mut out, 0);
    } // subscript, superscript, strikeout, family class
    out.extend_from_slice(&[0; 10]); // PANOSE
    for _ in 0..4 {
        be32(&mut out, 0);
    } // Unicode ranges
    out.extend_from_slice(b"PPKT");
    be16(&mut out, 0x40);
    be16(&mut out, first);
    be16(&mut out, last);
    i16be(&mut out, ascent);
    i16be(&mut out, descent);
    i16be(&mut out, 0);
    be16(&mut out, ascent as u16);
    be16(&mut out, (-descent) as u16);
    be32(&mut out, 0);
    be32(&mut out, 0); // code page ranges
    i16be(&mut out, ascent / 2);
    i16be(&mut out, ascent); // x-height, cap height
    be16(&mut out, 0);
    be16(&mut out, 0x20);
    be16(&mut out, 1);
    debug_assert_eq!(out.len(), 96);
    out
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn sfnt(mut tables: BTreeMap<[u8; 4], Vec<u8>>, magic: [u8; 4]) -> Vec<u8> {
    let count = tables.len();
    let power = 1usize << (usize::BITS - 1 - count.leading_zeros());
    let mut out = magic.to_vec();
    be16(&mut out, count as u16);
    be16(&mut out, (power * 16) as u16);
    be16(&mut out, power.trailing_zeros() as u16);
    be16(&mut out, ((count - power) * 16) as u16);
    let mut offset = 12 + count * 16;
    let mut head_offset = 0;
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        be32(&mut out, checksum(data));
        be32(&mut out, offset as u32);
        be32(&mut out, data.len() as u32);
        if tag == b"head" {
            head_offset = offset;
        }
        offset += (data.len() + 3) & !3;
    }
    for data in tables.values_mut() {
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    let adjustment = 0xb1b0_afba_u32.wrapping_sub(checksum(&out));
    out[head_offset + 8..head_offset + 12].copy_from_slice(&adjustment.to_be_bytes());
    out
}

fn build_otf(
    game: Game,
    mapping: &BTreeMap<char, i32>,
    outlines: &BTreeMap<i32, Vec<Vec<bool>>>,
) -> Result<Vec<u8>> {
    let indices = mapping.values().copied().collect::<BTreeSet<_>>();
    let baseline = if game == Game::Pp4 { 14 } else { 12 };
    let line_height = if game == Game::Pp4 { 16 } else { 12 };
    let upem = line_height * SCALE;
    let family = format!("{} ROM Dialogue", game.id().to_uppercase());
    let mut names = vec![".notdef".to_string()];
    let mut widths = vec![(12 * SCALE) as u16];
    let mut glyph_outlines = vec![None];
    let mut gid = BTreeMap::new();
    for (ordinal, index) in indices.into_iter().enumerate() {
        names.push(match index {
            -1 => "space".into(),
            -2 => "uni3000".into(),
            _ => format!("g{index:04X}"),
        });
        widths.push(
            (if index == -1 {
                4
            } else if index >= 0x1000 {
                8
            } else {
                12
            }) * SCALE as u16,
        );
        glyph_outlines.push(if index < 0 {
            None
        } else {
            Some(
                outlines
                    .get(&index)
                    .with_context(|| format!("missing mapped glyph {index:#x}"))?,
            )
        });
        gid.insert(index, (ordinal + 1) as u16);
    }
    let cmap: BTreeMap<u32, u16> = mapping
        .iter()
        .map(|(&ch, &index)| (ch as u32, gid[&index]))
        .collect();
    let bbox = [
        0,
        (baseline - line_height) * SCALE,
        12 * SCALE,
        baseline * SCALE,
    ];
    let bbox_i16 = bbox.map(|n| n as i16);
    let cff = cff_table(
        &family,
        &names,
        &widths,
        &glyph_outlines,
        baseline,
        upem,
        bbox,
    );
    let mut hmtx = Vec::new();
    for width in &widths {
        be16(&mut hmtx, *width);
        i16be(&mut hmtx, 0);
    }
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0000_5000);
    be16(&mut maxp, names.len() as u16);
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    post.extend_from_slice(&[0; 28]);
    let mut tables = BTreeMap::new();
    tables.insert(*b"CFF ", cff);
    tables.insert(
        *b"OS/2",
        os2_table(
            (baseline * SCALE) as i16,
            ((baseline - line_height) * SCALE) as i16,
            *cmap.keys().next().unwrap() as u16,
            *cmap.keys().next_back().unwrap() as u16,
            (12 * SCALE) as i16,
        ),
    );
    tables.insert(*b"cmap", cmap_table(&cmap));
    tables.insert(*b"head", head_table(upem as i16, bbox_i16, false));
    tables.insert(
        *b"hhea",
        hhea_table(
            (baseline * SCALE) as i16,
            ((baseline - line_height) * SCALE) as i16,
            &widths,
            bbox_i16,
        ),
    );
    tables.insert(*b"hmtx", hmtx);
    tables.insert(*b"maxp", maxp);
    tables.insert(*b"name", name_table(&family));
    tables.insert(*b"post", post);
    Ok(sfnt(tables, *b"OTTO"))
}

fn truetype_glyph(pixels: &[Vec<bool>], baseline: i32) -> (Vec<u8>, usize, i16) {
    let mut contours: Vec<[(i16, i16); 4]> = Vec::new();
    for (y, row) in pixels.iter().enumerate() {
        let mut x = 0;
        while x < row.len() {
            if !row[x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < row.len() && row[x] {
                x += 1;
            }
            let left = (start as i32 * SCALE) as i16;
            let right = (x as i32 * SCALE) as i16;
            let bottom = ((baseline - y as i32 - 1) * SCALE) as i16;
            let top = bottom + SCALE as i16;
            contours.push([(left, bottom), (right, bottom), (right, top), (left, top)]);
        }
    }
    if contours.is_empty() {
        return (Vec::new(), 0, 0);
    }
    let points = contours.iter().flatten().copied().collect::<Vec<_>>();
    let mut out = Vec::new();
    i16be(&mut out, contours.len() as i16);
    let x_min = points.iter().map(|p| p.0).min().unwrap();
    i16be(&mut out, x_min);
    i16be(&mut out, points.iter().map(|p| p.1).min().unwrap());
    i16be(&mut out, points.iter().map(|p| p.0).max().unwrap());
    i16be(&mut out, points.iter().map(|p| p.1).max().unwrap());
    for index in 0..contours.len() {
        be16(&mut out, (index * 4 + 3) as u16);
    }
    be16(&mut out, 0); // instruction length
    out.extend(std::iter::repeat_n(1u8, points.len())); // all points on curves, 16-bit deltas
    let mut previous = 0i16;
    for &(x, _) in &points {
        i16be(&mut out, x - previous);
        previous = x;
    }
    previous = 0;
    for &(_, y) in &points {
        i16be(&mut out, y - previous);
        previous = y;
    }
    (out, contours.len(), x_min)
}

fn build_ttf(
    game: Game,
    mapping: &BTreeMap<char, i32>,
    outlines: &BTreeMap<i32, Vec<Vec<bool>>>,
) -> Result<Vec<u8>> {
    let indices = mapping.values().copied().collect::<BTreeSet<_>>();
    let baseline = if game == Game::Pp4 { 14 } else { 12 };
    let line_height = if game == Game::Pp4 { 16 } else { 12 };
    let upem = line_height * SCALE;
    let family = format!("{} ROM Dialogue", game.id().to_uppercase());
    let mut widths = vec![(12 * SCALE) as u16];
    let mut bearings = vec![0i16];
    let mut glyphs = vec![Vec::new()]; // .notdef
    let mut gid = BTreeMap::new();
    let mut max_contours = 0;
    for (ordinal, index) in indices.into_iter().enumerate() {
        widths.push(
            (if index == -1 {
                4
            } else if index >= 0x1000 {
                8
            } else {
                12
            }) * SCALE as u16,
        );
        let (glyph, contours, bearing) = if index < 0 {
            (Vec::new(), 0, 0)
        } else {
            truetype_glyph(
                outlines
                    .get(&index)
                    .with_context(|| format!("missing mapped glyph {index:#x}"))?,
                baseline,
            )
        };
        max_contours = max_contours.max(contours);
        glyphs.push(glyph);
        bearings.push(bearing);
        gid.insert(index, (ordinal + 1) as u16);
    }
    let cmap: BTreeMap<u32, u16> = mapping
        .iter()
        .map(|(&ch, &index)| (ch as u32, gid[&index]))
        .collect();
    let bbox = [
        0,
        (baseline - line_height) * SCALE,
        12 * SCALE,
        baseline * SCALE,
    ];
    let bbox_i16 = bbox.map(|n| n as i16);
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for glyph in glyphs {
        be32(&mut loca, glyf.len() as u32);
        glyf.extend_from_slice(&glyph);
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }
    }
    be32(&mut loca, glyf.len() as u32);
    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, widths.len() as u16);
    be16(&mut maxp, (max_contours * 4) as u16);
    be16(&mut maxp, max_contours as u16);
    be16(&mut maxp, 0);
    be16(&mut maxp, 0); // composite maxima
    be16(&mut maxp, 1); // maxZones
    for _ in 0..8 {
        be16(&mut maxp, 0);
    }
    let mut hmtx = Vec::new();
    for (width, bearing) in widths.iter().zip(&bearings) {
        be16(&mut hmtx, *width);
        i16be(&mut hmtx, *bearing);
    }
    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    post.extend_from_slice(&[0; 28]);
    let mut tables = BTreeMap::new();
    tables.insert(
        *b"OS/2",
        os2_table(
            (baseline * SCALE) as i16,
            ((baseline - line_height) * SCALE) as i16,
            *cmap.keys().next().unwrap() as u16,
            *cmap.keys().next_back().unwrap() as u16,
            (12 * SCALE) as i16,
        ),
    );
    tables.insert(*b"cmap", cmap_table(&cmap));
    tables.insert(*b"glyf", glyf);
    tables.insert(*b"head", head_table(upem as i16, bbox_i16, true));
    tables.insert(
        *b"hhea",
        hhea_table(
            (baseline * SCALE) as i16,
            ((baseline - line_height) * SCALE) as i16,
            &widths,
            bbox_i16,
        ),
    );
    tables.insert(*b"hmtx", hmtx);
    tables.insert(*b"loca", loca);
    tables.insert(*b"maxp", maxp);
    tables.insert(*b"name", name_table(&family));
    tables.insert(*b"post", post);
    Ok(sfnt(tables, [0, 1, 0, 0]))
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use crate::rom::{PROFILES, verify_supported_rom};

    #[test]
    fn discovers_dialogue_font_tables_on_all_local_gba_profiles() {
        for profile in PROFILES {
            let Some(path) = crate::test_assets::rom(profile.id) else {
                continue;
            };
            let (_, rom) = verify_supported_rom(&path).unwrap();
            let expected = profile.font.unwrap_or_else(|| match profile.id {
                "pp4_v0" => (FontFormat::Pp4, 0x50ca4c),
                "pp4" => (FontFormat::Pp4, PP4_TABLE),
                "pp6" | "pp6_v0_clean" => (FontFormat::Pp6, PP6_TABLE),
                other => panic!("{other} needs a known font table for the comparison"),
            });
            assert_eq!(discover_tables(&rom), [expected], "{}", profile.id);
        }
    }
}
