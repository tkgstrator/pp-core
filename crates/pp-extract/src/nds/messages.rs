use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::path::Path;

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").unwrap();
    }
    output
}

#[derive(Serialize)]
#[serde(untagged)]
enum Part {
    Text { text: String },
    Control { control_hex: String },
}

#[derive(Serialize)]
struct Message {
    id: usize,
    offset: usize,
    attributes_hex: String,
    parts: Vec<Part>,
}

struct Format {
    little: bool,
    encoding: u8,
}

impl Format {
    fn word(&self, data: &[u8]) -> usize {
        let bytes = data[..4].try_into().unwrap();
        (if self.little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }) as usize
    }

    fn half(&self, data: &[u8]) -> u16 {
        let bytes = [data[0], data[1]];
        if self.little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    }

    fn text(&self, data: &[u8]) -> String {
        match self.encoding {
            1 => encoding_rs::WINDOWS_1252
                .decode_without_bom_handling(data)
                .0
                .into_owned(),
            2 => String::from_utf16_lossy(
                &data
                    .chunks_exact(2)
                    .map(|pair| self.half(pair))
                    .collect::<Vec<_>>(),
            ),
            3 => encoding_rs::SHIFT_JIS
                .decode_without_bom_handling(data)
                .0
                .into_owned(),
            _ => String::from_utf8_lossy(data).into_owned(),
        }
    }

    fn parts(&self, mut data: &[u8], budget: &mut usize) -> Result<Vec<Part>> {
        let unit = if self.encoding == 2 { 2 } else { 1 };
        let mut parts = Vec::new();
        loop {
            let mut at = 0;
            let marker = loop {
                let bytes = data
                    .get(at..at + unit)
                    .context("unterminated BMG message")?;
                ensure!(*budget >= unit, "BMG message byte limit exceeded");
                *budget -= unit;
                let value = if unit == 2 {
                    self.half(bytes)
                } else {
                    u16::from(bytes[0])
                };
                if matches!(value, 0 | 0x1a) {
                    break value;
                }
                at += unit;
            };
            if at != 0 {
                parts.push(Part::Text {
                    text: self.text(&data[..at]),
                });
            }
            if marker == 0 {
                return Ok(parts);
            }
            data = &data[at..];
            let length = usize::from(*data.get(unit).context("truncated BMG control length")?);
            ensure!(
                length >= unit + 2 && length <= *budget,
                "invalid BMG control length"
            );
            *budget -= length;
            let control = data.get(..length).context("truncated BMG control")?;
            parts.push(Part::Control {
                control_hex: hex(control),
            });
            data = &data[length..];
        }
    }
}

struct FlowSections<'a> {
    format: Format,
    flow: Option<&'a [u8]>,
    scripts: Option<&'a [u8]>,
}

impl FlowSections<'_> {
    fn index(&self) -> Result<Option<serde_json::Value>> {
        use serde_json::json;
        if self.flow.is_none() && self.scripts.is_none() {
            return Ok(None);
        }
        let mut instructions = Vec::new();
        let mut labels = Vec::new();
        let mut scripts = Vec::new();
        if let Some(flow) = self.flow {
            let header = flow.get(..8).context("truncated BMG FLW1 header")?;
            let count = usize::from(self.format.half(header));
            let label_count = usize::from(self.format.half(&header[2..]));
            let instruction_end = 8 + count * 8;
            let label_end = instruction_end + label_count * 2;
            let payload = flow
                .get(..label_end + label_count)
                .context("truncated BMG FLW1 tables")?;
            for (index, bytes) in payload[8..instruction_end].chunks_exact(8).enumerate() {
                instructions.push(json!({"index": index, "bytes_hex": hex(bytes)}));
            }
            for (slot, (index, &bmg)) in payload[instruction_end..label_end]
                .chunks_exact(2)
                .zip(&payload[label_end..])
                .enumerate()
            {
                labels.push(json!({
                    "slot": slot,
                    "instruction_index": self.format.half(index) as i16,
                    "bmg_id": bmg as i8,
                }));
            }
        }
        if let Some(table) = self.scripts {
            let header = table.get(..8).context("truncated BMG FLI1 header")?;
            let count = usize::from(self.format.half(header));
            let stride = usize::from(self.format.half(&header[2..]));
            ensure!(
                stride >= 8 && count <= (table.len() - 8) / stride,
                "invalid BMG script table"
            );
            for entry in table[8..8 + count * stride].chunks_exact(stride) {
                scripts.push(json!({
                    "script_id": self.format.word(entry),
                    "instruction_index": self.format.half(&entry[4..]),
                    "attributes_hex": hex(&entry[6..]),
                }));
            }
        }
        Ok(Some(
            json!({"instructions": instructions, "labels": labels, "scripts": scripts}),
        ))
    }
}

