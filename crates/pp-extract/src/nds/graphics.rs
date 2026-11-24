use super::{le_half, le_word};
use anyhow::{Context, Result, ensure};
use image::{Rgba, RgbaImage};
use std::path::Path;

fn optional_section<'a>(file: &'a [u8], tag: &[u8; 4]) -> Result<Option<&'a [u8]>> {
    let header = file.get(..16).context("truncated Nitro graphics header")?;
    ensure!(
        header[4..6] == [0xff, 0xfe],
        "unsupported Nitro graphics byte order"
    );
    let start = le_half(&header[12..]);
    let end = le_word(&header[8..]);
    ensure!(start >= 16, "invalid Nitro graphics header size");
    let mut blocks = file
        .get(start..end)
        .context("invalid Nitro graphics file size")?;
    for _ in 0..le_half(&header[14..]) {
        let header = blocks.get(..8).context("truncated Nitro graphics block")?;
        let size = le_word(&header[4..]);
        let data = blocks
            .get(8..size)
            .context("invalid Nitro graphics block size")?;
        if &header[..4] == tag {
            return Ok(Some(data));
        }
        blocks = &blocks[size..];
    }
    Ok(None)
}

pub(super) fn section<'a>(file: &'a [u8], tag: &[u8; 4]) -> Result<&'a [u8]> {
    optional_section(file, tag)?.context("missing Nitro graphics section")
}

pub(super) fn payload(
    data: &[u8],
    offset: usize,
    size: usize,
    header_size: usize,
) -> Result<&[u8]> {
    ensure!(offset >= header_size, "graphics payload overlaps header");
    data.get(offset..)
        .and_then(|rest| rest.get(..size))
        .context("graphics payload outside section")
}

fn tiles(file: &[u8]) -> Result<RgbaImage> {
    let data = section(file, b"RAHC")?;
    let header = data.get(..24).context("truncated NCGR character header")?;
    let (colors, divisor) = match le_word(&header[4..]) {
        3 => (16, 2),
        4 => (256, 1),
        depth => anyhow::bail!("unsupported NCGR depth {depth}"),
    };
    let bytes = payload(data, le_word(&header[20..]), le_word(&header[16..]), 24)?;
    let tiled = header[12] == 0;
    let (columns, rows) = (le_half(&header[2..]), le_half(header));
    let tiles = bytes
        .len()
        .checked_mul(divisor)
        .context("NCGR size overflow")?
        / 64;
    ensure!(
        tiles > 0 && bytes.len().is_multiple_of(64 / divisor),
        "incomplete NCGR tiles"
    );
    let (columns, rows) = if columns == 0xffff || rows == 0xffff {
        ensure!(tiled, "missing NCGR dimensions for linear graphics");
        let columns = tiles.min(16);
        (columns, tiles.div_ceil(columns))
    } else {
        ensure!(
            columns > 0 && columns.checked_mul(rows) == Some(tiles),
            "NCGR dimensions do not match pixel data"
        );
        (columns, rows)
    };
    let (width, height) = (columns * 8, rows * 8);
    ensure!(
        height <= 16384 && width <= 16384 && width * height <= 16 * 1024 * 1024,
        "NCGR image exceeds output limit"
    );
    Ok(RgbaImage::from_fn(width as u32, height as u32, |x, y| {
        let (x, y) = (x as usize, y as usize);
        let index = if tiled {
            ((y / 8) * columns + x / 8) * 64 + (y % 8) * 8 + x % 8
        } else {
            y * width + x
        };
        let Some(&raw) = bytes.get(index / divisor) else {
            return Rgba([0; 4]);
        };
        let value = if divisor == 2 {
            (raw >> (4 * (index % 2))) & 15
        } else {
            raw
        };
        let gray = (usize::from(value) * 255 / (colors - 1)) as u8;
        Rgba([gray, gray, gray, 255])
    }))
}

pub(super) struct Palette<'a> {
    pub(super) depth: usize,
    pub(super) bytes: &'a [u8],
    pub(super) offsets: [Option<usize>; 16],
    pub(super) extended: bool,
}

