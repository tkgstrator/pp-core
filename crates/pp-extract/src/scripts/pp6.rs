use super::semantic::{self, Command};
use super::{BASE, digest, glyph_table, hex, quote, word, write_file};
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

const FIRST: usize = 0x565070;
const END: usize = 0x6015d8;
const DISPATCH: usize = 0x55c724;
const SIZES: &[u8] = b"000000020011113333323**1111111110111400001011*01*******01*******01*******01******0222255001222222255555251111110";

struct Event {
    start: usize,
    end: usize,
    origins: Vec<usize>,
}
struct Row {
    at: usize,
    raw: Vec<u8>,
    kind: String,
    detail: String,
    text: Option<Vec<String>>,
}
type DecodeResult = (Vec<Row>, String, usize, BTreeMap<String, usize>);

fn discover(rom: &[u8]) -> Result<BTreeMap<usize, Event>> {
    let mut refs: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for address in (0..rom.len() - 3).step_by(4) {
        let target = word(rom, address).wrapping_sub(BASE);
        if (FIRST..END).contains(&target) && target.is_multiple_of(4) {
            refs.entry(target).or_default().push(address);
        }
    }
    let mut events = BTreeMap::new();
    for (desc, origins) in refs {
        let start = word(rom, desc).wrapping_sub(BASE);
        if !(FIRST..desc).contains(&start) || !start.is_multiple_of(4) || desc - start > 0x9000 {
            continue;
        }
        let mut end = desc;
        while end > desc - 3 && rom[end - 1] == 0 {
            end -= 1;
        }
        if rom[end - 1] == 0xff {
            events.insert(
                desc,
                Event {
                    start,
                    end,
                    origins,
                },
            );
        }
    }
    let mut prior_end = 0;
    for (desc, event) in &events {
        ensure!(event.start >= prior_end, "Overlapping event at {desc:#x}");
        prior_end = event.end;
    }
    Ok(events)
}

fn glyph(data: &[u8], pos: &mut usize, chars: &[Option<char>]) -> String {
    let start = *pos;
    let b = data[*pos];
    *pos += 1;
    if b == 0 {
        return "　".into();
    }
    let index = if b >= 0xe8 {
        if *pos >= data.len() {
            return format!("〈字形:{b:02X}〉");
        }
        let n = (b as usize - 0xe7) * 256 + data[*pos] as usize;
        *pos += 1;
        n
    } else {
        b as usize - 1
    };
    chars
        .get(index)
        .copied()
        .flatten()
        .map(|ch| ch.to_string())
        .unwrap_or_else(|| format!("〈字形:{}〉", hex(&data[start..*pos]).to_uppercase()))
}

fn caption(data: &[u8], chars: &[Option<char>]) -> String {
    let mut pos = 0;
    let mut out = String::new();
    while pos < data.len() {
        out.push_str(&glyph(data, &mut pos, chars));
    }
    out
}

fn forward(data: &[u8], pos: usize, marker: u8, start: usize) -> String {
    if let Some(found) = (pos..data.len().saturating_sub(2).min(pos + 4098))
        .find(|&i| data[i..i + 3] == [0xf8, marker, marker])
        && found <= pos + 4095
    {
        return format!("0x{:06x}", start + found + 3);
    }
    "未検出".into()
}

fn condition(op: u8) -> Option<(u8, &'static str)> {
    for first in [0x2d, 0x36, 0x3f, 0x48] {
        if op == first {
            return Some((first + 1, "bits_set"));
        }
        if (first + 3..=first + 8).contains(&op) {
            return Some((
                first + 1,
                ["==", "!=", ">", ">=", "<", "<="][(op - first - 3) as usize],
            ));
        }
    }
    None
}

fn reference(rom: &[u8], desc: usize, ref_end: usize, index: u8) -> String {
    let addr = desc + 4 + index as usize * 4;
    if addr + 4 <= ref_end {
        format!("ref[{index}]=0x{:08x}", word(rom, addr))
    } else {
        format!("ref[{index}]=未確認")
    }
}

