//! Structural scan and export of regular (4bpp, compressed-tile) GBA background
//! records from the PP4 / PP6 ROMs. Ports `tools/backgrounds/{rom_backgrounds,layers}.py`.
//!
//! Output (`out` = category directory): `<record>.png` (RGBA) for each decodable record
//! plus `catalog.tsv` listing every structural record, including failures.

use crate::Game;
use crate::gba::{self, ROM_BASE};
use anyhow::Result;
use image::RgbaImage;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

const OBJ_VRAM_OFFSET: u32 = 0x0600_0000;

/// A structurally valid background record found in the ROM.
#[derive(Clone, Debug)]
pub struct Record {
    pub record: usize,
    pub flags: u32,
    pub tiles: usize,
    pub vram_destination: u32,
    pub tile_bytes: usize,
    pub map: usize,
    /// `[x, y, width, height]` in tiles.
    pub layout: [u8; 4],
    /// BG palette bank number -> ROM offset of 16 BGR555 colours.
    pub palettes: BTreeMap<u32, usize>,
}

/// Bus address to ROM offset, requiring `count` readable bytes from there.
fn pointer(rom: &[u8], address: u32, count: usize) -> Option<usize> {
    let pos = i64::from(address) - i64::from(ROM_BASE);
    (pos >= 0 && pos as usize + count <= rom.len()).then_some(pos as usize)
}

fn u32_at(rom: &[u8], pos: usize) -> Option<u32> {
    gba::u32_at(rom, pos).ok()
}

/// Parse and validate the descriptor at `at`; `None` if it is not a background record.
pub fn descriptor(rom: &[u8], at: usize) -> Option<Record> {
    let flags = u32_at(rom, at)?;
    let t = pointer(rom, u32_at(rom, at + 4)?, 12)?;
    let p = pointer(rom, u32_at(rom, at + 8)?, 4)?;
    let m = pointer(rom, u32_at(rom, at + 12)?, 8)?;
    let src = u32_at(rom, t)?;
    let dst = u32_at(rom, t + 4)?;
    let length = u32_at(rom, t + 8)? as usize;
    let tiles = pointer(rom, src, 1)?;
    let maps = u32_at(rom, m)?;
    let [x, y, w, h] = [rom[m + 4], rom[m + 5], rom[m + 6], rom[m + 7]];
    if !(0 < w
        && w <= 64
        && 0 < h
        && h <= 64
        && 0 < length
        && length <= 65536
        && length.is_multiple_of(32)
        && (OBJ_VRAM_OFFSET..0x0601_0000).contains(&dst)
        && dst % 32 == 0)
    {
        return None;
    }
    let map = pointer(rom, maps, 2 * w as usize * h as usize)?;
    let banks = u32_at(rom, p)? as usize;
    if !(1..=16).contains(&banks) || p + 4 + banks * 4 > rom.len() {
        return None;
    }
    let mut palettes = BTreeMap::new();
    for i in 0..banks {
        let block = pointer(rom, u32_at(rom, p + 4 + i * 4)?, 36)?;
        let dest = i64::from(u32_at(rom, block)?) - 0x0500_0000;
        if !(0..=480).contains(&dest) || dest % 32 != 0 {
            return None;
        }
        palettes.insert((dest / 32) as u32, block + 4);
    }
    Some(Record {
        record: at,
        flags,
        tiles,
        vram_destination: dst,
        tile_bytes: length,
        map,
        layout: [x, y, w, h],
        palettes,
    })
}

/// All structural records in ROM order (4-byte aligned scan).
pub fn discover(rom: &[u8]) -> Vec<Record> {
    let mut found = Vec::new();
    let end = rom.len().saturating_sub(16);
    for at in (0..end).step_by(4) {
        if rom[at] & 0xE0 != 0x80 || rom[at + 1..at + 4] != [0, 0, 0] {
            continue;
        }
        if let Some(record) = descriptor(rom, at) {
            found.push(record);
        }
    }
    found
}

/// Render 4bpp tile-map entries over tile data with a 256-colour BGR555 palette.
fn render(
    entries: &[u16],
    tiles: &[u8],
    palette: &[u8; 512],
    width: usize,
    height: usize,
) -> RgbaImage {
    let mut canvas = RgbaImage::new(width as u32 * 8, height as u32 * 8);
    let mut cache: std::collections::HashMap<u16, [[u8; 4]; 64]> = Default::default();
    for (index, &entry) in entries.iter().enumerate() {
        let tile = cache.entry(entry).or_insert_with(|| {
            let mut tile = [[0u8; 4]; 64];
            let start = (entry as usize & 1023) * 32;
            for y in 0..8 {
                for x in 0..8 {
                    let sx = if entry & 1024 != 0 { 7 - x } else { x };
                    let sy = if entry & 2048 != 0 { 7 - y } else { y };
                    let byte = tiles[start + sy * 4 + sx / 2];
                    let color = (byte >> (4 * (sx % 2))) & 15;
                    if color != 0 {
                        let i = (color as usize + (entry >> 12) as usize * 16) * 2;
                        tile[y * 8 + x] =
                            gba::rgb(u16::from_le_bytes([palette[i], palette[i + 1]]));
                    }
                }
            }
            tile
        });
        let (ox, oy) = ((index % width * 8) as u32, (index / width * 8) as u32);
        for y in 0..8 {
            for x in 0..8 {
                canvas.put_pixel(ox + x as u32, oy + y as u32, image::Rgba(tile[y * 8 + x]));
            }
        }
    }
    canvas
}

