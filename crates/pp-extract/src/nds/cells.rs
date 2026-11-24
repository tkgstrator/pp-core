use super::{graphics::section, le_half, le_word};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

const SIZES: [[(u8, u8); 4]; 3] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)],
    [(16, 8), (32, 8), (32, 16), (64, 32)],
    [(8, 16), (8, 32), (16, 32), (32, 64)],
];

#[derive(Serialize)]
struct Object {
    attr: [usize; 3],
    x: i16,
    y: i16,
    size: Option<(u8, u8)>,
    affine_matrix: Option<usize>,
    double_size: bool,
    disabled: bool,
    flip_x: bool,
    flip_y: bool,
    mode: usize,
    bits_per_pixel: usize,
    tile: usize,
    priority: usize,
    palette: usize,
}

#[derive(Serialize)]
struct Bounds {
    min_x: i16,
    min_y: i16,
    max_x: i16,
    max_y: i16,
}

#[derive(Serialize)]
struct Cell {
    id: usize,
    attributes: usize,
    bounds: Option<Bounds>,
    objects: Vec<Object>,
    transfer: Option<Transfer>,
}

#[derive(Serialize)]
struct Transfer {
    source_offset: usize,
    size: usize,
}

#[derive(Serialize)]
struct Bank {
    mapping_mode: usize,
    bank_attributes: usize,
    vram_transfer_offset: usize,
    cells: Vec<Cell>,
}

fn object(record: &[u8]) -> Object {
    let attr0 = le_half(record);
    let attr1 = le_half(&record[2..]);
    let attr2 = le_half(&record[4..]);
    let affine = attr0 & 0x100 != 0;
    let sign_extend = |value: usize, bits: u32| ((value as i16) << (16 - bits)) >> (16 - bits);
    Object {
        attr: [attr0, attr1, attr2],
        x: sign_extend(attr1 & 0x1ff, 9),
        y: sign_extend(attr0 & 0xff, 8),
        size: SIZES.get(attr0 >> 14).map(|shape| shape[attr1 >> 14]),
        affine_matrix: affine.then_some((attr1 >> 9) & 31),
        double_size: affine && attr0 & 0x200 != 0,
        disabled: !affine && attr0 & 0x200 != 0,
        flip_x: !affine && attr1 & 0x1000 != 0,
        flip_y: !affine && attr1 & 0x2000 != 0,
        mode: (attr0 >> 10) & 3,
        bits_per_pixel: if attr0 & 0x2000 == 0 { 4 } else { 8 },
        tile: attr2 & 0x3ff,
        priority: (attr2 >> 10) & 3,
        palette: attr2 >> 12,
    }
}

fn parse(file: &[u8]) -> Result<Bank> {
    let bank = section(file, b"KBEC")?;
    let header = bank.get(..24).context("truncated NCER cell bank header")?;
    let count = le_half(header);
    let attributes = le_half(&header[2..]);
    let offset = le_word(&header[4..]);
    let mapping = le_word(&header[8..]);
    let vram_transfer_offset = le_word(&header[12..]);
    let transfers = if matches!(vram_transfer_offset, 0 | 0xffff_ffff) {
        None
    } else {
        let transfer = bank
            .get(vram_transfer_offset..)
            .context("NCER VRAM transfer outside bank")?;
        let transfer_header = transfer
            .get(..8)
            .context("truncated NCER VRAM transfer header")?;
        let offset = le_word(&transfer_header[4..]);
        ensure!(offset >= 8, "NCER VRAM transfer table overlaps header");
        Some(
            transfer
                .get(offset..)
                .and_then(|data| data.get(..count * 8))
                .context("truncated NCER VRAM transfer table")?,
        )
    };
    let stride = if attributes & 1 == 0 { 8 } else { 16 };
    ensure!(offset >= 24, "NCER cell table overlaps header");
    let data = bank.get(offset..).context("NCER cell table outside bank")?;
    let table = data
        .get(..count * stride)
        .context("truncated NCER cell table")?;
    let oam = &data[table.len()..];
    let mut cells = Vec::with_capacity(count);
    let mut budget = 65536;
    for (id, entry) in table.chunks_exact(stride).enumerate() {
        let count = le_half(entry);
        ensure!(count <= budget, "NCER object entry limit exceeded");
        budget -= count;
        let offset = le_word(&entry[4..]);
        let records = oam
            .get(offset..)
            .and_then(|data| data.get(..count * 6))
            .context("NCER object data outside bank")?;
        let bounds = entry
            .get(8..)
            .filter(|bounds| !bounds.is_empty())
            .map(|bounds| {
                let edge = |offset| le_half(&bounds[offset..]) as i16;
                Bounds {
                    min_x: edge(4),
                    min_y: edge(6),
                    max_x: edge(0),
                    max_y: edge(2),
                }
            });
        cells.push(Cell {
            id,
            attributes: le_half(&entry[2..]),
            bounds,
            objects: records.chunks_exact(6).map(object).collect(),
            transfer: transfers.map(|table| {
                let entry = &table[id * 8..];
                Transfer {
                    source_offset: le_word(entry),
                    size: le_word(&entry[4..]),
                }
            }),
        });
    }
    Ok(Bank {
        mapping_mode: mapping,
        bank_attributes: attributes,
        vram_transfer_offset,
        cells,
    })
}

