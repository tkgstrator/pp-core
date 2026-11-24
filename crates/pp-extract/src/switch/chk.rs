use super::word;
use anyhow::{Context, Result, bail, ensure};
use image::RgbaImage;
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct StringChunk<'a> {
    offset: usize,
    entries: Vec<&'a str>,
}

// Resource files may have a 32-byte RDB slot header before the CHK payload.
// The STRING table layout is documented in Switch-Pawapuro2024-KR.
pub(super) fn strings(data: &[u8]) -> Vec<StringChunk<'_>> {
    let Some(start) = chk_start(data) else {
        return Vec::new();
    };
    let mut chunks = Vec::new();
    let mut budget = 8 * 1024 * 1024;
    let mut offset = start;
    while let Some(header) = data.get(offset..offset + 16) {
        if header.starts_with(b"STRING") && header[6..8].iter().all(|b| matches!(b, 0 | b' ')) {
            let size = word(&header[8..]) as usize;
            if size.is_multiple_of(4)
                && let Some(body) = data
                    .get(offset..)
                    .and_then(|rest| rest.get(..size))
                    .and_then(|chunk| chunk.get(16..))
            {
                if let Some(entries) = string_table(body, &mut budget) {
                    chunks.push(StringChunk { offset, entries });
                    offset += size;
                    continue;
                }
            }
        }
        offset += 4;
    }
    chunks
}

fn chk_start(data: &[u8]) -> Option<usize> {
    [0, 32].into_iter().find(|&offset| {
        data.get(offset..)
            .is_some_and(|body| body.starts_with(b"CHK "))
    })
}

#[derive(Serialize)]
pub(super) struct Surface<'a> {
    pub name: &'a str,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub block_height_log2: u8,
    pub descriptor_offset: usize,
    pub data_offset: usize,
    pub size: usize,
}

impl Surface<'_> {
    pub(super) fn preview(&self, bytes: &[u8]) -> Result<RgbaImage> {
        use tegra_swizzle::BlockHeight;
        let bytes_per_block = match self.format {
            0x20 | 0x45 => 8,
            0x22 | 0x24 | 0x42 | 0x47 | 0x4b | 0x4d | 0x11 => 16,
            _ => bail!("unsupported NX SUR format {:#x}", self.format),
        };
        ensure!(
            self.width > 0
                && self.height > 0
                && u64::from(self.width) * u64::from(self.height) <= 16 * 1024 * 1024,
            "NX SUR dimensions exceed preview limit"
        );
        let block_height = [
            BlockHeight::One,
            BlockHeight::Two,
            BlockHeight::Four,
            BlockHeight::Eight,
            BlockHeight::Sixteen,
            BlockHeight::ThirtyTwo,
        ]
        .get(usize::from(self.block_height_log2))
        .copied()
        .context("invalid NX SUR block height")?;
        let blocks_x = self.width.div_ceil(4);
        let blocks_y = self.height.div_ceil(4);
        let linear = tegra_swizzle::swizzle::deswizzle_block_linear(
            blocks_x,
            blocks_y,
            1,
            bytes,
            block_height,
            bytes_per_block,
        )
        .context("untiling NX SUR texture")?;
        let mut image = RgbaImage::new(self.width, self.height);
        let mut rgba = [0; 64];
        for (index, block) in linear.chunks_exact(bytes_per_block as usize).enumerate() {
            match self.format {
                0x20 => bcdec_rs::bc1(block, &mut rgba, 16),
                0x22 | 0x24 | 0x42 => bcdec_rs::bc3(block, &mut rgba, 16),
                0x45 => {
                    let mut red = [0; 16];
                    bcdec_rs::bc4(block, &mut red, 4, false);
                    for (pixel, red) in rgba.chunks_exact_mut(4).zip(red) {
                        pixel.copy_from_slice(&[red, red, red, 255]);
                    }
                }
                0x47 => {
                    let mut rg = [0; 32];
                    bcdec_rs::bc5(block, &mut rg, 8, false);
                    for (pixel, channels) in rgba.chunks_exact_mut(4).zip(rg.chunks_exact(2)) {
                        pixel.copy_from_slice(&[channels[0], channels[1], 0, 255]);
                    }
                }
                _ => bcdec_rs::bc7(block, &mut rgba, 16),
            }
            let x = index as u32 % blocks_x * 4;
            let y = index as u32 / blocks_x * 4;
            for row in 0..4.min(self.height - y) {
                for column in 0..4.min(self.width - x) {
                    let start = (row * 16 + column * 4) as usize;
                    image.put_pixel(
                        x + column,
                        y + row,
                        image::Rgba(rgba[start..start + 4].try_into().unwrap()),
                    );
                }
            }
        }
        Ok(image)
    }
}