pub(super) fn palette_bytes(file: &[u8]) -> Result<Palette<'_>> {
    let data = section(file, b"TTLP")?;
    let header = data.get(..16).context("truncated NCLR palette header")?;
    let depth = le_word(header);
    let bank_bytes = match depth {
        3 => 32,
        4 => 512,
        _ => anyhow::bail!("unsupported NCLR palette depth"),
    };
    let offset = le_word(&header[12..]);
    let map = optional_section(file, b"PMCP")?;
    let mut offsets = [None; 16];
    let size = if let Some(map) = map {
        let header = map.get(..8).context("truncated NCLR palette map")?;
        let count = le_half(header);
        ensure!(
            (1..=16).contains(&count),
            "invalid NCLR mapped palette count"
        );
        let ids = payload(map, le_word(&header[4..]), count * 2, 8)?;
        for (stored, id) in ids.chunks_exact(2).map(le_half).enumerate() {
            let slot = offsets
                .get_mut(id)
                .context("NCLR palette ID outside bank range")?;
            ensure!(slot.is_none(), "duplicate NCLR palette ID");
            *slot = Some(stored * bank_bytes);
        }
        // Some PCMP writers record the omitted bytes in the TTLP size field.
        count * bank_bytes
    } else {
        let reported = le_word(&header[8..]);
        let size = if reported == 0 {
            data.len().saturating_sub(offset)
        } else {
            reported
        };
        for (id, slot) in offsets.iter_mut().take(size / bank_bytes).enumerate() {
            *slot = Some(id * bank_bytes);
        }
        size
    };
    let bytes = payload(data, offset, size, 16)?;
    ensure!(
        !bytes.is_empty() && bytes.len().is_multiple_of(2) && bytes.len() <= 8192,
        "invalid NCLR color count"
    );
    Ok(Palette {
        depth,
        bytes,
        offsets,
        extended: le_word(&header[4..]) != 0,
    })
}

fn palette(file: &[u8]) -> Result<RgbaImage> {
    let Palette { bytes, .. } = palette_bytes(file)?;
    let colors = bytes.len() / 2;
    Ok(RgbaImage::from_fn(
        128,
        (colors.div_ceil(16) * 8) as u32,
        |x, y| {
            let at = (y as usize / 8 * 16 + x as usize / 8) * 2;
            bytes.get(at..at + 2).map_or(Rgba([0; 4]), |color| {
                Rgba(crate::gba::rgb(le_half(color) as u16))
            })
        },
    ))
}

pub(super) fn export(file: &[u8], root: &Path, id: usize) -> Result<()> {
    let (kind, image) = if file.starts_with(b"RGCN") {
        ("tiles", tiles(file)?)
    } else {
        ("palettes", palette(file)?)
    };
    crate::gba::save_png(&image, &root.join(kind).join(format!("{id:04x}.png")))
}

// Text backgrounds store 32x32-tile panels in row-major order, clipping edge panels.
fn screen_entry_index(columns: usize, rows: usize, x: usize, y: usize) -> usize {
    let panel_x = x / 32 * 32;
    let panel_y = y / 32 * 32;
    let panel_width = (columns - panel_x).min(32);
    let panel_height = (rows - panel_y).min(32);
    panel_y * columns + panel_x * panel_height + (y % 32) * panel_width + x % 32
}

