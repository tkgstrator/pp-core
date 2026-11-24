//! PP4 OBJ bust banks and PP6 dialogue sprite (head/body) extraction.
//! Ports `tools/pp4_portraits/extract.py` and `tools/pp6_portraits/extract.py`.
//!
//! Output (`out` = category directory): PP4 `banks/<header>/<index>.png`,
//! `index/*.png`, `manifest.json`; PP6 `actors/<id>/appearance-<slot>.png`,
//! `unpaired-body/body-<index>.png`, `index/actors-*.png`, `manifest.json`.

use crate::Game;
use crate::gba::{self, SIZES, hex, i16_at, rom_pointer, u16_at, u32_at};
use anyhow::{Result, bail};
use image::{Rgb, RgbImage, RgbaImage};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

const PP4_BANKS: [(usize, usize); 7] = [
    (0x5DE1C0, 121),
    (0x5DF1BC, 25),
    (0x5E3B48, 130),
    (0x5E6428, 69),
    (0x5E7B34, 39),
    (0x5E9EB8, 62),
    (0x5EBA28, 47),
];
const PP4_SIZE: u32 = 56;
const OBJ_START: u32 = 0x0601_0000;
const OBJ_END: u32 = 0x0601_8000;

const PP6_STYLE_MAP: usize = 0x566714;
const PP6_BANKS: [(&str, usize, usize); 3] = [
    ("head_a", 0x6410F4, 255),
    ("head_b", 0x6453E0, 138),
    ("body", 0x649E90, 173),
];
/// Signed piece origins are kept until both layers are composited.
const ORIGIN: (i32, i32) = (8, 8);
const LAYER_SIZE: u32 = 72;
type Appearance = (u8, u8, serde_json::Map<String, Value>, Vec<usize>);

pub fn extract(rom: &[u8], out: &Path, game: Game) -> Result<()> {
    match game {
        Game::Pp4 => extract_pp4(rom, out).map(|_| ()),
        Game::Pp6 => extract_pp6(rom, out).map(|_| ()),
    }
}

fn write_manifest(out: &Path, manifest: &Value) -> Result<()> {
    std::fs::create_dir_all(out)?;
    // serde_json pretty printing uses two-space indent and unescaped non-ASCII like Python.
    std::fs::write(
        out.join("manifest.json"),
        serde_json::to_string_pretty(manifest)? + "\n",
    )?;
    Ok(())
}

/// Read the 16 BGR555 colours of an OBJ palette as RGBA (index 0 transparent).
fn read_palette(rom: &[u8], source: usize) -> Result<[[u8; 4]; 16]> {
    let mut colors = [[0u8; 4]; 16];
    for (i, c) in colors.iter_mut().enumerate() {
        *c = gba::obj_rgba(u16_at(rom, source + 2 * i)?, i);
    }
    Ok(colors)
}

/// Draw one OBJ piece (4bpp, 1D tile mapping) into `image` at (x0, y0).
fn blit_piece(
    image: &mut RgbaImage,
    tiles: &[u8],
    tile: usize,
    (w, h): (usize, usize),
    x0: u32,
    y0: u32,
    pal: &[[u8; 4]; 16],
) {
    for y in 0..h {
        for x in 0..w {
            let index = tile + (y / 8) * (w / 8) + x / 8;
            let packed = tiles[32 * index + 4 * (y % 8) + (x % 8) / 2];
            let ink = ((packed >> (4 * (x % 2))) & 15) as usize;
            if ink != 0 {
                image.put_pixel(x0 + x as u32, y0 + y as u32, image::Rgba(pal[ink]));
            }
        }
    }
}

// ---------------------------------------------------------------- PP4

fn bank_header(rom: &[u8], header: usize, expected: usize) -> Result<(u16, usize)> {
    let (compression, count) = (u16_at(rom, header)?, u16_at(rom, header + 2)?);
    if compression > 1 || count as usize != expected {
        bail!("Unexpected portrait bank header at {header:#x}");
    }
    if header + 8 + 4 * expected > rom.len() {
        bail!("PP4 bank layout table outside ROM at {header:#x}");
    }
    let graphics = rom_pointer(rom, header + 4)?;
    if graphics + 4 * expected > rom.len() {
        bail!("PP4 graphics table outside ROM at {header:#x}");
    }
    Ok((compression, graphics))
}

