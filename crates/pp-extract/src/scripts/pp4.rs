use super::semantic::{self, Command};
use super::{BASE, PP4_HASH, decode_pp4, digest, glyph_table, hex, quote, word, write_file};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;
use std::path::Path;

const ARGC: [i8; 112] = [
    0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 1, 1, 1, 1, 3, 3, 3, 3, 3, 2, 3, -1, -1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 0, 1, 1, 1, -2, 0, 0, 0, 0, 1, 0, 1, 1, -3, 0, 1, -3, -3, -3, -3, -3, -3, -3, 0, 1, -3, -3,
    -3, -3, -3, -3, -3, 0, 1, -3, -3, -3, -3, -3, -3, -3, 0, 1, -3, -3, -3, -3, -3, -3, 0, 2, 2, 2,
    2, 5, 5, 0, 0, 1, 2, 2, 2, 2, 2, 2, 2, 5, 5, 5, 5, 5, 2, 5, 1, 1, 0, 0, 0, 0, 0,
];

#[derive(Clone)]
struct Row {
    at: usize,
    raw: Vec<u8>,
    kind: String,
    caption: String,
}
#[derive(Clone)]
struct Parsed {
    rows: Vec<Row>,
    refs: Vec<(usize, usize, String)>,
    warnings: Vec<String>,
    choices: usize,
}

fn bounds(data: &[u8], desc: usize) -> Option<(usize, usize)> {
    if desc + 4 > data.len() {
        return None;
    }
    let start = word(data, desc).checked_sub(BASE)?;
    if !(start < desc && start % 4 == 0 && desc - start <= 0x4000) {
        return None;
    }
    let mut end = desc;
    while end > desc - 3 && data[end - 1] == 0 {
        end -= 1;
    }
    (data[end - 1] == 0xff).then_some((start, end))
}

fn find(data: &[u8], pattern: &[u8], begin: usize, end: usize) -> Option<usize> {
    if pattern.is_empty() || end < pattern.len() {
        return None;
    }
    (begin..=end - pattern.len()).find(|&i| &data[i..i + pattern.len()] == pattern)
}

fn roots(data: &[u8]) -> Result<BTreeMap<usize, BTreeSet<String>>> {
    let mut result: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    let mut add = |label: String, descriptor: usize| -> Result<()> {
        ensure!(
            bounds(data, descriptor).is_some(),
            "Invalid root {label}: {descriptor:#x}"
        );
        result.entry(descriptor).or_default().insert(label);
        Ok(())
    };
    let tables = [
        ("school.fixed", 0x5acea4, 55),
        ("school.alternate_A", 0x5acf80, 8),
        ("school.alternate_B", 0x5acfa0, 8),
        ("school.random_pool", 0x5acfc0, (0x5ad164 - 0x5acfc0) / 4),
        ("school.special_random", 0x5ad154, 4),
        ("school.action_0A", 0x5190e4, 13),
        ("school.action_1E", 0x519118, 6),
        ("school.action_28", 0x519130, 4),
        ("school.action_2D", 0x519140, 7),
        ("school.outing_table", 0x5ad1c0, 11),
        ("fantasy.indexed_3D2E6", 0x5ad46c, 24),
        ("fantasy.fixed", 0x5ad2e8, 30),
        ("fantasy.random_pool", 0x5ad370, (0x5ad43c - 0x5ad370) / 4),
    ];
    for (label, offset, count) in tables {
        for i in 0..count {
            let addr = word(data, offset + 4 * i);
            if addr != 0 {
                add(format!("{label}[{i}]@{:06X}", offset + 4 * i), addr - BASE)?;
            }
        }
    }
    for (label, sites) in [
        (
            "school",
            &[
                0x28840, 0x288d4, 0x2899c, 0x28a2c, 0x28ac0, 0x2d8c0, 0x2acb8, 0x2d1a0, 0x2d516,
                0x2d716, 0x2d7e4, 0x2e9fc, 0x2ea14,
            ][..],
        ),
        (
            "fantasy",
            &[
                0x3aebc, 0x3b7d2, 0x3c918, 0x3cbac, 0x3cbb8, 0x3d108, 0x42ea2, 0x43028,
            ][..],
        ),
    ] {
        for &pc in sites {
            let instruction = u16::from_le_bytes(data[pc..pc + 2].try_into().unwrap()) as usize;
            let pool = ((pc + 4) & !3) + 4 * (instruction & 255);
            add(format!("{label}.native@{pc:06X}"), word(data, pool) - BASE)?;
        }
    }
    add("school.outing_named_person@56E5DC".into(), 0x56e794)?;
    add("school.yui_store@56E6D5".into(), 0x58709c)?;
    Ok(result)
}

