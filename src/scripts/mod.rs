//! Dialogue VM script extraction for the verified Japanese cartridges.

mod pp4;
mod pp6;
mod semantic;

use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::Game;

pub fn extract(rom: &[u8], out: &Path, game: Game) -> Result<()> {
    let (hash, expected) = match game {
        Game::Pp4 => (PP4_HASH, "AP4J Rev. 01"),
        Game::Pp6 => (PP6_HASH, "BP6J"),
    };
    ensure!(
        digest(rom) == hash,
        "Expected verified Japanese {expected} ROM"
    );
    match game {
        Game::Pp4 => pp4::extract(rom, &out.join("native"), out),
        Game::Pp6 => pp6::extract(rom, &out.join("native"), out),
    }
}

const BASE: usize = 0x0800_0000;
const PP4_HASH: &str = "1f1746ecc664e6c56c41197755822236c2552fe3c1ceaf8c7192ef93a05bd236";
const PP6_HASH: &str = "72b6a3913d69710f74e9f96c7175a42901cfac486e623c2af6750bb4d487b157";

fn word(data: &[u8], at: usize) -> usize {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
}

fn digest(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        use std::fmt::Write;
        write!(out, "{b:02x}").unwrap();
    }
    out
}

fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            ch if ch <= '\x1f' => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn glyph_table(pp6: bool) -> Vec<Option<char>> {
    let mut chars = vec![None; 0x1100];
    for line in include_str!("glyphs.txt").lines() {
        let (index, ch) = line.split_once('\t').unwrap();
        chars[index.parse::<usize>().unwrap()] = ch.chars().next();
    }
    if pp6 {
        // Python PP6's canonical font names replace convenience ASCII aliases.
        for ch in "＋？！＠／（）％～￥".chars() {
            if let Some(index) = chars.iter().position(|item| {
                *item
                    == Some(match ch {
                        '＋' => '+',
                        '？' => '?',
                        '！' => '!',
                        '＠' => '@',
                        '／' => '/',
                        '（' => '(',
                        '）' => ')',
                        '％' => '%',
                        '～' => '~',
                        '￥' => '¥',
                        _ => unreachable!(),
                    })
            }) {
                chars[index] = Some(ch);
            }
        }
    } else {
        for (start, alphabet) in [
            (0x1051, "0123456789"),
            (0x105b, "ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
            (0x107e, "abcdefghijklmnopqrstuvwxyz"),
        ] {
            for (i, ch) in alphabet.chars().enumerate() {
                chars[start + i] = Some(ch);
            }
        }
        for (index, ch) in [
            (0x1005, 'ｶ'),
            (0x107d, '/'),
            (0x0d02, '屁'),
            (0x0d44, '璧'),
            (0x0d54, '祟'),
        ] {
            chars[index] = Some(ch);
        }
    }
    chars
}

fn decode_pp4(raw: &[u8], table: &[Option<char>]) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        i += 1;
        if c == 0 {
            result.push('　');
            continue;
        }
        let index = if (0xe8..=0xf7).contains(&c) && i < raw.len() {
            let index = (c as usize - 0xe7) * 256 + raw[i] as usize;
            i += 1;
            index
        } else {
            c as usize - 1
        };
        if let Some(ch) = table.get(index).copied().flatten() {
            result.push(ch);
        } else {
            result.push_str(&format!("〈glyph:{index:03X}〉"));
        }
    }
    result
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}
