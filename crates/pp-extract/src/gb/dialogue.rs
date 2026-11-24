use anyhow::{Context, Result, ensure};
use std::fmt::Write;
use std::io::Read;
use std::path::Path;

// Success dialogue character assignments and control widths for PP1 and PP2.
// https://w.atwiki.jp/pawapoke_dataokiba/pages/32.html
const SINGLE: &str = concat!(
    "　あいうえおかきくけこさしすせそ",
    "たちつてとなにぬねのはひふへほま",
    "みむめもやゆよらりるれろわをんぁ",
    "ぃぅぇぉっゃゅょがぎぐげござじず",
    "ぜぞだぢづでどばびぶべぼぱぴぷぺ",
    "ぽアイウエオカキクケコサシスセソ",
    "タチツテトナニヌネノハヒフヘホマ",
    "ミムメモヤユヨラリルレロワヲンァ",
    "ィゥェォッャュョガギグゲゴザジズ",
    "ゼゾダヂヅデドバビブベボパピプペ",
    "ポヴＡＢＣＤＥＦＧＨＩＪＫＬＭＮ",
    "ＯＰＱＲＳＴＵＶＷＸＹＺ０１２３",
    "４５６７８９！？・－❤ｒ％○×「",
    "」／吉▼▽→負員凶⇒（）塁投保ー",
    "二三遊左中右外野球手広角打法流守",
    "両高校巧速筋技対内。、－＋",
);

const EXTENDED: &str = concat!(
    "安気練習部血殺死室団自分試合人入",
    "下上男女出見大会学校言海走今日明",
    "★体～学園話以電変化恋能月回先生",
    "勝年社長位俸億万金円来酒軍利率狂",
    "低発子心実世多少知書文秘者早行場",
    "目私店間何休当評価良仲返事考身小",
    "食水火好他全前名帰思王楽方戻別土",
    "伍曹山空客首正元争千古新高駅公時",
    "士本兄供株式況氏斗寮仕春夏秋冬♪",
    "戦神様不彼天同訓練四代根性国赤字",
    "■界病呪凡伝自超伝説選送盗失敗初",
    "振番強弱花症短定悪運反応直寸仙呼",
    "向付肩術点席使備逆逃虫歯治信昨車",
    "重君形決足用数合牛丼止亡買払貯怪",
    "獣佐作魚丈夫件告汗句油東西南北動",
    "司令隊主再販売第弾放色父　き入Ｔ",
);

fn is_pp2(input: &Path, bytes: &[u8]) -> bool {
    if let Some((profile, _)) = super::detect(bytes) {
        return profile == "pp2_gbc_v0";
    }
    let manifest_profile = || -> Option<&'static str> {
        let source = input.canonicalize().ok()?;
        let banks = source.parent()?;
        if banks.file_name()? != "banks" {
            return None;
        }
        let file = std::fs::File::open(banks.parent()?.join("manifest.json")).ok()?;
        let manifest: serde_json::Value = serde_json::from_reader(file.take(64 * 1024)).ok()?;
        super::profile_for_hash(manifest["sha1"].as_str()?)
    };
    manifest_profile() == Some("pp2_gbc_v0")
}

pub(crate) fn export_dialogue(
    input: &Path,
    offset: usize,
    length: Option<usize>,
    pp2: bool,
    out: &Path,
) -> Result<()> {
    let data = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    let pp2 = pp2 || is_pp2(input, &data);
    let remaining = data
        .get(offset..)
        .context("dialogue offset exceeds input")?;
    let listing = if let Some(length) = length {
        let range = remaining
            .get(..length)
            .context("dialogue range exceeds input")?;
        render(range, offset, pp2, false)?
    } else {
        let bank_remaining = super::BANK_SIZE - offset % super::BANK_SIZE;
        render(
            &remaining[..remaining.len().min(bank_remaining)],
            offset,
            pp2,
            true,
        )?
    };
    if let Some(parent) = out.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, listing)?;
    Ok(())
}

pub(crate) fn find_dialogue(input: &Path, text: &str, pp2: bool) -> Result<()> {
    let bytes = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    let mut stdout = std::io::stdout().lock();
    search_dialogue(&bytes, text, pp2 || is_pp2(input, &bytes), &mut stdout)
}

fn glyph_tables(pp2: bool) -> (Vec<char>, Vec<char>) {
    let extended_count = if pp2 { 0x100 } else { 0x31 };
    (
        SINGLE.chars().collect(),
        EXTENDED.chars().take(extended_count).collect(),
    )
}

