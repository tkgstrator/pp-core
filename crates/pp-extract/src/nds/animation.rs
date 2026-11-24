use super::{graphics::section, le_half, le_word};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

#[derive(Serialize)]
struct Bank {
    sequences: Vec<Sequence>,
}

#[derive(Serialize)]
struct Sequence {
    id: usize,
    start_frame: usize,
    element_type: usize,
    target_type: usize,
    playback_mode: usize,
    frames: Vec<Frame>,
}

#[derive(Serialize)]
struct Frame {
    index: usize,
    target_index: usize,
    duration: usize,
    data_offset: usize,
    transform: Option<Transform>,
}

#[derive(Serialize)]
struct Transform {
    #[serde(skip_serializing_if = "Option::is_none")]
    rotation_turns_u16: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale_x_fx32: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale_y_fx32: Option<i32>,
    translate_x: i16,
    translate_y: i16,
}

fn parse(file: &[u8]) -> Result<Bank> {
    let bank = section(file, b"KNBA")?;
    let header = bank.get(..24).context("truncated NANR animation bank")?;
    let sequences = le_half(header);
    let frames = le_half(&header[2..]);
    let sequence_start = le_word(&header[4..]);
    let frame_start = le_word(&header[8..]);
    let data_start = le_word(&header[12..]);
    let sequence_end = sequence_start
        .checked_add(sequences * 16)
        .context("NANR sequence size overflow")?;
    let frame_end = frame_start
        .checked_add(frames * 8)
        .context("NANR frame size overflow")?;
    ensure!(
        sequence_start >= header.len()
            && sequence_end <= frame_start
            && frame_end <= data_start
            && data_start <= bank.len(),
        "invalid NANR table ranges"
    );
    let frame_data = &bank[frame_start..frame_end];
    let transforms = &bank[data_start..];
    let mut budget = 65536;
    let mut result = Vec::with_capacity(sequences);
    for (id, entry) in bank[sequence_start..sequence_end]
        .chunks_exact(16)
        .enumerate()
    {
        let count = le_half(entry);
        ensure!(count <= budget, "NANR frame export limit exceeded");
        budget -= count;
        let frame_offset = le_word(&entry[12..]);
        ensure!(
            frame_offset.is_multiple_of(8),
            "unaligned NANR frame offset"
        );
        let entries = frame_data
            .get(frame_offset..)
            .and_then(|rest| rest.get(..count * 8))
            .context("NANR sequence outside frame table")?;
        let element = le_half(&entry[4..]);
        let target = le_half(&entry[6..]);
        let length = match element {
            0 => 2,
            1 => 16,
            2 => 8,
            _ => anyhow::bail!("unsupported NANR element type {element}"),
        };
        let mut timeline = Vec::with_capacity(count);
        for (index, frame) in entries.chunks_exact(8).enumerate() {
            let offset = le_word(frame);
            let data = transforms
                .get(offset..)
                .and_then(|rest| rest.get(..length))
                .context("NANR transform outside animation data")?;
            let signed = |offset| le_half(&data[offset..]) as i16;
            let transform = match element {
                1 => Some(Transform {
                    rotation_turns_u16: Some(le_half(&data[2..])),
                    scale_x_fx32: Some(le_word(&data[4..]) as i32),
                    scale_y_fx32: Some(le_word(&data[8..]) as i32),
                    translate_x: signed(12),
                    translate_y: signed(14),
                }),
                2 => Some(Transform {
                    rotation_turns_u16: None,
                    scale_x_fx32: None,
                    scale_y_fx32: None,
                    translate_x: signed(4),
                    translate_y: signed(6),
                }),
                _ => None,
            };
            timeline.push(Frame {
                index,
                target_index: le_half(data),
                duration: le_half(&frame[4..]),
                data_offset: offset,
                transform,
            });
        }
        result.push(Sequence {
            id,
            start_frame: le_half(&entry[2..]),
            element_type: element,
            target_type: target,
            playback_mode: le_word(&entry[8..]),
            frames: timeline,
        });
    }
    Ok(Bank { sequences: result })
}

pub(super) fn export(file: &[u8], root: &Path, id: usize) -> Result<()> {
    let index = parse(file)?;
    let out = root.join("animations");
    std::fs::create_dir_all(&out)?;
    let writer =
        std::io::BufWriter::new(std::fs::File::create(out.join(format!("{id:04x}.json")))?);
    serde_json::to_writer_pretty(writer, &index)?;
    Ok(())
}

