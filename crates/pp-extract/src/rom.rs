use crate::fonts::FontFormat;
use crate::portraits::PortraitBank;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Game {
    Pp4,
    Pp6,
}

impl Game {
    pub(crate) fn accepts_sha256(self, digest: &str) -> bool {
        PROFILES
            .iter()
            .any(|profile| profile.game == Some(self) && profile.sha256 == digest)
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Pp4 => "pp4",
            Self::Pp6 => "pp6",
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::Pp4 => "AP4J",
            Self::Pp6 => "BP6J",
        }
    }

    pub const fn revision(self) -> u8 {
        match self {
            Self::Pp4 => 1,
            Self::Pp6 => 0,
        }
    }

    pub const fn expected_sha256(self) -> &'static str {
        match self {
            Self::Pp4 => "1f1746ecc664e6c56c41197755822236c2552fe3c1ceaf8c7192ef93a05bd236",
            Self::Pp6 => "72b6a3913d69710f74e9f96c7175a42901cfac486e623c2af6750bb4d487b157",
        }
    }
}

pub(crate) const PP4_V0_SHA256: &str =
    "97f00aac3b27e1507936c01f22e6ac4ab3c01cb9139978549487f26c77789a1b";

pub(crate) struct RomProfile {
    pub id: &'static str,
    pub code: &'static str,
    pub revision: u8,
    pub sha256: &'static str,
    pub game: Option<Game>,
    pub font: Option<(FontFormat, usize)>,
    pub samples: Option<crate::audio::SampleBank>,
    pub native_audio: Option<crate::audio::NativeAudioLayout>,
    pub busts: &'static [(usize, usize)],
    pub portrait_layers: &'static [PortraitBank],
    pub portrait_remaps: Option<[usize; 2]>,
    pub script_scan: &'static [(usize, usize)],
    pub script_listing: Option<crate::scripts::NativeListing>,
}

impl RomProfile {
    const fn complete(game: Game) -> Self {
        Self {
            id: game.id(),
            code: game.code(),
            revision: game.revision(),
            sha256: game.expected_sha256(),
            game: Some(game),
            font: None,
            samples: None,
            native_audio: None,
            busts: &[],
            portrait_layers: &[],
            portrait_remaps: None,
            script_scan: &[],
            script_listing: None,
        }
    }

    const fn images(
        id: &'static str,
        code: &'static str,
        revision: u8,
        sha256: &'static str,
    ) -> Self {
        Self {
            id,
            code,
            revision,
            sha256,
            game: None,
            font: None,
            samples: None,
            native_audio: None,
            busts: &[],
            portrait_layers: &[],
            portrait_remaps: None,
            script_scan: &[],
            script_listing: None,
        }
    }
}

const PP12_PORTRAIT_LAYERS: &[PortraitBank] = &[
    PortraitBank {
        header: 0x64aca0,
        count: 210,
        dialogue_id_base: None,
    },
    PortraitBank {
        header: 0x651ab8,
        count: 245,
        dialogue_id_base: None,
    },
];

// PP7 dialogue renderer selects banks at ID 0x100 and 0x140.
const PP7_DIALOGUE_PORTRAITS: &[PortraitBank] = &[
    PortraitBank {
        header: 0x65544c,
        count: 253,
        dialogue_id_base: Some(0),
    },
    PortraitBank {
        header: 0x6574a8,
        count: 57,
        dialogue_id_base: Some(0x100),
    },
    PortraitBank {
        header: 0x65f77c,
        count: 250,
        dialogue_id_base: Some(0x140),
    },
];

const fn pp3_playback(shift: usize) -> crate::audio::Playback {
    crate::audio::Playback {
        id: "pp3",
        psg_periods: 0x08d300 + shift,
        pcm_rates: 0x3b7cb8 + shift,
        transpose: 0x3bd46c + shift,
        sample_bases: &[0, 70],
        sample_count: 198,
    }
}