fn parse(file: &[u8]) -> Result<(Vec<Message>, FlowSections<'_>)> {
    let header = file.get(..32).context("truncated BMG header")?;
    ensure!(&header[..8] == b"MESGbmg1", "invalid BMG magic");
    let size_le = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let size_be = u32::from_be_bytes(header[8..12].try_into().unwrap()) as usize;
    let valid = |size| (32..=file.len()).contains(&size);
    ensure!(valid(size_le) || valid(size_be), "invalid BMG file size");
    let little = valid(size_le) && (!valid(size_be) || size_le <= size_be);
    let format = Format {
        little,
        encoding: header[16],
    };
    ensure!(
        (1..=4).contains(&format.encoding),
        "unsupported BMG text encoding"
    );
    let mut rest = &file[32..if little { size_le } else { size_be }];
    let count = format.word(&header[12..]);
    ensure!(count <= rest.len() / 8, "invalid BMG section count");
    let mut entries = None;
    let mut messages = None;
    let (mut flow, mut scripts) = (None, None);
    for _ in 0..count {
        let header = rest.get(..8).context("truncated BMG section")?;
        let size = format.word(&header[4..]);
        let section = rest.get(8..size).context("invalid BMG section size")?;
        match &header[..4] {
            b"INF1" => {
                ensure!(entries.is_none(), "duplicate BMG INF1 section");
                entries = Some(section);
            }
            b"DAT1" => {
                ensure!(messages.is_none(), "duplicate BMG DAT1 section");
                messages = Some(section);
            }
            b"FLW1" => flow = Some(section),
            b"FLI1" => scripts = Some(section),
            _ => {}
        }
        rest = &rest[size..];
    }
    let info = entries.context("missing BMG INF1 section")?;
    let data = messages.context("missing BMG DAT1 section")?;
    let header = info.get(..8).context("truncated BMG INF1 header")?;
    let count = usize::from(format.half(header));
    let stride = usize::from(format.half(&header[2..]));
    ensure!(
        stride >= 4 && count <= (info.len() - 8) / stride,
        "invalid BMG record table"
    );
    let mut budget = 32 * 1024 * 1024;
    let messages = info[8..8 + count * stride]
        .chunks_exact(stride)
        .enumerate()
        .map(|(id, entry)| {
            let offset = format.word(entry);
            let parts = format.parts(
                data.get(offset..)
                    .context("BMG message offset outside DAT1")?,
                &mut budget,
            )?;
            Ok(Message {
                id,
                offset,
                attributes_hex: hex(&entry[4..]),
                parts,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((
        messages,
        FlowSections {
            format,
            flow,
            scripts,
        },
    ))
}

pub(super) fn export(file: &[u8], root: &Path, id: usize) -> Result<()> {
    let (messages, flow) = parse(file)?;
    let out = root.join("messages");
    std::fs::create_dir_all(&out)?;
    let output =
        std::io::BufWriter::new(std::fs::File::create(out.join(format!("{id:04x}.json")))?);
    serde_json::to_writer_pretty(output, &messages)?;
    match flow.index() {
        Ok(Some(index)) => {
            let output = std::io::BufWriter::new(std::fs::File::create(
                out.join(format!("{id:04x}.flow.json")),
            )?);
            serde_json::to_writer_pretty(output, &index)?;
        }
        Ok(None) => {}
        Err(error) => eprintln!("BMG {id:04x}: skipping conversation flow: {error:#}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_japanese_text_and_control_bytes() {
        let mut data = vec![0; 32];
        data[..8].copy_from_slice(b"MESGbmg1");
        data[12] = 2;
        data[16] = 2;
        data.extend(b"INF1");
        data.extend(24u32.to_le_bytes());
        data.extend([1, 0, 8, 0, 0, 0, 0, 0, 2, 0, 0, 0, 1, 2, 3, 4]);
        let mut text = vec![0, 0];
        text.extend("野球".encode_utf16().flat_map(u16::to_le_bytes));
        text.extend([0x1a, 0, 6, 0x34, 0, 0]);
        text.extend("部".encode_utf16().flat_map(u16::to_le_bytes));
        text.extend([0, 0]);
        data.extend(b"DAT1");
        data.extend((8 + text.len() as u32).to_le_bytes());
        data.extend(text);
        let size = data.len() as u32;
        data[8..12].copy_from_slice(&size.to_le_bytes());

        let value = serde_json::to_value(parse(&data).unwrap().0).unwrap();
        assert_eq!(value[0]["attributes_hex"], "01020304");
        assert_eq!(value[0]["parts"][0]["text"], "野球");
        assert_eq!(value[0]["parts"][1]["control_hex"], "1a0006340000");
        assert_eq!(value[0]["parts"][2]["text"], "部");
        let length_byte = 32 + 24 + 8 + 2 + 4 + 2;
        data[length_byte] = 0;
        assert!(parse(&data).is_err());
    }

    #[test]
    fn preserves_flow_slots_and_script_entries() {
        let format = Format {
            little: true,
            encoding: 2,
        };
        let flow = [
            2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 1, 0, 0xff,
        ];
        let scripts = [
            1, 0, 8, 0, 0, 0, 0, 0, 0x78, 0x56, 0x34, 0x12, 1, 0, 0xaa, 0xbb,
        ];
        let sections = FlowSections {
            format,
            flow: Some(&flow),
            scripts: Some(&scripts),
        };
        let index = sections.index().unwrap().unwrap();
        assert_eq!(index["instructions"][0]["bytes_hex"], "0000000000000000");
        assert_eq!(index["instructions"][1]["index"], 1);
        assert_eq!(index["labels"][0]["instruction_index"], 1);
        assert_eq!(index["labels"][0]["bmg_id"], -1);
        assert_eq!(index["scripts"][0]["script_id"], 0x12345678);
        assert_eq!(index["scripts"][0]["attributes_hex"], "aabb");
        assert!(
            FlowSections {
                flow: Some(&flow[..26]),
                ..sections
            }
            .index()
            .is_err()
        );
    }

    #[test]
    fn decodes_shift_jis() {
        let format = Format {
            little: false,
            encoding: 3,
        };
        let parts = format
            .parts(&[0x83, 0x70, 0x83, 0x8f, 0], &mut 128)
            .unwrap();
        assert_eq!(serde_json::to_value(parts).unwrap()[0]["text"], "パワ");
    }
}
