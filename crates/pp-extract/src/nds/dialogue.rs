//! Power Pro Kun Pocket Success dialogue, distinct from standard DS BMG messages.

use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use std::fmt::Write;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum DialogueGame {
    PpKoushien,
    PpDsKoushien,
    Pp8,
    Pp9,
    Pp10,
    Pp11,
    Pp12,
    Pp13,
    Pp14,
}

impl DialogueGame {
    fn numbered(self) -> bool {
        !matches!(self, Self::PpKoushien | Self::PpDsKoushien)
    }
}

fn infer_game(input: &Path) -> Option<DialogueGame> {
    if input
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("nds"))
        && let Some(game) = game_from_rom_header(input)
    {
        return Some(game);
    }

    input
        .canonicalize()
        .ok()?
        .ancestors()
        .find_map(|parent| game_from_manifest(&parent.join("rom.json")))
}

fn game_from_rom_header(input: &Path) -> Option<DialogueGame> {
    let mut header = [0; 16];
    std::fs::File::open(input)
        .ok()?
        .read_exact(&mut header)
        .ok()?;
    super::GAMES
        .iter()
        .find(|(code, _, _)| code.as_slice() == &header[12..])
        .map(|(_, _, game)| *game)
}

fn game_from_manifest(manifest: &Path) -> Option<DialogueGame> {
    let file = std::fs::File::open(manifest).ok()?;
    let value: serde_json::Value = serde_json::from_reader(file.take(64 * 1024)).ok()?;
    let code = value["game_code"].as_str()?;
    let profile = value["profile"].as_str()?;
    super::GAMES
        .iter()
        .find(|(candidate, id, _)| candidate.as_slice() == code.as_bytes() && *id == profile)
        .map(|(_, _, game)| *game)
}

pub(crate) fn export_dialogue(
    input: &Path,
    offset: usize,
    length: usize,
    game: Option<DialogueGame>,
    out: &Path,
) -> Result<()> {
    let mut file = std::fs::File::open(input).with_context(|| input.display().to_string())?;
    let end = offset
        .checked_add(length)
        .context("dialogue range overflow")?;
    ensure!(
        end as u64 <= file.metadata()?.len(),
        "dialogue range exceeds input"
    );
    file.seek(SeekFrom::Start(offset as u64))?;
    let mut data = vec![0; length];
    file.read_exact(&mut data)?;
    if let Some(parent) = out.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let game = game.or_else(|| infer_game(input));
    std::fs::write(out, render(&data, offset, game))?;
    Ok(())
}

fn comparison(data: &[u8], short_modes: bool, kind: &'static str) -> Option<(usize, &'static str)> {
    Some((
        match data.get(2) {
            None => 3,
            Some(0) if short_modes => 5,
            Some(1) if short_modes => 6,
            Some(2) => 8,
            _ => return None,
        },
        kind,
    ))
}

// Opcode widths documented for the DS titles:
// https://w.atwiki.jp/pawapoke_dataokiba/pages/33.html
fn token(data: &[u8], game: Option<DialogueGame>) -> Option<(usize, &'static str)> {
    use DialogueGame::*;
    let early = matches!(game, Some(PpKoushien | PpDsKoushien | Pp8 | Pp9));
    let koushien = matches!(game, Some(PpKoushien | PpDsKoushien));
    let late = matches!(game, Some(Pp10 | Pp11 | Pp12 | Pp13 | Pp14));
    Some(match data {
        [0, ..] => (1, "space"),
        [0x01..=0xe7, ..] => (1, "glyph"),
        [0xe8..=0xf7, ..] => (2, "glyph"),
        [0xfa, ..] => (1, "newline"),
        [0xfb, ..] => (1, "wait_newline"),
        [0xfc, ..] if matches!(game, Some(PpKoushien)) => (2, "background"),
        [0xfc, ..] if game.is_some_and(DialogueGame::numbered) => (3, "background"),
        [0xfd, ..] if game.is_some_and(DialogueGame::numbered) => (3, "left_portrait"),
        [0xfe, ..] if game.is_some_and(DialogueGame::numbered) => (3, "right_portrait"),
        // The remaining bytes may include alignment padding and event addresses.
        [0xff, ..] => (data.len(), "event_address_tail"),
        [0xf8, opcode, ..] => match opcode {
            0x00 => (2, "wait"),
            0x01 => (2, "wait_clear"),
            0x02 => (2, "end_event"),
            0x03 => (2, "fast_text"),
            0x04 => (2, "normal_text"),
            0x05 => (2, "slow_text"),
            0x06 => (2, "slower_text"),
            0x07 => (4, "play_sound"),
            0x08 => (2, "emphasis_on"),
            0x09 => (2, "emphasis_off"),
            0x0b if game.is_some_and(DialogueGame::numbered) => (4, "center_picture"),
            0x15 | 0x16 => (3, "choices"),
            0x17 => (3, "switch"),
            0x18 if late => (3, "case"),
            0x18..=0x1f if !late && data.get(2).is_none_or(|byte| byte == opcode) => (3, "case"),
            0x20 => (2, "case_end"),
            0x21 if data.get(2).is_none_or(|byte| byte == opcode) => (3, "switch_end"),
            0x22 => (3, "call_event"),
            0x23 => (3, "call_event_alt"),
            0x24 if game.is_some_and(DialogueGame::numbered) => (2, "control_f824"),
            0x25 => (2, "player_name"),
            0x26 => (2, "player_position"),
            0x27 => (2, "control_f827"),
            0x28 => (2, "team_or_character_name"),
            0x29 => (3, "delay"),
            0x2b | 0x2c => (3, "control"),
            0x2d | 0x30..=0x35 => return comparison(data, koushien, "condition"),
            0x2e if data.get(2).is_none_or(|byte| byte == opcode) => (3, "else"),
            0x2f if data.get(2).is_none_or(|byte| byte == opcode) => (3, "condition_end"),
            0x36 if late => (2, "control_f836"),
            0x36 | 0x39..=0x3f if early => {
                return comparison(data, koushien, "nested_condition");
            }
            0x37 | 0x40 | 0x49 if early && data.get(2).is_none_or(|byte| byte == opcode) => {
                (3, "else")
            }
            0x38 | 0x41 | 0x4a if early && data.get(2).is_none_or(|byte| byte == opcode) => {
                (3, "condition_end")
            }
            0x42..=0x48 | 0x4b..=0x4f if early => {
                return comparison(data, true, "nested_condition");
            }
            0x51 => (2, "apply_stat_changes"),
            0x52 | 0x53 if matches!(game, Some(PpDsKoushien)) => (4, "stat_change"),
            0x54..=0x57
                if matches!(game, Some(PpKoushien)) || game.is_some_and(DialogueGame::numbered) =>
            {
                (8, "stat_change")
            }
            0x58 | 0x59 if koushien => (4, "stat_change"),
            0x5a => (3, "text_color"),
            0x5b..=0x5d if game.is_some_and(DialogueGame::numbered) => (4, "variable_update"),
            0x60 | 0x61 => (4, "variable_update"),
            0x62..=0x66 | 0x68 => (7, "variable_update"),
            0x67 => (4, "variable_copy"),
            0x69 if matches!(game, Some(PpKoushien | Pp8)) => (3, "display_variable_byte"),
            0x6a => (3, "display_variable"),
            0x6b if game.is_some_and(DialogueGame::numbered) => (3, "speech_bubble"),
            0x6d if matches!(game, Some(Pp14)) => (2, "ruby_start"),
            0x6e if matches!(game, Some(Pp14)) => (2, "ruby_end"),
            0x6f if matches!(game, Some(PpDsKoushien)) => (4, "control_f86f"),
            0x70 | 0x71 if koushien => (4, "teammate_stat_change"),
            0x72 if koushien => (2, "teammate_name"),
            0x73 if koushien => (2, "school_name"),
            0x77 if matches!(game, Some(PpDsKoushien)) => (3, "left_portrait"),
            0x78 if matches!(game, Some(PpDsKoushien)) => (3, "right_portrait"),
            0x7e if koushien => (3, "speech_bubble"),
            0x80 if koushien => (2, "prefecture_name"),
            0x81 if matches!(game, Some(PpDsKoushien))
                && data.get(2).is_none_or(|&byte| byte == 0x81) =>
            {
                (4, "control_f881")
            }
            0xf8 => (2, "choice_end"),
            _ => return None,
        },
        [0xf8] => (2, "control"),
        _ => return None,
    })
}