const PP12_SCRIPT_LISTING: crate::scripts::NativeListing = crate::scripts::NativeListing {
    dispatch: 0x55a548,
    effect_dispatch: Some(0x55a708),
    event_entry: 0x7a8c,
    indexed_entrypoints: &[
        (0x3cf4c, 48),
        (0x3d004, 12),
        (0x3d0fe, 18),
        (0x3d1c8, 12),
        (0x3d202, 16),
        (0x3dc32, 20),
        (0x3dcac, 4),
        (0x3dd34, 8),
        (0x4462a, 14),
        (0x50d14, 70),
        (0x50d92, 18),
        (0x50e6c, 18),
        (0x50f44, 9),
        (0x50f68, 9),
        (0x5193e, 20),
        (0x519b8, 4),
        (0x51a3c, 8),
        (0x5619e, 14),
        (0x594b4, 22),
        (0x59572, 18),
        (0x59af0, 15),
    ],
    unchecked_opcodes: &[],
    // All four handlers call 0x9990, which advances the bytecode cursor by one.
    operand_overrides: &[(0x6b, 1), (0x6c, 1), (0x6d, 1), (0x6e, 1)],
    portrait_sentinels: crate::scripts::PortraitSentinels::Unchecked,
};

const PP12_AUDIO: crate::audio::NativeAudioLayout = crate::audio::NativeAudioLayout {
    driver: Game::Pp6,
    playback: Some(crate::audio::Playback {
        id: "pp1_2",
        psg_periods: 0x107cd0,
        pcm_rates: 0x551e8c,
        transpose: 0x55c2a0,
        sample_bases: &[0, 56],
        sample_count: 208,
    }),
    track_format: crate::audio::TrackFormat::Legacy,
    bgm: (0x13efbc, 0x40, 175, 8, 0x1080d1, 0x13efbc),
    se: (0x143f98, 0x200, 410, 8, 0x13f534, 0x143f98),
    percussion: (0x55c56c, 0x144c68, 0x144da8),
};