fn render_screen(screen: &[u8], chars: &[u8], palette: &[u8]) -> Result<RgbaImage> {
    let map = section(screen, b"NRCS")?;
    let header = map.get(..12).context("truncated NSCR screen header")?;
    let (width, height) = (le_half(header), le_half(&header[2..]));
    ensure!(
        width > 0
            && height > 0
            && width.is_multiple_of(8)
            && height.is_multiple_of(8)
            && width <= 4096
            && height <= 4096,
        "invalid NSCR dimensions"
    );
    let color_mode = le_half(&header[4..]);
    let format = le_half(&header[6..]);
    ensure!(
        color_mode <= 2 && format <= 2 && (format != 1 || color_mode <= 1),
        "unsupported NSCR screen format"
    );
    let entries = payload(map, 12, le_word(&header[8..]), 12)?;
    let entry_bytes = if format == 1 { 1 } else { 2 };
    ensure!(
        entries.len() == width / 8 * (height / 8) * entry_bytes,
        "NSCR screen dimensions do not match map"
    );

    let source = section(chars, b"RAHC")?;
    let header = source
        .get(..24)
        .context("truncated NCGR character header")?;
    let depth = le_word(&header[4..]);
    ensure!(
        matches!(depth, 3 | 4) && header[12] == 0,
        "unsupported NCGR screen tiles"
    );
    let pixels = payload(source, le_word(&header[20..]), le_word(&header[16..]), 24)?;
    let Palette {
        depth: palette_depth,
        bytes: colors,
        offsets,
        ..
    } = palette_bytes(palette)?;
    ensure!(depth == palette_depth, "NCGR and NCLR color depths differ");
    ensure!(
        (format == 0 && color_mode == 0) || depth == 4,
        "NSCR color mode and NCGR depth differ"
    );
    let tile_bytes = if depth == 3 { 32 } else { 64 };
    let bank_id = |entry: usize| {
        if depth == 3 || color_mode == 2 {
            entry >> 12
        } else {
            0
        }
    };
    let read_entry = |bytes: &[u8]| {
        if entry_bytes == 1 {
            usize::from(bytes[0])
        } else {
            le_half(bytes)
        }
    };
    for entry in entries.chunks_exact(entry_bytes).map(read_entry) {
        let tile = entry & 0x3ff;
        let bank = bank_id(entry);
        ensure!(
            (tile + 1) * tile_bytes <= pixels.len(),
            "NSCR references a missing tile"
        );
        ensure!(offsets[bank].is_some(), "NSCR references a missing palette");
    }

    Ok(RgbaImage::from_fn(width as u32, height as u32, |x, y| {
        let (x, y) = (x as usize, y as usize);
        let index = if format == 0 {
            screen_entry_index(width / 8, height / 8, x / 8, y / 8)
        } else {
            (y / 8) * (width / 8) + x / 8
        };
        let entry = read_entry(&entries[index * entry_bytes..]);
        let tile = entry & 0x3ff;
        let bank = bank_id(entry);
        let sx = if entry & 0x400 != 0 { 7 - x % 8 } else { x % 8 };
        let sy = if entry & 0x800 != 0 { 7 - y % 8 } else { y % 8 };
        let dot = sy * 8 + sx;
        let index = if depth == 3 {
            (pixels[tile * 32 + dot / 2] >> (4 * (dot % 2))) & 15
        } else {
            pixels[tile * 64 + dot]
        };
        let color = le_half(&colors[offsets[bank].unwrap() + usize::from(index) * 2..]);
        Rgba(crate::gba::obj_rgba(color as u16, usize::from(index)))
    }))
}

pub(super) fn export_background(
    screen: &Path,
    tiles: &Path,
    palette: &Path,
    out: &Path,
) -> Result<()> {
    let load = |path: &Path, magic: &[u8; 4]| -> Result<Vec<u8>> {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if data.starts_with(magic) {
            return Ok(data);
        }
        let decoded = super::compression::decompress(&data, 64 * 1024 * 1024)
            .with_context(|| format!("decoding {}", path.display()))?;
        ensure!(
            decoded.starts_with(magic),
            "unexpected Nitro resource format in {}",
            path.display()
        );
        Ok(decoded)
    };
    let image = render_screen(
        &load(screen, b"RCSN")?,
        &load(tiles, b"RGCN")?,
        &load(palette, b"RLCN")?,
    )?;
    crate::gba::save_png(&image, out)
}

#[derive(Clone, Copy)]
struct ScreenFile {
    id: usize,
    expanded: bool,
}

#[derive(Clone, Copy, Default)]
enum Candidate {
    #[default]
    Missing,
    Exact(ScreenFile),
    Ambiguous,
}