// The DS titles share these assignments with the earlier games, with the
// following display differences: https://w.atwiki.jp/pawapoke_dataokiba/pages/51.html
fn glyph(bytes: &[u8], table: &[Option<char>], game: Option<DialogueGame>) -> Option<char> {
    match bytes {
        [code @ 0xa2..=0xab] => char::from_u32(0xff10 + u32::from(*code - 0xa2)),
        [code @ 0xac..=0xc5] => char::from_u32(0xff21 + u32::from(*code - 0xac)),
        [0xd6] => Some('－'),
        [0xdc] => Some('❤'),
        [0xe4] => Some('♪'),
        [0xe5] => Some('♫'),
        [0xe6] if matches!(game, Some(DialogueGame::Pp14)) => Some('贊'),
        [0xe6] if game.is_some_and(DialogueGame::numbered) => Some('･'),
        [0xe7] => Some('･'),
        [code @ 0x01..=0xe3] => table[usize::from(*code) - 1],
        // The table retains the older 栢 spelling. Its revision note specifies
        // 柏 from PP5 onwards, including the DS titles.
        [0xe9, 0x75] if game.is_some() => Some('柏'),
        [0xe9, 0x75] => None,
        [0xf2, 0x29] => Some('・'),
        [high, low] if (0xe800..=0xf396).contains(&u16::from_be_bytes([*high, *low])) => {
            table[usize::from(u16::from_be_bytes([*high, *low]) - 0xe700)]
        }
        [0xf3, low @ 0x97..=0xbf] => extended_glyph(*low, game?),
        [high @ 0xf5..=0xf6, low]
            if matches!(
                game,
                Some(
                    DialogueGame::Pp10
                        | DialogueGame::Pp11
                        | DialogueGame::Pp12
                        | DialogueGame::Pp13
                        | DialogueGame::Pp14
                )
            ) =>
        {
            wifi_name_glyph(u16::from_be_bytes([*high, *low]))
        }
        [0xf7, low] if game.is_some() => narrow_glyph(*low, table),
        [high, low] if matches!(game, Some(DialogueGame::Pp14)) => {
            pp14_glyph(u16::from_be_bytes([*high, *low]))
        }
        [0xf4, low @ 0xb0..=0xbf] => name_glyph(*low, game?),
        [0xf4, low @ 0xc0..=0xff] | [0xf5..=0xf6, low]
            if matches!(
                game,
                Some(
                    DialogueGame::PpKoushien
                        | DialogueGame::PpDsKoushien
                        | DialogueGame::Pp8
                        | DialogueGame::Pp9
                )
            ) =>
        {
            table[usize::from(*low & 0x3f)]
        }
        _ => None,
    }
}