pub(super) fn export(file: &[u8], root: &Path, id: usize) -> Result<()> {
    let data = parse(file)?;
    let out = root.join("cells");
    std::fs::create_dir_all(&out)?;
    let writer =
        std::io::BufWriter::new(std::fs::File::create(out.join(format!("{id:04x}.json")))?);
    serde_json::to_writer_pretty(writer, &data)?;
    Ok(())
}

pub(super) fn render(
    file: &[u8],
    chars: &[u8],
    palette: &[u8],
    out: &Path,
) -> Result<BTreeMap<usize, (i16, i16)>> {
    use super::graphics::{palette_bytes, payload};

    let bank = parse(file)?;
    ensure!(bank.mapping_mode <= 4, "unknown NCER tile mapping");
    let data = section(chars, b"RAHC")?;
    let header = data.get(..24).context("truncated NCGR character header")?;
    let depth = le_word(&header[4..]);
    ensure!(
        matches!(depth, 3 | 4) && header[12] == 0,
        "unsupported NCGR cell tiles"
    );
    let tiles = payload(data, le_word(&header[20..]), le_word(&header[16..]), 24)?;
    let palette = palette_bytes(palette)?;
    let columns = le_half(&header[2..]);
    let layout = if bank.mapping_mode == 4 {
        let columns = if matches!(columns, 0 | 0xffff) {
            if depth == 3 { 32 } else { 16 }
        } else {
            columns
        };
        TileLayout::TwoDimensional(columns)
    } else {
        TileLayout::OneDimensional(32 << bank.mapping_mode)
    };
    ensure!(depth == palette.depth, "NCGR and NCLR color depths differ");
    let mut origins = BTreeMap::new();
    let mut index = String::from("cell\torigin_x\torigin_y\tpath\n");
    std::fs::create_dir_all(out)?;
    for cell in bank.cells {
        match render_cell(&cell, tiles, &palette, layout) {
            Ok(Some((image, x, y))) => {
                origins.insert(cell.id, (x, y));
                let name = format!("{:04x}.png", cell.id);
                crate::gba::save_png(&image, &out.join(&name))?;
                writeln!(index, "{}\t{x}\t{y}\t{name}", cell.id)?;
            }
            Ok(None) => {}
            Err(error) => eprintln!("NCER cell {:04x}: {error:#}", cell.id),
        }
    }
    std::fs::write(out.join("index.tsv"), index)?;
    Ok(origins)
}

#[derive(Clone, Copy)]
enum TileLayout {
    OneDimensional(usize),
    TwoDimensional(usize),
}

