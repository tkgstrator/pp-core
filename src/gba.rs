//! Shared GBA helpers for the graphics extractors: ROM pointers, the portrait/tile
//! stream decompressor (ROM 0x14DC), BGR555 colours, OBJ shapes and PNG/JSON utilities.

use anyhow::{Result, anyhow};
use image::RgbaImage;
use sha2::{Digest, Sha256};
use std::path::Path;

pub const ROM_BASE: u32 = 0x0800_0000;

/// OBJ shape index (`attr0/attr1` shape+size) to (width, height) in pixels.
pub const SIZES: [(usize, usize); 12] = [
    (8, 8),
    (16, 16),
    (32, 32),
    (64, 64),
    (16, 8),
    (32, 8),
    (32, 16),
    (64, 32),
    (8, 16),
    (8, 32),
    (16, 32),
    (32, 64),
];

pub fn u16_at(rom: &[u8], offset: usize) -> Result<u16> {
    rom.get(offset..offset + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| anyhow!("read outside ROM at 0x{offset:X}"))
}

pub fn u32_at(rom: &[u8], offset: usize) -> Result<u32> {
    rom.get(offset..offset + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| anyhow!("read outside ROM at 0x{offset:X}"))
}

pub fn i16_at(rom: &[u8], offset: usize) -> Result<i16> {
    Ok(u16_at(rom, offset)? as i16)
}

/// Read a bus pointer stored at `offset` and return the zero-based ROM offset.
pub fn rom_pointer(rom: &[u8], offset: usize) -> Result<usize> {
    let value = u32_at(rom, offset).map_err(|_| anyhow!("Invalid ROM pointer at 0x{offset:X}"))?;
    let result = value.wrapping_sub(ROM_BASE) as usize;
    if value < ROM_BASE || result >= rom.len() {
        return Err(anyhow!("Invalid ROM pointer at 0x{offset:X}"));
    }
    Ok(result)
}

/// The portrait/background tile stream loader: literal runs, 64-byte fills, byte
/// repeats and 3-byte fills, terminated by a zero command. Returns the expanded data
/// and the exclusive end offset (after the terminator).
pub fn decompress(
    rom: &[u8],
    mut position: usize,
    expected_size: usize,
) -> Result<(Vec<u8>, usize)> {
    let oob = || anyhow!("index out of range");
    let mut output: Vec<u8> = Vec::with_capacity(expected_size + 64);
    while output.len() < expected_size {
        let command = *rom.get(position).ok_or_else(oob)?;
        position += 1;
        if command == 0 {
            return Err(anyhow!("Early end of portrait tile stream"));
        } else if command < 0x80 {
            let n = command as usize;
            let start = position.min(rom.len());
            let end = (position + n).min(rom.len());
            output.extend_from_slice(&rom[start..end]);
            position += n;
        } else if command == 0x80 || command == 0x81 {
            output.extend(std::iter::repeat_n(
                if command == 0x80 { 0x00 } else { 0xFF },
                64,
            ));
        } else if command < 0xC0 {
            if let Some(&b) = rom.get(position) {
                output.extend(std::iter::repeat_n(b, (command & 63) as usize + 1));
            }
            position += 1;
        } else {
            output.extend(std::iter::repeat_n(command & 63, 3));
        }
    }
    if output.len() != expected_size || *rom.get(position).ok_or_else(oob)? != 0 {
        return Err(anyhow!(
            "Invalid expanded length or terminator at 0x{position:X}"
        ));
    }
    Ok((output, position + 1))
}

/// GBA BGR555 to RGBA8 (components scaled by floor(n*255/31)); alpha is always 255.
pub fn rgb(color: u16) -> [u8; 4] {
    let c = |shift: u32| ((((color >> shift) & 31) as u32) * 255 / 31) as u8;
    [c(0), c(5), c(10), 255]
}