fn bust_layer(
    rom: &[u8],
    header: usize,
    graphics: usize,
    index: usize,
    compression: u16,
) -> Result<(RgbaImage, Value)> {
    let record = rom_pointer(rom, graphics + 4 * index)?;
    let source = rom_pointer(rom, record)?;
    let destination = u32_at(rom, record + 4)?;
    let length = u32_at(rom, record + 8)?;
    if !((OBJ_START..OBJ_END).contains(&destination)
        && destination % 32 == 0
        && length == 0x620
        && destination + length <= OBJ_END)
    {
        bail!("Invalid PP4 OBJ destination or tile length at {record:#x}");
    }
    let tile_base = ((destination - OBJ_START) / 32) as i64;
    let length = length as usize;
    let (tiles, end) = if compression == 1 {
        gba::decompress(rom, source, length)?
    } else {
        let end = source + length;
        let Some(raw) = rom.get(source..end) else {
            bail!("Raw OBJ tiles outside ROM at {source:#x}");
        };
        (raw.to_vec(), end)
    };

    let palette_group = rom_pointer(rom, record + 12)?;
    if u32_at(rom, palette_group)? != 1 {
        bail!("Invalid PP4 palette group at {palette_group:#x}");
    }
    let palette_record = rom_pointer(rom, palette_group + 4)?;
    let palette_destination = u32_at(rom, palette_record)?;
    if !((0x0500_0200..=0x0500_03E0).contains(&palette_destination)
        && palette_destination % 32 == 0)
    {
        bail!("Invalid PP4 OBJ palette destination at {palette_record:#x}");
    }
    let palette_bank = (palette_destination - 0x0500_0200) / 32;
    let palette_source = palette_record + 4;
    let colors = read_palette(rom, palette_source)?;

    let layout = rom_pointer(rom, header + 8 + 4 * index)?;
    let count = u32_at(rom, layout)? as usize;
    if count != 9 {
        bail!("Expected nine PP4 bust pieces at {layout:#x}");
    }
    let pieces = rom_pointer(rom, layout + 4)?;
    if pieces + 8 * count > rom.len() {
        bail!("PP4 piece table outside ROM at {pieces:#x}");
    }
    let mut image = RgbaImage::new(PP4_SIZE, PP4_SIZE);
    for i in 0..count {
        let at = pieces + 8 * i;
        let absolute_tile = u16_at(rom, at)? as i64;
        let (dx, dy) = (
            i64::from(i16_at(rom, at + 2)?),
            i64::from(i16_at(rom, at + 4)?),
        );
        let flags = u16_at(rom, at + 6)?;
        let shape = (flags >> 4) as usize;
        if shape >= SIZES.len() || u32::from(flags & 15) != palette_bank {
            bail!("Invalid PP4 piece shape or palette at {at:#x}");
        }
        let (w, h) = SIZES[shape];
        let local_tile = absolute_tile - tile_base;
        if !((0..=i64::from(PP4_SIZE) - w as i64).contains(&dx)
            && (0..=i64::from(PP4_SIZE) - h as i64).contains(&dy))
        {
            bail!("PP4 piece outside 56x56 canvas at {at:#x}");
        }
        if local_tile < 0 || local_tile as usize * 32 + w * h / 2 > length {
            bail!("PP4 piece tile outside graphics at {at:#x}");
        }
        blit_piece(
            &mut image,
            &tiles,
            local_tile as usize,
            (w, h),
            dx as u32,
            dy as u32,
            &colors,
        );
    }
    let Some(bbox) = gba::alpha_bbox(&image) else {
        bail!("Empty PP4 bust at {header:#x} index {index}");
    };
    let (start_key, end_key) = if compression == 1 {
        ("rle_start", "rle_end_exclusive")
    } else {
        ("raw_start", "raw_end_exclusive")
    };
    let info = json!({
        "index": index, "graphics_record": hex(record as u64),
        (start_key): hex(source as u64), (end_key): hex(end as u64),
        "expanded_bytes": length, "expanded_sha256": gba::sha256_hex(&tiles),
        "vram_destination": hex(destination), "source_tile_base": tile_base,
        "palette_group": hex(palette_group as u64), "palette": hex(palette_source as u64),
        "palette_vram_destination": hex(palette_destination),
        "layout_record": hex(layout as u64), "piece_table": hex(pieces as u64), "piece_count": count,
        "alpha_bbox_xyxy": bbox, "rgba_sha256": gba::sha256_hex(image.as_raw()),
    });
    Ok((image, info))
}