fn pp14_glyph(code: u16) -> Option<char> {
    let (start, row) = match code {
        0xf3c0..=0xf4af => (
            0xf3c0,
            concat!(
                "丼亞仗伉佗佛佗伜俶俯會偸傀傴傲儂",
                "儚儡儺冕冤冦冩冲凉凛几凭凰刎刧刮",
                "刹剋剌剴劔辧勁匈匐匣卍卷厠咀咬哈",
                "咤哦唏唔哭啗嘔嘖墫壺壻奢埃娑娚媚",
                "尅將屁嶌崋崑崔廣弩彗徂彿忿怱恚恷",
                "舉暝曄曚朦霸杞杠柩枸檜桍梟梵棕楜",
                "洟浣涅淹渕淆淒淮溽溯潯潭澡澤澪濱",
                "瀉瀰灑炬炸烙焙犂犒狢狡狹猜猥獏獨",
                "珀琲瑜璋璧瓔珱畸當疵慮眸睇瞑矮砌",
                "磋祀祠祗祟笘筐筧篏籔籬籵糅經綺綮",
                "綣罸羂聚肛肓肚胚胱脛腋苳苺茹茫茗",
                "莵荳荵莉菘萋菲萠蒄葫萬蒹蔔蓼薔藪",
                "蕾號蛄蛟蛛蜥蠱衒袒袙裃裼褌褪褝覬",
                "訃訖詭詬詢誅誂諚諫踵蹉踪躬躰躾軋",
                "迪遑遒邨邯邱邵郢郤釡閠隹餃饅駝遙",
            ),
        ),
        0xf4b0..=0xf4bf => (0xf4b0, "瑤昻朗珉﨑颯緖逸郞鄕鄧    條"),
        _ => return None,
    };
    row.chars()
        .nth(usize::from(code - start))
        .filter(|&ch| ch != ' ')
}

fn wifi_name_glyph(code: u16) -> Option<char> {
    let (start, row) = match code {
        0xf515..=0xf51f => (0xf515, "♠♦♥♣→←↑↓×　!"),
        0xf520..=0xf57e => (
            0xf520,
            concat!(
                "\"#$%&'()*＋,-./01",
                "23456789:;<=>?@A",
                "BCDEFGHIJKLMNOPQ",
                "RSTUVWXYZ[〵]^_'a",
                "bcdefghijklmnopq",
                "rstuvwxyz｛l｝~€.",
            ),
        ),
        0xf587..=0xf5ff => (
            0xf587,
            concat!(
                "● ™≻œ¡¢£¨",
                "©®°±´+¿ÀÁÂÃÄÅÆÇÈ",
                "ÉÊËÌÍÎÏÐÑÒÓÔÕÖ×Ø",
                "ÙÚÛÜÝßàáâãäåæçèé",
                "êëìíîïðñòóôõö÷øù",
                "úûüý 、。，．･：；？！゛゜",
                "′‵̈͡￣＿ゝヾ々‒—–／＼～❘",
                "…‘'“”（）〔〕［］｛｝＜＞「",
            ),
        ),
        // Wi-Fi player names use their own kana ordering.
        0xf600..=0xf6cb => (
            0xf600,
            concat!(
                "」+-±×÷=∞∴°′″＆☆★○",
                "●◎◇◆□■△▲▽▼※〒→←↑↓",
                "♯♭♪ぁあぃいぅうぇえぉおかがき",
                "ぎくぐけげこごさざしじすずせぜそ",
                "ぞただちぢっつづてでとどなにぬね",
                "のはばぱひびぴふぶぷへべぺほぼぽ",
                "まみむめもゃやゅゆょよらりるれろ",
                "ゎわゐゑをんァアィイゥウェエォオ",
                "カガキギクグケゲコゴサザシジスズ",
                "セゼソゾタダチヂッツヅテデトドナ",
                "ニヌネノハバパヒビピフブプヘベペ",
                "ホボポマミムメモャヤュユョヨラリ",
                "ルレロヮワヰヱヲンヴヵヶ",
            ),
        ),
        _ => return None,
    };
    row.chars()
        .nth(usize::from(code - start))
        .filter(|&ch| ch != ' ')
}

fn narrow_glyph(code: u8, table: &[Option<char>]) -> Option<char> {
    match code {
        0x00..=0x50 => concat!(
            "アイウエオカキクケコサシスセソタ",
            "チツテトナニヌネノハヒフヘホマミ",
            "ムメモヤユヨラリルレロワヲンヴァ",
            "ィゥェォッャュョガギグゲゴザジズ",
            "ゼゾダヂヅデドバビブベボパピプペポ",
        )
        .chars()
        .nth(usize::from(code)),
        0x51..=0x5a => char::from_u32(0xff10 + u32::from(code - 0x51)),
        0x5b..=0x74 => char::from_u32(0xff21 + u32::from(code - 0x5b)),
        0x75..=0x7d => "＿＋－？！･「」／".chars().nth(usize::from(code - 0x75)),
        0x7e..=0x96 => char::from_u32(0xff41 + u32::from(code - 0x7e)),
        0x97 => Some('z'),
        0x98..=0xa1 => "⁰¹²³⁴⁵⁶⁷⁸⁹".chars().nth(usize::from(code - 0x98)),
        0xc0..=0xff => table[usize::from(code - 0xc0)],
        _ => None,
    }
}

fn name_glyph(code: u8, game: DialogueGame) -> Option<char> {
    use DialogueGame::*;

    let row = match game {
        PpKoushien | Pp8 | Pp9 => "瑤昻朗珉珣實緖逸郞鄕鄧    條",
        PpDsKoushien => "瑤昻朗珉珣實緖逸郞鄕鄧｜＿＜＞條",
        Pp10 | Pp11 | Pp12 | Pp13 => "瑤昻朗珉珣實緖逸郞鄕鄧    條",
        Pp14 => return None,
    };
    row.chars()
        .nth(usize::from(code - 0xb0))
        .filter(|&ch| ch != ' ')
}