pub(crate) const PROFILES: &[RomProfile] = &[
    RomProfile::complete(Game::Pp4),
    RomProfile::complete(Game::Pp6),
    RomProfile {
        id: "pp6_v0_clean",
        sha256: "65f994ecf0353ebe1d90f21fc783b3a4ae7994503f986005c0052eb87c36003e",
        ..RomProfile::complete(Game::Pp6)
    },
    RomProfile {
        native_audio: Some(PP12_AUDIO),
        script_listing: Some(PP12_SCRIPT_LISTING),
        portrait_layers: PP12_PORTRAIT_LAYERS,
        portrait_remaps: Some([0x563e84, 0x563f7c]),
        script_scan: &[(0x55224c, 0x60f5cc), (0x6126b0, 0x6128fc)],
        samples: Some((0x728e34, 0x7f8bac, 208, 0x55bf58)),
        font: Some((FontFormat::Pp6, 0x552574)),
        ..RomProfile::images(
            "pp1_2_v0",
            "B8PJ",
            0,
            "6e5830a7c385546205d3e50343a028aba54ff0d315f41d0604fd021e59d4497d",
        )
    },
    RomProfile {
        native_audio: Some(PP12_AUDIO),
        script_listing: Some(PP12_SCRIPT_LISTING),
        portrait_layers: PP12_PORTRAIT_LAYERS,
        portrait_remaps: Some([0x563e84, 0x563f7c]),
        script_scan: &[(0x55224c, 0x60f5cc), (0x6126b0, 0x6128fc)],
        samples: Some((0x728e34, 0x7f8bac, 208, 0x55bf58)),
        font: Some((FontFormat::Pp6, 0x552574)),
        ..RomProfile::images(
            "pp1_2_v0_p",
            "B8PJ",
            0,
            "7808ccf2ac1ec89d6252291464313b3b98218e3b3f363d6d41f07a2f31e10da7",
        )
    },
    RomProfile {
        script_listing: Some(crate::scripts::NativeListing {
            dispatch: 0x3bbd44,
            effect_dispatch: Some(0x3bbf04),
            event_entry: 0x538c,
            indexed_entrypoints: &[
                (0x243f0, 58),
                (0x24416, 6),
                (0x244c4, 18),
                (0x24562, 51),
                (0x24e5a, 23),
                (0x24ef0, 4),
                (0x24f5c, 7),
                (0x25592, 7),
            ],
            unchecked_opcodes: &[],
            operand_overrides: &[],
            portrait_sentinels: crate::scripts::PortraitSentinels::Standard,
        }),
        script_scan: &[(0x3b81a4, 0x410f68)],
        busts: &[
            (0x43d8ac, 79),
            (0x440c90, 90),
            (0x442e10, 57),
            (0x443e2c, 27),
        ],
        native_audio: Some(crate::audio::NativeAudioLayout {
            driver: Game::Pp4,
            playback: Some(pp3_playback(0)),
            track_format: crate::audio::TrackFormat::Legacy,
            bgm: (0x3bc304, 0x40, 112, 32, 0x8d680, 0xb2cf1),
            se: (0xb479c, 0x200, 249, 8, 0xb2cf1, 0xb479c),
            percussion: (0x3bd750, 0xb4f64, 0xb50a4),
        }),
        samples: Some((0x5d316c, 0x77aebc, 198, 0x3bd14c)),
        font: Some((FontFormat::Pp4, 0x3b841c)),
        ..RomProfile::images(
            "pp3_v0",
            "AP3J",
            0,
            "3a75c1ad9fc27375bac764b820b542e9444a3ba71ec10d99b6ca89f0517b2cbc",
        )
    },
    RomProfile {
        script_listing: Some(crate::scripts::NativeListing {
            dispatch: 0x3bc67c,
            effect_dispatch: Some(0x3bc83c),
            event_entry: 0x5390,
            indexed_entrypoints: &[
                (0x24504, 58),
                (0x2452a, 6),
                (0x245d8, 18),
                (0x24676, 51),
                (0x24f6e, 23),
                (0x25004, 4),
                (0x25070, 7),
                (0x256a6, 7),
            ],
            unchecked_opcodes: &[],
            operand_overrides: &[],
            portrait_sentinels: crate::scripts::PortraitSentinels::Standard,
        }),
        script_scan: &[(0x3b8adc, 0x411918)],
        busts: &[
            (0x43e25c, 79),
            (0x441640, 90),
            (0x4437c0, 57),
            (0x4447dc, 27),
        ],
        native_audio: Some(crate::audio::NativeAudioLayout {
            driver: Game::Pp4,
            playback: Some(pp3_playback(0x938)),
            track_format: crate::audio::TrackFormat::Legacy,
            bgm: (0x3bcc3c, 0x40, 112, 32, 0x8dfb8, 0xb3629),
            se: (0xb50d4, 0x200, 249, 8, 0xb3629, 0xb50d4),
            percussion: (0x3be088, 0xb589c, 0xb59dc),
        }),
        samples: Some((0x5d3b1c, 0x77b86c, 198, 0x3bda84)),
        font: Some((FontFormat::Pp4, 0x3b8d54)),
        ..RomProfile::images(
            "pp3_v1",
            "AP3J",
            1,
            "ca6fafb3746782199beaedd25a2fdeee7bb44049bf35f80bb8e0115e3eae17d7",
        )
    },
    RomProfile {
        game: Some(Game::Pp4),
        ..RomProfile::images("pp4_v0", "AP4J", 0, PP4_V0_SHA256)
    },
    RomProfile {
        script_listing: Some(crate::scripts::NativeListing {
            dispatch: 0x4e5dd4,
            effect_dispatch: Some(0x4e5f94),
            event_entry: 0x7140,
            indexed_entrypoints: &[
                (0x32c34, 73),
                (0x32c62, 12),
                (0x32d3c, 18),
                (0x32dce, 12),
                (0x33a34, 20),
                (0x33ada, 4),
                (0x33b44, 4),
                (0x33bb0, 8),
                (0x34200, 11),
                (0x4232a, 9),
            ],
            unchecked_opcodes: &[],
            operand_overrides: &[],
            portrait_sentinels: crate::scripts::PortraitSentinels::Standard,
        }),
        script_scan: &[(0x4e0754, 0x597390)],
        busts: &[(0x5c8670, 217)],
        native_audio: Some(crate::audio::NativeAudioLayout {
            driver: Game::Pp4,
            playback: Some(crate::audio::Playback::pp5(0xf22dc, 0x4e0394, 0x4e77d0)),
            track_format: crate::audio::TrackFormat::Legacy,
            bgm: (0x4e6394, 0x40, 117, 32, 0xf26dd, 0x13749f),
            se: (0x13a354, 0x200, 274, 8, 0x13749f, 0x13a354),
            percussion: (0x4e7a94, 0x13abe4, 0x13ad18),
        }),
        samples: Some((0x6f8f64, 0x7f7378, 156, 0x4e7558)),
        font: Some((FontFormat::Pp5, 0x4e09bc)),
        ..RomProfile::images(
            "pp5_v0",
            "A5PJ",
            0,
            "e70347f919664dd96af44e35062b5712daeed38e91e377d464777c6d4d436ddd",
        )
    },
    RomProfile {
        script_listing: Some(crate::scripts::NativeListing {
            dispatch: 0x4e77f4,
            effect_dispatch: Some(0x4e79b4),
            event_entry: 0x7170,
            indexed_entrypoints: &[
                (0x32c50, 73),
                (0x32c7e, 12),
                (0x32d58, 18),
                (0x32dea, 12),
                (0x33a68, 20),
                (0x33b0e, 4),
                (0x33b78, 4),
                (0x33be4, 8),
                (0x34234, 11),
                (0x422fe, 9),
            ],
            unchecked_opcodes: &[],
            operand_overrides: &[],
            portrait_sentinels: crate::scripts::PortraitSentinels::Standard,
        }),
        script_scan: &[(0x4e2174, 0x598d28)],
        busts: &[(0x5ca008, 217)],
        native_audio: Some(crate::audio::NativeAudioLayout {
            driver: Game::Pp4,
            playback: Some(crate::audio::Playback::pp5(0xf3a90, 0x4e1db4, 0x4e91f0)),
            track_format: crate::audio::TrackFormat::Legacy,
            bgm: (0x4e7db4, 0x40, 117, 32, 0xf3e91, 0x138c53),
            se: (0x13bb08, 0x200, 274, 8, 0x138c53, 0x13bb08),
            percussion: (0x4e94b4, 0x13c398, 0x13c4cc),
        }),
        samples: Some((0x6fa8fc, 0x7f8d10, 156, 0x4e8f78)),
        font: Some((FontFormat::Pp5, 0x4e23dc)),
        ..RomProfile::images(
            "pp5_v0_p",
            "A5PJ",
            0,
            "2800f7f7f8762b6169c8273249a013d90006eeedb4d36d96ddc4ca70e3040294",
        )
    },
    RomProfile {
        script_listing: Some(crate::scripts::NativeListing {
            dispatch: 0x55361c,
            effect_dispatch: Some(0x5537dc),
            event_entry: 0x7e74,
            indexed_entrypoints: &[
                (0x6b14, 4),
                (0x44b7c, 49),
                (0x44baa, 10),
                (0x44c62, 18),
                (0x44cfe, 10),
                (0x4570a, 20),
                (0x45798, 4),
                (0x45808, 8),
                (0x4b368, 6),
                (0x4c93e, 7),
                // Six-slot table at 0x55f804.
                (0x50dea, 6),
                (0x50fca, 6),
                (0x51126, 6),
                (0x5159e, 3),
                (0x51696, 3),
                (0x5183c, 3),
                // Nine-slot table at 0x55f828, indexed by current or next step.
                (0x51dc0, 9),
                (0x51e6e, 9),
                (0x51f46, 9),
                (0x51ffa, 9),
                (0x520c8, 9),
                (0x52224, 9),
                (0x522a8, 9),
                (0x52324, 9),
                (0x52386, 9),
                (0x52eb2, 2),
                (0x5316a, 2),
                // The selector byte arrays reference IDs through 14.
                (0x57a80, 14),
                (0x57b10, 9),
                // Index 7 is handled before the table load.
                (0x58562, 7),
            ],
            unchecked_opcodes: &[],
            // The shared handler at 0x9c4c consumes one portrait operand.
            operand_overrides: &[(0x6b, 1), (0x6c, 1), (0x6d, 1), (0x6e, 1)],
            portrait_sentinels: crate::scripts::PortraitSentinels::Pp7,
        }),
        portrait_layers: PP7_DIALOGUE_PORTRAITS,
        script_scan: &[(0x54a12c, 0x618584)],
        native_audio: Some(crate::audio::NativeAudioLayout {
            driver: Game::Pp6,
            playback: Some(crate::audio::Playback {
                id: "pp7",
                psg_periods: 0x11ad6c,
                pcm_rates: 0x549d6c,
                transpose: 0x55578c,
                sample_bases: &[0, 56],
                sample_count: 204,
            }),
            track_format: crate::audio::TrackFormat::Pp7,
            bgm: (0x160ac4, 0x40, 184, 8, 0x11b16d, 0x160ac4),
            se: (0x16a39c, 0x200, 727, 8, 0x161084, 0x16a39c),
            percussion: (0x555a64, 0x16ba54, 0x16bb94),
        }),
        samples: Some((0x743744, 0x7feca4, 204, 0x555454)),
        font: Some((FontFormat::Pp6, 0x54a3e4)),
        ..RomProfile::images(
            "pp7_v0",
            "BP7J",
            0,
            "54fe895a178b51c6b93e15c8410c32df1aaa04c2c6bdb717695b33ec692f50bf",
        )
    },
];