/// 80-image overview sheets; labels are bank indices (digits only).
fn pp4_contact_sheets(
    root: &Path,
    header: usize,
    assets: &[(usize, RgbaImage)],
) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for page in assets.chunks(80) {
        let (cw, ch) = (64u32, 76u32);
        let mut sheet =
            RgbImage::from_pixel(10 * cw, page.len().div_ceil(10) as u32 * ch, Rgb([0xe8; 3]));
        for (slot, (index, image)) in page.iter().enumerate() {
            let (x, y) = (
                (slot % 10) as i64 * cw as i64,
                (slot / 10) as i64 * ch as i64,
            );
            gba::paste_alpha(&mut sheet, image, x + 4, y + 2);
            gba::draw_digits(
                &mut sheet,
                x + 19,
                y + 58,
                &format!("{index:03}"),
                [0x20; 3],
            );
        }
        let relative = format!(
            "index/{header:06x}-{:03}-{:03}.png",
            page[0].0,
            page[page.len() - 1].0
        );
        let path = root.join(&relative);
        std::fs::create_dir_all(path.parent().unwrap())?;
        sheet.save_with_format(&path, image::ImageFormat::Png)?;
        files.push(relative);
    }
    Ok(files)
}

pub fn extract_pp4(rom: &[u8], root: &Path) -> Result<Value> {
    let digest = gba::sha256_hex(rom);
    let relocation = if digest == Game::Pp4.expected_sha256() {
        0
    } else if digest == crate::rom::PP4_V0_SHA256 {
        0x878
    } else {
        bail!("Unsupported PP4 ROM SHA-256: {digest}");
    };
    let banks = PP4_BANKS.map(|(header, count)| (header - relocation, count));
    export_busts(
        rom,
        root,
        &banks,
        "seven structurally verified 56x56 OBJ bust banks; likely dialogue portraits",
    )
}

pub(crate) fn discover_busts(rom: &[u8]) -> Vec<(usize, usize)> {
    let mut banks = Vec::new();
    for (word, prefix) in rom.windows(8).step_by(4).enumerate() {
        let compression = u16::from_le_bytes([prefix[0], prefix[1]]);
        let count = usize::from(u16::from_le_bytes([prefix[2], prefix[3]]));
        if compression > 1 || !(4..=512).contains(&count) {
            continue;
        }
        let header = word * 4;
        let Ok((compression, graphics)) = bank_header(rom, header, count) else {
            continue;
        };
        // Check the first pair of pointers and the fixed tile size before rendering.
        let Some(record) = rom_pointer(rom, graphics).ok() else {
            continue;
        };
        if u32_at(rom, record + 8).ok() != Some(0x620) {
            continue;
        }
        if (0..count).all(|index| bust_layer(rom, header, graphics, index, compression).is_ok()) {
            banks.push((header, count));
        }
    }
    banks
}