fn search_dialogue(
    bytes: &[u8],
    text: &str,
    pp2: bool,
    output: &mut impl std::io::Write,
) -> Result<()> {
    let needle: Vec<char> = text.chars().collect();
    ensure!(!needle.is_empty(), "dialogue search text is empty");
    let (single, extended) = glyph_tables(pp2);
    for c in &needle {
        ensure!(
            single.contains(c) || extended.contains(c),
            "character {c:?} has no dialogue encoding for the selected title"
        );
    }

    writeln!(output, "offset\tlength")?;
    for start in 0..bytes.len() {
        let mut end = start;
        let matches = needle.iter().all(|&c| {
            let (glyph, width) = match bytes.get(end..) {
                Some([0xfd, code, ..]) => (extended.get(usize::from(*code)), 2),
                Some([code, ..]) => (single.get(usize::from(*code)), 1),
                _ => (None, 0),
            };
            if glyph == Some(&c) {
                end += width;
                true
            } else {
                false
            }
        });
        if matches {
            writeln!(output, "0x{start:08X}\t{}", end - start)?;
        }
    }
    output.flush()?;
    Ok(())
}

fn control(opcode: u8, pp2: bool) -> Option<(usize, &'static str)> {
    Some(match opcode {
        0x00 => (2, "newline"),
        0x01 => (2, "wait"),
        0x02 => (2, "wait_newline"),
        0x03 => (2, "wait_clear"),
        0x04 => (2, "player_name"),
        0x09 => (2, "control"),
        0x0a..=0x0d => (2, "text_speed"),
        0x10 => (3, "play_sound"),
        0x11 => (2, "emphasis_on"),
        0x12 => (2, "emphasis_off"),
        0x13 => (4, "display_variable"),
        0x14 => (3, "center_picture"),
        0x15 => (3, "left_portrait"),
        0x16 => (3, "right_portrait"),
        0x17..=0x19 => (5, "control"),
        0x1c => (4, "switch"),
        0x1d..=0x24 => (2, "case"),
        0x25 => (2, "case_end"),
        0x26 => (2, "switch_end"),
        0x27 => (5, "condition"),
        0x28 => (2, "condition_open"),
        0x29 => (2, "condition_close"),
        0x2d => (2, "player_position"),
        0x2e => (2, "character_name"),
        0x32 => (3, "delay"),
        0x35 => (4, "stat_add"),
        0x36 => (4, "stat_subtract"),
        0x3a => (2, "apply_stat_changes"),
        0x2b | 0x2f | 0x34 | 0x37 | 0x39 | 0x42..=0x45 => (4, "control"),
        0x30 | 0x3b..=0x3d | 0x3f | 0x41 | 0x47..=0x49 => (5, "control"),
        0x31 => (6, "control"),
        0x33 => (2, "control"),
        0x38 => (3, "control"),
        0x3e | 0x40 | 0x4a | 0x4d..=0x4f | 0x51 if pp2 => (5, "control"),
        0x4b | 0x4c | 0x57 | 0x58 if pp2 => (2, "control"),
        0x53 if pp2 => (2, "preferred_team"),
        0x54 | 0x55 | 0x59 if pp2 => (4, "control"),
        0x56 if pp2 => (3, "text_color"),
        _ => return None,
    })
}