// Catalog fingerprints for GBA editions that lack a verified extraction layout.
pub(crate) const SCAN_PROFILES: &[ScanProfile<'static>] = &[
    (
        "pp5_v1",
        b"A5PJ",
        "29101619ef0ede2fe0174ba2ab6bd2427c1fe0a1",
    ),
    (
        "ppdash",
        b"BBQJ",
        "598db3dd414ec3acb8bea47192761816e401df15",
    ),
];

pub(crate) fn scan_gba(path: &Path, out: &Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading ROM {}", path.display()))?;
    let sha1 = format!("{:x}", sha1::Sha1::digest(&bytes));
    let profile = match_scan_profile(&bytes, &sha1, SCAN_PROFILES)
        .with_context(|| format!("unrecognized GBA scan profile, SHA-1 {sha1}"))?;
    export_gba_scan(&bytes, out, profile)
}

type ScanProfile<'a> = (&'a str, &'a [u8; 4], &'a str);

fn match_scan_profile<'a>(
    bytes: &[u8],
    sha1: &str,
    profiles: &[ScanProfile<'a>],
) -> Option<ScanProfile<'a>> {
    profiles
        .iter()
        .copied()
        .find(|(_, code, hash)| bytes.get(0xac..0xb0) == Some(code.as_slice()) && *hash == sha1)
}