pub(crate) fn export_busts(
    rom: &[u8],
    root: &Path,
    bank_layout: &[(usize, usize)],
    classification: &str,
) -> Result<Value> {
    let (mut banks, mut sheets) = (Vec::new(), Vec::new());
    for &(header, expected) in bank_layout {
        let (compression, graphics) = bank_header(rom, header, expected)?;
        let (mut assets, mut images) = (Vec::new(), Vec::new());
        for index in 0..expected {
            let (image, info) = bust_layer(rom, header, graphics, index, compression)?;
            let relative = format!("banks/{header:06x}/{index:03}.png");
            gba::save_png(&image, &root.join(&relative))?;
            let mut asset = json!({ "png": relative });
            asset
                .as_object_mut()
                .unwrap()
                .extend(info.as_object().unwrap().clone());
            assets.push(asset);
            images.push((index, image));
        }
        let bank_sheets = pp4_contact_sheets(root, header, &images)?;
        sheets.extend(bank_sheets.iter().cloned());
        banks.push(json!({
            "header": hex(header as u64), "compression": compression, "count": expected,
            "graphics_table": hex(graphics as u64), "assets": assets, "contact_sheets": bank_sheets,
        }));
    }
    let pngs: usize = bank_layout.iter().map(|b| b.1).sum();
    let manifest = json!({
        "rom_game_code": String::from_utf8_lossy(&rom[0xac..0xb0]), "rom_sha256": gba::sha256_hex(rom),
        "classification": classification,
        "actor_style_mapping": "unverified", "runtime_coverage": "unverified",
        "file_offsets": "hex, zero based, without GBA bus base 0x08000000",
        "pixel_format": "RGBA8, GBA BGR555 scaled by floor(n*255/31), OBJ palette index 0 transparent",
        "orientation": "original 56x56 ROM piece coordinates; no actor ID or display-side assignment",
        "counts": {"banks": banks.len(), "pngs": pngs, "contact_sheets": sheets.len()},
        "banks": banks, "contact_sheets": sheets,
    });
    write_manifest(root, &manifest)?;
    Ok(manifest)
}

// ---------------------------------------------------------------- PP6

struct Layer {
    image: RgbaImage,
    info: Value,
}

fn pp6_layer(rom: &[u8], header: usize, graphics: usize, index: usize) -> Result<Layer> {
    let record = rom_pointer(rom, graphics + index * 4)?;
    let source = rom_pointer(rom, record)?;
    let length = u32_at(rom, record + 8)? as usize;
    if u32_at(rom, record + 4)? != OBJ_START || !length.is_multiple_of(32) {
        bail!("Invalid OBJ tile record at 0x{record:X}");
    }
    let (data, end) = gba::decompress(rom, source, length)?;
    let palette_group = rom_pointer(rom, record + 12)?;
    if u32_at(rom, palette_group)? != 1 {
        bail!("Expected one 16-color OBJ palette");
    }
    let palette_source = rom_pointer(rom, palette_group + 4)? + 4;
    let palette = read_palette(rom, palette_source)?;

    let layout = rom_pointer(rom, header + 8 + index * 4)?;
    let count = u32_at(rom, layout)? as usize;
    let pieces = rom_pointer(rom, layout + 4)?;
    let mut image = RgbaImage::new(LAYER_SIZE, LAYER_SIZE);
    for i in 0..count {
        let at = pieces + i * 8;
        let tile = u16_at(rom, at)? as usize;
        let (dx, dy) = (
            i32::from(i16_at(rom, at + 2)?),
            i32::from(i16_at(rom, at + 4)?),
        );
        let flags = u16_at(rom, at + 6)?;
        let Some(&(w, h)) = SIZES.get((flags >> 4) as usize) else {
            bail!("tuple index out of range");
        };
        let (x0, y0) = (dx + ORIGIN.0, dy + ORIGIN.1);
        if !((0..=LAYER_SIZE as i32 - w as i32).contains(&x0)
            && (0..=LAYER_SIZE as i32 - h as i32).contains(&y0))
        {
            bail!("Piece exceeds layer bounds at 0x{at:X}");
        }
        if tile * 32 + w * h / 2 > length {
            bail!("Piece exceeds source tiles at 0x{at:X}");
        }
        blit_piece(
            &mut image,
            &data,
            tile,
            (w, h),
            x0 as u32,
            y0 as u32,
            &palette,
        );
    }
    let info = json!({
        "index": index, "graphics_record": hex(record as u64), "rle_start": hex(source as u64),
        "rle_end_exclusive": hex(end as u64), "expanded_bytes": length,
        "expanded_sha256": gba::sha256_hex(&data), "palette": hex(palette_source as u64),
        "layout_record": hex(layout as u64), "piece_table": hex(pieces as u64), "piece_count": count,
    });
    Ok(Layer { image, info })
}

