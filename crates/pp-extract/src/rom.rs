use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Game {
    Pp4,
    Pp6,
}

impl Game {
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

#[derive(Debug)]
pub struct VerifiedRom {
    pub game: Game,
    pub bytes: Vec<u8>,
    pub sha256: String,
}

pub fn verify_rom(path: &Path) -> Result<VerifiedRom> {
    let bytes = std::fs::read(path).with_context(|| format!("reading ROM {}", path.display()))?;
    if bytes.len() <= 0xbc {
        bail!("ROM is too short to contain a GBA game code and revision");
    }
    let code = &bytes[0xac..0xb0];
    let game = match code {
        b"AP4J" => Game::Pp4,
        b"BP6J" => Game::Pp6,
        _ => bail!(
            "unsupported game code {:?}; expected AP4J or BP6J",
            String::from_utf8_lossy(code)
        ),
    };
    if bytes[0xbc] != game.revision() {
        bail!(
            "expected {} revision {}; got revision {}",
            game.code(),
            game.revision(),
            bytes[0xbc]
        );
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    if sha256 != game.expected_sha256() {
        bail!("{} SHA-256 mismatch: {sha256}", game.code());
    }
    Ok(VerifiedRom {
        game,
        bytes,
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
    fn rejects_wrong_revision_before_digest_check() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("wrong-revision.gba");
        let mut bytes = vec![0; 0xbd];
        bytes[0xac..0xb0].copy_from_slice(b"AP4J");
        bytes[0xbc] = 0;
        fs::write(&path, bytes).unwrap();
        assert!(
            verify_rom(&path)
                .unwrap_err()
                .to_string()
                .contains("revision 1")
        );
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
        for (file, expected_game, expected_digest) in [
            (
                "pp4.gba",
                Game::Pp4,
                "1f1746ecc664e6c56c41197755822236c2552fe3c1ceaf8c7192ef93a05bd236",
            ),
            (
                "pp6.gba",
                Game::Pp6,
                "72b6a3913d69710f74e9f96c7175a42901cfac486e623c2af6750bb4d487b157",
            ),
        ] {
            let Some(path) = crate::test_assets::rom(expected_game.id()) else {
                continue;
            };
            assert_eq!(path.file_name().unwrap(), file);
            let rom = verify_rom(&path).unwrap();
            assert_eq!(rom.game, expected_game);
            assert_eq!(rom.sha256, expected_digest);
            assert_eq!(rom.bytes.len(), 8 * 1024 * 1024);
        }
    }
}