pub(crate) fn catalog_scan_profile(bytes: &[u8]) -> Option<ScanProfile<'static>> {
    let code = bytes.get(0xac..0xb0)?;
    if !SCAN_PROFILES
        .iter()
        .any(|(_, candidate, _)| candidate.as_slice() == code)
    {
        return None;
    }
    let sha1 = format!("{:x}", sha1::Sha1::digest(bytes));
    match_scan_profile(bytes, &sha1, SCAN_PROFILES)
}

pub(crate) fn export_gba_scan(
    bytes: &[u8],
    out: &Path,
    (id, code, sha1): ScanProfile<'_>,
) -> Result<()> {
    let images = out.join(id).join("images");
    let counts = crate::images::export(bytes, &images)?;
    let index = serde_json::json!({
        "profile": id,
        "game_code": std::str::from_utf8(code)?,
        "revision": bytes[0xbc],
        "sha1": sha1,
        "sha256": format!("{:x}", Sha256::digest(bytes)),
        "structural_matches": counts.structural,
        "decoded_candidates": counts.decoded,
        "failed_candidates": counts.failed,
    });
    std::fs::write(
        images.join("scan.json"),
        serde_json::to_string_pretty(&index)?,
    )?;
    println!(
        "{id}: {} ({} decoded image candidates)",
        images.display(),
        counts.decoded
    );
    Ok(())
}