/// Trim to the alpha bounding box, save, and return (manifest entry, trimmed image).
fn write_pose(
    image: &RgbaImage,
    root: &Path,
    relative: &str,
) -> Result<(serde_json::Map<String, Value>, RgbaImage)> {
    let Some([x0, y0, x1, y1]) = gba::alpha_bbox(image) else {
        bail!("Empty sprite {relative}");
    };
    let trimmed = image::imageops::crop_imm(image, x0, y0, x1 - x0, y1 - y0).to_image();
    gba::save_png(&trimmed, &root.join(relative))?;
    let bounds = [
        x0 as i32 - ORIGIN.0,
        y0 as i32 - ORIGIN.1,
        x1 as i32 - ORIGIN.0,
        y1 as i32 - ORIGIN.1,
    ];
    let entry = json!({
        "png": relative, "size": [trimmed.width(), trimmed.height()],
        "bounds_xyxy": bounds, "rgba_sha256": gba::sha256_hex(trimmed.as_raw()),
    });
    let Value::Object(entry) = entry else {
        unreachable!()
    };
    Ok((entry, trimmed))
}

pub(crate) struct PortraitBank {
    pub header: usize,
    pub count: usize,
    pub dialogue_id_base: Option<usize>,
}

pub(crate) fn export_layers(
    rom: &[u8],
    root: &Path,
    bank_layout: &[PortraitBank],
    remap_tables: Option<[usize; 2]>,
) -> Result<()> {
    let mut banks = Vec::new();
    for &PortraitBank {
        header,
        count,
        dialogue_id_base,
    } in bank_layout
    {
        let (compression, graphics) = bank_header(rom, header, count)?;
        if compression != 1 {
            bail!("Expected compressed portrait layers at {header:#x}");
        }
        let mut assets = Vec::new();
        for index in 0..count {
            let Layer { image, info } = pp6_layer(rom, header, graphics, index)?;
            let mut entry = info.as_object().unwrap().clone();
            if let Some(base) = dialogue_id_base {
                entry.insert("dialogue_portrait_id".into(), json!(base + index));
            }
            if gba::alpha_bbox(&image).is_some() {
                let path = format!("layers/{header:06x}/{index:03}.png");
                let (pose, _) = write_pose(&image, root, &path)?;
                entry.extend(pose);
            } else {
                entry.insert("empty".into(), json!(true));
            }
            assets.push(entry);
        }
        banks.push(json!({
            "header": hex(header as u64),
            "graphics_table": hex(graphics as u64),
            "count": count,
            "assets": assets,
        }));
    }
    let mapped = bank_layout
        .iter()
        .all(|bank| bank.dialogue_id_base.is_some());
    let mut manifest = json!({
        "rom_game_code": String::from_utf8_lossy(&rom[0xac..0xb0]),
        "rom_sha256": gba::sha256_hex(rom),
        "classification": "dialogue portrait banks",
        "actor_style_mapping": if mapped {
            "dialogue portrait IDs from the runtime bank selector"
        } else {
            "scene-dependent dialogue portrait IDs from the runtime callbacks"
        },
        "banks": banks,
    });
    if let Some(tables) = remap_tables {
        manifest["dialogue_portrait_map"] = pp12_dialogue_map(rom, bank_layout, tables)?;
    }
    write_manifest(root, &manifest)
}

