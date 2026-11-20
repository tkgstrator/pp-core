use crate::rom::{Game, verify_rom};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "ppextractor",
    about = "Extract assets from verified PP4 and PP6 GBA ROMs"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List the exact supported ROM revisions and SHA-256 hashes.
    Supported,
    /// Extract assets from one verified ROM.
    Extract {
        /// Path to a PP4 or PP6 Japanese GBA ROM.
        rom: PathBuf,
        /// Output root; files are placed under its 4 or 6 directory.
        #[arg(long)]
        out: PathBuf,
        /// Asset category to extract.
        #[arg(long, value_enum, default_value_t = Only::All)]
        only: Only,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Only {
    All,
    Images,
    Fonts,
    Scripts,
    Portraits,
    Audio,
}

impl Only {
    /// Output directory relative to the game folder (`4` or `6`).
    pub const fn directory(self) -> &'static str {
        match self {
            Self::All => "",
            Self::Images => "images",
            Self::Fonts => "fonts",
            Self::Scripts => "scripts",
            Self::Portraits => "images/portraits",
            Self::Audio => "audio",
        }
    }
}

/// Output folder name under the output root: `4` for PP4, `6` for PP6.
pub const fn game_dir(game: Game) -> &'static str {
    match game {
        Game::Pp4 => "4",
        Game::Pp6 => "6",
    }
}

pub fn supported_text() -> String {
    [Game::Pp4, Game::Pp6]
        .into_iter()
        .map(|game| {
            format!(
                "{}\t{}\trevision {}\tSHA-256 {}",
                game.id(),
                game.code(),
                game.revision(),
                game.expected_sha256()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn execute(
    cli: Cli,
    mut extract: impl FnMut(Only, &[u8], &Path, Game) -> Result<()>,
) -> Result<()> {
    match cli.command {
        Command::Supported => println!("{}", supported_text()),
        Command::Extract { rom, out, only } => {
            let verified = verify_rom(&rom)?;
            let categories: &[Only] = match only {
                Only::All => &[
                    Only::Images,
                    Only::Fonts,
                    Only::Scripts,
                    Only::Portraits,
                    Only::Audio,
                ],
                _ => std::slice::from_ref(&only),
            };
            for &category in categories {
                let path = out.join(game_dir(verified.game)).join(category.directory());
                std::fs::create_dir_all(&path)
                    .with_context(|| format!("creating output directory {}", path.display()))?;
                extract(category, &verified.bytes, &path, verified.game).with_context(|| {
                    format!(
                        "extracting {} from {}",
                        category.directory(),
                        verified.game.id()
                    )
                })?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use std::cell::Cell;
    use std::fs;

    #[test]
    fn parses_requested_category_and_requires_output_root() {
        let cli = Cli::try_parse_from([
            "ppextractor",
            "extract",
            "rom.gba",
            "--out",
            "result",
            "--only",
            "scripts",
        ])
        .unwrap();
        match cli.command {
            Command::Extract { rom, out, only } => {
                assert_eq!(rom, PathBuf::from("rom.gba"));
                assert_eq!(out, PathBuf::from("result"));
                assert_eq!(only, Only::Scripts);
            }
            _ => panic!("expected extract command"),
        }
        assert!(Cli::try_parse_from(["ppextractor", "extract", "rom.gba"]).is_err());
        Cli::command().debug_assert();
    }

    #[test]
    fn supported_lists_exact_revisions_and_hashes() {
        let text = supported_text();
        assert!(text.contains("pp4\tAP4J\trevision 1\tSHA-256 1f1746ecc664e6c56c41197755822236c2552fe3c1ceaf8c7192ef93a05bd236"));
        assert!(text.contains("pp6\tBP6J\trevision 0\tSHA-256 72b6a3913d69710f74e9f96c7175a42901cfac486e623c2af6750bb4d487b157"));
    }

    #[test]
    fn category_layout_nests_portraits_under_images() {
        assert_eq!(game_dir(Game::Pp4), "4");
        assert_eq!(game_dir(Game::Pp6), "6");
        assert_eq!(Only::Images.directory(), "images");
        assert_eq!(Only::Portraits.directory(), "images/portraits");
        assert_eq!(Only::Fonts.directory(), "fonts");
        assert_eq!(Only::Scripts.directory(), "scripts");
        assert_eq!(Only::Audio.directory(), "audio");
    }

    #[test]
    fn invalid_rom_does_not_create_output_or_call_extractor() {
        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("invalid.gba");
        let out = temp.path().join("out");
        fs::write(&rom, b"invalid").unwrap();
        let called = Cell::new(false);
        let result = execute(
            Cli {
                command: Command::Extract {
                    rom,
                    out: out.clone(),
                    only: Only::All,
                },
            },
            |_, _, _, _| {
                called.set(true);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!out.exists());
        assert!(!called.get());
    }

    #[test]
    fn verified_rom_uses_game_folder_for_selected_category() {
        let Some(source) = crate::test_assets::rom("pp4") else {
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("out");
        let called = Cell::new(false);
        execute(
            Cli {
                command: Command::Extract {
                    rom: source,
                    out: out.clone(),
                    only: Only::Scripts,
                },
            },
            |category, bytes, path, game| {
                assert_eq!(category, Only::Scripts);
                assert_eq!(bytes.len(), 8 * 1024 * 1024);
                assert_eq!(path, out.join("4/scripts"));
                assert_eq!(game, Game::Pp4);
                assert!(path.is_dir());
                called.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(called.get());
    }

    #[test]
    fn all_extracts_five_categories() {
        let Some(source) = crate::test_assets::rom("pp6") else {
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let mut actual = Vec::new();
        execute(
            Cli {
                command: Command::Extract {
                    rom: source,
                    out: temp.path().to_path_buf(),
                    only: Only::All,
                },
            },
            |category, _, path, game| {
                assert_eq!(game, Game::Pp6);
                let rel = path.strip_prefix(temp.path().join("6")).unwrap();
                assert_eq!(rel, Path::new(category.directory()));
                actual.push(category);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            actual,
            [
                Only::Images,
                Only::Fonts,
                Only::Scripts,
                Only::Portraits,
                Only::Audio
            ]
        );
    }
}
