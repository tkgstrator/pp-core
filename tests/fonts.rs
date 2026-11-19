use ppextractor::{Game, fonts};
use serde_json::Value;

#[allow(dead_code)]
#[path = "../src/test_assets.rs"]
mod test_assets;

fn parity(game: Game, name: &str) {
    let Some(rom_path) = test_assets::rom(name) else {
        return;
    };
    let Some(expected) = test_assets::oracle(name, "fonts", "glyphs.json") else {
        return;
    };
    let verified = ppextractor::verify_rom(&rom_path).unwrap();
    assert_eq!(verified.game, game);
    let temp = tempfile::tempdir().unwrap();
    fonts::extract(&verified.bytes, temp.path(), game).unwrap();

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
