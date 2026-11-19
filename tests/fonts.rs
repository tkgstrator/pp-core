use ppextractor::{Game, fonts};
use serde_json::Value;
use std::path::Path;

fn parity(game: Game, name: &str) {
    let source = Path::new("/Users/devonly/Developer/biccame-pocket/assets");
    let rom_path = source.join("roms").join(format!("{name}.gba"));
    if !rom_path.exists() {
        eprintln!("skipping unavailable ROM: {}", rom_path.display());
        return;
    }
    let rom = std::fs::read(rom_path).unwrap();
    let temp = tempfile::tempdir().unwrap();
    fonts::extract(&rom, temp.path(), game).unwrap();
    let expected = source.join("extracted").join(name).join("fonts");

    let got = image::open(temp.path().join("glyphs.png"))
        .unwrap()
        .to_rgba8();
    let want = image::open(expected.join("glyphs.png")).unwrap().to_rgba8();
    assert_eq!(got, want, "{name} atlas pixels differ");

    let got: Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("glyphs.json")).unwrap()).unwrap();
    let want: Value =
        serde_json::from_slice(&std::fs::read(expected.join("glyphs.json")).unwrap()).unwrap();
    assert_eq!(got, want, "{name} font metadata differs");

    let otf = std::fs::read(temp.path().join("dialogue.otf")).unwrap();
    assert_eq!(&otf[..4], b"OTTO", "{name} font must be CFF OpenType");
    assert!(
        otf.windows(4).any(|tag| tag == b"CFF "),
        "{name} CFF table missing"
    );

    let ttf = std::fs::read(temp.path().join("dialogue.ttf")).unwrap();
    assert_eq!(&ttf[..4], &[0, 1, 0, 0], "{name} font must be TrueType");
    assert!(
        ttf.windows(4).any(|tag| tag == b"glyf"),
        "{name} glyf table missing"
    );
}

#[test]
fn pp4_font_matches_python_oracle() {
    parity(Game::Pp4, "pp4");
}

#[test]
fn pp6_font_matches_python_oracle() {
    parity(Game::Pp6, "pp6");
}