fn extended_glyph(code: u8, game: DialogueGame) -> Option<char> {
    use DialogueGame::*;

    if matches!(game, Pp14) {
        return "髙   ｻ巫脩翔圀邉燁枡朗曉攝  E 姜秦邊齊炳壽奎洸眞嵜"
            .chars()
            .nth(usize::from(code - 0x97))
            .filter(|&ch| ch != ' ');
    }
    if (0x97..=0xa1).contains(&code) {
        return "髙礒臺晟﨑巫脩翔圀邉燁"
            .chars()
            .nth(usize::from(code - 0x97));
    }

    // Each position starts at F3A2. Spaces mark composite or unassigned glyphs.
    let row = match game {
        PpKoushien => "  佐藤",
        PpDsKoushien => "♯＄＆＝ａｂｃｄｅｆｇｈｉｊｋｌｍｎｏｐｑｒｓｔｕｖｗｘｙｚ",
        Pp8 => "   ﾉ  E",
        Pp9 => " ﾘ    E 姜秦邊",
        Pp10 => " ﾘ    E 姜秦邊 炳壽",
        Pp11 => " 朗曉   E 姜秦邊 炳壽",
        Pp12 => " 朗曉攝  E 姜秦邊齊炳壽奎洸踐",
        Pp13 => " 朗曉攝  E 姜秦邊齊炳壽奎洸踐杋",
        Pp14 => return None,
    };
    row.chars()
        .nth(usize::from(code - 0xa2))
        .filter(|&ch| ch != ' ')
}

pub(crate) fn find_dialogue(input: &Path, text: &str, game: Option<DialogueGame>) -> Result<()> {
    use std::io::Write as _;
    let search = DialogueSearch::new(
        text,
        game.or_else(|| infer_game(input))
            .or_else(|| game_from_revision_directory(input)),
    )?;
    let mut output = std::io::stdout().lock();
    if input.is_dir() {
        writeln!(output, "path\toffset\tlength")?;
        search_directory(input, input, &search, &mut output)?;
    } else {
        let bytes = std::fs::read(input).with_context(|| input.display().to_string())?;
        writeln!(output, "offset\tlength")?;
        search.scan(&bytes, |offset, length| {
            writeln!(output, "0x{offset:08X}\t{length}")?;
            Ok(())
        })?;
    }
    output.flush()?;
    Ok(())
}

// An nds-files revision directory contains one hash directory per distinct dump.
fn game_from_revision_directory(input: &Path) -> Option<DialogueGame> {
    let mut games = std::fs::read_dir(input).ok()?.filter_map(|entry| {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name = name.to_str()?;
        if name.len() != 64 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        game_from_manifest(&entry.path().join("rom.json"))
    });
    let first = games.next()?;
    games.all(|game| game == first).then_some(first)
}

fn search_directory(
    root: &Path,
    directory: &Path,
    search: &DialogueSearch,
    output: &mut impl std::io::Write,
) -> Result<()> {
    let mut entries = std::fs::read_dir(directory)
        .with_context(|| directory.display().to_string())?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_unstable_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            search_directory(root, &path, search, output)?;
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("bin"))
        {
            let bytes = std::fs::read(&path).with_context(|| path.display().to_string())?;
            let relative = path.strip_prefix(root)?;
            let name = relative
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('\t', "\\t")
                .replace('\r', "\\r")
                .replace('\n', "\\n");
            search.scan(&bytes, |offset, length| {
                writeln!(output, "{name}\t0x{offset:08X}\t{length}")?;
                Ok(())
            })?;
        }
    }
    Ok(())
}

struct DialogueSearch {
    game: Option<DialogueGame>,
    needle: Vec<char>,
    single: [Option<char>; 256],
    double: [[Option<char>; 256]; 16],
}

impl DialogueSearch {
    fn new(text: &str, game: Option<DialogueGame>) -> Result<Self> {
        let needle: Vec<char> = text.chars().collect();
        ensure!(!needle.is_empty(), "dialogue search text is empty");
        let table = crate::scripts::glyph_table(false);
        let mut single = [None; 256];
        single[0] = Some(' ');
        for code in 1u8..=0xe7 {
            single[usize::from(code)] = glyph(&[code], &table, game);
        }
        let mut double = [[None; 256]; 16];
        for high in 0xe8u8..=0xf7 {
            for low in 0u8..=255 {
                double[usize::from(high - 0xe8)][usize::from(low)] =
                    glyph(&[high, low], &table, game);
            }
        }
        for ch in &needle {
            ensure!(
                single.contains(&Some(*ch))
                    || double.iter().any(|row| row.contains(&Some(*ch)))
                    || [(0xf3, 0x98..=0xbf), (0xf4, 0xbb..=0xbe)].into_iter().any(
                        |(high, mut codes)| codes.any(|low| {
                            composite_glyph(&[high, low], game)
                                .is_some_and(|glyph| glyph.contains(*ch))
                        })
                    ),
                "character {ch:?} has no known dialogue encoding for the selected title"
            );
        }
        Ok(Self {
            game,
            needle,
            single,
            double,
        })
    }

    fn scan(&self, bytes: &[u8], mut hit: impl FnMut(usize, usize) -> Result<()>) -> Result<()> {
        for start in 0..bytes.len() {
            let mut end = start;
            let mut matched = 0;
            while matched < self.needle.len() {
                if let Some(pair) = bytes.get(end..).and_then(|tail| tail.get(..2)) {
                    if let Some(glyph) = composite_glyph(pair, self.game) {
                        let width = glyph.chars().count();
                        if !glyph
                            .chars()
                            .eq(self.needle[matched..].iter().copied().take(width))
                        {
                            break;
                        }
                        matched += width;
                        end += 2;
                        continue;
                    }
                }
                let (decoded, width) = match bytes.get(end..) {
                    Some([high @ 0xe8..=0xf7, low, ..]) => {
                        (self.double[usize::from(*high - 0xe8)][usize::from(*low)], 2)
                    }
                    Some([code, ..]) => (self.single[usize::from(*code)], 1),
                    _ => (None, 0),
                };
                if decoded != Some(self.needle[matched]) {
                    break;
                }
                end += width;
                matched += 1;
            }
            if matched == self.needle.len() {
                hit(start, end - start)?;
            }
        }
        Ok(())
    }
}