fn metadata_chunks(body: &[u8], first: usize, end: usize) -> Option<Vec<(usize, &[u8])>> {
    if first < 28 {
        return None;
    }
    let mut chunks = body.get(first..end)?;
    let mut offset = first;
    let mut records = Vec::new();
    while !chunks.is_empty() {
        let header = chunks.get(..24)?;
        let length = word(&header[8..]) as usize;
        if length < 24 || !length.is_multiple_of(4) {
            return None;
        }
        let (chunk, rest) = chunks.split_at_checked(length)?;
        records.push((offset, chunk));
        offset += length;
        chunks = rest;
    }
    Some(records)
}

#[derive(Serialize)]
struct Glyph {
    codepoint: u32,
    metrics: u32,
    declared_offset: usize,
    bitmap_offset: Option<usize>,
}

#[derive(Serialize)]
struct Font {
    kind: &'static str,
    chunk_offset: usize,
    width: usize,
    height: usize,
    bytes_per_glyph: usize,
    glyphs: Vec<Glyph>,
}

fn font(chunk: &[u8], offset: usize) -> Option<Font> {
    if chunk.get(16..24)? != b"UNCDFONT" {
        return None;
    }
    let (kind, size, adjustment) = match chunk.get(..4)? {
        b"FNTL" => ("FNTL", 56, -6),
        b"FNTS" => ("FNTS", 44, 0),
        _ => return None,
    };
    let header = chunk.get(..0x3c)?;
    let count = word(&header[0x20..]) as usize;
    let width = word(&header[0x24..]) as usize;
    let height = word(&header[0x28..]) as usize;
    if (width, height) != (size, size) || !(1..=8192).contains(&count) {
        return None;
    }
    let bytes_per_glyph = width * height / 2;
    let table_end = 0x3c + 12 * count;
    let glyphs = chunk
        .get(0x3c..table_end)?
        .chunks_exact(12)
        .map(|record| {
            let declared_offset = word(&record[4..]) as usize;
            let bitmap_offset = declared_offset
                .checked_add_signed(adjustment)
                .filter(|&start| {
                    start >= table_end
                        && start
                            .checked_add(bytes_per_glyph)
                            .is_some_and(|end| end <= chunk.len())
                })
                .map(|start| offset + start);
            Glyph {
                codepoint: word(record),
                metrics: word(&record[8..]),
                declared_offset,
                bitmap_offset,
            }
        })
        .collect::<Vec<_>>();
    glyphs
        .iter()
        .any(|glyph| glyph.bitmap_offset.is_some())
        .then_some(Font {
            kind,
            chunk_offset: offset,
            width,
            height,
            bytes_per_glyph,
            glyphs,
        })
}

pub(super) fn export_fonts(
    data: &[u8],
    root: &std::path::Path,
    id: usize,
) -> Result<Option<String>> {
    let Some(base) = chk_start(data) else {
        return Ok(None);
    };
    let body = &data[base..];
    let Some(fields) = body.get(16..24) else {
        return Ok(None);
    };
    let Some(chunks) = metadata_chunks(body, word(fields) as usize, word(&fields[4..]) as usize)
    else {
        return Ok(None);
    };
    let fonts = chunks
        .iter()
        .filter_map(|&(start, chunk)| font(chunk, base + start).map(|info| (info, chunk)))
        .collect::<Vec<_>>();
    if fonts.is_empty() {
        return Ok(None);
    }
    let folder = format!("fonts/{id:06x}");
    std::fs::create_dir_all(root.join(&folder))?;
    let mut records = Vec::new();
    for (index, (font, chunk)) in fonts.into_iter().enumerate() {
        let path = format!("{index:04x}.bin");
        std::fs::write(root.join(&folder).join(&path), chunk)?;
        records.push(serde_json::json!({ "path": path, "font": font }));
    }
    let path = format!("{folder}/index.json");
    let file = std::fs::File::create(root.join(&path))?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), &records)?;
    Ok(Some(path))
}

