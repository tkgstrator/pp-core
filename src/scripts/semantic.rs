//! Shared, ROM-independent dialogue scene representation.

use super::{quote, write_file};
use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

pub(super) enum Value {
    Text(String),
    Number(i64),
    Texts(Vec<String>),
    Cases(Vec<(usize, String)>),
}

pub(super) struct Command {
    pub kind: &'static str,
    pub fields: Vec<(&'static str, Value)>,
}

impl Command {
    pub fn new(kind: &'static str) -> Self {
        Self {
            kind,
            fields: Vec::new(),
        }
    }

    pub fn text(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.fields.push((name, Value::Text(value.into())));
        self
    }

    pub fn number(mut self, name: &'static str, value: usize) -> Self {
        self.fields.push((name, Value::Number(value as i64)));
        self
    }

    pub fn signed_number(mut self, name: &'static str, value: i64) -> Self {
        self.fields.push((name, Value::Number(value)));
        self
    }

    pub fn texts(mut self, name: &'static str, value: Vec<String>) -> Self {
        self.fields.push((name, Value::Texts(value)));
        self
    }

    pub fn cases(mut self, value: Vec<(usize, String)>) -> Self {
        self.fields.push(("cases", Value::Cases(value)));
        self
    }
}

pub(super) fn unknown() -> Command {
    Command::new("UNRESOLVED").text("reason", "unverified_semantics")
}

pub(super) fn scene_id(game: &str, ordinal: usize) -> String {
    format!("{game}_scene_{ordinal:06}")
}

pub(super) fn state_ids(
    game: &str,
    addresses: impl IntoIterator<Item = usize>,
) -> BTreeMap<usize, String> {
    addresses
        .into_iter()
        .enumerate()
        .map(|(i, address)| (address, format!("{game}_state_{:06}", i + 1)))
        .collect()
}

pub(super) fn write_state_index(out: &Path, ids: &BTreeMap<usize, String>) -> Result<()> {
    let mut tsv = String::from("state_id\tnative_address\n");
    for (address, id) in ids {
        writeln!(tsv, "{id}\t0x{address:08x}")?;
    }
    write_file(&out.join("state-links.tsv"), &tsv)
}

pub(super) fn label(ordinal: usize) -> String {
    format!("c{ordinal:06}")
}

pub(super) fn labels(offsets: impl IntoIterator<Item = usize>) -> BTreeMap<usize, String> {
    offsets
        .into_iter()
        .enumerate()
        .map(|(i, at)| (at, label(i + 1)))
        .collect()
}

pub(super) fn target_label(
    target: usize,
    end: usize,
    labels: &BTreeMap<usize, String>,
) -> Option<String> {
    if target == end {
        Some("END".into())
    } else {
        labels.get(&target).cloned()
    }
}

fn marker(data: &[u8], from: usize, end: usize, id: u8) -> Option<usize> {
    if from >= end || end > data.len() || end < 3 {
        return None;
    }
    (from..end - 2).find(|&at| data[at..at + 3] == [0xf8, id, id])
}

pub(super) fn branch(
    data: &[u8],
    from: usize,
    end: usize,
    marker_id: u8,
    labels: &BTreeMap<usize, String>,
) -> Command {
    marker(data, from, end, marker_id)
        .and_then(|at| target_label(at + 3, end, labels))
        .map(|target| Command::new("JUMP").text("target", target))
        .unwrap_or_else(unknown)
}

pub(super) fn condition(
    data: &[u8],
    raw: &[u8],
    from: usize,
    end: usize,
    labels: &BTreeMap<usize, String>,
    signed_word: bool,
    variable: &str,
) -> Command {
    if raw.len() < 5 {
        return unknown();
    }
    let op = raw[1];
    let mode = raw[2];
    if mode > 2 {
        return unknown();
    }
    let width = [1, 2, 4][mode as usize];
    if raw.len() < 4 + width {
        return unknown();
    }
    let group = [0x49, 0x40, 0x37, 0x2e].into_iter().find(|&g| op >= g - 1);
    let Some(group) = group else {
        return unknown();
    };
    let relation = if op == group - 1 {
        "bits_set"
    } else {
        ["==", "!=", ">", ">=", "<", "<="]
            .get((op - group - 2) as usize)
            .copied()
            .unwrap_or("unknown")
    };
    if relation == "unknown" {
        return unknown();
    }
    let Some(target) =
        marker(data, from, end, group).and_then(|at| target_label(at + 3, end, labels))
    else {
        return unknown();
    };
    let value = raw[4..4 + width]
        .iter()
        .fold(0usize, |a, b| (a << 8) | *b as usize);
    let signed_value = if signed_word && width == 4 {
        (value as u32 as i32) as i64
    } else {
        value as i64
    };
    let value_type = if signed_word && width == 4 {
        "s32".to_string()
    } else {
        format!("u{}", width * 8)
    };
    Command::new("IF")
        .text("variable", variable)
        .text("operator", relation)
        .signed_number("value", signed_value)
        .text("value_type", value_type)
        .text("false_target", target)
}

pub(super) fn switch(
    data: &[u8],
    from: usize,
    end: usize,
    variable: &str,
    labels: &BTreeMap<usize, String>,
) -> Command {
    let Some(close) = marker(data, from, end, 0x21) else {
        return unknown();
    };
    let mut cases = Vec::new();
    for number in 0..8 {
        if let Some(target) = marker(data, from, close, 0x18 + number)
            .and_then(|at| target_label(at + 3, end, labels))
        {
            cases.push((number as usize, target));
        }
    }
    Command::new("SWITCH")
        .text("variable", variable)
        .cases(cases)
}

pub(super) fn write_scene(
    out: &Path,
    game: &str,
    id: &str,
    commands: &[Command],
) -> Result<String> {
    let relative = format!("scenes/{id}.yaml");
    let mut yaml = format!(
        "---\nformat_version: 2\ngame: {}\nscene_id: {}\ncommands:\n",
        quote(game),
        quote(id)
    );
    for (index, command) in commands.iter().enumerate() {
        writeln!(
            yaml,
            "  -\n    label: {}\n    type: {}",
            quote(&label(index + 1)),
            quote(command.kind)
        )?;
        for (key, value) in &command.fields {
            match value {
                Value::Text(s) => writeln!(yaml, "    {key}: {}", quote(s))?,
                Value::Number(n) => writeln!(yaml, "    {key}: {n}")?,
                Value::Texts(items) => {
                    if items.is_empty() {
                        writeln!(yaml, "    {key}: []")?;
                    } else {
                        writeln!(yaml, "    {key}:")?;
                        for item in items {
                            writeln!(yaml, "      - {}", quote(item))?;
                        }
                    }
                }
                Value::Cases(cases) => {
                    if cases.is_empty() {
                        writeln!(yaml, "    {key}: []")?;
                    } else {
                        writeln!(yaml, "    {key}:")?;
                        for (value, target) in cases {
                            writeln!(
                                yaml,
                                "      -\n        value: {value}\n        target: {}",
                                quote(target)
                            )?;
                        }
                    }
                }
            }
        }
    }
    write_file(&out.join(&relative), &yaml)?;
    Ok(relative)
}