fn pp12_dialogue_map(rom: &[u8], banks: &[PortraitBank], tables: [usize; 2]) -> Result<Value> {
    // The setup routine at 0x3abfc swaps the two banks when either
    // scenario-state word is nonzero. Only the left callbacks remap indices.
    const CALLBACK_ID_COUNT: usize = 245;
    let mut configurations = Vec::new();
    for alternate in [false, true] {
        let mut routes = Vec::new();
        for id_group in 0..2 {
            let bank_index = id_group ^ usize::from(alternate);
            let bank = &banks[bank_index];
            let table = tables[bank_index];
            let remaps = rom
                .get(table..table + CALLBACK_ID_COUNT)
                .ok_or_else(|| anyhow::anyhow!("Portrait remap outside ROM at {table:#x}"))?;
            for (index, &remap) in remaps.iter().enumerate() {
                let left_index = if remap == 0 {
                    index
                } else {
                    usize::from(remap)
                };
                routes.push(json!({
                    "dialogue_portrait_id": id_group * 0x100 + index,
                    "bank_header": hex(bank.header as u64),
                    "left_asset_index": (left_index < bank.count).then_some(left_index),
                    "right_asset_index": (index < bank.count).then_some(index),
                    "left_x_shift": if remap == 0 { 0 } else { 56 },
                }));
            }
        }
        configurations.push(json!({
            "state": if alternate { "either_nonzero" } else { "both_zero" },
            "routes": routes,
        }));
    }
    Ok(json!({
        "state_word_addresses": ["0x03002600", "0x03002464"],
        "left_remap_table_offsets": tables.map(|offset| hex(offset as u64)),
        "configurations": configurations,
    }))
}

fn pp6_contact_sheets(root: &Path, actors: &[(u32, RgbaImage)]) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for page in actors.chunks(80) {
        let (w, h) = (72i64, 72i64);
        let mut sheet = RgbImage::from_pixel(
            10 * w as u32,
            page.len().div_ceil(10) as u32 * h as u32,
            Rgb([0xe8; 3]),
        );
        for (cell, (id, sprite)) in page.iter().enumerate() {
            let (x, y) = ((cell % 10) as i64 * w, (cell / 10) as i64 * h);
            let (sw, sh) = (i64::from(sprite.width()), i64::from(sprite.height()));
            gba::paste_alpha(&mut sheet, sprite, x + (w - sw).div_euclid(2), y + 56 - sh);
            gba::draw_digits(&mut sheet, x + 25, y + 57, &format!("{id:03}"), [0x25; 3]);
        }
        let relative = format!(
            "index/actors-{:03}-{:03}.png",
            page[0].0,
            page[page.len() - 1].0
        );
        std::fs::create_dir_all(root.join("index"))?;
        sheet.save_with_format(root.join(&relative), image::ImageFormat::Png)?;
        files.push(relative);
    }
    Ok(files)
}