fn decode(
    rom: &[u8],
    desc: usize,
    start: usize,
    end: usize,
    ref_end: usize,
    chars: &[Option<char>],
) -> Result<DecodeResult> {
    let data = &rom[start..end];
    let mut pos = 0;
    let mut rows = Vec::new();
    let mut text = String::new();
    let mut menus = 0;
    let mut unknown = BTreeMap::new();
    while pos < data.len() {
        let at = pos;
        let byte = data[pos];
        pos += 1;
        let mut kind: String;
        let mut detail = String::new();
        let mut script_text = None;
        if byte < 0xf8 {
            pos = at;
            while pos < data.len() && data[pos] < 0xf8 {
                text.push_str(&glyph(data, &mut pos, chars));
            }
            kind = "TEXT".into();
            detail = caption(&data[at..pos], chars);
            script_text = Some(vec![detail.clone()]);
        } else if matches!(byte, 0xfa | 0xfb | 0xff) {
            kind = match byte {
                0xfa => "LINE",
                0xfb => "PAGE",
                _ => "END_OR_RETURN",
            }
            .into();
            text.push_str(if byte == 0xfa { "\n" } else { "\n\n" });
        } else if matches!(byte, 0xfc..=0xfe) {
            ensure!(
                pos < data.len(),
                "Incomplete display command at {:#x}",
                start + at
            );
            let n = data[pos];
            pos += 1;
            kind = match byte {
                0xfc => "BG",
                0xfd => "PORTRAIT_LEFT",
                _ => "PORTRAIT_RIGHT",
            }
            .into();
            detail = format!("id=0x{n:02x}");
        } else {
            ensure!(
                pos < data.len(),
                "Incomplete extended command at {:#x}",
                start + at
            );
            let op = data[pos];
            pos += 1;
            kind = format!("OP_{byte:02X}_{op:02X}");
            if matches!(op, 0x15 | 0x16) {
                let count = data[pos];
                pos += 1;
                let mut options = Vec::new();
                for _ in 0..count.max(2) {
                    let begin = pos;
                    while pos < data.len() && data.get(pos..pos + 2) != Some(&[0xf8, 0xf8]) {
                        pos += if data[pos] > 0xe7 { 2 } else { 1 };
                    }
                    ensure!(
                        pos + 2 <= data.len(),
                        "Unterminated options in {desc:#x} / {:#x}",
                        start + at
                    );
                    options.push(caption(&data[begin..pos], chars));
                    pos += 2;
                }
                kind = "CHOICE".into();
                menus += 1;
                detail = format!("op={op:02x} count_byte={count} {}", options.join(" ／ "));
                script_text = Some(options.clone());
                text.push_str(&format!("\n【選択肢】{}\n", options.join(" ／ ")));
            } else if let Some((marker, comparison)) = condition(op) {
                let mode = data[pos];
                let size = match mode {
                    0 => 3,
                    1 => 4,
                    2 => 6,
                    _ => 1,
                };
                pos += size;
                kind = "CONDITIONAL".into();
                let mut condition = format!("mode={mode} unresolved");
                if size > 1 {
                    let constant = data[at + 4..pos]
                        .iter()
                        .fold(0usize, |a, b| (a << 8) | *b as usize);
                    let width = ["u8", "u16", "s32"][mode as usize];
                    condition = format!(
                        "{} {comparison} 0x{constant:x} ({width})",
                        reference(rom, desc, ref_end, data[at + 3])
                    );
                }
                detail = format!(
                    "op=0x{op:02x} {condition} false→{}",
                    forward(data, pos, marker, start)
                );
            } else if matches!(op, 0x2e | 0x37 | 0x40 | 0x49 | 0x20) {
                let marker = match op {
                    0x2e => 0x2f,
                    0x37 => 0x38,
                    0x40 => 0x41,
                    0x49 => 0x4a,
                    _ => 0x21,
                };
                detail = format!("op=0x{op:02x} →{}", forward(data, pos, marker, start));
                kind = "SKIP_TO_MARKER".into();
                if op != 0x20 && data.get(pos) == Some(&op) {
                    pos += 1;
                }
            } else if (0x18..0x20).contains(&op) || matches!(op, 0x21 | 0x2f | 0x38 | 0x41 | 0x4a) {
                pos += 1;
                kind = "MARKER".into();
                detail = format!("id=0x{op:02x}");
            } else {
                ensure!(
                    (op as usize) < SIZES.len(),
                    "Unhandled command 0x{op:02x} at {:#x}",
                    start + at
                );
                pos += (SIZES[op as usize] - b'0') as usize;
                let args = &data[at + 2..pos];
                let mut memory = None;
                for (first, width) in [(0x0e, 16), (0x5b, 8), (0x62, 32)] {
                    if (first..first + 7).contains(&op) {
                        memory = match op - first {
                            0 => Some((width, "set")),
                            1 => Some((width, "add")),
                            2 => Some((width, "subtract")),
                            4 => Some((width, "and")),
                            5 => Some((width, "copy")),
                            6 => Some((width, "or")),
                            _ => None,
                        };
                    }
                }
                if let Some((width, action)) = memory {
                    kind = format!("{}_U{width}", action.to_uppercase());
                    let operand = if action == "copy" {
                        reference(rom, desc, ref_end, args[1])
                    } else {
                        format!(
                            "0x{:x}",
                            args[1..].iter().fold(0usize, |a, b| (a << 8) | *b as usize)
                        )
                    };
                    detail = format!(
                        "{} operand={operand}",
                        reference(rom, desc, ref_end, args[0])
                    );
                } else if op == 0x22 {
                    kind = "CALL_EVENT".into();
                    detail = reference(rom, desc, ref_end, args[0]);
                } else if op == 0x17 {
                    kind = "INDEXED_BRANCH".into();
                    detail = format!(
                        "{} → marker 0x18 + runtime byte",
                        reference(rom, desc, ref_end, args[0])
                    );
                } else if op == 0x01 {
                    kind = "WAIT_PAGE".into();
                    text.push_str("\n\n");
                } else if matches!(op, 0x08 | 0x09) {
                    kind = "TEXT_COLOR".into();
                    detail = if op == 0x08 { "emphasis" } else { "restore" }.into();
                } else if op == 0x25 {
                    kind = "PLAYER_NAME".into();
                    text.push_str("〈主人公名〉");
                } else {
                    *unknown.entry(kind.clone()).or_insert(0) += 1;
                    detail = "semantic=unresolved".into();
                }
                if !args.is_empty() {
                    detail.push_str(&format!(
                        " operands={}",
                        args.iter()
                            .map(|x| format!("{x:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ));
                }
            }
            ensure!(pos <= data.len(), "Operand past end at {:#x}", start + at);
            let handler = word(rom, DISPATCH + 4 * op as usize) - BASE - 1;
            detail.push_str(&format!(" handler=0x{handler:06x}"));
        }
        rows.push(Row {
            at: start + at,
            raw: data[at..pos].to_vec(),
            kind,
            detail,
            text: script_text,
        });
    }
    let transcript = text
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    let mut transcript = transcript;
    while transcript.contains("\n\n\n") {
        transcript = transcript.replace("\n\n\n", "\n\n");
    }
    Ok((rows, transcript, menus, unknown))
}

fn semantic_commands(
    rom: &[u8],
    desc: usize,
    end: usize,
    rows: &[Row],
    scene_ids: &BTreeMap<usize, String>,
    state_ids: &BTreeMap<usize, String>,
) -> Vec<Command> {
    let labels = semantic::labels(rows.iter().map(|row| row.at));
    rows.iter()
        .map(|row| {
            let raw = &row.raw;
            match row.kind.as_str() {
                "TEXT" => Command::new("TEXT").text("text", row.text.as_ref().unwrap()[0].clone()),
                "LINE" => Command::new("LINE"),
                "PAGE" => Command::new("PAGE"),
                "END_OR_RETURN" => Command::new("END_OR_RETURN"),
                "BG" => Command::new("EFFECT")
                    .text("effect", "background")
                    .number("asset_id", raw[1] as usize),
                "PORTRAIT_LEFT" => Command::new("EFFECT")
                    .text("effect", "portrait")
                    .text("side", "left")
                    .number("asset_id", raw[1] as usize),
                "PORTRAIT_RIGHT" => Command::new("EFFECT")
                    .text("effect", "portrait")
                    .text("side", "right")
                    .number("asset_id", raw[1] as usize),
                "CHOICE" => {
                    Command::new("CHOICE").texts("options", row.text.clone().unwrap_or_default())
                }
                "CONDITIONAL" => semantic::condition(
                    rom,
                    raw,
                    row.at + raw.len(),
                    end,
                    &labels,
                    true,
                    &state_ids[&word(rom, desc + 4 + 4 * raw[3] as usize)],
                ),
                "SKIP_TO_MARKER" => {
                    let marker = match raw[1] {
                        0x2e => 0x2f,
                        0x37 => 0x38,
                        0x40 => 0x41,
                        0x49 => 0x4a,
                        _ => 0x21,
                    };
                    semantic::branch(rom, row.at + raw.len(), end, marker, &labels)
                }
                "MARKER" => Command::new("MARKER"),
                "CALL_EVENT" => {
                    let target = word(rom, desc + 4 + 4 * raw[2] as usize).wrapping_sub(BASE);
                    scene_ids
                        .get(&target)
                        .map(|id| Command::new("CALL").text("target", id))
                        .unwrap_or_else(semantic::unknown)
                }
                "INDEXED_BRANCH" => semantic::switch(
                    rom,
                    row.at + raw.len(),
                    end,
                    &state_ids[&word(rom, desc + 4 + 4 * raw[2] as usize)],
                    &labels,
                ),
                "WAIT_PAGE" => Command::new("WAIT").text("until", "page_ack"),
                "TEXT_COLOR" => Command::new("EFFECT").text("effect", "text_style").text(
                    "mode",
                    if raw[1] == 0x08 {
                        "emphasis"
                    } else {
                        "restore"
                    },
                ),
                "PLAYER_NAME" => Command::new("PLAYER_NAME"),
                kind if kind.ends_with("_U8")
                    || kind.ends_with("_U16")
                    || kind.ends_with("_U32") =>
                {
                    let (action, width) = kind.rsplit_once("_U").unwrap();
                    let mapped = match action {
                        "SET" => "SET",
                        "ADD" => "ADD",
                        "SUBTRACT" => "SUBTRACT",
                        "AND" => "BIT_AND",
                        "COPY" => "COPY",
                        "OR" => "BIT_OR",
                        _ => return semantic::unknown(),
                    };
                    let mut command = Command::new(mapped)
                        .text(
                            "variable",
                            &state_ids[&word(rom, desc + 4 + 4 * raw[2] as usize)],
                        )
                        .text("value_type", format!("u{width}"));
                    if mapped == "COPY" {
                        command = command.text(
                            "source",
                            &state_ids[&word(rom, desc + 4 + 4 * raw[3] as usize)],
                        );
                    } else {
                        command = command.number(
                            "value",
                            raw[3..].iter().fold(0usize, |a, b| (a << 8) | *b as usize),
                        );
                    }
                    command
                }
                _ => semantic::unknown(),
            }
        })
        .collect()
}

pub(super) fn extract(rom: &[u8], out: &Path, semantic_out: &Path, rom_sha256: &str) -> Result<()> {
    let events = discover(rom)?;
    let entries = super::native_entrypoints(
        rom,
        0x71e8,
        FIRST,
        &[
            (0x6058, 4),
            (0x3b7e8, 49),
            (0x3b816, 10),
            (0x3b8fc, 18),
            (0x3b98e, 17),
            (0x3c298, 20),
            (0x3c310, 4),
            (0x3c37c, 8),
            (0x414bc, 6),
            (0x4c4ea, 12),
            (0x4c674, 37),
            (0x4c69e, 10),
            (0x4c72a, 9),
            (0x4d542, 15),
            (0x4d566, 15),
        ],
    );
    let supplemental = entries
        .iter()
        .filter(|entry| !events.contains_key(&entry.descriptor))
        .map(|entry| (entry.descriptor, entry.span))
        .collect::<BTreeMap<_, _>>();

    let starts: Vec<_> = events.values().map(|e| e.start).collect();
    let chars = glyph_table(true);
    // This event was added after the original supplemental scenes.
    let appended_descriptor = 0x561910;
    let supplemental_order = supplemental
        .keys()
        .filter(|&&desc| desc != appended_descriptor)
        .chain(
            supplemental
                .get_key_value(&appended_descriptor)
                .map(|(desc, _)| desc),
        );
    let scene_ids = events
        .keys()
        .chain(supplemental_order.clone())
        .enumerate()
        .map(|(i, desc)| (*desc, semantic::scene_id("pp6", i + 1)))
        .collect::<BTreeMap<_, _>>();
    let mut state_words = BTreeSet::new();
    let mut supplemental_states = BTreeSet::new();
    let mut supplemental_rows = BTreeMap::new();
    for (desc, start, end, ref_end, is_supplemental) in events
        .iter()
        .map(|(&desc, event)| {
            let i = starts.partition_point(|&s| s < desc);
            (
                desc,
                event.start,
                event.end,
                starts.get(i).copied().unwrap_or(END),
                false,
            )
        })
        .chain(supplemental.iter().map(|(&desc, &(start, end))| {
            let ref_end = supplemental
                .range(desc + 1..)
                .next()
                .map_or(FIRST, |(_, &(start, _))| start);
            (desc, start, end, ref_end, true)
        }))
    {
        let (rows, _, _, _) = decode(rom, desc, start, end, ref_end, &chars)?;
        let states = if is_supplemental {
            &mut supplemental_states
        } else {
            &mut state_words
        };
        for row in &rows {
            let raw = &row.raw;
            let indexes: Vec<u8> = match row.kind.as_str() {
                "CONDITIONAL" => vec![raw[3]],
                "INDEXED_BRANCH" => vec![raw[2]],
                "COPY_U8" | "COPY_U16" | "COPY_U32" => vec![raw[2], raw[3]],
                kind if kind.ends_with("_U8")
                    || kind.ends_with("_U16")
                    || kind.ends_with("_U32") =>
                {
                    vec![raw[2]]
                }
                _ => vec![],
            };
            for index in indexes {
                states.insert(word(rom, desc + 4 + 4 * index as usize));
            }
        }
        if is_supplemental {
            supplemental_rows.insert(desc, rows);
        }
    }
    let state_ids = semantic::state_ids(
        "pp6",
        state_words
            .iter()
            .copied()
            .chain(supplemental_states.difference(&state_words).copied()),
    );
    let mut semantic_index = String::from("scene_id\tscene_file\tnative_scene\n");
    let mut schedule = String::from(
        "grid\tmajor_index\tmonth_index\tslot_index\trom_offset\tencoded_id\tevent_id\tdescriptor\n",
    );
    let mut scheduled: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut slots = 0;
    for (grid, base) in [0x6015d8, 0x601698].into_iter().enumerate() {
        for (i, entry) in rom[base..base + 192].iter().copied().enumerate() {
            if entry == 0 {
                continue;
            }
            let event_id = entry & 0x7f;
            let table = if entry & 0x80 != 0 {
                0x60181c
            } else {
                0x601758
            };
            let target = word(rom, table + 4 * (event_id as usize - 1)) - BASE;
            scheduled
                .entry(target)
                .or_default()
                .push(format!("{grid}:{i}"));
            writeln!(
                schedule,
                "{grid}\t{}\t{}\t{}\t{:06x}\t{entry:02x}\t{event_id}\t{target:06x}",
                i / 96,
                i / 8 % 12,
                i % 8,
                base + i
            )?;
            slots += 1;
        }
    }
    ensure!(
        scheduled.keys().all(|d| events.contains_key(d)),
        "Missing a scheduled event"
    );
    let mut manifest = String::from(
        "descriptor\tstart\tend_exclusive\tbytes\tsha256\tmenus\tgrid:index\tincoming_word_offsets\tfile\n",
    );
    let mut unresolved: BTreeMap<String, usize> = BTreeMap::new();
    let mut count_choices = 0;
    let mut raw_bytes = 0;
    for (desc, event) in &events {
        let i = starts.partition_point(|&s| s < *desc);
        let ref_end = starts.get(i).copied().unwrap_or(END);
        let (rows, plain, menus, unknown) =
            decode(rom, *desc, event.start, event.end, ref_end, &chars)?;
        let scene_id = &scene_ids[desc];
        let commands = semantic_commands(rom, *desc, event.end, &rows, &scene_ids, &state_ids);
        let scene_file = semantic::write_scene(semantic_out, "pp6", scene_id, &commands)?;
        writeln!(
            semantic_index,
            "{scene_id}\t{scene_file}\tnative/scenes/{:02x}/{desc:06x}.yaml",
            desc >> 16
        )?;
        let relative = format!("scenes/{:02x}/{desc:06x}.txt", desc >> 16);
        let mut listing = format!(
            "PP6  台本 0x{desc:06x}\n本文 0x{:06x}–0x{:06x} (終端の次)\n\n{plain}\n\n［命令と原バイト列］\nROM位置\tバイト列\t種別\t内容\n",
            event.start, event.end
        );
        for row in &rows {
            writeln!(
                listing,
                "{:06x}\t{}\t{}\t{}",
                row.at,
                hex(&row.raw),
                row.kind,
                row.detail
            )?;
        }
        write_file(&out.join(&relative), &listing)?;
        let body_hash = digest(&rom[event.start..event.end]);
        let mut yaml = format!(
            "---\nformat_version: 1\ngame: \"pp6\"\nrom_sha256: {sha}\ndescriptor_offset: {desc_q}\nbody:\n  start_offset: {start_q}\n  end_offset_exclusive: {end_q}\n  sha256: {body_q}\n  padding_hex: {padding}\nprovenance:\n  incoming_word_offsets:",
            sha = quote(rom_sha256),
            desc_q = quote(&format!("0x{desc:06x}")),
            start_q = quote(&format!("0x{:06x}", event.start)),
            end_q = quote(&format!("0x{:06x}", event.end)),
            body_q = quote(&body_hash),
            padding = quote(&hex(&rom[event.end..*desc]))
        );
        if event.origins.is_empty() {
            yaml.push_str(" []\n");
        } else {
            yaml.push('\n');
            for origin in &event.origins {
                writeln!(yaml, "    - {}", quote(&format!("0x{origin:06x}")))?;
            }
        }
        yaml.push_str("  fixed_grid_slots:");
        if let Some(slots) = scheduled.get(desc) {
            yaml.push('\n');
            for slot in slots {
                writeln!(yaml, "    - {}", quote(slot))?;
            }
        } else {
            yaml.push_str(" []\n");
        }
        yaml.push_str("  classification: \"referenced_descriptor\"\ninstructions:");
        if rows.is_empty() {
            yaml.push_str(" []\n");
        } else {
            yaml.push('\n');
        }
        for row in &rows {
            writeln!(
                yaml,
                "  -\n    offset: {}\n    bytes_hex: {}\n    kind: {}",
                quote(&format!("0x{:06x}", row.at)),
                quote(&hex(&row.raw)),
                quote(&row.kind)
            )?;
            if row.kind == "TEXT" {
                if let Some(t) = &row.text {
                    writeln!(yaml, "    text: {}", quote(&t[0]))?;
                }
            } else {
                if let Some(t) = &row.text {
                    yaml.push_str("    text:\n");
                    for option in t {
                        writeln!(yaml, "      - {}", quote(option))?;
                    }
                }
                let operands = if row.raw.first() == Some(&0xf8) || row.raw.first() == Some(&0xf9) {
                    &row.raw[2..]
                } else {
                    &row.raw[1..]
                };
                writeln!(yaml, "    args:\n      raw_hex: {}", quote(&hex(operands)))?;
                if row.raw.len() > 1 && matches!(row.raw[0], 0xf8 | 0xf9) {
                    writeln!(
                        yaml,
                        "      opcode: {}",
                        quote(&format!("0x{:02x}", row.raw[1]))
                    )?;
                }
                if !row.detail.is_empty() {
                    writeln!(yaml, "      detail: {}", quote(&row.detail))?;
                }
                if row.kind.starts_with("OP_") || row.detail.contains("semantic=unresolved") {
                    yaml.push_str("    semantics: \"unresolved\"\n");
                }
            }
        }
        write_file(&out.join(relative.replace(".txt", ".yaml")), &yaml)?;
        writeln!(
            manifest,
            "{desc:06x}\t{:06x}\t{:06x}\t{}\t{body_hash}\t{menus}\t{}\t{}\t{relative}",
            event.start,
            event.end,
            event.end - event.start,
            scheduled.get(desc).map(|s| s.join(",")).unwrap_or_default(),
            event
                .origins
                .iter()
                .map(|x| format!("{x:06x}"))
                .collect::<Vec<_>>()
                .join(",")
        )?;
        count_choices += menus;
        raw_bytes += event.end - event.start;
        for (key, count) in unknown {
            *unresolved.entry(key).or_insert(0) += count;
        }
    }
    let supplemental_dir = out.join("entrypoint-events");
    std::fs::create_dir_all(&supplemental_dir)?;
    let mut supplemental_index =
        String::from("descriptor\tstart\tend_exclusive\tsha256\tfile\tlisting\n");
    for &desc in supplemental_order {
        let (start, end) = supplemental[&desc];
        let rows = &supplemental_rows[&desc];
        let scene_id = &scene_ids[&desc];
        let commands = semantic_commands(rom, desc, end, rows, &scene_ids, &state_ids);
        let scene_file = semantic::write_scene(semantic_out, "pp6", scene_id, &commands)?;
        writeln!(
            semantic_index,
            "{scene_id}\t{scene_file}\tnative/entrypoint-events/{desc:06x}.tsv"
        )?;
        let mut listing = String::from("offset\tbytes\ttype\tinterpretation\n");
        for row in rows {
            writeln!(
                listing,
                "{:06x}\t{}\t{}\t{}",
                row.at,
                hex(&row.raw),
                row.kind,
                row.detail.replace(['\t', '\r', '\n'], " ")
            )?;
        }
        let relative = format!("entrypoint-events/{desc:06x}");
        let body = &rom[start..end];
        std::fs::write(out.join(format!("{relative}.bin")), body)?;
        write_file(&out.join(format!("{relative}.tsv")), &listing)?;
        writeln!(
            supplemental_index,
            "{desc:06x}\t{start:06x}\t{end:06x}\t{}\t{relative}.bin\t{relative}.tsv",
            digest(body)
        )?;
    }
    write_file(&supplemental_dir.join("index.tsv"), &supplemental_index)?;
    super::write_entrypoints(out, &entries, |descriptor| {
        events.contains_key(&descriptor) || supplemental.contains_key(&descriptor)
    })?;
    write_file(&out.join("manifest.tsv"), &manifest)?;
    write_file(&out.join("schedule.tsv"), &schedule)?;
    let mut unresolved_tsv = String::from("opcode\tcount\n");
    for (op, count) in &unresolved {
        writeln!(unresolved_tsv, "{op}\t{count}")?;
    }
    write_file(&out.join("unresolved.tsv"), &unresolved_tsv)?;
    let unique: BTreeSet<_> = scheduled.keys().collect();
    let summary = format!(
        "{{\n  \"rom_sha256\": {},\n  \"descriptors\": {},\n  \"script_bytes\": {},\n  \"choice_menus\": {},\n  \"nonzero_fixed_slots\": {},\n  \"unique_fixed_descriptors\": {},\n  \"unknown_opcode_occurrences\": {}\n}}\n",
        quote(rom_sha256),
        events.len(),
        raw_bytes,
        count_choices,
        slots,
        unique.len(),
        unresolved.values().sum::<usize>()
    );
    write_file(&out.join("summary.json"), &summary)?;
    write_file(&semantic_out.join("index.tsv"), &semantic_index)?;
    semantic::write_state_index(semantic_out, &state_ids)?;
    Ok(())
}