/// Decode a record into an RGBA image. `Err` text matches the Python catalog errors.
pub fn draw(rom: &[u8], rec: &Record) -> std::result::Result<RgbaImage, String> {
    if rec.flags & 0xE0 != 0x80 {
        return Err("Unsupported encoding flags".into());
    }
    let (w, h) = (rec.layout[2] as usize, rec.layout[3] as usize);
    let entries: Vec<u16> = (0..w * h)
        .map(|i| gba::u16_at(rom, rec.map + 2 * i).map_err(|_| "index out of range".to_string()))
        .collect::<std::result::Result<_, _>>()?;
    let start = (rec.vram_destination - OBJ_VRAM_OFFSET) as usize % 16384;
    let (decoded, _) =
        gba::decompress(rom, rec.tiles, rec.tile_bytes).map_err(|e| e.to_string())?;
    let mut tiles = vec![0u8; start];
    tiles.extend_from_slice(&decoded);
    let mut palette = [0u8; 512];
    for (&bank, &source) in &rec.palettes {
        let src = rom.get(source..source + 32).ok_or("index out of range")?;
        palette[bank as usize * 32..bank as usize * 32 + 32].copy_from_slice(src);
    }
    if entries
        .iter()
        .any(|&e| ((e as usize & 1023) + 1) * 32 > tiles.len() || (e as usize & 1023) * 32 < start)
    {
        return Err("References tiles outside descriptor".into());
    }
    if entries
        .iter()
        .any(|&e| !rec.palettes.contains_key(&u32::from(e >> 12)) && (e & 1023) != 0)
    {
        return Err("References another palette group".into());
    }
    Ok(render(&entries, &tiles, &palette, w, h))
}

/// Per-category counts, as reported by the Python exporter.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    pub structural: usize,
    pub decoded: usize,
    pub failed: usize,
}

pub fn export(rom: &[u8], out: &Path) -> Result<Counts> {
    std::fs::create_dir_all(out)?;
    let mut counts = Counts::default();
    let mut tsv = String::from(
        "record\ttile_width\ttile_height\ttile_data\tmap\tflags\tstatus\terror\timage\trgba_sha256\n",
    );
    for rec in discover(rom) {
        counts.structural += 1;
        let [_, _, w, h] = rec.layout;
        let (status, error, image, digest) = match draw(rom, &rec) {
            Ok(img) => {
                let name = format!("{:06x}.png", rec.record);
                gba::save_png(&img, &out.join(&name))?;
                counts.decoded += 1;
                (
                    "decoded",
                    String::new(),
                    name,
                    gba::sha256_hex(img.as_raw()),
                )
            }
            Err(e) => {
                counts.failed += 1;
                ("failed", e, String::new(), String::new())
            }
        };
        writeln!(
            tsv,
            "{:#x}\t{w}\t{h}\t{:#x}\t{:#x}\t{:#x}\t{status}\t{error}\t{image}\t{digest}",
            rec.record, rec.tiles, rec.map, rec.flags
        )?;
    }
    std::fs::write(out.join("catalog.tsv"), tsv)?;
    Ok(counts)
}

/// Extract every structurally matched background into `out` (the images category directory).
pub fn extract(rom: &[u8], out: &Path, _game: Game) -> Result<()> {
    export(rom, out).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage() {
        assert!(discover(&vec![0xFF; 4096]).is_empty());
        assert!(descriptor(&[0u8; 8], 0).is_none());
    }

    #[test]
    fn render_flips_and_transparency() {
        // One tile: pixel (0,0) colour 1, others 0. Map entries: plain, hflip, vflip.
        let mut tiles = vec![0u8; 32];
        tiles[0] = 0x01;
        let mut pal = [0u8; 512];
        pal[2..4].copy_from_slice(&0x7FFFu16.to_le_bytes());
        let img = render(&[0, 1024, 2048], &tiles, &pal, 3, 1);
        assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 0, 0, 0]);
        assert_eq!(img.get_pixel(8 + 7, 0).0, [255, 255, 255, 255]);
        assert_eq!(img.get_pixel(16, 7).0, [255, 255, 255, 255]);
    }

    /// Compare against the Python oracle catalog and per-record RGBA digests.
    #[test]
    fn matches_python_oracle_when_available() {
        for (game, file) in [("pp4", "pp4.gba"), ("pp6", "pp6.gba")] {
            let Some(rom_path) = crate::test_assets::rom(game) else {
                continue;
            };
            let Some(dir) = crate::test_assets::oracle(game, "images", "catalog.tsv") else {
                continue;
            };
            assert_eq!(rom_path.file_name().unwrap(), file);
            let rom = crate::verify_rom(&rom_path).unwrap().bytes;
            let oracle = std::fs::read_to_string(dir.join("catalog.tsv")).unwrap();
            let temp = tempfile::tempdir().unwrap();
            let counts = export(&rom, temp.path()).unwrap();
            let got = std::fs::read_to_string(temp.path().join("catalog.tsv")).unwrap();
            assert_eq!(got, oracle, "{game} catalog differs");
            for line in oracle.lines().skip(1) {
                let f: Vec<&str> = line.split('\t').collect();
                if f[8].is_empty() {
                    continue;
                }
                let want = image::open(dir.join(f[8])).unwrap().to_rgba8();
                let have = image::open(temp.path().join(f[8])).unwrap().to_rgba8();
                assert_eq!(want.dimensions(), have.dimensions(), "{game} {}", f[8]);
                assert!(
                    want.as_raw() == have.as_raw(),
                    "{game} {} pixels differ",
                    f[8]
                );
            }
            eprintln!("{game}: {counts:?}");
        }
    }
}
