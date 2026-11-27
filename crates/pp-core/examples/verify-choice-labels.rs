use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::io::{self, Read};
use std::process::ExitCode;

use pp_core::script::{DialogueWarning, Script, ScriptSite};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct AuditInput {
    source: String,
    relationship_stats: BTreeMap<String, String>,
    event_boundary: String,
    addresses: Vec<ScriptSite>,
    replies: Vec<ScriptSite>,
    glyph_texts: Vec<String>,
}

#[derive(Serialize)]
struct AuditResult {
    dialogue: Vec<DialogueWarning>,
    missing_glyphs: Vec<(usize, BTreeSet<char>)>,
}

fn run() -> Result<bool, Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let (max_width, font_index, audit) = match args.as_slice() {
        [width, font] => (width, font, false),
        [width, font, flag] if flag == "--audit" => (width, font, true),
        _ => {
            return Err(
                "usage: verify-choice-labels <max-width> <font-index> [--audit] < script.yaml"
                    .into(),
            );
        }
    };
    let max_width: usize = max_width.parse()?;
    let glyphs: HashMap<char, [usize; 3]> =
        serde_saphyr::from_str(&std::fs::read_to_string(font_index)?)?;
    let mut source = String::new();
    io::stdin().read_to_string(&mut source)?;
    let audit = audit
        .then(|| serde_saphyr::from_str::<AuditInput>(&source))
        .transpose()?;
    let script = Script::from_yaml(audit.as_ref().map_or(&source, |payload| &payload.source))?;
    let issues = script.verify_choice_labels(max_width, |label| {
        label
            .chars()
            .map(|ch| glyphs.get(&ch).map_or(12, |cell| cell[2]))
            .sum()
    });
    for issue in &issues {
        let reason = if issue.multiline {
            "contains a line break".into()
        } else {
            format!("{} dots, over {max_width}", issue.width)
        };
        eprintln!(
            "{} {}: {reason}: {:?}",
            issue.scene, issue.path, issue.label
        );
    }
    if let Some(audit) = audit {
        let boundary = Some(audit.event_boundary.as_str());
        let mut dialogue = script.verify_relationship_changes(&audit.relationship_stats, boundary);
        dialogue.extend(script.verify_protagonist_replies(
            &audit.addresses,
            &audit.replies,
            boundary,
        ));
        let missing_glyphs = audit
            .glyph_texts
            .iter()
            .enumerate()
            .filter_map(|(i, text)| {
                let missing: BTreeSet<_> = text
                    .chars()
                    .filter(|ch| !ch.is_whitespace() && !glyphs.contains_key(ch))
                    .collect();
                (!missing.is_empty()).then_some((i, missing))
            })
            .collect();
        print!(
            "{}",
            serde_saphyr::to_string(&AuditResult {
                dialogue,
                missing_glyphs
            })?
        );
    }
    Ok(issues.is_empty())
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