// The CHK header contains a separate blob-size table. A surface refers to
// the next matching blob size, skipping blobs used by other chunk types.
pub(super) fn surfaces(data: &[u8]) -> Option<Vec<Surface<'_>>> {
    let base = chk_start(data)?;
    let body = &data[base..];
    let field = |offset| body.get(offset..offset + 4).map(word);
    let first = field(16)? as usize;
    let data_start = field(20)? as usize;
    let count = field(24)? as usize;
    if count > 65536 {
        return None;
    }
    let sizes_end = 28 + 4 * count;
    if !(sizes_end..=data_start).contains(&first) {
        return None;
    }
    let mut blobs = Vec::with_capacity(count);
    let mut start = data_start;
    for size in body
        .get(28..sizes_end)?
        .chunks_exact(4)
        .map(|size| word(size) as usize)
    {
        let end = start.checked_add(size)?;
        body.get(start..end)?;
        blobs.push((start, size));
        start = end;
    }

    let mut result = Vec::new();
    let mut blobs = blobs.into_iter();
    for (offset, chunk) in metadata_chunks(body, first, data_start)? {
        if &chunk[16..24] == b"NX  SUR " {
            let dimensions = chunk.get(64..72)?;
            let width = word(dimensions);
            let height = word(&dimensions[4..]);
            let name = chunk.get(32..64)?;
            let stop = name
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(name.len());
            let name = std::str::from_utf8(&name[..stop]).ok()?;
            let size = word(chunk.get(0x70..0x74)?) as usize;
            let block_height_log2 = *chunk.get(0xdc)?;
            if width == 0 || height == 0 || name.is_empty() || size == 0 || block_height_log2 > 5 {
                return None;
            }
            let (blob, _) = blobs.find(|&(_, blob_size)| blob_size == size)?;
            result.push(Surface {
                name,
                width,
                height,
                format: word(chunk.get(0x54..0x58)?),
                block_height_log2,
                descriptor_offset: base + offset,
                data_offset: base + blob,
                size,
            });
        }
    }
    Some(result)
}