fn composite_glyph(bytes: &[u8], game: Option<DialogueGame>) -> Option<&'static str> {
    use DialogueGame::*;

    match (game?, bytes) {
        (Pp14, [0xf3, code @ 0x98..=0x9a]) => Some(["ｶﾙ", "ﾛｽ", "ﾛ･"][usize::from(*code - 0x98)]),
        (game, [0xf3, code @ 0xa2..=0xbf]) => {
            let row = match game {
                PpKoushien => "G･ G･",
                Pp8 => "ﾌｪ ﾘｼ ｱｰ - MI CH - AL",
                Pp9 | Pp10 => "ﾌｪ - ｼｱ ｰﾉ MI CH - AL",
                Pp11 | Pp12 | Pp13 => "G. - - - MI CH - AL",
                Pp14 => "- - - - MI CH - AL",
                PpDsKoushien => return None,
            };
            match code {
                0xa2..=0xa9 => row
                    .split_ascii_whitespace()
                    .nth(usize::from(*code - 0xa2))
                    .filter(|&text| text != "-"),
                0xb0..=0xb3 if matches!(game, Pp10 | Pp11) => {
                    Some(["TH", "UY", "OS", "HI"][usize::from(*code - 0xb0)])
                }
                0xb4..=0xb7 if matches!(game, Pp10 | Pp11 | Pp12 | Pp13 | Pp14) => {
                    Some(["ｸﾞﾗ", "ｲｼ", "ﾝｶ", "ﾞｰ"][usize::from(*code - 0xb4)])
                }
                0xb8..=0xbf => {
                    let row = match game {
                        Pp12 => ["ﾌｧ", "ﾙｹ", "ﾝﾎﾞ", "ｰｸﾞ", "ﾊﾞ", "ｰﾅ", "ﾑJ", "r."],
                        Pp13 | Pp14 => ["ﾌｧ", "ﾙｹ", "ﾝﾎﾞ", "ｰｸﾞ", "ﾒｯ", "ｾﾝ", "ｼﾞ", "ｬｰ"],
                        _ => ["ﾌｪ", "ﾙﾅ", "ﾝﾃ", "ﾞｽ", "ﾌｪ", "ﾙﾅ", "ﾝﾃ", "ﾞｽ"],
                    };
                    Some(row[usize::from(*code - 0xb8)])
                }
                _ => None,
            }
        }
        (game, [0xf4, code @ 0xbb..=0xbe]) if !matches!(game, PpDsKoushien) => {
            Some(["ﾌｪ", "ﾙﾅ", "ﾝﾃ", "ﾞｽ"][usize::from(*code - 0xbb)])
        }
        _ => None,
    }
}