pub fn extract_pp6(rom: &[u8], root: &Path) -> Result<Value> {
    let sha256 = gba::sha256_hex(rom);
    if !Game::Pp6.accepts_sha256(&sha256) {
        bail!("Unsupported ROM SHA-256: {sha256}");
    }
    let mut layers: Vec<(&str, Vec<Layer>)> = Vec::new();
    let mut bank_info = serde_json::Map::new();
    for (name, header, expected) in PP6_BANKS {
        let (compression, count) = (u16_at(rom, header)?, u16_at(rom, header + 2)?);
        if compression != 1 || count as usize != expected {
            bail!("Unexpected bank header at 0x{header:X}");
        }
        let graphics = rom_pointer(rom, header + 4)?;
        let bank = (0..expected)
            .map(|i| pp6_layer(rom, header, graphics, i))
            .collect::<Result<Vec<_>>>()?;
        bank_info.insert(
            name.to_string(),
            json!({
                "header": hex(header as u64), "graphics_table": hex(graphics as u64), "count": expected,
                "assets": bank.iter().map(|l| l.info.clone()).collect::<Vec<_>>(),
            }),
        );
        layers.push((name, bank));
    }
    let bank = |name: &str| &layers.iter().find(|(n, _)| *n == name).unwrap().1;

    let (mut actors, mut sheet_sprites) = (Vec::new(), Vec::new());
    let mut used_bodies = BTreeSet::new();
    let mut png_count = 0usize;
    for actor_id in (0..249u32).chain(256..394) {
        let bank_name = if actor_id < 256 { "head_a" } else { "head_b" };
        let row_offset = PP6_STYLE_MAP + 8 * actor_id as usize;
        let row = &rom[row_offset..row_offset + 8];
        // (head, body, entry, style_slots) in first-seen order
        let mut appearances: Vec<Appearance> = Vec::new();
        for slot in 0..4 {
            let head = if row[2 * slot] == 255 {
                row[0]
            } else {
                row[2 * slot]
            };
            let body = if row[2 * slot + 1] == 255 {
                row[1]
            } else {
                row[2 * slot + 1]
            };
            if let Some(found) = appearances.iter_mut().find(|a| (a.0, a.1) == (head, body)) {
                found.3.push(slot);
                continue;
            }
            let mut image = RgbaImage::new(LAYER_SIZE, LAYER_SIZE);
            if body != 255 {
                let Some(layer) = bank("body").get(body as usize) else {
                    bail!("list index out of range")
                };
                overlay(&mut image, &layer.image);
                used_bodies.insert(body);
            }
            let Some(layer) = bank(bank_name).get(head as usize) else {
                bail!("list index out of range")
            };
            overlay(&mut image, &layer.image);
            let relative = format!("actors/{actor_id:03}/appearance-{slot}.png");
            let (mut entry, trimmed) = write_pose(&image, root, &relative)?;
            entry.insert("head".into(), head.into());
            entry.insert(
                "body".into(),
                if body == 255 {
                    Value::Null
                } else {
                    body.into()
                },
            );
            if appearances.is_empty() {
                sheet_sprites.push((actor_id, trimmed));
            }
            png_count += 1;
            appearances.push((head, body, entry, vec![slot]));
        }
        let appearances: Vec<Value> = appearances
            .into_iter()
            .map(|(_, _, mut entry, slots)| {
                entry.insert("style_slots".into(), slots.into());
                Value::Object(entry)
            })
            .collect();
        actors.push(json!({
            "id": actor_id, "head_bank": bank_name, "style_row": hex(row_offset as u64),
            "raw_style_bytes": row.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "appearances": appearances,
        }));
    }

    let mut extra = Vec::new();
    for (index, layer) in bank("body").iter().enumerate() {
        if !used_bodies.contains(&(index as u8)) {
            let (info, _) = write_pose(
                &layer.image,
                root,
                &format!("unpaired-body/body-{index:03}.png"),
            )?;
            let mut entry = json!({ "index": index });
            entry.as_object_mut().unwrap().extend(info);
            extra.push(entry);
        }
    }
    let sheets = pp6_contact_sheets(root, &sheet_sprites)?;
    let title: String = rom[0xA0..0xAC]
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect();
    let manifest = json!({
        "rom_title": title,
        "rom_game_code": String::from_utf8_lossy(&rom[0xAC..0xB0]), "rom_sha256": sha256,
        "file_offsets": "hex, zero based, without the 0x08000000 GBA bus base",
        "pixel_format": "RGBA8, GBA BGR555 components scaled by floor(n*255/31), OBJ index 0 transparent",
        "orientation": "unflipped (right-hand dialogue slot), bounds relative to the ROM metasprite origin",
        "counts": {
            "actor_ids": actors.len(), "style_slots": 4 * actors.len(), "assembled_pngs": png_count,
            "head_graphics": bank("head_a").len() + bank("head_b").len(),
            "body_graphics": bank("body").len(), "bodies_used_in_style_map": used_bodies.len(),
            "unpaired_body_pngs": extra.len(), "index_sheets": sheets.len(),
        },
        "style_map": hex(PP6_STYLE_MAP as u64), "banks": bank_info, "actors": actors,
        "unpaired_bodies": extra, "index_sheets": sheets,
    });
    write_manifest(root, &manifest)?;
    Ok(manifest)
}

