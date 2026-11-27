use std::collections::HashMap;
use std::error::Error;
use std::io::{self, Read};
use std::process::ExitCode;

use pp_core::script::Script;

fn run() -> Result<bool, Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [max_width, font_index] = args.as_slice() else {
        return Err("usage: verify-choice-labels <max-width> <font-index> < script.yaml".into());
    };
    let max_width: usize = max_width.parse()?;
    let glyphs: HashMap<char, [usize; 3]> =
        serde_saphyr::from_str(&std::fs::read_to_string(font_index)?)?;
    let mut source = String::new();
    io::stdin().read_to_string(&mut source)?;
    let script = Script::from_yaml(&source)?;
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