fn render(data: &[u8], base: usize, game: Option<DialogueGame>) -> String {
    let glyphs = crate::scripts::glyph_table(false);
    let mut output = String::from("offset\tbytes\tkind\ttext\n");
    let mut offset = 0;
    while offset < data.len() {
        let remaining = &data[offset..];
        let (width, kind) = match token(remaining, game) {
            Some((width, kind)) if width <= remaining.len() => (width, kind),
            Some(_) => (remaining.len(), "truncated"),
            None => (remaining.len(), "unknown_tail"),
        };
        write!(output, "{:08X}\t", base + offset).unwrap();
        for byte in &remaining[..width] {
            write!(output, "{byte:02X}").unwrap();
        }
        write!(output, "\t{kind}\t").unwrap();
        match (&remaining[..width], kind) {
            ([0], "space") => output.push(' '),
            (bytes, "glyph") => {
                if let Some(ch) = glyph(bytes, &glyphs, game) {
                    output.push(ch);
                } else if let Some(text) = composite_glyph(bytes, game) {
                    output.push_str(text);
                }
            }
            _ => {}
        }
        output.push('\n');
        offset += width;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        DialogueGame, DialogueSearch, export_dialogue, game_from_revision_directory, infer_game,
        render, search_directory,
    };

    #[test]
    fn preserves_operands_glyph_codes_and_unknown_tail() {
        let data = [
            1, 0, 0xf5, 0xfa, 0xf8, 7, 0xf8, 2, 0xfa, 0xf8, 2, 0xfc, 0xaa,
        ];
        assert_eq!(
            render(&data, 16, None),
            concat!(
                "offset\tbytes\tkind\ttext\n",
                "00000010\t01\tglyph\tあ\n",
                "00000011\t00\tspace\t \n",
                "00000012\tF5FA\tglyph\t\n",
                "00000014\tF807F802\tplay_sound\t\n",
                "00000018\tFA\tnewline\t\n",
                "00000019\tF802\tend_event\t\n",
                "0000001B\tFCAA\tunknown_tail\t\n",
            )
        );
    }

    #[test]
    fn uses_title_specific_widths_and_preserves_address_data() {
        let data = [0xfc, 0, 0xfa, 0xf8, 0x18, 0x42, 0xff, 0, 0x01];
        let late = render(&data, 0, Some(DialogueGame::Pp10));
        assert!(late.contains("00000000\tFC00FA\tbackground"));
        assert!(late.contains("00000003\tF81842\tcase"));
        assert!(late.contains("00000006\tFF0001\tevent_address_tail"));
        assert!(render(&data, 0, Some(DialogueGame::Pp8)).contains("F81842FF0001\tunknown_tail"));
        let koushien = render(&data, 0, Some(DialogueGame::PpKoushien));
        assert!(koushien.contains("00000000\tFC00\tbackground"));
        assert!(koushien.contains("00000002\tFA\tnewline"));
        assert!(koushien.contains("F81842FF0001\tunknown_tail"));
    }

    #[test]
    fn distinguishes_early_and_late_case_opcodes() {
        let early = Some(DialogueGame::Pp9);
        let late = Some(DialogueGame::Pp10);
        assert!(render(&[0xf8, 0x19, 0x19, 1], 0, early).contains("F81919\tcase"));
        let raw = render(&[0xf8, 0x19, 0x19, 1], 0, None);
        assert!(raw.contains("00000000\tF81919\tcase"));
        assert!(raw.contains("00000003\t01\tglyph\tあ"));
        assert!(render(&[0xf8, 0x18, 0x19, 1], 0, None).contains("F8181901\tunknown_tail"));
        assert!(render(&[0xf8, 0x19, 0x19, 1], 0, late).contains("F8191901\tunknown_tail"));
        assert!(render(&[0xf8, 0x18, 0x19, 1], 0, late).contains("F81819\tcase"));
        assert!(render(&[0xf8, 0x19], 0, early).contains("F819\ttruncated"));
        assert!(render(&[0xf8, 0x21], 0, late).contains("F821\ttruncated"));
    }

    #[test]
    fn keeps_ds_koushien_f881_operands_out_of_dialogue() {
        let game = Some(DialogueGame::PpDsKoushien);
        let output = render(&[0xf8, 0x81, 0x81, 0xfa, 1], 0, game);
        assert!(output.contains("00000000\tF88181FA\tcontrol_f881"));
        assert!(output.contains("00000004\t01\tglyph\tあ"));
        assert!(render(&[0xf8, 0x81, 0x81], 0, game).contains("F88181\ttruncated"));
    }

    #[test]
    fn preserves_deep_branch_and_short_stat_operands() {
        let data = [
            0xf8, 0x48, 0, 1, 0xfa, 0xf8, 0x49, 0x49, 0xf8, 0x52, 1, 0xfa, 0xf8, 0x4a, 0x4a, 1,
        ];
        let output = render(&data, 0, Some(DialogueGame::PpDsKoushien));
        for token in [
            "F8480001FA\tnested_condition",
            "F84949\telse",
            "F85201FA\tstat_change",
            "F84A4A\tcondition_end",
            "0000000F\t01\tglyph\tあ",
        ] {
            assert!(output.contains(token), "{output}");
        }
        assert!(render(&data, 0, Some(DialogueGame::Pp10)).contains("\tunknown_tail"));
    }

    #[test]
    fn decodes_shared_kanji_and_title_specific_symbols() {
        let data = [
            0xe8, 0x05, 0xf2, 0x29, 0xa2, 0xc5, 0xe6, 0xe9, 0x75, 0xf5, 0,
        ];
        let late = render(&data, 0, Some(DialogueGame::Pp14));
        assert!(late.contains("E805\tglyph\t愛"));
        assert!(late.contains("F229\tglyph\t・"));
        assert!(late.contains("A2\tglyph\t０"));
        assert!(late.contains("C5\tglyph\tＺ"));
        assert!(late.contains("E6\tglyph\t贊"));
        assert!(late.contains("E975\tglyph\t柏"));
        assert!(render(&data, 0, None).contains("E975\tglyph\t\n"));
        assert!(late.contains("F500\tglyph\t\n"));
        assert!(render(&data, 0, Some(DialogueGame::Pp13)).contains("E6\tglyph\t･"));
        assert!(render(&data, 0, None).contains("E6\tglyph\t\n"));
    }

    #[test]
    fn decodes_and_searches_title_specific_extended_glyphs() {
        let data = [0xf3, 0x97, 0xf3, 0xa5, 0xf3, 0xb3, 0xf3, 0xbf];
        for (game, expected) in [
            (DialogueGame::PpKoushien, "F3A5\tglyph\t藤"),
            (DialogueGame::PpDsKoushien, "F3BF\tglyph\tｚ"),
            (DialogueGame::Pp8, "F3A5\tglyph\tﾉ"),
            (DialogueGame::Pp13, "F3B3\tglyph\t杋"),
        ] {
            let output = render(&data, 0, Some(game));
            assert!(output.contains("F397\tglyph\t髙"));
            assert!(output.contains(expected));
        }
        let mut hits = Vec::new();
        DialogueSearch::new("髙攝杋", Some(DialogueGame::Pp13))
            .unwrap()
            .scan(&data, |offset, len| {
                hits.push((offset, len));
                Ok(())
            })
            .unwrap();
        assert_eq!(hits, [(0, 6)]);
    }

    #[test]
    fn decodes_title_specific_name_glyphs() {
        let data = [0xf4, 0xb2, 0xf4, 0xbe, 0xf4, 0xbf];
        let koushien = render(&data, 0, Some(DialogueGame::PpDsKoushien));
        assert!(koushien.contains("F4B2\tglyph\t朗"));
        assert!(koushien.contains("F4BE\tglyph\t＞"));
        assert!(koushien.contains("F4BF\tglyph\t條"));
        let pp10 = render(&data, 0, Some(DialogueGame::Pp10));
        assert!(pp10.contains("F4B2\tglyph\t朗"));
        assert!(pp10.contains("F4BE\tglyph\tﾞｽ"));
        let mut hits = Vec::new();
        DialogueSearch::new("朗ﾞｽ條", Some(DialogueGame::Pp10))
            .unwrap()
            .scan(&data, |offset, len| {
                hits.push((offset, len));
                Ok(())
            })
            .unwrap();
        assert_eq!(hits, [(0, 6)]);
    }

    #[test]
    fn searches_adjacent_title_specific_composites() {
        let data = [0xf3, 0xbc, 0xf3, 0xbd];
        let game = Some(DialogueGame::Pp13);
        assert!(render(&data, 0, game).contains("F3BC\tglyph\tﾒｯ"));
        assert!(render(&data, 0, Some(DialogueGame::Pp12)).contains("F3BC\tglyph\tﾊﾞ"));
        let mut hits = Vec::new();
        DialogueSearch::new("ﾒｯｾﾝ", game)
            .unwrap()
            .scan(&data, |offset, len| {
                hits.push((offset, len));
                Ok(())
            })
            .unwrap();
        assert_eq!(hits, [(0, 4)]);
    }

    #[test]
    fn decodes_repeated_early_title_kana_pages() {
        let data = [0xf4, 0xc0, 0xf5, 0x41, 0xf6, 0xbf];
        let pp9 = render(&data, 0, Some(DialogueGame::Pp9));
        assert!(pp9.contains("F4C0\tglyph\tあ"));
        assert!(pp9.contains("F541\tglyph\tい"));
        assert!(pp9.contains("F6BF\tglyph\tぜ"));
        assert!(render(&data, 0, Some(DialogueGame::Pp10)).contains("F541\tglyph\tC"));
    }

    #[test]
    fn decodes_narrow_two_byte_glyphs() {
        let data = [0xf7, 0x50, 0xf7, 0x51, 0xf7, 0x97, 0xf7, 0xa1, 0xf7, 0xff];
        let game = Some(DialogueGame::Pp14);
        let output = render(&data, 0, game);
        for expected in [
            "F750\tglyph\tポ",
            "F751\tglyph\t０",
            "F797\tglyph\tz",
            "F7A1\tglyph\t⁹",
            "F7FF\tglyph\tぜ",
        ] {
            assert!(output.contains(expected));
        }
        let mut hits = Vec::new();
        DialogueSearch::new("z⁹ぜ", game)
            .unwrap()
            .scan(&data, |offset, len| {
                hits.push((offset, len));
                Ok(())
            })
            .unwrap();
        assert_eq!(hits, [(4, 6)]);
    }

    #[test]
    fn decodes_pp14_kanji_banks() {
        let data = [0xf3, 0xa2, 0xf3, 0xb3, 0xf3, 0xc0, 0xf4, 0xaf, 0xf4, 0xb5];
        let output = render(&data, 0, Some(DialogueGame::Pp14));
        for expected in [
            "F3A2\tglyph\t枡",
            "F3B3\tglyph\t嵜",
            "F3C0\tglyph\t丼",
            "F4AF\tglyph\t遙",
            "F4B5\tglyph\t颯",
        ] {
            assert!(output.contains(expected), "{output}");
        }
    }

    #[test]
    fn decodes_and_searches_pp14_composite_glyphs() {
        let data = [0xf3, 0x98, 0xf3, 0x99, 0xf3, 0xa2, 0xf4, 0xbb];
        let game = Some(DialogueGame::Pp14);
        let output = render(&data, 0, game);
        for expected in [
            "F398\tglyph\tｶﾙ",
            "F399\tglyph\tﾛｽ",
            "F3A2\tglyph\t枡",
            "F4BB\tglyph\tﾌｪ",
        ] {
            assert!(output.contains(expected));
        }
        let mut hits = Vec::new();
        DialogueSearch::new("ｶﾙﾛｽ枡ﾌｪ", game)
            .unwrap()
            .scan(&data, |offset, len| {
                hits.push((offset, len));
                Ok(())
            })
            .unwrap();
        assert_eq!(hits, [(0, 8)]);
    }

    #[test]
    fn decodes_pp14_wifi_name_kana() {
        let data = [0xf6, 0x23, 0xf6, 0x75, 0xf6, 0x76, 0xf6, 0xcb, 0xf6, 0xcc];
        let output = render(&data, 0, Some(DialogueGame::Pp14));
        for expected in [
            "F623\tglyph\tぁ",
            "F675\tglyph\tん",
            "F676\tglyph\tァ",
            "F6CB\tglyph\tヶ",
            "F6CC\tglyph\t\n",
        ] {
            assert!(output.contains(expected), "{output}");
        }
        assert!(render(&data, 0, Some(DialogueGame::Pp9)).contains("F623\tglyph\tや"));
    }

    #[test]
    fn decodes_wifi_name_latin_glyphs() {
        let data = [0xf5, 0x00, 0xf5, 0x1e, 0xf5, 0x3f, 0xf5, 0xa0, 0xf5, 0xff];
        for game in [
            DialogueGame::Pp10,
            DialogueGame::Pp11,
            DialogueGame::Pp12,
            DialogueGame::Pp13,
            DialogueGame::Pp14,
        ] {
            let output = render(&data, 0, Some(game));
            for expected in [
                "F500\tglyph\t\n",
                "F51E\tglyph\t　",
                "F53F\tglyph\tA",
                "F5A0\tglyph\tÉ",
                "F5FF\tglyph\t「",
            ] {
                assert!(output.contains(expected), "{output}");
            }
        }
    }

    #[test]
    fn finds_mixed_width_text_and_rejects_unmapped_characters() {
        let mut hits = Vec::new();
        let data = [0xff, 0xe8, 0x05, 0, 1, 0xe8, 0x05, 0, 1, 0xe8];
        DialogueSearch::new("愛 あ", None)
            .unwrap()
            .scan(&data, |start, length| {
                hits.push((start, length));
                Ok(())
            })
            .unwrap();
        assert_eq!(hits, [(1, 4), (5, 4)]);
        assert!(DialogueSearch::new("愛 A", None).is_err());
    }

    #[test]
    fn finds_dialogue_in_decompressed_archive_members() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let expanded = root.join("archives/0000/decompressed");
        std::fs::create_dir_all(&expanded).unwrap();
        std::fs::write(root.join("0000.bin"), [0xff, 1, 2]).unwrap();
        std::fs::write(expanded.join("0001.bin"), [1, 2]).unwrap();
        std::fs::write(root.join("index.tsv"), [1, 2]).unwrap();
        let mut out = Vec::new();
        let search = DialogueSearch::new("あい", None).unwrap();
        search_directory(root, root, &search, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "0000.bin\t0x00000001\t2\narchives/0000/decompressed/0001.bin\t0x00000000\t2\n"
        );
    }

    #[test]
    fn reports_incomplete_tokens() {
        assert!(render(&[0xf8, 7, 0], 0, None).contains("F80700\ttruncated"));
        assert!(render(&[0xe8], 0, None).contains("E8\ttruncated"));
    }
    #[test]
    fn respects_comparison_modes_and_preserves_payload_bytes() {
        let data = [
            0xf8, 0x30, 2, 0xfd, 0xfa, 0xf8, 0xff, 0x01, 0x01, 0xf8, 0x2e, 0x2e, 0x02, 0xf8, 0x2f,
            0x2f, 0xf8, 0x54, 1, 2, 0xfa, 0xfd, 0xff, 0xf8, 0xf8, 0x5a, 3, 0x01,
        ];
        let output = render(&data, 0, Some(DialogueGame::Pp14));
        assert!(output.contains("00000000\tF83002FDFAF8FF01\tcondition"));
        assert!(output.contains("00000009\tF82E2E\telse"));
        assert!(output.contains("00000010\tF8540102FAFDFFF8\tstat_change"));
        assert!(output.contains("00000018\tF85A03\ttext_color"));
        let short = [0xf8, 0x30, 1, 0xff, 0xf8, 0xfa, 1];
        assert!(
            render(&short, 0, Some(DialogueGame::PpDsKoushien)).contains("F83001FFF8FA\tcondition")
        );
        assert!(
            render(&short, 0, Some(DialogueGame::Pp9)).contains("F83001FFF8FA01\tunknown_tail")
        );
        assert!(render(&[0xf8, 0x2d], 0, None).contains("F82D\ttruncated"));
    }
    #[test]
    fn reads_ruby_and_decodes_title_specific_bubbles() {
        let pp14 = [0xe8, 0x05, 0xf8, 0x6d, 1, 0xf8, 0x6e, 0xf8, 0x6b, 0xff, 2];
        let output = render(&pp14, 0, Some(DialogueGame::Pp14));
        assert!(output.contains("00000002\tF86D\truby_start"));
        assert!(output.contains("00000005\tF86E\truby_end"));
        assert!(output.contains("00000007\tF86BFF\tspeech_bubble"));
        assert!(output.contains("0000000A\t02\tglyph\tい"));
        assert!(
            render(&pp14, 0, Some(DialogueGame::Pp13)).contains("F86D01F86EF86BFF02\tunknown_tail")
        );
        let koushien = [0xf8, 0x7e, 0, 0xf8, 0x72, 0xf8, 0x80];
        let output = render(&koushien, 0, Some(DialogueGame::PpDsKoushien));
        assert!(output.contains("00000000\tF87E00\tspeech_bubble"));
        assert!(output.contains("00000003\tF872\tteammate_name"));
        assert!(output.contains("00000005\tF880\tprefecture_name"));
    }
    #[test]
    fn infers_game_for_directory_of_multiple_revision_dumps() {
        let directory = tempfile::tempdir().unwrap();
        for (hash, manifest) in [
            ('a', br#"{"profile":"pp14","game_code":"VP4J"}"#),
            ('b', br#"{"profile":"pp14","game_code":"VP4J"}"#),
        ] {
            let dump = directory.path().join(hash.to_string().repeat(64));
            std::fs::create_dir(&dump).unwrap();
            std::fs::write(dump.join("rom.json"), manifest).unwrap();
        }
        assert_eq!(
            game_from_revision_directory(directory.path()),
            Some(DialogueGame::Pp14)
        );
        std::fs::write(
            directory.path().join("b".repeat(64)).join("rom.json"),
            br#"{"profile":"pp13","game_code":"VPLJ"}"#,
        )
        .unwrap();
        assert_eq!(game_from_revision_directory(directory.path()), None);
    }

    #[test]
    fn infers_game_from_rom_and_extracted_members_unless_overridden() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("sample.NDS");
        let mut header = [0; 16];
        header[12..].copy_from_slice(b"VP4J");
        std::fs::write(&source, header).unwrap();
        assert!(matches!(infer_game(&source), Some(DialogueGame::Pp14)));

        let root = dir.path().join("dump");
        let member = root.join("narc/0000/decompressed/000a.nds");
        std::fs::create_dir_all(member.parent().unwrap()).unwrap();
        std::fs::write(&member, [0xe6]).unwrap();
        std::fs::write(
            member.with_file_name("rom.json"),
            br#"{"profile":"unrelated"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("rom.json"),
            br#"{"profile":"pp14","game_code":"VP4J"}"#,
        )
        .unwrap();
        let out = dir.path().join("text.tsv");
        export_dialogue(&member, 0, 1, None, &out).unwrap();
        assert!(
            std::fs::read_to_string(&out)
                .unwrap()
                .contains("E6\tglyph\t贊")
        );
        export_dialogue(&member, 0, 1, Some(DialogueGame::Pp13), &out).unwrap();
        assert!(
            std::fs::read_to_string(&out)
                .unwrap()
                .contains("E6\tglyph\t･")
        );
    }
}
