use ppextractor::Game;

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
    let source = std::path::Path::new("/Users/devonly/Developer/biccame-pocket/assets");
    let rom_path = source.join("roms").join(format!("{name}.gba"));
    if !rom_path.exists() {
        eprintln!("skipping unavailable ROM: {}", rom_path.display());
        return;
    }
    let rom = std::fs::read(rom_path).unwrap();
    let temp = tempfile::tempdir().unwrap();
    scripts::extract(&rom, temp.path(), game).unwrap();
    compare_tree(
        &temp.path().join("native"),
        &source.join("extracted").join(name).join("scripts"),
    );
    let scene_dir = temp.path().join("scenes");
    let mut scenes = std::fs::read_dir(&scene_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    scenes.sort();
    assert_eq!(scenes.len(), if name == "pp4" { 2297 } else { 1638 });
    let index = std::fs::read_to_string(temp.path().join("index.tsv")).unwrap();
    assert_eq!(index.lines().count(), scenes.len() + 1);
    assert_eq!(index.lines().next(), Some("scene_id\tscene_file\tnative_scene"));
    for (i, scene) in scenes.iter().enumerate() {
        let yaml = std::fs::read_to_string(scene).unwrap();
        assert!(yaml.starts_with("---\nformat_version: 2\n"), "{}", scene.display());
        assert!(yaml.contains(&format!("scene_id: \"{name}_scene_{:06}\"", i + 1)));
        assert!(yaml.contains("\ncommands:\n"));
        assert!(yaml.contains("    type: "));
        for forbidden in ["offset:", "bytes_hex:", "args:"] {
            assert!(!yaml.contains(forbidden), "{} contains {forbidden}", scene.display());
        }
        assert!(!yaml.contains("0x08"), "{} leaked ROM pointer", scene.display());
    }
    let combined = scenes.iter().map(|p| std::fs::read_to_string(p).unwrap()).collect::<String>();
    for known in ["TEXT", "EFFECT", "CHOICE", "IF", "CALL", "UNRESOLVED"] {
        assert!(combined.contains(&format!("type: \"{known}\"")), "missing {known}");
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