fn string_table<'a>(body: &'a [u8], budget: &mut usize) -> Option<Vec<&'a str>> {
    let table_size = word(body.get(..4)?) as usize;
    if table_size < 4 || !table_size.is_multiple_of(4) || table_size / 4 > 65536 {
        return None;
    }
    let table = body.get(..table_size)?;
    let mut remaining = *budget;
    let mut entries = Vec::new();
    for pointer in table.chunks_exact(4) {
        let offset = word(pointer) as usize;
        let text = body.get(offset..)?;
        if offset < table_size {
            return None;
        }
        let end = text
            .get(..text.len().min(remaining))?
            .iter()
            .position(|&b| b == 0)?;
        entries.push(std::str::from_utf8(&text[..end]).ok()?);
        remaining -= end + 1;
    }
    *budget = remaining;
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::strings;

    #[test]
    fn reads_multiple_utf8_offsets_and_stays_inside_the_chunk() {
        let mut data = vec![0; 32];
        data.extend_from_slice(b"CHK ");
        data.extend_from_slice(b"STRING\0\0");
        data.extend_from_slice(&32u32.to_le_bytes());
        data.extend_from_slice(&17u32.to_le_bytes());
        data.extend_from_slice(&8u32.to_le_bytes());
        data.extend_from_slice(&12u32.to_le_bytes());
        data.extend_from_slice("甲\0乙\0".as_bytes());
        let tables = strings(&data);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].offset, 36);
        assert_eq!(tables[0].entries, ["甲", "乙"]);
        let last = data.len() - 1;
        data[last] = b'x';
        data.push(0);
        assert!(strings(&data).is_empty());
    }
    #[test]
    fn pairs_surface_with_the_next_matching_blob_size() {
        let mut data = vec![0; 32 + 36 + 0xe0 + 12];
        let body = &mut data[32..];
        body[..4].copy_from_slice(b"CHK ");
        for (at, value) in [(16, 36u32), (20, 36 + 0xe0), (24, 2), (28, 4), (32, 8)] {
            body[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let chunk = &mut body[36..36 + 0xe0];
        chunk[8] = 0xe0;
        chunk[16..24].copy_from_slice(b"NX  SUR ");
        chunk[32..37].copy_from_slice(b"x.tsr");
        chunk[64] = 16;
        chunk[68] = 8;
        chunk[0x54] = 0x20;
        chunk[0x70] = 8;
        let entries = super::surfaces(&data).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "x.tsr");
        assert_eq!((entries[0].width, entries[0].height), (16, 8));
        assert_eq!(entries[0].data_offset, 32 + 36 + 0xe0 + 4);
        assert!(super::surfaces(&data[..data.len() - 1]).is_none());
    }
    #[test]
    fn renders_tiled_bc1_blocks_in_order_and_crops_partial_blocks() {
        let surface = super::Surface {
            name: "x.tsr",
            width: 5,
            height: 3,
            format: 0x20,
            block_height_log2: 0,
            descriptor_offset: 0,
            data_offset: 0,
            size: 512,
        };
        let mut linear = [0; 16];
        // The two BC1 blocks use their first RGB565 color for all pixels.
        linear[..2].copy_from_slice(&0xf800u16.to_le_bytes());
        linear[8..10].copy_from_slice(&0x07e0u16.to_le_bytes());
        let tiled = tegra_swizzle::swizzle::swizzle_block_linear(
            2,
            1,
            1,
            &linear,
            tegra_swizzle::BlockHeight::One,
            8,
        )
        .unwrap();
        let image = surface.preview(&tiled).unwrap();
        assert_eq!(image.dimensions(), (5, 3));
        assert_eq!(image.get_pixel(3, 2).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(4, 2).0, [0, 255, 0, 255]);
        assert!(surface.preview(&tiled[..16]).is_err());
    }
    #[test]
    fn exports_font_chunks_with_adjusted_and_checked_bitmap_offsets() {
        let size = 56 * 56 / 2;
        let mut body = vec![0; 32 + 0x54 + size];
        body[..4].copy_from_slice(b"CHK ");
        body[16..20].copy_from_slice(&32u32.to_le_bytes());
        let end = body.len() as u32;
        body[20..24].copy_from_slice(&end.to_le_bytes());
        let chunk = &mut body[32..];
        chunk[..4].copy_from_slice(b"FNTL");
        chunk[16..24].copy_from_slice(b"UNCDFONT");
        for (at, value) in [
            (8, (0x54 + size) as u32),
            (0x20, 2),
            (0x24, 56),
            (0x28, 56),
            (0x3c, u32::from('球')),
            (0x40, 0x5a),
            (0x44, 56 << 8),
            (0x48, 0x3042),
            (0x4c, 1),
        ] {
            chunk[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        chunk[0x54] = 0xa5;
        let out = tempfile::tempdir().unwrap();
        let path = super::export_fonts(&body, out.path(), 0).unwrap().unwrap();
        let index = out.path().join(path);
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
        let first = &metadata[0]["font"]["glyphs"][0];
        assert_eq!(first["codepoint"], u32::from('球'));
        assert_eq!(first["bitmap_offset"], 32 + 0x54);
        assert!(metadata[0]["font"]["glyphs"][1]["bitmap_offset"].is_null());
        assert_eq!(
            std::fs::read(index.parent().unwrap().join("0000.bin")).unwrap(),
            &body[32..]
        );
    }
}