impl Candidate {
    fn available(self) -> Option<ScreenFile> {
        match self {
            Self::Exact(file) => Some(file),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct ScreenSets<'a>(std::collections::BTreeMap<&'a str, [Candidate; 5]>);

impl<'a> ScreenSets<'a> {
    pub(super) fn add(&mut self, name: &'a str, body: &[u8], id: usize, expanded: bool) {
        let (slot, extensions): (_, &[&str]) = match body.get(..4) {
            Some(b"RGCN") => (0, &["ncgr", "zcg"]),
            Some(b"RLCN") => (1, &["nclr", "zcl"]),
            Some(b"RCSN") => (2, &["nscr", "zsc"]),
            Some(b"RECN") => (3, &["ncer", "zce"]),
            Some(b"RNAN") => (4, &["nanr", "zan"]),
            _ => return,
        };
        let Some((stem, extension)) = name.rsplit_once('.') else {
            return;
        };
        let matches_type = |suffix: &str| {
            extensions
                .iter()
                .any(|item| suffix.eq_ignore_ascii_case(item))
        };
        let stem = if expanded && super::compression::recognized_suffix(extension) {
            match stem.rsplit_once('.') {
                Some((inner, kind)) if matches_type(kind) => inner,
                _ => stem,
            }
        } else if matches_type(extension) {
            stem
        } else {
            return;
        };
        let entry = &mut self.0.entry(stem).or_default()[slot];
        *entry = match entry {
            Candidate::Missing => Candidate::Exact(ScreenFile { id, expanded }),
            _ => Candidate::Ambiguous,
        };
    }

    pub(super) fn render(&self, root: &Path) -> Result<()> {
        for set in self.0.values() {
            let [Some(chars), Some(palette), screen, cell_bank, animation] =
                set.map(Candidate::available)
            else {
                continue;
            };
            let load = |file: ScreenFile| {
                let folder = if file.expanded {
                    root.join("decompressed")
                } else {
                    root.to_path_buf()
                };
                std::fs::read(folder.join(format!("{:04x}.bin", file.id)))
            };
            if let Some(screen) = screen {
                match render_screen(&load(screen)?, &load(chars)?, &load(palette)?) {
                    Ok(image) => crate::gba::save_png(
                        &image,
                        &root
                            .join("backgrounds")
                            .join(format!("{:04x}.png", screen.id)),
                    )?,
                    Err(error) => eprintln!("NSCR {:04x}: {error:#}", screen.id),
                }
            }
            if let Some(cell_bank) = cell_bank {
                let sprites = root.join("sprites").join(format!("{:04x}", cell_bank.id));
                match super::cells::render(
                    &load(cell_bank)?,
                    &load(chars)?,
                    &load(palette)?,
                    &sprites,
                ) {
                    Ok(origins) => {
                        if let Some(animation) = animation
                            && let Err(error) = super::animation::render(
                                &load(animation)?,
                                &sprites,
                                &origins,
                                &root
                                    .join("animations")
                                    .join(format!("{:04x}", animation.id)),
                            )
                        {
                            eprintln!("NANR {:04x}: {error:#}", animation.id);
                        }
                    }
                    Err(error) => eprintln!("NCER {:04x}: {error:#}", cell_bank.id),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(magic: &[u8; 4], tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut file = magic.to_vec();
        file.extend([0xff, 0xfe, 0, 1]);
        file.extend((24 + body.len() as u32).to_le_bytes());
        file.extend([16, 0, 1, 0]);
        file.extend(tag);
        file.extend((8 + body.len() as u32).to_le_bytes());
        file.extend(body);
        file
    }

    #[test]
    fn pairs_compressed_graphics_and_skips_ambiguous_resources() {
        let mut sets = ScreenSets::default();
        sets.add("folder/bg.ncgr.LZ", b"RGCN", 1, true);
        sets.add("folder/bg.zcl", b"RLCN", 2, true);
        sets.add("folder/bg.NSCR", b"RCSN", 3, false);
        let grouped = &sets.0["folder/bg"];
        assert_eq!(grouped[0].available().unwrap().id, 1);
        assert!(grouped[1].available().unwrap().expanded);
        assert_eq!(grouped[2].available().unwrap().id, 3);
        sets.add("folder/bg.ncgr", b"RGCN", 4, false);
        assert!(sets.0["folder/bg"][0].available().is_none());
        sets.add("other/bg.ncgr", b"RGCN", 5, false);
        assert!(sets.0["other/bg"][0].available().is_some());
    }

    #[test]
    fn renders_named_non_affine_sprite_with_origin() {
        let out = tempfile::tempdir().unwrap();
        let root = out.path();
        let mut chars = [0; 88];
        chars[4] = 3;
        chars[16] = 64;
        chars[20] = 24;
        chars[24] = 1;
        let mut colors = [0; 48];
        colors[0] = 3;
        colors[8] = 32;
        colors[12] = 16;
        colors[18..20].copy_from_slice(&0x001fu16.to_le_bytes());
        let mut bank = [0; 38];
        bank[0] = 1;
        bank[4] = 24;
        bank[24] = 1;
        bank[32..34].copy_from_slice(&0x40feu16.to_le_bytes());
        bank[34..36].copy_from_slice(&0x11fbu16.to_le_bytes());
        let files = [
            ("face/model.ncgr", fixture(b"RGCN", b"RAHC", &chars)),
            ("face/model.nclr", fixture(b"RLCN", b"TTLP", &colors)),
            ("face/model.ncer", fixture(b"RECN", b"KBEC", &bank)),
        ];
        let mut sets = ScreenSets::default();
        for (id, (name, data)) in files.iter().enumerate() {
            std::fs::write(root.join(format!("{id:04x}.bin")), data).unwrap();
            sets.add(name, data, id, false);
        }
        sets.render(root).unwrap();
        let folder = root.join("sprites/0002");
        let image = image::open(folder.join("0000.png")).unwrap().into_rgba8();
        assert_eq!(image.dimensions(), (16, 8));
        assert_eq!(image.get_pixel(15, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(0, 0).0, [0; 4]);
        assert_eq!(
            std::fs::read_to_string(folder.join("index.tsv"))
                .unwrap()
                .lines()
                .nth(1),
            Some("0\t-5\t-2\t0000.png")
        );
    }

    #[test]
    fn reconstructs_named_text_background_with_palette_and_flips() {
        let out = tempfile::tempdir().unwrap();
        let root = out.path();
        let mut chars = [0; 56];
        chars[4] = 3;
        chars[16] = 32;
        chars[20] = 24;
        chars[24] = 1;
        let mut colors = [0; 80];
        colors[0] = 3;
        colors[8] = 64;
        colors[12] = 16;
        colors[18..20].copy_from_slice(&0x001fu16.to_le_bytes());
        colors[50..52].copy_from_slice(&0x7c00u16.to_le_bytes());
        let mut screen = [0; 16];
        screen[0] = 16;
        screen[2] = 8;
        screen[8] = 4;
        screen[14..16].copy_from_slice(&0x1c00u16.to_le_bytes());

        let files = [
            ("room/floor.NCGR", fixture(b"RGCN", b"RAHC", &chars)),
            ("room/floor.NCLR", fixture(b"RLCN", b"TTLP", &colors)),
            ("room/floor.NSCR", fixture(b"RCSN", b"NRCS", &screen)),
        ];
        let mut sets = ScreenSets::default();
        for (id, (name, data)) in files.iter().enumerate() {
            std::fs::write(root.join(format!("{id:04x}.bin")), data).unwrap();
            sets.add(name, data, id, false);
        }
        sets.render(root).unwrap();
        let image = image::open(root.join("backgrounds/0002.png"))
            .unwrap()
            .into_rgba8();
        assert_eq!(image.dimensions(), (16, 8));
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(15, 7).0, [0, 0, 255, 255]);
        assert_eq!(image.get_pixel(8, 0).0, [0; 4]);
        let packed_map = root.join("layout.bin");
        let map_data = &files[2].1;
        let mut packed = ((map_data.len() as u32) << 8 | 0x10).to_le_bytes().to_vec();
        for bytes in map_data.chunks(8) {
            packed.push(0);
            packed.extend(bytes);
        }
        std::fs::write(&packed_map, packed).unwrap();
        let manual = root.join("manual.png");
        export_background(
            &packed_map,
            &root.join("0000.bin"),
            &root.join("0001.bin"),
            &manual,
        )
        .unwrap();
        assert_eq!(image::open(manual).unwrap().into_rgba8(), image);
    }

    #[test]
    fn maps_sparse_palette_ids_and_indexes_clipped_panels() {
        let mut colors = [0; 80];
        colors[0] = 3;
        colors[8] = 1; // The map determines how many bytes are actually stored.
        colors[12] = 16;
        colors[18..20].copy_from_slice(&0x001fu16.to_le_bytes());
        colors[50..52].copy_from_slice(&0x7c00u16.to_le_bytes());
        let mut file = fixture(b"RLCN", b"TTLP", &colors);
        let map = [2, 0, 0xef, 0xbe, 8, 0, 0, 0, 5, 0, 1, 0];
        file.extend(b"PMCP");
        file.extend(20u32.to_le_bytes());
        file.extend(map);
        let len = file.len() as u32;
        file[8..12].copy_from_slice(&len.to_le_bytes());
        file[14] = 2;
        let mapped = palette_bytes(&file).unwrap();
        assert_eq!(mapped.offsets[0], None);
        assert_eq!(mapped.offsets[5], Some(0));
        assert_eq!(mapped.offsets[1], Some(32));
        let mut chars = [0; 56];
        chars[4] = 3;
        chars[16] = 32;
        chars[20] = 24;
        chars[24] = 1;
        let mut screen = [0; 16];
        screen[0] = 16;
        screen[2] = 8;
        screen[8] = 4;
        screen[13] = 0x50;
        screen[15] = 0x10;
        let image = render_screen(
            &fixture(b"RCSN", b"NRCS", &screen),
            &fixture(b"RGCN", b"RAHC", &chars),
            &file,
        )
        .unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(8, 0).0, [0, 0, 255, 255]);

        assert_eq!(screen_entry_index(35, 34, 32, 0), 1024);
        assert_eq!(screen_entry_index(35, 34, 34, 31), 1119);
        assert_eq!(screen_entry_index(35, 34, 0, 32), 1120);
        assert_eq!(screen_entry_index(35, 34, 34, 33), 1189);
    }

    #[test]
    fn renders_affine_maps_in_linear_order_with_both_entry_sizes() {
        let mut chars = [0; 24 + 128];
        chars[4] = 4;
        chars[16] = 128;
        chars[20] = 24;
        chars[24] = 1;
        chars[88] = 2;
        let chars = fixture(b"RGCN", b"RAHC", &chars);
        let mut colors = [0; 16 + 1024];
        colors[0] = 4;
        colors[9] = 4;
        colors[12] = 16;
        colors[18..20].copy_from_slice(&0x001fu16.to_le_bytes());
        colors[20..22].copy_from_slice(&0x03e0u16.to_le_bytes());
        colors[530..532].copy_from_slice(&0x7c00u16.to_le_bytes());
        let colors = fixture(b"RLCN", b"TTLP", &colors);

        let mut affine = [0; 12 + 66];
        affine[..2].copy_from_slice(&264u16.to_le_bytes());
        affine[2] = 16;
        affine[4] = 1;
        affine[6] = 1;
        affine[8] = 66;
        affine[12 + 33] = 1;
        let image = render_screen(&fixture(b"RCSN", b"NRCS", &affine), &chars, &colors).unwrap();
        assert_eq!(image.get_pixel(0, 8).0, [0, 255, 0, 255]);
        assert_eq!(image.get_pixel(8, 8).0, [255, 0, 0, 255]);

        let mut extended = [0; 14];
        extended[0] = 8;
        extended[2] = 8;
        extended[4] = 2;
        extended[6] = 2;
        extended[8] = 2;
        extended[12..14].copy_from_slice(&0x1c00u16.to_le_bytes());
        let image = render_screen(&fixture(b"RCSN", b"NRCS", &extended), &chars, &colors).unwrap();
        assert_eq!(image.get_pixel(7, 7).0, [0, 0, 255, 255]);
        assert_eq!(image.get_pixel(0, 0).0, [0; 4]);
    }

    #[test]
    fn previews_tiled_indices_and_bgr555_swatches() {
        let mut chars = [0; 24 + 64];
        chars[0] = 1;
        chars[2] = 2;
        chars[4] = 3;
        chars[16] = 64;
        chars[20] = 24;
        chars[24] = 0x21;
        chars[56] = 0x4f;
        let image = tiles(&fixture(b"RGCN", b"RAHC", &chars)).unwrap();
        assert_eq!(image.dimensions(), (16, 8));
        assert_eq!(image.get_pixel(0, 0).0, [17, 17, 17, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [34, 34, 34, 255]);
        assert_eq!(image.get_pixel(8, 0).0, [255, 255, 255, 255]);
        assert_eq!(image.get_pixel(9, 0).0, [68, 68, 68, 255]);

        let mut colors = [0; 20];
        colors[0] = 3;
        colors[8] = 4;
        colors[12] = 16;
        colors[16..20].copy_from_slice(&[31, 0, 0, 124]);
        let image = palette(&fixture(b"RLCN", b"TTLP", &colors)).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(8, 0).0, [0, 0, 255, 255]);
        assert_eq!(image.get_pixel(16, 0).0, [0; 4]);
    }
}