fn name(op: u8) -> String {
    match op {
        0x08 => "text_style_on".into(),
        0x09 => "text_style_restore".into(),
        0x15 | 0x16 => "choice".into(),
        0x17 => "switch_u8".into(),
        0x20 => "leave_case".into(),
        0x21 => "end_switch".into(),
        0x22 => "call_event".into(),
        0x23 => "jump_event".into(),
        0x24 => "jump_indexed".into(),
        0x25 => "player_name".into(),
        0x2b => "call_native".into(),
        0x18..=0x1f => format!("case_{}", op - 0x18),
        _ => format!("op_{op:02X}"),
    }
}

fn parse(
    data: &[u8],
    desc: usize,
    start: usize,
    end: usize,
    chars: &[Option<char>],
) -> Result<Parsed> {
    let mut rows = Vec::new();
    let mut refs = Vec::new();
    let mut warnings = Vec::new();
    let mut choices = 0;
    let mut p = start;
    while p < end {
        let at = p;
        let mut first = data[p];
        p += 1;
        let mut note = String::new();
        let kind: String;
        if first <= 0xf7 {
            loop {
                if first >= 0xe8 {
                    p += 1;
                }
                if p >= end || data[p] > 0xf7 {
                    break;
                }
                first = data[p];
                p += 1;
            }
            kind = "text".into();
            note = decode_pp4(&data[at..p], chars);
        } else if first >= 0xfa {
            kind = match first {
                0xfa => "newline",
                0xfb => "page",
                0xfc => "scene",
                0xfd => "actor_left",
                0xfe => "actor_right",
                _ => "end",
            }
            .into();
            if matches!(first, 0xfc..=0xfe) {
                p += 1;
            }
        } else {
            if p >= end || data[p] as usize >= ARGC.len() {
                warnings.push(format!("unknown extended opcode at {at:06X}"));
                rows.push(Row {
                    at,
                    raw: data[at..end].to_vec(),
                    kind: "raw_tail".into(),
                    caption: String::new(),
                });
                break;
            }
            let op = data[p];
            p += 1;
            let width = ARGC[op as usize];
            let mut k = name(op);
            if width == -1 {
                ensure!(p < end, "Missing choice count");
                let count = data[p].max(2) as usize;
                p += 1;
                let mut labels = Vec::new();
                for _ in 0..count {
                    let label_start = p;
                    while p < end && data.get(p..p + 2) != Some(&[0xf8, 0xf8]) {
                        p += if (0xe8..=0xf7).contains(&data[p]) {
                            2
                        } else {
                            1
                        };
                    }
                    if p >= end {
                        break;
                    }
                    labels.push(decode_pp4(&data[label_start..p], chars));
                    p += 2;
                }
                if labels.len() != count {
                    warnings.push(format!("incomplete choice at {at:06X}"));
                    rows.push(Row {
                        at,
                        raw: data[at..end].to_vec(),
                        kind: "raw_tail".into(),
                        caption: String::new(),
                    });
                    break;
                }
                choices += 1;
                note = format!(
                    "[{}]",
                    labels
                        .iter()
                        .map(|s| quote(s))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            } else if width == -2 {
                p += 3;
            } else if matches!(op, 0x2e | 0x37 | 0x40 | 0x49) && p < end && data[p] == op {
                p += 1;
                k = "else_marker".into();
                if let Some(target) = find(data, &[0xf8, op + 1, op + 1], p, end) {
                    note = format!("skip else body -> 0x{:06X}", target + 3);
                }
            } else if width == -3 {
                if p >= end || data[p] > 2 {
                    warnings.push(format!("unsupported comparison width at {at:06X}"));
                    rows.push(Row {
                        at,
                        raw: data[at..end].to_vec(),
                        kind: "raw_tail".into(),
                        caption: String::new(),
                    });
                    break;
                }
                let typed = data[p] as usize;
                let size = [1, 2, 4][typed];
                let index = data[p + 1] as usize;
                let value = data[p + 2..p + 2 + size]
                    .iter()
                    .fold(0usize, |a, b| (a << 8) | *b as usize);
                let variable = word(data, desc + 4 + 4 * index);
                let group = [0x49, 0x40, 0x37, 0x2e]
                    .into_iter()
                    .find(|&g| op >= g - 1)
                    .unwrap();
                let relation_id = if op == group - 1 {
                    0
                } else {
                    (op - group - 1) as usize
                };
                let relation = ["&", "==", "!=", ">", ">=", "<", "<="][relation_id];
                let target = find(data, &[0xf8, group, group], p + size + 2, end);
                note = format!("u{} [0x{variable:08X}] {relation} 0x{value:X}", size * 8);
                if let Some(target) = target {
                    write!(note, "; if false -> 0x{:06X}", target + 3)?;
                }
                p += size + 2;
            } else {
                p += width as usize;
            }
            if matches!(op, 0x17 | 0x22 | 0x23 | 0x24 | 0x2b | 0x2c) && at + 2 < end {
                let arg = data[at + 2] as usize;
                let value = word(data, desc + 4 + 4 * arg);
                note = format!("arg[{arg}] = 0x{value:08X}");
                if op == 0x17
                    && let Some(close) = find(data, &[0xf8, 0x21, 0x21], p, end)
                {
                    let mut alternatives = Vec::new();
                    for number in 0..8 {
                        let marker = 0x18 + number;
                        if let Some(dest) = find(data, &[0xf8, marker, marker], p, close) {
                            alternatives.push(format!("{number}:0x{:06X}", dest + 3));
                        }
                    }
                    note.push_str(" -> ");
                    note.push_str(&alternatives.join(", "));
                }
                if matches!(op, 0x22 | 0x23) {
                    refs.push((at, value.wrapping_sub(BASE), k.clone()));
                }
            }
            if op == 0x24 && p < end {
                rows.push(Row {
                    at,
                    raw: data[at..p].to_vec(),
                    kind: k,
                    caption: note,
                });
                let table_end = end - 1;
                let selectors = &data[p..table_end];
                for idx in selectors.iter().copied().collect::<BTreeSet<_>>() {
                    let target = word(data, desc + 4 + idx as usize * 4).wrapping_sub(BASE);
                    if target < data.len() - 4 && bounds(data, target).is_some() {
                        refs.push((p, target, format!("indexed arg[{idx}]")));
                    }
                }
                if !selectors.is_empty() {
                    rows.push(Row {
                        at: p,
                        raw: selectors.to_vec(),
                        kind: "indexed_tail".into(),
                        caption: "inline bytes after terminal jump; see arg references".into(),
                    });
                }
                p = table_end;
                continue;
            }
            kind = k;
        }
        if p > end {
            warnings.push(format!("operand crosses boundary at {at:06X}"));
            rows.push(Row {
                at,
                raw: data[at..end].to_vec(),
                kind: "raw_tail".into(),
                caption: String::new(),
            });
            break;
        }
        if kind == "end" && p != end {
            note = "end of this execution path".into();
        }
        rows.push(Row {
            at,
            raw: data[at..p].to_vec(),
            kind,
            caption: note,
        });
    }
    ensure!(
        rows.iter()
            .flat_map(|r| r.raw.iter())
            .copied()
            .collect::<Vec<_>>()
            == data[start..end],
        "Byte gap at descriptor {desc:06X}"
    );
    Ok(Parsed {
        rows,
        refs,
        warnings,
        choices,
    })
}

fn semantic_commands(
    data: &[u8],
    desc: usize,
    end: usize,
    parsed: &Parsed,
    scene_ids: &BTreeMap<usize, String>,
    state_ids: &BTreeMap<usize, String>,
    chars: &[Option<char>],
) -> Vec<Command> {
    let labels = semantic::labels(parsed.rows.iter().map(|row| row.at));
    parsed
        .rows
        .iter()
        .map(|row| {
            let raw = &row.raw;
            match row.kind.as_str() {
                "text" => Command::new("TEXT").text("text", &row.caption),
                "newline" => Command::new("LINE"),
                "page" => Command::new("PAGE"),
                "scene" => Command::new("EFFECT")
                    .text("effect", "background")
                    .number("asset_id", raw[1] as usize),
                "actor_left" => Command::new("EFFECT")
                    .text("effect", "portrait")
                    .text("side", "left")
                    .number("asset_id", raw[1] as usize),
                "actor_right" => Command::new("EFFECT")
                    .text("effect", "portrait")
                    .text("side", "right")
                    .number("asset_id", raw[1] as usize),
                "end" => Command::new("END_OR_RETURN"),
                "text_style_on" => Command::new("EFFECT")
                    .text("effect", "text_style")
                    .text("mode", "on"),
                "text_style_restore" => Command::new("EFFECT")
                    .text("effect", "text_style")
                    .text("mode", "restore"),
                "player_name" => Command::new("PLAYER_NAME"),
                "choice" => {
                    if raw.len() < 3 {
                        return semantic::unknown();
                    }
                    let mut p = 3;
                    let mut options = Vec::new();
                    for _ in 0..raw[2].max(2) {
                        let begin = p;
                        while p + 1 < raw.len() && raw[p..p + 2] != [0xf8, 0xf8] {
                            p += if (0xe8..=0xf7).contains(&raw[p]) {
                                2
                            } else {
                                1
                            };
                        }
                        if p + 2 > raw.len() {
                            return semantic::unknown();
                        }
                        options.push(decode_pp4(&raw[begin..p], chars));
                        p += 2;
                    }
                    Command::new("CHOICE").texts("options", options)
                }
                "call_event" | "jump_event" => {
                    let target = word(data, desc + 4 + 4 * raw[2] as usize).wrapping_sub(BASE);
                    scene_ids
                        .get(&target)
                        .map(|id| {
                            Command::new(if row.kind == "call_event" {
                                "CALL"
                            } else {
                                "JUMP"
                            })
                            .text("target", id)
                        })
                        .unwrap_or_else(semantic::unknown)
                }
                "switch_u8" => semantic::switch(
                    data,
                    row.at + raw.len(),
                    end,
                    &state_ids[&word(data, desc + 4 + 4 * raw[2] as usize)],
                    &labels,
                ),
                "leave_case" => semantic::branch(data, row.at + raw.len(), end, 0x21, &labels),
                "end_switch" => Command::new("MARKER"),
                "else_marker" => {
                    semantic::branch(data, row.at + raw.len(), end, raw[1] + 1, &labels)
                }
                "indexed_tail" => {
                    Command::new("INLINE_DATA").text("purpose", "indexed_jump_selectors")
                }
                kind if kind.starts_with("case_") => Command::new("MARKER"),
                kind if kind.starts_with("op_")
                    && raw.len() >= 2
                    && ARGC[raw[1] as usize] == -3 =>
                {
                    semantic::condition(
                        data,
                        raw,
                        row.at + raw.len(),
                        end,
                        &labels,
                        false,
                        &state_ids[&word(data, desc + 4 + 4 * raw[3] as usize)],
                    )
                }
                kind if kind.starts_with("op_")
                    && raw.len() == 3
                    && raw[1] == raw[2]
                    && matches!(raw[1], 0x21 | 0x2f | 0x38 | 0x41 | 0x4a) =>
                {
                    Command::new("MARKER")
                }
                _ => semantic::unknown(),
            }
        })
        .collect()
}

pub(super) fn extract(data: &[u8], out: &Path, semantic_out: &Path) -> Result<()> {
    let root_map = roots(data)?;
    let chars = glyph_table(false);
    let mut candidates = BTreeMap::new();
    for desc in (0x50c000..0x5acd84).step_by(4) {
        if let Some(bound) = bounds(data, desc) {
            candidates.insert(desc, bound);
        }
    }
    let mut first_desc = BTreeMap::new();
    for (desc, (start, _)) in &candidates {
        first_desc.entry(*start).or_insert(*desc);
    }
    candidates.retain(|desc, (start, _)| first_desc[start] == *desc);
    let candidate_addrs = candidates.keys().copied().collect::<Vec<_>>();
    candidates.retain(|desc, (start, _)| {
        candidate_addrs.get(candidate_addrs.partition_point(|&x| x < *start)) == Some(desc)
    });
    for desc in root_map.keys() {
        candidates.insert(*desc, bounds(data, *desc).unwrap());
    }
    let mut results = BTreeMap::new();
    for (desc, (start, end)) in &candidates {
        if !root_map.contains_key(desc)
            && !matches!(data[*start], 0xf8 | 0xf9 | 0xfc | 0xfd | 0xfe)
            && !data[*start..*end].contains(&0xf8)
        {
            continue;
        }
        results.insert(*desc, parse(data, *desc, *start, *end, &chars)?);
    }
    let mut queue = results.keys().copied().collect::<VecDeque<_>>();
    while let Some(desc) = queue.pop_front() {
        for (_, target, _) in results[&desc].refs.clone() {
            if target < data.len() - 4
                && let Some(bound) = bounds(data, target)
                && let std::collections::btree_map::Entry::Vacant(entry) = results.entry(target)
            {
                entry.insert(parse(data, target, bound.0, bound.1, &chars)?);
                candidates.insert(target, bound);
                queue.push_back(target);
            }
        }
    }
    let mut route_sets: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    let mut visits = VecDeque::new();
    for (desc, sources) in &root_map {
        for source in sources {
            visits.push_back((*desc, source.split('.').next().unwrap().to_string()));
        }
    }
    while let Some((key, mut route)) = visits.pop_front() {
        if key == 0x5aa44c {
            route = "test_menu".into();
        }
        if !results.contains_key(&key) || !route_sets.entry(key).or_default().insert(route.clone())
        {
            continue;
        }
        for (_, target, _) in &results[&key].refs {
            if results.contains_key(target) {
                visits.push_back((*target, route.clone()));
            }
        }
    }
    let mut index = String::from(
        "descriptor\tbody\tend_exclusive\tprovenance\tbytes\tchoice_menus\tfirst_text\tlisting\n",
    );
    let mut semantic_index = String::from("scene_id\tscene_file\tnative_scene\n");
    let scene_ids = results
        .keys()
        .enumerate()
        .map(|(i, desc)| (*desc, semantic::scene_id("pp4", i + 1)))
        .collect::<BTreeMap<_, _>>();
    let state_words = results
        .iter()
        .flat_map(|(desc, parsed)| {
            parsed.rows.iter().filter_map(|row| {
                let raw = &row.raw;
                let index = if row.kind == "switch_u8" {
                    raw.get(2).copied()
                } else if row.kind.starts_with("op_")
                    && raw.len() >= 4
                    && ARGC[raw[1] as usize] == -3
                {
                    raw.get(3).copied()
                } else {
                    None
                }?;
                Some(word(data, *desc + 4 + 4 * index as usize))
            })
        })
        .collect::<BTreeSet<_>>();
    let state_ids = semantic::state_ids("pp4", state_words);
    let mut edge_rows = String::from("source_desc\topcode_offset\tkind\ttarget_desc\texported\n");
    let mut root_rows = String::from("source\tdescriptor\n");
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut count_order = Vec::<String>::new();
    let mut inc = |key: &str, add: usize| {
        if !totals.contains_key(key) {
            count_order.push(key.to_string());
        }
        *totals.entry(key.to_string()).or_insert(0) += add;
    };
    let mut issues: Vec<(usize, Vec<String>)> = Vec::new();
    let mut dig = Sha256::new();
    let mut edges = 0;
    for (desc, parsed) in &results {
        let (start, end) = candidates[desc];
        let routes = route_sets.get(desc).cloned().unwrap_or_default();
        let mut source = if routes.is_empty() {
            "layout_only".into()
        } else {
            routes.iter().cloned().collect::<Vec<_>>().join("+")
        };
        if source == "layout_only" && !parsed.warnings.is_empty() {
            source = "layout_partial".into();
        }
        let relative = format!("events/{:02x}/{desc:06x}.txt", desc >> 16);
        let scene_id = &scene_ids[desc];
        let commands = semantic_commands(data, *desc, end, parsed, &scene_ids, &state_ids, &chars);
        let scene_file = semantic::write_scene(semantic_out, "pp4", scene_id, &commands)?;
        writeln!(
            semantic_index,
            "{scene_id}\t{scene_file}\tnative/{}",
            relative.replace(".txt", ".yaml")
        )?;
        let mut listing = format!(
            "# AP4J descriptor=0x{desc:06X} body=0x{start:06X} end_exclusive=0x{end:06X}\n# provenance={source} padding={}\n# offset | original bytes | type | Japanese text / argument\n",
            hex(&data[end..*desc])
        );
        for row in &parsed.rows {
            writeln!(
                listing,
                "{:06X}\t{}\t{}\t{}",
                row.at,
                hex(&row.raw),
                row.kind,
                row.caption
            )?;
        }
        write_file(&out.join(&relative), &listing)?;
        let mut yaml = format!(
            "---\nformat_version: 1\ngame: \"pp4\"\nrom_sha256: {}\ndescriptor_offset: {}\nbody:\n  start_offset: {}\n  end_offset_exclusive: {}\n  sha256: {}\n  padding_hex: {}\nprovenance:\n  routes:",
            quote(PP4_HASH),
            quote(&format!("0x{desc:06x}")),
            quote(&format!("0x{start:06x}")),
            quote(&format!("0x{end:06x}")),
            quote(&digest(&data[start..end])),
            quote(&hex(&data[end..*desc]))
        );
        if routes.is_empty() {
            writeln!(yaml, "\n    - {}", quote(&source))?;
        } else {
            yaml.push('\n');
            for route in &routes {
                writeln!(yaml, "    - {}", quote(route))?;
            }
        }
        yaml.push_str("  roots:");
        if let Some(root_set) = root_map.get(desc) {
            yaml.push('\n');
            for root in root_set {
                writeln!(yaml, "    - {}", quote(root))?;
            }
        } else {
            yaml.push_str(" []\n");
        }
        writeln!(yaml, "  classification: {}\ninstructions:", quote(&source))?;
        for row in &parsed.rows {
            writeln!(
                yaml,
                "  -\n    offset: {}\n    bytes_hex: {}\n    kind: {}",
                quote(&format!("0x{:06x}", row.at)),
                quote(&hex(&row.raw)),
                quote(&row.kind)
            )?;
            if row.kind == "text" {
                writeln!(yaml, "    text: {}", quote(&row.caption))?;
            } else {
                let operand_bytes: &[u8] =
                    if matches!(row.kind.as_str(), "raw_tail" | "indexed_tail") {
                        &row.raw
                    } else if matches!(row.raw.first(), Some(0xf8 | 0xf9)) {
                        &row.raw[2..]
                    } else {
                        &row.raw[1..]
                    };
                writeln!(
                    yaml,
                    "    args:\n      raw_hex: {}",
                    quote(&hex(operand_bytes))
                )?;
                if !matches!(row.kind.as_str(), "raw_tail" | "indexed_tail")
                    && row.raw.len() > 1
                    && matches!(row.raw[0], 0xf8 | 0xf9)
                {
                    writeln!(
                        yaml,
                        "      opcode: {}",
                        quote(&format!("0x{:02x}", row.raw[1]))
                    )?;
                }
                if !row.caption.is_empty() {
                    writeln!(yaml, "      detail: {}", quote(&row.caption))?;
                }
                if row.kind == "choice" {
                    // The scalar content is JSON array syntax, valid YAML 1.2 flow syntax.
                    writeln!(
                        yaml,
                        "    text:\n{}",
                        row.caption
                            .trim_start_matches('[')
                            .trim_end_matches(']')
                            .split(", ")
                            .map(|s| format!("      - {s}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )?;
                }
                if row.kind.starts_with("op_") || row.kind == "raw_tail" {
                    yaml.push_str("    semantics: \"unresolved\"\n");
                }
            }
        }
        write_file(&out.join(relative.replace(".txt", ".yaml")), &yaml)?;
        dig.update((*desc as u32).to_le_bytes());
        dig.update((start as u32).to_le_bytes());
        dig.update((end as u32).to_le_bytes());
        dig.update(&data[start..end]);
        let preview = parsed
            .rows
            .iter()
            .find(|r| r.kind == "text" && !r.caption.trim().is_empty())
            .map(|r| r.caption.chars().take(44).collect::<String>())
            .unwrap_or_default();
        writeln!(
            index,
            "{desc:06X}\t{start:06X}\t{end:06X}\t{source}\t{}\t{}\t{preview}\t{relative}",
            end - start,
            parsed.choices
        )?;
        for (pc, target, kind) in &parsed.refs {
            writeln!(
                edge_rows,
                "{desc:06X}\t{pc:06X}\t{kind}\t{target:06X}\t{}",
                usize::from(results.contains_key(target))
            )?;
            edges += 1;
        }
        if let Some(root_set) = root_map.get(desc) {
            for root in root_set {
                writeln!(root_rows, "{root}\t{desc:06X}")?;
            }
        }
        inc(
            "unmapped_glyphs",
            parsed
                .rows
                .iter()
                .map(|r| r.caption.matches("〈glyph:").count())
                .sum(),
        );
        inc(&source, 1);
        inc("choice_records", parsed.choices);
        inc("source_bytes", end - start);
        if !parsed.warnings.is_empty() {
            issues.push((*desc, parsed.warnings.clone()));
        }
    }
    write_file(&out.join("index.tsv"), &index)?;
    write_file(&out.join("roots.tsv"), &root_rows)?;
    write_file(&out.join("edges.tsv"), &edge_rows)?;
    let mut coverage = String::new();
    for key in &count_order {
        writeln!(coverage, "    {}: {},", quote(key), totals[key])?;
    }
    writeln!(
        coverage,
        "    \"events\": {},\n    \"registered_roots\": {},\n    \"edges\": {edges},\n    \"partial_events\": {}",
        results.len(),
        root_map.len(),
        issues.len()
    )?;
    let issue_json = if issues.is_empty() {
        "[]".into()
    } else {
        format!(
            "[{}]",
            issues
                .iter()
                .map(|(desc, notes)| format!(
                    "{{\"descriptor\": {}, \"notes\": [{}]}}",
                    quote(&format!("{desc:06X}")),
                    notes
                        .iter()
                        .map(|s| quote(s))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let manifest = format!(
        "{{\n  \"rom_sha256\": {},\n  \"coverage\": {{\n{coverage}  }},\n  \"roundtrip_sha256\": {},\n  \"roundtrip_input\": \"sorted little-endian descriptor,body,end (u32), then body bytes\",\n  \"vm_dispatch\": \"0x511A34\",\n  \"partial_events\": {issue_json}\n}}\n",
        quote(PP4_HASH),
        quote(&hex(&dig.finalize()))
    );
    write_file(&out.join("manifest.json"), &manifest)?;
    write_file(&semantic_out.join("index.tsv"), &semantic_index)?;
    semantic::write_state_index(semantic_out, &state_ids)?;
    Ok(())
}
