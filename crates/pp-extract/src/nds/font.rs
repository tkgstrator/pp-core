use super::{graphics::section, le_half, le_word};
use anyhow::{Context, Result, ensure};
use image::{Rgba, RgbaImage};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;
use std::path::Path;

fn linked_block<'a>(file: &'a [u8], offset: usize, tag: &[u8; 4]) -> Result<&'a [u8]> {
    let start = offset
        .checked_sub(8)
        .context("invalid NFTR linked block offset")?;
    let block = file
        .get(start..)
        .context("NFTR linked block outside file")?;
    let header = block.get(..8).context("truncated NFTR linked block")?;
    ensure!(
        &header[..4] == tag,
        "unexpected NFTR linked block signature"
    );
    block
        .get(8..le_word(&header[4..]))
        .context("invalid NFTR linked block size")
}

#[derive(Clone, Copy)]
struct Metrics {
    left: i8,
    width: u8,
    advance: Option<i8>,
}

fn width_table(file: &[u8], info: &[u8], glyphs: usize, version: usize) -> Result<Vec<Metrics>> {
    let stride = if version == 1 { 2 } else { 3 };
    let decode = |entry: &[u8]| Metrics {
        left: entry[0] as i8,
        width: entry[1],
        advance: entry.get(2).map(|&value| value as i8),
    };
    let default = decode(&info[4..4 + stride]);
    let mut widths = vec![None; glyphs];
    let mut offset = le_word(&info[12..]);
    let mut visited = HashSet::new();
    let mut budget = glyphs * 2;
    while offset != 0 {
        ensure!(visited.insert(offset), "cyclic NFTR width table");
        let block = linked_block(file, offset, b"HDWC")?;
        let header = block.get(..8).context("truncated NFTR width header")?;
        let first = le_half(header);
        let last = le_half(&header[2..]);
        let range = widths
            .get_mut(first..=last)
            .filter(|slots| !slots.is_empty())
            .context("invalid NFTR width range")?;
        ensure!(range.len() <= budget, "NFTR width table limit exceeded");
        budget -= range.len();
        let entries = block
            .get(8..8 + stride * range.len())
            .context("truncated NFTR width entries")?;
        for (slot, entry) in range.iter_mut().zip(entries.chunks_exact(stride)) {
            slot.get_or_insert_with(|| decode(entry));
        }
        offset = le_word(&header[4..]);
    }
    Ok(widths
        .into_iter()
        .map(|width| width.unwrap_or(default))
        .collect())
}

fn code_map(file: &[u8], mut offset: usize, glyphs: usize) -> Result<BTreeMap<u16, usize>> {
    let mut seen = HashSet::new();
    let mut codes = BTreeMap::new();
    let mut mapping_budget = 0x20000;
    while offset != 0 {
        ensure!(seen.insert(offset), "cyclic NFTR character map");
        let block = linked_block(file, offset, b"PAMC")?;
        let header = block
            .get(..12)
            .context("truncated NFTR character map header")?;
        let entries = &block[12..];
        let low = le_half(header);
        let high = le_half(&header[2..]);
        let mut add = |code, glyph| -> Result<()> {
            ensure!(mapping_budget > 0, "NFTR character map limit exceeded");
            mapping_budget -= 1;
            if glyph != 0xffff {
                ensure!(glyph < glyphs, "NFTR character references missing glyph");
                codes.entry(code as u16).or_insert(glyph);
            }
            Ok(())
        };
        match le_half(&header[4..]) {
            method @ (0 | 1) => {
                ensure!(high >= low, "invalid NFTR character range");
                let count = high - low + 1;
                if method == 0 {
                    let base = le_half(entries.get(..2).context("missing NFTR direct map base")?);
                    for index in 0..count {
                        add(low + index, base + index)?;
                    }
                } else {
                    let entries = entries
                        .get(..count * 2)
                        .context("truncated NFTR lookup map")?;
                    for (index, entry) in entries.chunks_exact(2).enumerate() {
                        add(low + index, le_half(entry))?;
                    }
                }
            }
            2 => {
                let count = le_half(entries.get(..2).context("missing NFTR scan map count")?);
                let entries = entries
                    .get(2..2 + count * 4)
                    .context("truncated NFTR scan map")?;
                for entry in entries.chunks_exact(4) {
                    add(le_half(entry), le_half(&entry[2..]))?;
                }
            }
            method => anyhow::bail!("unsupported NFTR character map method {method}"),
        }
        offset = le_word(&header[8..]);
    }
    Ok(codes)
}

fn glyph_sheet(data: &[u8]) -> Result<(RgbaImage, usize)> {
    let header = data.get(..8).context("truncated NFTR glyph header")?;
    let (width, height, depth) = (
        usize::from(header[0]),
        usize::from(header[1]),
        usize::from(header[6]),
    );
    let stride = le_half(&header[2..]);
    ensure!(
        width > 0 && height > 0 && (1..=8).contains(&depth),
        "invalid NFTR glyph geometry"
    );
    let image_bytes = (width * height * depth).div_ceil(8);
    ensure!(
        stride >= image_bytes && stride - image_bytes < 3,
        "unsupported NFTR glyph stride"
    );
    let cells = &data[8..];
    let count = cells.len() / stride;
    ensure!(count > 0, "empty NFTR glyph sheet");
    let columns = count.min(16);
    let rows = count.div_ceil(columns);
    ensure!(
        rows * columns * width * height <= 16 * 1024 * 1024,
        "NFTR glyph sheet exceeds output limit"
    );
    let image = RgbaImage::from_fn((columns * width) as u32, (rows * height) as u32, |x, y| {
        let (x, y) = (x as usize, y as usize);
        let glyph = (y / height) * columns + x / width;
        if glyph >= count {
            return Rgba([0; 4]);
        }
        let pixel = (y % height) * width + x % width;
        let mut value = 0;
        for bit in pixel * depth..(pixel + 1) * depth {
            value =
                (value << 1) | usize::from((cells[glyph * stride + bit / 8] >> (7 - bit % 8)) & 1);
        }
        let alpha = (value * 255 / ((1 << depth) - 1)) as u8;
        Rgba([255, 255, 255, alpha])
    });
    Ok((image, count))
}

