use ppextractor::{
    Game,
    audio::{AudioOptions, extract_with_options},
};
use serde_json::Value;
use std::collections::BTreeSet;

#[allow(dead_code)]
#[path = "../src/test_assets.rs"]
mod test_assets;

#[test]
fn pp4_audio_exports_rom_banks_and_tracks() {
    let Some(rom_path) = test_assets::rom("pp4") else {
        return;
    };
    let rom = ppextractor::verify_rom(&rom_path).unwrap();
    assert_eq!(rom.game, Game::Pp4);
    let tmp = tempfile::tempdir().unwrap();
    let options = AudioOptions {
        render_tracks: false,
        ..AudioOptions::default()
    };
    let root = tmp.path().join("pp4/audio");
    extract_with_options(&rom.bytes, &root, Game::Pp4, &options).unwrap();
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["counts"]["sample_wavs"], 155);
    assert_eq!(manifest["counts"]["bgm_sequences"], 164);
    assert_eq!(manifest["counts"]["se_sequences"], 190);
    assert_eq!(
        manifest["samples"][0]["raw_record_sha256"],
        "956c61880a92fecac870aae5da49d3507158ea7852fbdb62d42fea6a4b4bbfb4"
    );
    assert!(root.join("native/bgm/040/track_00.bin").exists());
    assert!(root.join("samples/6f101c.mid").exists());
    assert!(!root.join("bgm/040.mid").exists());
}

#[test]
fn selected_rom_audio_matches_reference_wave_and_midi() {
    for (name, game, song) in [("pp4", Game::Pp4, "040"), ("pp6", Game::Pp6, "07b")] {
        let Some(rom_path) = test_assets::rom(name) else {
            continue;
        };
        let Some(reference) = test_assets::oracle(name, "audio", "") else {
            continue;
        };
        let rom = ppextractor::verify_rom(&rom_path).unwrap();
        assert_eq!(rom.game, game);
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join(name).join("audio");
        let selected_ids = BTreeSet::from([u32::from_str_radix(song, 16).unwrap()]);
        let options = AudioOptions {
            selected_ids: Some(selected_ids),
            ..AudioOptions::default()
        };
        extract_with_options(&rom.bytes, &output, game, &options).unwrap();
        let sample = if name == "pp4" { "6f101c" } else { "722da8" };
        for relative in [
            format!("native/samples/{sample}.bin"),
            format!("samples/{sample}.wav"),
            format!("bgm/{song}.mid"),
            format!("bgm/{song}.wav"),
        ] {
            let got = std::fs::read(output.join(&relative)).unwrap();
            let reference_relative = if relative.starts_with("native/samples/") {
                relative.trim_start_matches("native/").to_string()
            } else if relative.starts_with("bgm/") {
                let suffix = if relative.ends_with(".mid") {
                    "score.mid"
                } else {
                    "render.wav"
                };
                format!("bgm/{song}/{suffix}")
            } else {
                relative.clone()
            };
            let want = std::fs::read(reference.join(&reference_relative)).unwrap();
            assert_eq!(got.len(), want.len(), "{name} {relative} length");
            if relative.starts_with("bgm/") && relative.ends_with(".wav") {
                assert_eq!(&got[..44], &want[..44], "{name} {relative} header");
                let differences = got[44..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(want[44..].as_chunks::<2>().0.iter())
                    .map(|(a, b)| {
                        (i16::from_le_bytes([a[0], a[1]]) as i32
                            - i16::from_le_bytes([b[0], b[1]]) as i32)
                            .abs()
                    })
                    .filter(|&delta| delta != 0)
                    .collect::<Vec<_>>();
                assert!(
                    differences.len() <= 1000 && differences.iter().all(|&delta| delta <= 1),
                    "{name} {relative}: {} differing samples, max delta {:?}",
                    differences.len(),
                    differences.iter().max()
                );
                continue;
            }
            if let Some((offset, (a, b))) =
                got.iter().zip(&want).enumerate().find(|(_, (a, b))| a != b)
            {
                let lo = offset.saturating_sub(12);
                let hi = (offset + 12).min(got.len());
                panic!(
                    "{name} {relative} first difference at {offset:#x}: {a:#x} vs {b:#x}; got={:02x?}; want={:02x?}",
                    &got[lo..hi],
                    &want[lo..hi]
                );
            }
        }
        assert_eq!(
            &std::fs::read(output.join(format!("samples/{sample}.mid"))).unwrap()[..4],
            b"MThd"
        );
        assert_eq!(
            &std::fs::read(output.join("rom.sf2")).unwrap()[..4],
            b"RIFF"
        );
    }
}