/// OBJ palette colour: index 0 is transparent.
pub fn obj_rgba(color: u16, index: usize) -> [u8; 4] {
    if index == 0 { [0; 4] } else { rgb(color) }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn hex(value: impl Into<u64>) -> String {
    format!("{:#x}", value.into())
}

pub fn save_png(image: &RgbaImage, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image.save_with_format(path, image::ImageFormat::Png)?;
    Ok(())
}

/// Bounding box (x0, y0, x1, y1 exclusive) of pixels with non-zero alpha.
pub fn alpha_bbox(image: &RgbaImage) -> Option<[u32; 4]> {
    let mut bbox: Option<[u32; 4]> = None;
    for (x, y, p) in image.enumerate_pixels() {
        if p[3] != 0 {
            bbox = Some(match bbox {
                None => [x, y, x + 1, y + 1],
                Some([a, b, c, d]) => [a.min(x), b.min(y), c.max(x + 1), d.max(y + 1)],
            });
        }
    }
    bbox
}

/// Copy opaque pixels of `src` over `dst` at a possibly out-of-range signed offset
/// (equivalent to PIL `paste(src, pos, src)` for binary alpha).
pub fn paste_alpha(dst: &mut image::RgbImage, src: &RgbaImage, ox: i64, oy: i64) {
    for (x, y, p) in src.enumerate_pixels() {
        let (dx, dy) = (ox + x as i64, oy + y as i64);
        if dx < 0 || dy < 0 || dx >= dst.width() as i64 || dy >= dst.height() as i64 || p[3] == 0 {
            continue;
        }
        let d = dst.get_pixel_mut(dx as u32, dy as u32);
        if p[3] == 255 {
            d.0 = [p[0], p[1], p[2]];
        } else {
            for i in 0..3 {
                d.0[i] =
                    ((p[i] as u32 * p[3] as u32 + d.0[i] as u32 * (255 - p[3] as u32)) / 255) as u8;
            }
        }
    }
}

/// Pillow's default antialiased digit coverage as used by the Python contact sheets.
/// The 6x8 glyphs start two pixels below the supplied text position.
/// Coverage was derived from the existing PP4/PP6 oracle sheets, then checked
/// against every label in both sets.
const DIGIT_COVERAGE: [[[u8; 6]; 8]; 10] = [
    [
        // 0
        [15, 176, 196, 176, 15, 0],
        [138, 114, 0, 115, 136, 0],
        [212, 26, 0, 27, 211, 0],
        [235, 3, 0, 3, 234, 0],
        [235, 3, 0, 3, 234, 0],
        [212, 26, 0, 27, 211, 0],
        [138, 114, 0, 115, 137, 0],
        [15, 176, 196, 176, 15, 0],
    ],
    [
        // 1
        [0, 16, 160, 241, 0, 0],
        [0, 195, 109, 240, 0, 0],
        [0, 24, 0, 240, 0, 0],
        [0, 0, 0, 240, 0, 0],
        [0, 0, 0, 240, 0, 0],
        [0, 0, 0, 240, 0, 0],
        [0, 0, 0, 240, 0, 0],
        [0, 0, 0, 240, 0, 0],
    ],
    [
        // 2
        [0, 98, 195, 208, 70, 0],
        [21, 193, 3, 40, 215, 0],
        [40, 85, 0, 11, 234, 0],
        [0, 0, 0, 96, 157, 0],
        [0, 0, 35, 212, 21, 0],
        [0, 12, 204, 55, 0, 0],
        [1, 173, 89, 0, 0, 0],
        [97, 244, 192, 192, 180, 0],
    ],
    [
        // 3
        [10, 169, 192, 205, 75, 0],
        [118, 116, 0, 25, 224, 0],
        [17, 7, 0, 68, 219, 0],
        [0, 0, 163, 245, 67, 0],
        [0, 0, 0, 79, 162, 0],
        [147, 4, 0, 3, 237, 0],
        [162, 84, 0, 61, 198, 0],
        [30, 191, 193, 187, 40, 0],
    ],
    [
        // 4
        [0, 0, 0, 74, 247, 0],
        [0, 0, 13, 198, 242, 0],
        [0, 0, 149, 86, 240, 0],
        [0, 60, 175, 0, 240, 0],
        [7, 200, 26, 0, 240, 0],
        [90, 213, 192, 192, 253, 162],
        [0, 0, 0, 0, 240, 0],
        [0, 0, 0, 0, 240, 0],
    ],
    [
        // 5
        [69, 222, 192, 192, 132, 0],
        [93, 105, 0, 0, 0, 0],
        [118, 79, 0, 0, 0, 0],
        [143, 154, 196, 178, 35, 0],
        [142, 107, 0, 82, 190, 0],
        [32, 3, 0, 3, 235, 0],
        [168, 79, 0, 58, 190, 0],
        [37, 196, 192, 184, 35, 0],
    ],
    [
        // 6
        [3, 147, 199, 194, 34, 0],
        [106, 133, 0, 81, 174, 0],
        [198, 36, 0, 7, 118, 0],
        [235, 114, 192, 175, 32, 0],
        [241, 83, 0, 83, 189, 0],
        [218, 3, 0, 3, 235, 0],
        [152, 65, 0, 65, 188, 0],
        [22, 179, 191, 187, 35, 0],
    ],
    [
        // 7
        [168, 192, 192, 198, 236, 0],
        [0, 0, 0, 100, 130, 0],
        [0, 0, 1, 207, 22, 0],
        [0, 0, 78, 154, 0, 0],
        [0, 0, 193, 39, 0, 0],
        [0, 55, 177, 0, 0, 0],
        [0, 173, 62, 0, 0, 0],
        [36, 199, 0, 0, 0, 0],
    ],
    [
        // 8
        [67, 198, 191, 195, 58, 0],
        [224, 33, 0, 35, 214, 0],
        [205, 52, 0, 54, 225, 0],
        [54, 249, 211, 249, 76, 0],
        [193, 83, 0, 85, 158, 0],
        [238, 3, 0, 3, 236, 0],
        [199, 61, 0, 60, 204, 0],
        [47, 193, 193, 192, 49, 0],
    ],
    [
        // 9
        [33, 185, 192, 180, 23, 0],
        [187, 67, 0, 67, 151, 0],
        [235, 3, 0, 3, 217, 0],
        [189, 81, 0, 84, 240, 0],
        [33, 175, 191, 115, 234, 0],
        [122, 7, 0, 37, 196, 0],
        [174, 80, 0, 133, 107, 0],
        [36, 196, 199, 147, 3, 0],
    ],
];

/// Draw decimal digits with Pillow-compatible coverage and six-pixel advance.
pub fn draw_digits(image: &mut image::RgbImage, x: i64, y: i64, text: &str, color: [u8; 3]) {
    for (i, ch) in text.chars().enumerate() {
        let Some(d) = ch.to_digit(10) else { continue };
        for (row, pixels) in DIGIT_COVERAGE[d as usize].iter().enumerate() {
            for (col, &coverage) in pixels.iter().enumerate() {
                if coverage == 0 {
                    continue;
                }
                let (px, py) = (x + i as i64 * 6 + col as i64, y + 2 + row as i64);
                if px < 0 || py < 0 || px >= image.width() as i64 || py >= image.height() as i64 {
                    continue;
                }
                let pixel = image.get_pixel_mut(px as u32, py as u32);
                for (channel, &ink) in color.iter().enumerate() {
                    pixel.0[channel] = ((pixel.0[channel] as u32 * (255 - coverage as u32)
                        + ink as u32 * coverage as u32
                        + 127)
                        / 255) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompress_all_commands() {
        // literal 2, 64x00, 64xFF, repeat 0xAA x3 (0x82), 3x0x05 (0xC5), terminator
        let rom = [2, 1, 2, 0x80, 0x81, 0x82, 0xAA, 0xC5, 0];
        let want = 2 + 64 + 64 + 3 + 3;
        let (out, end) = decompress(&rom, 0, want).unwrap();
        assert_eq!(end, rom.len());
        assert_eq!(&out[..2], &[1, 2]);
        assert!(out[2..66].iter().all(|&b| b == 0));
        assert!(out[66..130].iter().all(|&b| b == 0xFF));
        assert_eq!(&out[130..133], &[0xAA; 3]);
        assert_eq!(&out[133..], &[5; 3]);
    }

    #[test]
    fn decompress_rejects_bad_streams() {
        assert!(
            decompress(&[0], 0, 4)
                .unwrap_err()
                .to_string()
                .contains("Early end")
        );
        assert!(
            decompress(&[1, 9, 1], 0, 1)
                .unwrap_err()
                .to_string()
                .contains("terminator")
        );
        assert_eq!(
            decompress(&[1, 9], 0, 1).unwrap_err().to_string(),
            "index out of range"
        );
    }

    #[test]
    fn colour_scaling() {
        assert_eq!(rgb(0x7FFF), [255, 255, 255, 255]);
        assert_eq!(rgb(0x0010), [131, 0, 0, 255]);
        assert_eq!(obj_rgba(0x7FFF, 0), [0; 4]);
    }
}