pub(super) fn export(file: &[u8], root: &Path, id: usize) -> Result<()> {
    let info = section(file, b"FNIF")?;
    let info = info.get(..20).context("truncated NFTR font info")?;
    let glyphs = section(file, b"PLGC")?;
    let (image, count) = glyph_sheet(glyphs)?;
    let version = le_half(&file[6..]);
    let encoding = info[if version == 1 { 6 } else { 7 }];
    let file = &file[..le_word(&file[8..])];
    let codes = code_map(file, le_word(&info[16..]), count)?;
    let widths = width_table(file, info, count, version)?;
    let mut metrics = String::from("glyph_id\tleft_bearing\tink_width\tadvance_width\n");
    for (glyph, width) in widths.iter().enumerate() {
        let advance = width
            .advance
            .map_or_else(String::new, |value| value.to_string());
        writeln!(
            metrics,
            "{glyph}\t{}\t{}\t{advance}",
            width.left, width.width
        )?;
    }
    let mut index = String::from("code_hex\tglyph_id\tx\ty\n");
    let (width, height) = (usize::from(glyphs[0]), usize::from(glyphs[1]));
    let columns = count.min(16);
    for (code, glyph) in codes {
        writeln!(
            index,
            "{code:04x}\t{glyph}\t{}\t{}",
            (glyph % columns) * width,
            (glyph / columns) * height
        )?;
    }
    let out = root.join("fonts").join(format!("{id:04x}"));
    crate::gba::save_png(&image, &out.join("glyphs.png"))?;
    std::fs::write(out.join("glyphs.tsv"), index)?;
    std::fs::write(out.join("metrics.tsv"), metrics)?;
    let meta = serde_json::json!({
        "version": version,
        "encoding": match encoding { 0 => "UTF-8", 1 => "UTF-16", 2 => "Shift_JIS", 3 => "ASCII", _ => "unknown" },
        "cell_width": width,
        "cell_height": height,
        "line_height": info[1],
        "fallback_glyph": le_half(&info[2..]),
        "bits_per_pixel": glyphs[6],
        "bitmap_flags": glyphs[7],
        "glyph_count": count,
    });
    std::fs::write(out.join("font.json"), serde_json::to_string_pretty(&meta)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_eight_dot_glyphs_and_linked_character_maps() {
        fn block(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut result = tag.to_vec();
            result.extend((8 + body.len() as u32).to_le_bytes());
            result.extend(body);
            result
        }

        let mut info = [0; 20];
        info[1] = 9;
        info[7] = 2;
        info[12..16].copy_from_slice(&98u32.to_le_bytes());
        info[16..20].copy_from_slice(&76u32.to_le_bytes());
        let mut glyphs = [0; 16];
        glyphs[..8].copy_from_slice(&[8, 8, 8, 0, 7, 8, 1, 0]);
        glyphs[8] = 0x81;
        let map = [0x60, 0x82, 0x60, 0x82, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let mut file = b"RTFN\xff\xfe\x00\x01\0\0\0\0\x10\0\x04\0".to_vec();
        file.extend(block(b"FNIF", &info));
        file.extend(block(b"PLGC", &glyphs));
        file.extend(block(b"PAMC", &map));
        file.extend(block(b"HDWC", &[0, 0, 0, 0, 0, 0, 0, 0, 0xff, 6, 7]));
        let size = file.len() as u32;
        file[8..12].copy_from_slice(&size.to_le_bytes());

        let out = tempfile::tempdir().unwrap();
        export(&file, out.path(), 5).unwrap();
        let out = out.path().join("fonts/0005");
        let sheet = image::open(out.join("glyphs.png")).unwrap().into_rgba8();
        assert_eq!(sheet.dimensions(), (8, 8));
        assert_eq!(sheet.get_pixel(0, 0).0[3], 255);
        assert_eq!(sheet.get_pixel(1, 0).0[3], 0);
        assert_eq!(sheet.get_pixel(7, 0).0[3], 255);
        assert!(
            std::fs::read_to_string(out.join("glyphs.tsv"))
                .unwrap()
                .contains("8260\t0\t0\t0")
        );
        assert!(
            std::fs::read_to_string(out.join("font.json"))
                .unwrap()
                .contains("Shift_JIS")
        );
        assert_eq!(
            std::fs::read_to_string(out.join("metrics.tsv"))
                .unwrap()
                .lines()
                .nth(1),
            Some("0\t-1\t6\t7")
        );
        file[84..88].copy_from_slice(&76u32.to_le_bytes());
        assert!(
            code_map(&file, 76, 1)
                .unwrap_err()
                .to_string()
                .contains("cyclic")
        );
    }
}