/// `alpha_composite` for binary-alpha layers: opaque source pixels replace the destination.
fn overlay(dst: &mut RgbaImage, src: &RgbaImage) {
    for (x, y, p) in src.enumerate_pixels() {
        if p[3] != 0 {
            dst.put_pixel(x, y, *p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(dir: &Path, sub: &str) -> Vec<String> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir.join(sub)).unwrap() {
            let entry = entry.unwrap();
            let leaf = entry.file_name().to_string_lossy().to_string();
            let name = if sub.is_empty() {
                leaf
            } else {
                format!("{sub}/{leaf}")
            };
            if entry.file_type().unwrap().is_dir() {
                out.extend(files(dir, &name));
            } else {
                out.push(name);
            }
        }
        out
    }

    /// Compare every exported PNG against the Python oracle, including sheet labels.
    fn compare(game: &str, got: &Path, want: &Path) {
        let (mut compared, mut sheets) = (0, 0);
        for name in files(want, "") {
            if name == "manifest.json" {
                let a = std::fs::read_to_string(got.join(&name)).unwrap();
                let b = std::fs::read_to_string(want.join(&name)).unwrap();
                assert_eq!(a, b, "{game} manifest differs");
                continue;
            }
            let a = image::open(got.join(&name))
                .unwrap_or_else(|_| panic!("{game} missing {name}"))
                .to_rgba8();
            let b = image::open(want.join(&name)).unwrap().to_rgba8();
            assert_eq!(a.dimensions(), b.dimensions(), "{game} {name}");
            if name.starts_with("index/") {
                assert!(a.as_raw() == b.as_raw(), "{game} {name} pixels differ");
                sheets += 1;
            } else {
                assert!(a.as_raw() == b.as_raw(), "{game} {name} pixels differ");
                compared += 1;
            }
        }
        eprintln!("{game}: {compared} PNGs and {sheets} contact sheets pixel-identical");
    }

    #[test]
    fn pp4_matches_python_oracle_when_available() {
        let Some(rom_path) = crate::test_assets::rom("pp4") else {
            return;
        };
        let Some(want) = crate::test_assets::oracle("pp4", "portraits", "manifest.json") else {
            return;
        };
        let rom = crate::verify_rom(&rom_path).unwrap();
        assert_eq!(rom.game, Game::Pp4);
        let temp = tempfile::tempdir().unwrap();
        extract(&rom.bytes, temp.path(), Game::Pp4).unwrap();
        compare("pp4", temp.path(), &want);
    }

    #[test]
    fn pp6_matches_python_oracle_when_available() {
        let Some(rom_path) = crate::test_assets::rom("pp6") else {
            return;
        };
        let Some(want) = crate::test_assets::oracle("pp6", "portraits", "manifest.json") else {
            return;
        };
        let rom = crate::verify_rom(&rom_path).unwrap();
        assert_eq!(rom.game, Game::Pp6);
        let temp = tempfile::tempdir().unwrap();
        extract(&rom.bytes, temp.path(), Game::Pp6).unwrap();
        compare("pp6", temp.path(), &want);
    }

    #[test]
    fn rejects_unverified_roms() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            extract_pp4(&[0; 64], temp.path())
                .unwrap_err()
                .to_string()
                .contains("Unsupported PP4")
        );
        assert!(
            extract_pp6(&[0; 64], temp.path())
                .unwrap_err()
                .to_string()
                .contains("Unsupported ROM")
        );
    }
}

#[cfg(test)]
mod bust_discovery_tests {
    use super::*;

    #[test]
    fn discovers_the_same_bust_banks_in_both_pp5_retail_variants() {
        for id in ["pp5_v0", "pp5_v0_p"] {
            let Some(path) = crate::test_assets::rom(id) else {
                continue;
            };
            let (profile, rom) = crate::rom::verify_supported_rom(&path).unwrap();
            assert_eq!(discover_busts(&rom), profile.busts, "{id}");
        }
    }
}