fn render_cell(
    cell: &Cell,
    tiles: &[u8],
    palette: &super::graphics::Palette<'_>,
    layout: TileLayout,
) -> Result<Option<(image::RgbaImage, i16, i16)>> {
    let visible: Vec<_> = cell.objects.iter().filter(|obj| !obj.disabled).collect();
    if visible.is_empty() {
        return Ok(None);
    }
    let (mut left, mut top, mut right, mut bottom) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
    for obj in &visible {
        ensure!(
            obj.mode == 0 && obj.affine_matrix.is_none(),
            "unsupported NCER object mode"
        );
        let (w, h) = obj.size.context("invalid NCER object size")?;
        left = left.min(obj.x);
        top = top.min(obj.y);
        right = right.max(obj.x + i16::from(w));
        bottom = bottom.max(obj.y + i16::from(h));
    }
    let tiles = if let Some(transfer) = &cell.transfer {
        tiles
            .get(transfer.source_offset..)
            .and_then(|data| data.get(..transfer.size))
            .context("NCER transfer outside NCGR")?
    } else {
        tiles
    };
    let mut canvas = image::RgbaImage::new((right - left) as u32, (bottom - top) as u32);
    let is_4bpp = palette.depth == 3;
    let tile_bytes = if is_4bpp { 32 } else { 64 };
    for obj in visible.into_iter().rev() {
        ensure!(
            obj.bits_per_pixel == if is_4bpp { 4 } else { 8 },
            "NCER and NCGR depths differ"
        );
        let (width, height) = obj.size.unwrap();
        let (width, height) = (usize::from(width), usize::from(height));
        let (boundary, row_tiles) = match layout {
            TileLayout::OneDimensional(boundary) => (boundary, width / 8),
            TileLayout::TwoDimensional(columns) => (32, columns),
        };
        let tile_start = obj.tile * boundary / tile_bytes * tile_bytes;
        let span = ((height / 8 - 1) * row_tiles + width / 8) * tile_bytes;
        let data = tiles
            .get(tile_start..tile_start + span)
            .context("NCER tile range outside NCGR")?;
        let bank = if is_4bpp || palette.extended {
            obj.palette
        } else {
            0
        };
        let color_start = palette.offsets[bank].context("NCER references missing palette")?;
        for y in 0..height {
            for x in 0..width {
                let sx = if obj.flip_x { width - 1 - x } else { x };
                let sy = if obj.flip_y { height - 1 - y } else { y };
                let dot = ((sy / 8) * row_tiles + sx / 8) * 64 + (sy % 8) * 8 + sx % 8;
                let index = if is_4bpp {
                    (data[dot / 2] >> (4 * (dot % 2))) & 15
                } else {
                    data[dot]
                };
                if index != 0 {
                    let color = le_half(&palette.bytes[color_start + usize::from(index) * 2..]);
                    canvas.put_pixel(
                        (obj.x - left) as u32 + x as u32,
                        (obj.y - top) as u32 + y as u32,
                        image::Rgba(crate::gba::rgb(color as u16)),
                    );
                }
            }
        }
    }
    Ok(Some((canvas, left, top)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_two_dimensional_tiles_from_cell_transfer_base() {
        let obj = object(&[0, 0x80, 0, 0, 0, 0]); // 8x16, tile zero
        let cell = Cell {
            id: 0,
            attributes: 0,
            bounds: None,
            objects: vec![obj],
            transfer: Some(Transfer {
                source_offset: 32,
                size: 96,
            }),
        };
        let mut tiles = [0; 128];
        tiles[32] = 1;
        tiles[96] = 2;
        let mut colors = [0; 32];
        colors[2] = 31;
        colors[5] = 124;
        let mut offsets = [None; 16];
        offsets[0] = Some(0);
        let palette = super::super::graphics::Palette {
            depth: 3,
            bytes: &colors,
            offsets,
            extended: false,
        };
        let (image, _, _) = render_cell(&cell, &tiles, &palette, TileLayout::TwoDimensional(2))
            .unwrap()
            .unwrap();
        assert_eq!(image.dimensions(), (8, 16));
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(0, 8).0, [0, 0, 255, 255]);
    }

    #[test]
    fn decodes_cell_bounds_and_signed_object_coordinates() {
        let mut bank = vec![0; 24 + 16];
        bank[0] = 1;
        bank[2] = 1;
        bank[4] = 24;
        bank[24] = 1;
        for (offset, value) in [(32, 4i16), (34, 8), (36, -5), (38, -2)] {
            bank[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
        for value in [0x40feu16, 0x51fb, 0x3405] {
            bank.extend(value.to_le_bytes());
        }
        let mut file = b"RECN\xff\xfe\0\x01\0\0\0\0\x10\0\x01\0KBEC".to_vec();
        file[8..12].copy_from_slice(&(24 + bank.len() as u32).to_le_bytes());
        file.extend((8 + bank.len() as u32).to_le_bytes());
        file.extend(bank);
        let result = serde_json::to_value(parse(&file).unwrap()).unwrap();
        let cell = &result["cells"][0];
        assert_eq!(cell["bounds"]["min_x"], -5);
        assert_eq!(cell["bounds"]["max_y"], 8);
        let obj = &cell["objects"][0];
        assert_eq!(obj["x"], -5);
        assert_eq!(obj["y"], -2);
        assert_eq!(obj["size"], serde_json::json!([32, 8]));
        assert_eq!(obj["flip_x"], true);
        assert_eq!(obj["tile"], 5);
        assert_eq!(obj["palette"], 3);
        assert!(parse(&file[..file.len() - 1]).is_err());
    }
}