#[derive(Debug)]
pub struct VerifiedRom {
    pub game: Game,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

pub fn verify_rom(path: &Path) -> Result<VerifiedRom> {
    let (profile, bytes) = verify_supported_rom(path)?;
    let game = profile.game.context("ROM has no complete extractor")?;
    Ok(VerifiedRom {
        game,
        bytes,
        sha256: profile.sha256.to_owned(),
    })
}

pub(crate) fn verify_supported_rom(path: &Path) -> Result<(&'static RomProfile, Vec<u8>)> {
    read_profile(path, PROFILES)
}

fn read_profile(
    path: &Path,
    profiles: &'static [RomProfile],
) -> Result<(&'static RomProfile, Vec<u8>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading ROM {}", path.display()))?;
    let profile = match_profile(&bytes, profiles)?;
    Ok((profile, bytes))
}

pub(crate) fn match_profile(
    bytes: &[u8],
    profiles: &'static [RomProfile],
) -> Result<&'static RomProfile> {
    if bytes.len() <= 0xbc {
        bail!("ROM is too short to contain a GBA game code and revision");
    }
    let code = &bytes[0xac..0xb0];
    let revision = bytes[0xbc];
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let mut matching_code = profiles
        .iter()
        .filter(|p| p.code.as_bytes() == code)
        .peekable();
    if matching_code.peek().is_none() {
        bail!(
            "unsupported game code {:?}, revision {revision}, SHA-256 {sha256}",
            String::from_utf8_lossy(code)
        );
    }
    let mut matching_revision = matching_code
        .clone()
        .filter(|p| p.revision == revision)
        .peekable();
    if matching_revision.peek().is_none() {
        let revisions = matching_code
            .map(|p| p.revision)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|revision| revision.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "unsupported {} revision {revision}; registered revisions: {revisions}; SHA-256 {sha256}",
            String::from_utf8_lossy(code)
        );
    }
    let profile = matching_revision
        .find(|p| p.sha256 == sha256)
        .with_context(|| {
            format!(
                "{} revision {revision} SHA-256 mismatch: {sha256}",
                String::from_utf8_lossy(code)
            )
        })?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_profiles_require_a_hash_match_and_emit_a_candidate_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = vec![0; 0xbd];
        bytes[0xac..0xb0].copy_from_slice(b"BBQJ");
        assert!(catalog_scan_profile(&bytes).is_none());
        assert!(match_scan_profile(&bytes, SCAN_PROFILES[0].2, SCAN_PROFILES).is_none());

        let sha1 = format!("{:x}", sha1::Sha1::digest(&bytes));
        let profiles = [("fixture", b"BBQJ", sha1.as_str())];
        let profile = match_scan_profile(&bytes, &sha1, &profiles).unwrap();
        export_gba_scan(&bytes, dir.path(), profile).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("fixture/images/scan.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["game_code"], "BBQJ");
        assert_eq!(manifest["sha1"], sha1);
        assert_eq!(manifest["decoded_candidates"], 0);
    }

    #[test]
    fn rejects_unknown_header() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("unknown.gba");
        let mut bytes = vec![0; 0xbd];
        bytes[0xac..0xb0].copy_from_slice(b"XXXX");
        fs::write(&path, bytes).unwrap();
        assert!(
            verify_rom(&path)
                .unwrap_err()
                .to_string()
                .contains("unsupported game code")
        );
    }

    #[test]
    fn reports_fingerprint_for_wrong_revision() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wrong-revision.gba");
        let mut bytes = vec![0; 0xbd];
        bytes[0xac..0xb0].copy_from_slice(b"AP4J");
        bytes[0xbc] = 2;
        fs::write(&path, &bytes).unwrap();
        let error = verify_rom(&path).unwrap_err().to_string();
        assert!(error.contains("AP4J revision 2; registered revisions: 0, 1"));
        assert!(error.contains(&format!("{:x}", Sha256::digest(&bytes))));
    }

    #[test]
    fn rejects_matching_header_with_wrong_digest() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("modified.gba");
        let mut bytes = vec![0; 0xbd];
        bytes[0xac..0xb0].copy_from_slice(b"BP6J");
        fs::write(&path, bytes).unwrap();
        assert!(
            verify_rom(&path)
                .unwrap_err()
                .to_string()
                .contains("SHA-256 mismatch")
        );
    }

    #[test]
    fn accepts_verified_local_roms_when_available() {
        for (expected_game, expected_digest) in [
            (
                Game::Pp4,
                "1f1746ecc664e6c56c41197755822236c2552fe3c1ceaf8c7192ef93a05bd236",
            ),
            (
                Game::Pp6,
                "72b6a3913d69710f74e9f96c7175a42901cfac486e623c2af6750bb4d487b157",
            ),
        ] {
            let Some(path) = crate::test_assets::rom(expected_game.id()) else {
                continue;
            };
            let rom = verify_rom(&path).unwrap();
            assert_eq!(rom.game, expected_game);
            assert_eq!(rom.sha256, expected_digest);
            assert_eq!(rom.bytes.len(), 8 * 1024 * 1024);
        }
    }
}