fn render(bytes: &[u8], base: usize, pp2: bool, through_end: bool) -> Result<String> {
    let (single, extended) = glyph_tables(pp2);
    let mut output = String::from("offset\tbytes\tkind\ttext\n");
    let mut offset = 0;
    let mut choices_remaining = 0;
    while offset < bytes.len() {
        let rest = &bytes[offset..];
        let (width, kind, glyph) = match rest {
            [0xfd, code, ..] => {
                let ch = extended.get(usize::from(*code)).copied();
                (2, "glyph", ch)
            }
            [0xfe, ..] if choices_remaining > 0 => {
                choices_remaining -= 1;
                (1, "choice_end", None)
            }
            [0xfd] | [0xfe] | [0xff] => (1, "truncated", None),
            [0xfe, 0x1a | 0x1b, count, ..] if *count >= 2 => {
                choices_remaining = *count;
                (3, "choices", None)
            }
            [0xfe, 0x1a | 0x1b] => (2, "truncated", None),
            [0xfe, 0x1a | 0x1b, ..] => (rest.len(), "unknown_tail", None),
            [0xfe, opcode, ..] => match control(*opcode, pp2) {
                Some((width, kind)) if width <= rest.len() => (width, kind, None),
                Some(_) => (rest.len(), "truncated", None),
                None => (rest.len(), "unknown_tail", None),
            },
            [0xff, 0xff, ..] => (2, "end", None),
            [0xff, ..] => (rest.len(), "unknown_tail", None),
            [code, ..] => (1, "glyph", Some(single[usize::from(*code)])),
            [] => unreachable!(),
        };
        if through_end {
            ensure!(
                !matches!(kind, "truncated" | "unknown_tail"),
                "cannot determine event length at file offset 0x{:X} ({kind})",
                base + offset
            );
            ensure!(
                kind != "end" || choices_remaining == 0,
                "event ended inside a choice list at file offset 0x{:X}",
                base + offset
            );
        }
        write!(output, "{:08X}\t", base + offset).unwrap();
        for byte in &rest[..width] {
            write!(output, "{byte:02X}").unwrap();
        }
        write!(output, "\t{kind}\t").unwrap();
        if let Some(ch) = glyph {
            output.push(ch);
        }
        output.push('\n');
        offset += width;
        if through_end && kind == "end" {
            return Ok(output);
        }
    }
    ensure!(
        !through_end,
        "no event terminator before the end of the input or ROM bank"
    );
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_title_specific_glyphs_and_preserves_control_operands() {
        assert_eq!(SINGLE.chars().count(), 0xfd);
        assert_eq!(EXTENDED.chars().count(), 0x100);
        let bytes = [
            1, 0xfd, 0x30, 0xfd, 0x31, 0xfe, 0x10, 0xff, 0xff, 0xff, 0xfd,
        ];
        let pp2 = render(&bytes, 4, true, false).unwrap();
        assert!(pp2.contains("00000007\tFD31\tglyph\t年"));
        assert!(
            render(&bytes, 4, false, false)
                .unwrap()
                .contains("00000007\tFD31\tglyph\t\n")
        );
        assert!(pp2.contains("00000009\tFE10FF\tplay_sound\t\n"));
        assert!(pp2.contains("0000000C\tFFFF\tend\t\n"));
        assert!(pp2.contains("0000000E\tFD\ttruncated\t\n"));
        assert!(
            render(&[0xfe, 0x1a, 1, 1, 0xfe], 0, true, false)
                .unwrap()
                .contains("FE1A0101FE\tunknown_tail")
        );
    }
    #[test]
    fn reads_explicit_choices_and_resumes_after_the_last_delimiter() {
        let data = [
            0xfe, 0x1b, 2, 1, 0xfd, 0xfe, 0xfe, 2, 0xfe, 0xfe, 0x56, 4, 0xff, 0xff,
        ];
        let output = render(&data, 0, true, false).unwrap();
        assert_eq!(output.matches("FE\tchoice_end").count(), 2);
        assert!(output.contains("00000004\tFDFE\tglyph\t入"));
        assert!(output.contains("00000009\tFE5604\ttext_color"));
        assert!(output.contains("0000000C\tFFFF\tend"));
        assert!(
            render(&data, 0, false, false)
                .unwrap()
                .contains("FE5604FFFF\tunknown_tail")
        );
    }
    #[test]
    fn searches_both_encodings_for_a_shared_glyph() {
        let mut out = Vec::new();
        // 入 has two encodings in PP2, including one with FE as the low byte.
        let data = [1, 0xfd, 0x0f, 1, 0xfd, 0xfe, 1, 0xfe, 0x00];
        search_dialogue(&data, "あ入", true, &mut out).unwrap();
        assert_eq!(out, b"offset\tlength\n0x00000000\t3\n0x00000003\t3\n");
        out.clear();
        search_dialogue(&data, "あ入", false, &mut out).unwrap();
        assert_eq!(out, b"offset\tlength\n0x00000000\t3\n");
        assert!(search_dialogue(&data, "年", false, &mut out).is_err());
    }
    #[test]
    fn automatic_event_range_checks_the_terminator_and_ignores_following_bytes() {
        let bytes = [1, 0xfe, 0x10, 0xff, 2, 0xff, 0xff, 3];
        let output = render(&bytes, 0x4000, false, true).unwrap();
        assert!(output.ends_with("00004005\tFFFF\tend\t\n"));
        assert!(render(&bytes[..6], 0x4000, false, true).is_err());
        assert!(render(&[0xfe, 0x2c, 0xff, 0xff], 0x4000, false, true).is_err());
        assert!(render(&[0xfe, 0x1a, 2, 0xff, 0xff], 0x4000, false, true).is_err());
    }
    #[test]
    fn infers_pp2_from_a_catalogued_bank_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let banks = dir.path().join("banks");
        std::fs::create_dir(&banks).unwrap();
        let bank = banks.join("003.bin");
        std::fs::write(&bank, [0xfd, 0x31, 0xff, 0xff]).unwrap();
        let manifest = dir.path().join("manifest.json");
        let out = dir.path().join("dialogue.tsv");
        std::fs::write(
            &manifest,
            br#"{"sha1":"5c0ba404cf30296dceeb427a54e3644415c9e8db"}"#,
        )
        .unwrap();
        export_dialogue(&bank, 0, None, false, &out).unwrap();
        assert!(
            std::fs::read_to_string(&out)
                .unwrap()
                .contains("FD31\tglyph\t年")
        );
        std::fs::write(&manifest, br#"{"sha1":"not catalogued"}"#).unwrap();
        export_dialogue(&bank, 0, None, false, &out).unwrap();
        assert!(
            std::fs::read_to_string(&out)
                .unwrap()
                .contains("FD31\tglyph\t\n")
        );
    }
}