fn transform_image(
    image: image::RgbaImage,
    origin: (i16, i16),
    transform: Option<&Transform>,
    budget: usize,
) -> Result<(image::RgbaImage, i32, i32)> {
    let (ox, oy) = (f64::from(origin.0), f64::from(origin.1));
    let Some(t) = transform else {
        return Ok((image, origin.0.into(), origin.1.into()));
    };
    let tx = f64::from(t.translate_x);
    let ty = f64::from(t.translate_y);
    let sx = f64::from(t.scale_x_fx32.unwrap_or(4096)) / 4096.0;
    let sy = f64::from(t.scale_y_fx32.unwrap_or(4096)) / 4096.0;
    let angle = t.rotation_turns_u16.unwrap_or(0);
    if angle == 0 && sx == 1.0 && sy == 1.0 {
        return Ok((image, (ox + tx) as i32, (oy + ty) as i32));
    }
    ensure!(sx != 0.0 && sy != 0.0, "NANR frame has zero scale");
    let (sin, cos) = match angle {
        0 => (0.0, 1.0),
        0x4000 => (1.0, 0.0),
        0x8000 => (0.0, -1.0),
        0xc000 => (-1.0, 0.0),
        _ => (angle as f64 * std::f64::consts::TAU / 65536.0).sin_cos(),
    };
    let (w, h) = (f64::from(image.width()), f64::from(image.height()));
    let (mut left, mut top, mut right, mut bottom) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (x, y) in [(ox, oy), (ox + w, oy), (ox, oy + h), (ox + w, oy + h)] {
        let px = cos * sx * x - sin * sy * y + tx;
        let py = sin * sx * x + cos * sy * y + ty;
        left = left.min(px);
        top = top.min(py);
        right = right.max(px);
        bottom = bottom.max(py);
    }
    let (left, top) = (left.floor(), top.floor());
    let (width, height) = (
        (right.ceil() - left) as usize,
        (bottom.ceil() - top) as usize,
    );
    let pixels = width
        .checked_mul(height)
        .context("NANR image size overflow")?;
    ensure!(
        width <= 4096 && height <= 4096 && pixels <= budget,
        "NANR image exceeds preview limit"
    );
    let rendered = image::RgbaImage::from_fn(width as u32, height as u32, |x, y| {
        let dx = f64::from(x) + left + 0.5 - tx;
        let dy = f64::from(y) + top + 0.5 - ty;
        let px = (cos * dx + sin * dy) / sx - ox;
        let py = (-sin * dx + cos * dy) / sy - oy;
        if px >= 0.0 && py >= 0.0 && px < w && py < h {
            *image.get_pixel(px as u32, py as u32)
        } else {
            image::Rgba([0; 4])
        }
    });
    Ok((rendered, left as i32, top as i32))
}

pub(super) fn render(
    file: &[u8],
    sprites: &Path,
    origins: &BTreeMap<usize, (i16, i16)>,
    out: &Path,
) -> Result<()> {
    let bank = parse(file)?;
    let mut index = String::from("sequence\tframe\tcell\tduration\torigin_x\torigin_y\tpath\n");
    std::fs::create_dir_all(out)?;
    let mut budget = 32 * 1024 * 1024;
    for sequence in bank
        .sequences
        .into_iter()
        .filter(|seq| seq.target_type == 1)
    {
        for frame in sequence.frames {
            let Some(&origin) = origins.get(&frame.target_index) else {
                continue;
            };
            let source = sprites.join(format!("{:04x}.png", frame.target_index));
            let image = image::open(source)?.into_rgba8();
            match transform_image(image, origin, frame.transform.as_ref(), budget) {
                Ok((image, x, y)) => {
                    let pixels = (image.width() * image.height()) as usize;
                    ensure!(pixels <= budget, "NANR animation exceeds preview limit");
                    budget -= pixels;
                    let name = format!("{:04x}/{:04x}.png", sequence.id, frame.index);
                    crate::gba::save_png(&image, &out.join(&name))?;
                    writeln!(
                        index,
                        "{}\t{}\t{}\t{}\t{x}\t{y}\t{name}",
                        sequence.id, frame.index, frame.target_index, frame.duration
                    )?;
                }
                Err(error) => eprintln!(
                    "NANR sequence {} frame {}: {error:#}",
                    sequence.id, frame.index
                ),
            }
        }
    }
    std::fs::write(out.join("frames.tsv"), index)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_rotation_scale_and_translation_to_cell_pixels() {
        let mut source = image::RgbaImage::new(2, 1);
        source.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        source.put_pixel(1, 0, image::Rgba([0, 0, 255, 255]));
        let transform = Transform {
            rotation_turns_u16: Some(0x4000),
            scale_x_fx32: Some(8192),
            scale_y_fx32: Some(4096),
            translate_x: 5,
            translate_y: 9,
        };
        let (frame, x, y) = transform_image(source, (-1, 0), Some(&transform), 16).unwrap();
        assert_eq!((x, y, frame.width(), frame.height()), (4, 7, 1, 4));
        assert_eq!(frame.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(frame.get_pixel(0, 3).0, [0, 0, 255, 255]);
    }

    #[test]
    fn preserves_cell_index_duration_and_signed_transforms() {
        let mut body = vec![0; 64];
        body[0] = 1;
        body[2] = 1;
        body[4] = 24;
        body[8] = 40;
        body[12] = 48;
        body[24] = 1;
        body[28] = 1;
        body[30] = 1;
        body[32] = 2;
        body[44] = 7;
        body[48] = 3;
        body[51] = 0x40;
        body[52..56].copy_from_slice(&4096i32.to_le_bytes());
        body[56..60].copy_from_slice(&(-4096i32).to_le_bytes());
        body[60..62].copy_from_slice(&(-5i16).to_le_bytes());
        let mut file = b"RNAN\xff\xfe\0\x01\0\0\0\0\x10\0\x01\0KNBA".to_vec();
        file[8..12].copy_from_slice(&(24 + body.len() as u32).to_le_bytes());
        file.extend((8 + body.len() as u32).to_le_bytes());
        file.extend(body);
        let index = serde_json::to_value(parse(&file).unwrap()).unwrap();
        let sequence = &index["sequences"][0];
        assert_eq!(sequence["playback_mode"], 2);
        assert_eq!(sequence["target_type"], 1);
        let frame = &sequence["frames"][0];
        assert_eq!(frame["target_index"], 3);
        assert_eq!(frame["duration"], 7);
        assert_eq!(frame["transform"]["rotation_turns_u16"], 0x4000);
        assert_eq!(frame["transform"]["scale_y_fx32"], -4096);
        assert_eq!(frame["transform"]["translate_x"], -5);
        file[64] = 100;
        assert!(parse(&file).is_err());
    }
}
