use ppextractor::Game;
use std::collections::BTreeSet;

#[allow(dead_code)]
#[path = "../src/test_assets.rs"]
mod test_assets;

#[path = "../src/scripts/mod.rs"]
mod scripts;

fn compare_tree(actual: &std::path::Path, expected: &std::path::Path) {
    let mut names = std::fs::read_dir(expected)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    names.sort();
    let mut actual_names = std::fs::read_dir(actual)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    actual_names.sort();
    assert_eq!(
        actual_names,
        names,
        "directory entries differ at {}",
        expected.display()
    );
    for name in names {
        let expected_file = expected.join(&name);
        let actual_file = actual.join(&name);
        if expected_file.is_dir() {
            compare_tree(&actual_file, &expected_file);
        } else {
            let expected_bytes = std::fs::read(&expected_file).unwrap();
            let actual_bytes = std::fs::read(&actual_file).unwrap();
            assert_eq!(
                actual_bytes,
                expected_bytes,
                "file differs: {}",
                expected_file.display()
            );
        }
    }
}

fn parity(game: Game, name: &str) {
    let Some(rom_path) = test_assets::rom(name) else {
        return;
    };
    let Some(expected) = test_assets::oracle(name, "scripts", "") else {
        return;
    };
    let verified = ppextractor::verify_rom(&rom_path).unwrap();
    assert_eq!(verified.game, game);
    let temp = tempfile::tempdir().unwrap();
    scripts::extract(&verified.bytes, temp.path(), game).unwrap();
    compare_tree(&temp.path().join("native"), &expected);
    let scene_dir = temp.path().join("scenes");
    let mut scenes = std::fs::read_dir(&scene_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    scenes.sort();
    assert_eq!(scenes.len(), if name == "pp4" { 2297 } else { 1638 });
    let index = std::fs::read_to_string(temp.path().join("index.tsv")).unwrap();
    assert_eq!(index.lines().count(), scenes.len() + 1);
    assert_eq!(
        index.lines().next(),
        Some("scene_id\tscene_file\tnative_scene")
    );
    let links = index
        .lines()
        .skip(1)
        .map(|line| {
            let parts = line.split('\t').collect::<Vec<_>>();
            assert_eq!(parts.len(), 3);
            (
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
            )
        })
        .collect::<Vec<_>>();
    let scene_ids = links
        .iter()
        .map(|row| row.0.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(scene_ids.len(), scenes.len());
    let states = std::fs::read_to_string(temp.path().join("state-links.tsv")).unwrap();
    assert_eq!(states.lines().next(), Some("state_id\tnative_address"));
    let state_ids = states
        .lines()
        .skip(1)
        .map(|line| line.split('\t').next().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    for (i, scene) in scenes.iter().enumerate() {
        assert_eq!(
            scene.file_name().unwrap().to_str().unwrap(),
            format!("{}.yaml", links[i].0)
        );
        assert!(temp.path().join(&links[i].1).exists());
        let native_path = temp.path().join(&links[i].2);
        assert!(native_path.exists());
        let yaml = std::fs::read_to_string(scene).unwrap();
        assert!(
            yaml.starts_with("---\nformat_version: 2\n"),
            "{}",
            scene.display()
        );
        assert!(yaml.contains(&format!("scene_id: \"{name}_scene_{:06}\"", i + 1)));
        assert!(yaml.contains("\ncommands:\n"));
        assert!(yaml.contains("    type: "));
        for forbidden in ["offset:", "bytes_hex:", "args:"] {
            assert!(
                !yaml.contains(forbidden),
                "{} contains {forbidden}",
                scene.display()
            );
        }
        assert!(
            !yaml.contains("0x08"),
            "{} leaked ROM pointer",
            scene.display()
        );
        let native = std::fs::read_to_string(native_path).unwrap();
        assert_eq!(
            yaml.lines()
                .filter(|line| line.starts_with("    type: "))
                .count(),
            native
                .lines()
                .filter(|line| line.starts_with("    kind: "))
                .count()
        );
        let labels = yaml
            .lines()
            .filter_map(|line| line.trim().strip_prefix("label: \""))
            .map(|line| line.trim_end_matches('"').to_string())
            .collect::<BTreeSet<_>>();
        for line in yaml.lines() {
            let field = line.trim();
            if let Some(value) = field
                .strip_prefix("target: \"")
                .or_else(|| field.strip_prefix("false_target: \""))
            {
                let target = value.trim_end_matches('"');
                assert!(
                    target == "END" || labels.contains(target) || scene_ids.contains(target),
                    "{} has missing target {target}",
                    scene.display()
                );
            }
            if let Some(value) = field
                .strip_prefix("variable: \"")
                .or_else(|| field.strip_prefix("source: \""))
            {
                let state = value.trim_end_matches('"');
                assert!(
                    state_ids.contains(state),
                    "{} has missing state {state}",
                    scene.display()
                );
            }
        }
    }
    let combined = scenes
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect::<String>();
    for known in ["TEXT", "EFFECT", "CHOICE", "IF", "CALL", "UNRESOLVED"] {
        assert!(
            combined.contains(&format!("type: \"{known}\"")),
            "missing {known}"
        );
    }
    if name == "pp6" {
        assert!(combined.contains("type: \"SET\""));
        assert!(combined.contains("type: \"WAIT\""));
    } else {
        assert!(combined.contains("type: \"JUMP\""));
    }
}

#[test]
fn pp4_scripts_match_python_oracle() {
    parity(Game::Pp4, "pp4");
}

#[test]
fn pp6_scripts_match_python_oracle() {
    parity(Game::Pp6, "pp6");
}
