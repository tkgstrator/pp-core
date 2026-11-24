use crate::rom::{Game, PROFILES, RomProfile, match_profile};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "ppextractor",
    about = "Extract assets from Power Pro Kun Pocket ROMs"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List the exact supported ROM revisions and SHA-256 hashes.
    Supported {
        /// Include catalogued revisions and their current extraction entry points.
        #[arg(long)]
        catalog: bool,
    },
    /// Identify ROM files using full-image SHA-1 and SHA-256 hashes.
    Identify {
        #[arg(required = true)]
        roms: Vec<PathBuf>,
    },
    /// Extract raw cartridge files from a Power Pro Kun Pocket Nintendo DS ROM.
    NdsFiles {
        rom: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Inspect a known byte range of a Power Pro Kun Pocket DS Success script.
    NdsDialogue {
        input: PathBuf,
        #[arg(long, value_parser = crate::gb::parse_offset)]
        offset: usize,
        #[arg(long, value_parser = crate::gb::parse_offset)]
        length: usize,
        #[arg(long, value_enum)]
        game: Option<crate::nds::DialogueGame>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Find candidate occurrences of a known line in a ROM, extracted file, or extraction directory.
    NdsFindDialogue {
        input: PathBuf,
        #[arg(long)]
        text: String,
        #[arg(long, value_enum)]
        game: Option<crate::nds::DialogueGame>,
    },
    /// Render a DS background from explicitly selected NSCR, NCGR, and NCLR files.
    NdsBackground {
        #[arg(long)]
        screen: PathBuf,
        #[arg(long)]
        tiles: PathBuf,
        #[arg(long)]
        palette: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Extract raw ROM banks from a catalogued GB/GBC Power Pro Kun Pocket cartridge.
    GbBanks {
        rom: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Inspect a known Success dialogue range from Game Boy PP1 or PP2.
    GbDialogue {
        input: PathBuf,
        #[arg(long, value_parser = crate::gb::parse_offset)]
        offset: usize,
        /// Byte count. If omitted, stop at the first parsed event terminator.
        #[arg(long, value_parser = crate::gb::parse_offset)]
        length: Option<usize>,
        /// Use PP2's extended character assignments.
        #[arg(long)]
        pp2: bool,
        #[arg(long)]
        out: PathBuf,
    },
    /// Find a known Success dialogue line in a Game Boy PP1 or PP2 ROM or bank.
    GbFindDialogue {
        input: PathBuf,
        #[arg(long)]
        text: String,
        /// Use PP2's extended character assignments.
        #[arg(long)]
        pp2: bool,
    },
    /// Preview 2bpp tiles at an explicit offset in a catalogued GB/GBC ROM.
    GbTiles {
        rom: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_parser = crate::gb::parse_offset)]
        offset: usize,
        #[arg(long)]
        count: usize,
    },
    /// Probe catalogued GBA ROMs that lack a verified background layout.
    GbaScan {
        rom: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Index the partitions and raw file ranges of a Switch cartridge image.
    XciIndex {
        rom: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Copy the raw files in this partition into the output directory.
        #[arg(long)]
        copy_partition: Option<String>,
    },
    /// Extract a named program NCA from an XCI secure partition and decode its sections.
    XciFiles {
        input: PathBuf,
        #[arg(long)]
        program: String,
        #[arg(long)]
        keyset: PathBuf,
        #[arg(long, default_value = "hactool")]
        hactool: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        decode_main: bool,
        #[arg(long)]
        decode_res00: bool,
    },
    /// Index file names and byte ranges in a Switch NSP package.
    NspIndex {
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Use local hactool and a keyset to extract a Switch NCA.
    NcaFiles {
        input: PathBuf,
        /// NCA filename when the input is an NSP package.
        #[arg(long, visible_alias = "content")]
        program: Option<String>,
        /// Base program NCA for an update NCA with a BKTR RomFS.
        #[arg(long)]
        base_nca: Option<PathBuf>,
        #[arg(long)]
        keyset: PathBuf,
        #[arg(long, default_value = "hactool")]
        hactool: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        decode_main: bool,
        #[arg(long)]
        decode_res00: bool,
    },
    /// Extract files from an already decrypted Switch ExeFS PFS0 image.
    ExefsFiles {
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Also extract the segments of the main NSO0 executable.
        #[arg(long)]
        decode_main: bool,
    },
    /// Extract segments from an already decrypted Switch ExeFS NSO0 file.
    NsoSegments {
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Extract files from an already decrypted Switch RomFS image.
    RomfsFiles {
        image: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Try decoding each unambiguous RES00.RDI and RES00.RDB pair found in RomFS.
        #[arg(long)]
        decode_res00: bool,
    },
    /// Copy raw resources using a Switch RES00 index.
    Res00Files {
        #[arg(long)]
        index: PathBuf,
        #[arg(long)]
        data: PathBuf,
        /// Optional RES11.RDB archive for raw patch ranges.
        #[arg(long)]
        patch_data: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        /// Try the published RES00 chunk transform and inflate supported payloads.
        #[arg(long)]
        decode_chunks: bool,
    },
    /// Extract assets from a GBA ROM, GB/GBC cartridge, or Nintendo DS ROM.
    Extract {
        /// Path to a supported Japanese ROM.
        rom: PathBuf,
        /// Output root containing one directory per ROM profile.
        #[arg(long)]
        out: PathBuf,
        /// Asset category for a GBA ROM.
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
    Samples,
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
            Self::Audio | Self::Samples => "audio",
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

const CATEGORIES: &[Only] = &[
    Only::Images,
    Only::Fonts,
    Only::Scripts,
    Only::Portraits,
    Only::Audio,
];

fn available_categories(profile: &RomProfile) -> Vec<Only> {
    if profile.game.is_some() {
        return CATEGORIES.to_vec();
    }
    let mut categories = vec![Only::Images];
    if profile.font.is_some() {
        categories.push(Only::Fonts);
    }
    if !profile.script_scan.is_empty() {
        categories.push(Only::Scripts);
    }
    if !profile.busts.is_empty() || !profile.portrait_layers.is_empty() {
        categories.push(Only::Portraits);
    }
    if profile.native_audio.is_some() {
        categories.push(Only::Audio);
    } else if profile.samples.is_some() {
        categories.push(Only::Samples);
    }
    categories
}

fn category_names(categories: &[Only]) -> String {
    categories
        .iter()
        .map(|category| category.to_possible_value().unwrap().get_name().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn supported_text() -> String {
    PROFILES
        .iter()
        .map(|profile| {
            let categories = category_names(&available_categories(profile));
            let note = if profile.script_listing.is_some() {
                " (scripts: native bodies and provisional listings)"
            } else if !profile.script_scan.is_empty() {
                " (scripts: native bodies)"
            } else {
                ""
            };
            let audio_note = match &profile.native_audio {
                Some(layout) if layout.playback.is_none() => " (audio: native sequences and PCM)",
                _ => "",
            };
            format!(
                "{}\t{}\trevision {}\tSHA-256 {}\tcategories: {categories}{note}{audio_note}",
                profile.id, profile.code, profile.revision, profile.sha256
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(serde::Serialize)]
struct CatalogEntry {
    platform: &'static str,
    profile: String,
    game_code: Option<&'static str>,
    revision: Option<u8>,
    hash_type: &'static str,
    hash: &'static str,
    command: &'static str,
    coverage: String,
}

fn catalog_entries() -> Vec<CatalogEntry> {
    let mut entries = Vec::new();
    entries.extend(PROFILES.iter().map(|profile| CatalogEntry {
        platform: "GBA",
        profile: profile.id.into(),
        game_code: Some(profile.code),
        revision: Some(profile.revision),
        hash_type: "SHA-256",
        hash: profile.sha256,
        command: "extract",
        coverage: category_names(&available_categories(profile)),
    }));
    entries.extend(
        crate::rom::SCAN_PROFILES
            .iter()
            .map(|&(id, code, hash)| CatalogEntry {
                platform: "GBA",
                profile: id.into(),
                game_code: Some(std::str::from_utf8(code).unwrap()),
                revision: Some(if id == "pp5_v1" { 1 } else { 0 }),
                hash_type: "SHA-1",
                hash,
                command: "extract",
                coverage: "candidate scans, fixture pending".into(),
            }),
    );
    entries.extend(crate::gb::PROFILES.iter().map(|&(id, hash)| {
        CatalogEntry {
            platform: "GBC",
            profile: id.into(),
            game_code: None,
            revision: id
                .rsplit_once("_v")
                .and_then(|(_, revision)| revision.parse().ok()),
            hash_type: "SHA-1",
            hash,
            command: "gb-banks",
            coverage: "raw banks, fixture pending".into(),
        }
    }));
    entries.extend(
        crate::nds::CATALOG
            .iter()
            .map(|&(code, revision, _, hash)| {
                let id = crate::nds::profile_id(code).unwrap();
                CatalogEntry {
                    platform: "DS",
                    profile: format!("{id}_v{revision}"),
                    game_code: Some(std::str::from_utf8(code).unwrap()),
                    revision: Some(revision),
                    hash_type: "SHA-1",
                    hash,
                    command: "nds-files",
                    coverage: "Nitro assets, fixture pending".into(),
                }
            }),
    );
    entries.push(CatalogEntry {
        platform: "Switch",
        profile: "pp_r_jp".into(),
        game_code: Some("A4L4A"),
        revision: None,
        hash_type: "SHA-1",
        hash: crate::switch::PP_R_SHA1,
        command: "xci-files",
        coverage: "NCA, ExeFS and RomFS, fixture pending".into(),
    });
    entries
}

fn catalog_text() -> String {
    use std::fmt::Write as _;
    let mut text =
        String::from("platform\tprofile\tgame_code\trevision\thash_type\thash\tcommand\tcoverage");
    for entry in catalog_entries() {
        let revision = entry
            .revision
            .map(|value| value.to_string())
            .unwrap_or_default();
        write!(
            text,
            "\n{}\t{}\t{}\t{revision}\t{}\t{}\t{}\t{}",
            entry.platform,
            entry.profile,
            entry.game_code.unwrap_or_default(),
            entry.hash_type,
            entry.hash,
            entry.command,
            entry.coverage
        )
        .unwrap();
    }
    text
}

fn identify(roms: &[PathBuf]) -> Result<Vec<serde_json::Value>> {
    use sha2::Digest;
    use std::io::Read;
    let catalog = catalog_entries();
    let mut buffer = vec![0; 1024 * 1024];
    roms.iter()
        .map(|path| {
            let mut file =
                std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
            let mut sha1 = sha1::Sha1::new();
            let mut sha256 = sha2::Sha256::new();
            let mut bytes = 0u64;
            loop {
                let count = file
                    .read(&mut buffer)
                    .with_context(|| format!("reading {}", path.display()))?;
                if count == 0 {
                    break;
                }
                bytes += count as u64;
                sha1.update(&buffer[..count]);
                sha256.update(&buffer[..count]);
            }
            let sha1 = format!("{:x}", sha1.finalize());
            let sha256 = format!("{:x}", sha256.finalize());
            let matched = catalog.iter().find(|entry| {
                if entry.hash_type == "SHA-256" {
                    entry.hash == sha256
                } else {
                    entry.hash == sha1
                }
            });
            Ok(serde_json::json!({
                "path": path,
                "bytes": bytes,
                "sha1": sha1,
                "sha256": sha256,
                "catalog": matched,
            }))
        })
        .collect()
}

fn extract_scanned_fonts(rom: &[u8], out: &Path, profile: &str) -> Result<usize> {
    let tables = crate::fonts::discover_tables(rom);
    let root = out.join(profile).join(Only::Fonts.directory());
    for &(format, table) in &tables {
        let path = if tables.len() == 1 {
            root.clone()
        } else {
            root.join(format!("table_{table:08x}"))
        };
        crate::fonts::export(rom, &path, profile, format, table)?;
        println!("{profile}: {} (font table at {table:#x})", path.display());
    }
    Ok(tables.len())
}

fn extract_scanned_audio(rom: &[u8], out: &Path, profile: &str, only: Only) -> Result<()> {
    let banks = crate::audio::discover_sample_banks(rom);
    if banks.is_empty() {
        let detail = format!("{profile}: no compatible indexed PCM banks");
        if only != Only::All {
            bail!("{detail}");
        }
        eprintln!("{detail}");
        return Ok(());
    }
    let root = out.join(profile).join(Only::Audio.directory());
    let mut exported_native = false;
    for &bank in &banks {
        let (start, _, count, index) = bank;
        let path = if banks.len() == 1 {
            root.clone()
        } else {
            root.join(format!("bank_{start:08x}_{index:08x}"))
        };
        if only != Only::Samples
            && profile == "pp5_v1"
            && let Some(layout) = crate::audio::discover_pp5_audio(rom, bank)
        {
            crate::audio::export_native_audio(rom, &path, bank, &layout)?;
            println!(
                "{profile}: {} (PP5 BGM, effects and samples)",
                path.display()
            );
            exported_native = true;
        } else if only != Only::Audio {
            crate::audio::export_samples(rom, &path, bank)?;
            println!(
                "{profile}: {} ({count} indexed PCM samples)",
                path.display()
            );
        }
    }
    if only == Only::Audio && !exported_native {
        bail!("{profile}: no verified native audio layout; try --only samples");
    }
    Ok(())
}

pub fn execute(
    cli: Cli,
    mut extract: impl FnMut(Only, &[u8], &Path, Game) -> Result<()>,
) -> Result<()> {
    match cli.command {
        Command::Supported { catalog } => println!(
            "{}",
            if catalog {
                catalog_text()
            } else {
                supported_text()
            }
        ),
        Command::Identify { roms } => {
            println!("{}", serde_json::to_string_pretty(&identify(&roms)?)?);
        }
        Command::NdsFiles { rom, out } => crate::nds::extract(&rom, &out)?,
        Command::NdsDialogue {
            input,
            offset,
            length,
            game,
            out,
        } => crate::nds::export_dialogue(&input, offset, length, game, &out)?,
        Command::NdsFindDialogue { input, text, game } => {
            crate::nds::find_dialogue(&input, &text, game)?
        }
        Command::NdsBackground {
            screen,
            tiles,
            palette,
            out,
        } => crate::nds::render_background(&screen, &tiles, &palette, &out)?,
        Command::GbBanks { rom, out } => crate::gb::extract(&rom, &out)?,
        Command::GbDialogue {
            input,
            offset,
            length,
            pp2,
            out,
        } => crate::gb::export_dialogue(&input, offset, length, pp2, &out)?,
        Command::GbFindDialogue { input, text, pp2 } => {
            crate::gb::find_dialogue(&input, &text, pp2)?
        }
        Command::GbTiles {
            rom,
            out,
            offset,
            count,
        } => crate::gb::export_tiles(&rom, &out, offset, count)?,
        Command::GbaScan { rom, out } => crate::rom::scan_gba(&rom, &out)?,
        Command::XciIndex {
            rom,
            out,
            copy_partition,
        } => crate::switch::index(&rom, &out, copy_partition.as_deref())?,
        Command::XciFiles {
            input,
            program,
            keyset,
            hactool,
            out,
            decode_main,
            decode_res00,
        } => {
            let temp = tempfile::Builder::new().prefix("pp-xci-").tempdir()?;
            let nca = temp.path().join("program.nca");
            crate::switch::copy_secure_nca(&input, &program, &nca)?;
            crate::switch::nca::extract(
                &nca,
                None,
                &keyset,
                &hactool,
                &out,
                decode_main,
                decode_res00,
            )?;
        }
        Command::NspIndex { input, out } => crate::switch::exefs::index_nsp(&input, &out)?,
        Command::NcaFiles {
            input,
            program,
            base_nca,
            keyset,
            hactool,
            out,
            decode_main,
            decode_res00,
        } => {
            let package = if let Some(program) = program {
                let temp = tempfile::Builder::new().prefix("pp-nsp-").tempdir()?;
                let nca = temp.path().join("program.nca");
                crate::switch::exefs::copy_nsp_nca(&input, &program, &nca)?;
                Some((temp, nca))
            } else {
                None
            };
            let nca = package
                .as_ref()
                .map_or(input.as_path(), |(_, nca)| nca.as_path());
            crate::switch::nca::extract(
                nca,
                base_nca.as_deref(),
                &keyset,
                &hactool,
                &out,
                decode_main,
                decode_res00,
            )?;
        }
        Command::ExefsFiles {
            input,
            out,
            decode_main,
        } => crate::switch::exefs::extract(&input, &out, decode_main)?,
        Command::NsoSegments { input, out } => crate::switch::nso::extract(&input, &out)?,
        Command::RomfsFiles {
            image,
            out,
            decode_res00,
        } => crate::switch::romfs::extract(&image, &out, decode_res00)?,
        Command::Res00Files {
            index,
            data,
            out,
            decode_chunks,
            patch_data,
        } => {
            use crate::switch::res00::PatchArchive;
            let patch = patch_data
                .as_deref()
                .map_or(PatchArchive::Absent, PatchArchive::Required);
            crate::switch::res00::extract(&index, &data, &out, decode_chunks, patch)?;
        }
        Command::Extract { rom, out, only } => {
            if let Some(extension) = rom.extension().and_then(|ext| ext.to_str()) {
                let platform = match extension.to_ascii_lowercase().as_str() {
                    "gb" | "gbc" => Some((
                        "GB/GBC",
                        crate::gb::extract as fn(&Path, &Path) -> Result<()>,
                    )),
                    "nds" => Some((
                        "Nintendo DS",
                        crate::nds::extract as fn(&Path, &Path) -> Result<()>,
                    )),
                    _ => None,
                };
                if let Some((name, extract_rom)) = platform {
                    if only != Only::All {
                        bail!("{name} extraction supports --only all");
                    }
                    return extract_rom(&rom, &out);
                }
            }

            let bytes =
                std::fs::read(&rom).with_context(|| format!("reading ROM {}", rom.display()))?;
            let profile = match match_profile(&bytes, PROFILES) {
                Ok(profile) => profile,
                Err(error) => {
                    if let Some((profile, sha1)) = crate::gb::detect(&bytes) {
                        if only != Only::All {
                            bail!("GB/GBC extraction supports --only all");
                        }
                        return crate::gb::extract_bytes(&bytes, &out, profile, &sha1);
                    }
                    if crate::nds::detect(&bytes) {
                        if only != Only::All {
                            bail!("Nintendo DS extraction supports --only all");
                        }
                        return crate::nds::extract_bytes(&bytes, &out);
                    }
                    let Some(scan) = crate::rom::catalog_scan_profile(&bytes) else {
                        return Err(error);
                    };
                    if !matches!(
                        only,
                        Only::All
                            | Only::Images
                            | Only::Fonts
                            | Only::Scripts
                            | Only::Portraits
                            | Only::Samples
                            | Only::Audio
                    ) {
                        bail!(
                            "{}: requested {only:?}; scan categories: images, fonts, scripts, portraits, and samples",
                            scan.0
                        );
                    }
                    if matches!(only, Only::All | Only::Images) {
                        crate::rom::export_gba_scan(&bytes, &out, scan)?;
                    }
                    if matches!(only, Only::All | Only::Audio | Only::Samples) {
                        extract_scanned_audio(&bytes, &out, scan.0, only)?;
                    }
                    if matches!(only, Only::All | Only::Fonts)
                        && extract_scanned_fonts(&bytes, &out, scan.0)? == 0
                    {
                        let detail = format!("{}: no compatible font tables", scan.0);
                        if only == Only::Fonts {
                            bail!("{detail}");
                        }
                        eprintln!("{detail}");
                    }
                    if matches!(only, Only::All | Only::Portraits) {
                        let banks = crate::portraits::discover_busts(&bytes);
                        if banks.is_empty() {
                            let detail = format!("{}: no compatible 56x56 portrait banks", scan.0);
                            if only == Only::Portraits {
                                bail!("{detail}");
                            }
                            eprintln!("{detail}");
                        } else {
                            let path = out.join(scan.0).join(Only::Portraits.directory());
                            crate::portraits::export_busts(
                                &bytes,
                                &path,
                                &banks,
                                "scanned 56x56 OBJ bust banks; likely dialogue portraits",
                            )?;
                            println!(
                                "{}: {} ({} portrait banks)",
                                scan.0,
                                path.display(),
                                banks.len()
                            );
                        }
                    }
                    if matches!(only, Only::All | Only::Scripts) {
                        let path = out.join(scan.0).join(Only::Scripts.directory());
                        let listing = if scan.0 == "pp5_v1" {
                            crate::scripts::discover_pp5_listing(&bytes)
                        } else {
                            None
                        };
                        let count = crate::scripts::export_scanned_events(&bytes, &path, listing)?;
                        if count == 0 {
                            let detail =
                                format!("{}: no referenced event candidates found", scan.0);
                            if only == Only::Scripts {
                                bail!("{detail}");
                            }
                            eprintln!("{detail}");
                        } else {
                            println!("{}: {} ({count} script candidates)", scan.0, path.display());
                        }
                    }
                    return Ok(());
                }
            };
            let available = available_categories(profile);
            let categories = if only == Only::All {
                if available != CATEGORIES {
                    eprintln!(
                        "{}: available categories: {}",
                        profile.id,
                        category_names(&available)
                    );
                }
                available.as_slice()
            } else if available.contains(&only)
                || (only == Only::Samples && available.contains(&Only::Audio))
            {
                std::slice::from_ref(&only)
            } else {
                bail!(
                    "{}: requested {only:?}; available categories: {}",
                    profile.id,
                    category_names(&available)
                );
            };
            let dir = profile
                .game
                .filter(|game| game.id() == profile.id)
                .map_or(profile.id, game_dir);
            for &category in categories {
                let path = out.join(dir).join(category.directory());
                std::fs::create_dir_all(&path)
                    .with_context(|| format!("creating output directory {}", path.display()))?;
                let result = match (profile.game, category) {
                    (game, Only::Samples) => {
                        let bank = match game {
                            Some(game) => crate::audio::sample_params(
                                game,
                                game == Game::Pp4 && profile.revision == 0,
                            ),
                            None => profile.samples.context("missing PCM bank")?,
                        };
                        crate::audio::export_samples(&bytes, &path, bank)
                    }
                    (Some(game), _) => extract(category, &bytes, &path, game),
                    (None, Only::Audio) => crate::audio::export_native_audio(
                        &bytes,
                        &path,
                        profile.samples.context("missing PCM bank")?,
                        profile
                            .native_audio
                            .as_ref()
                            .context("missing native audio layout")?,
                    ),
                    (None, Only::Images) => crate::images::export(&bytes, &path).map(|_| ()),
                    (None, Only::Fonts) => {
                        let (layout, table) = profile.font.context("missing font layout")?;
                        crate::fonts::export(&bytes, &path, profile.id, layout, table)
                    }
                    (None, Only::Scripts) => crate::scripts::export_native_events(
                        &bytes,
                        &path,
                        profile.script_scan,
                        profile.script_listing,
                    ),
                    (None, Only::Portraits) if !profile.portrait_layers.is_empty() => {
                        crate::portraits::export_layers(
                            &bytes,
                            &path,
                            profile.portrait_layers,
                            profile.portrait_remaps,
                        )
                    }
                    (None, Only::Portraits) => crate::portraits::export_busts(
                        &bytes,
                        &path,
                        profile.busts,
                        "structurally verified 56x56 OBJ bust banks; likely dialogue portraits",
                    )
                    .map(|_| ()),
                    _ => bail!("missing extractor for {category:?}"),
                };
                result.with_context(|| {
                    format!("extracting {} from {}", category.directory(), profile.id)
                })?;
                if profile.game.is_none() {
                    println!("{}: {}", profile.id, path.display());
                }
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
    fn exports_each_discovered_font_table() {
        const STRIDE: usize = 0x1200;
        const PIXELS: usize = 0x1000;
        let tables = [0x100, 0x300];
        let mut rom = vec![0; PIXELS + 58 * STRIDE];
        rom[PIXELS] = 0x22;
        for table in tables {
            for page in 0..67 {
                let physical_page = match page {
                    55..=63 => 0,
                    64..=66 => page - 9,
                    _ => page,
                };
                let pointer = (0x0800_0000 + PIXELS + physical_page * STRIDE) as u32;
                let entry = table + page * 4;
                rom[entry..entry + 4].copy_from_slice(&pointer.to_le_bytes());
            }
        }
        let out = tempfile::tempdir().unwrap();
        assert_eq!(
            extract_scanned_fonts(&rom, out.path(), "ppdash").unwrap(),
            2
        );
        let fonts = out.path().join("ppdash/fonts");
        assert_eq!(fs::read_dir(&fonts).unwrap().count(), 2);
        for table in tables {
            let path = fonts.join(format!("table_{table:08x}"));
            for file in ["glyphs.png", "glyphs.json", "dialogue.otf", "dialogue.ttf"] {
                assert!(fs::metadata(path.join(file)).unwrap().len() > 0);
            }
        }
    }

    #[test]
    fn exports_each_discovered_pcm_bank() {
        let mut rom = vec![0; 8 * 1024 * 1024];
        let banks = [(0x1000, 0x5000, 1), (0x2000, 0x6000, 4)];
        for (start, index, sample_length) in banks {
            let stride = (20 + sample_length) & !3;
            for id in 0..32 {
                let offset = id * stride;
                let pos = start + offset;
                rom[pos + 4..pos + 8].copy_from_slice(&(8000u32 * 1024).to_le_bytes());
                rom[pos + 12..pos + 16].copy_from_slice(&(sample_length as u32).to_le_bytes());
                rom[pos + 16..pos + 16 + sample_length].fill(0x42);
                let entry = index + id * 4;
                rom[entry..entry + 4].copy_from_slice(&(offset as u32).to_le_bytes());
            }
        }
        let out = tempfile::tempdir().unwrap();
        extract_scanned_audio(&rom, out.path(), "ppdash", Only::Samples).unwrap();
        let audio = out.path().join("ppdash/audio");
        assert_eq!(fs::read_dir(&audio).unwrap().count(), 2);
        for (start, index, _) in banks {
            let manifest = audio
                .join(format!("bank_{start:08x}_{index:08x}"))
                .join("samples-manifest.json");
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
            assert_eq!(value["samples"].as_array().unwrap().len(), 32);
        }
    }

    #[test]
    fn discovered_pp5_audio_matches_the_existing_export() {
        let Some(source) = crate::test_assets::rom("pp5_v0") else {
            return;
        };
        let Some(expected) = crate::test_assets::oracle("pp5_v0", "audio", "manifest.json") else {
            return;
        };
        let (_, rom) = crate::rom::verify_supported_rom(&source).unwrap();
        let out = tempfile::tempdir().unwrap();
        extract_scanned_audio(&rom, out.path(), "pp5_v1", Only::Audio).unwrap();
        let actual = out.path().join("pp5_v1/audio");

        // The audio manifest records hashes for the rendered tracks, native
        // payloads, and previews. Compare the SoundFont itself as well.
        for file in ["manifest.json", "rom.sf2"] {
            assert_eq!(
                fs::read(actual.join(file)).unwrap(),
                fs::read(expected.join(file)).unwrap(),
                "{file}"
            );
        }
    }

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
    fn supported_lists_image_only_roms_and_available_categories() {
        let text = supported_text();
        assert_eq!(text.lines().count(), 11);
        assert!(text.contains("pp4_v0\tAP4J\trevision 0"));
        assert!(text.contains("categories: images"));
    }

    #[test]
    fn catalog_lists_all_registered_and_catalog_only_revisions() {
        let text = catalog_text();
        let rows: Vec<_> = text
            .lines()
            .skip(1)
            .map(|line| line.split('\t').collect::<Vec<_>>())
            .collect();
        assert_eq!(rows.len(), 28);
        assert!(
            rows.iter()
                .all(|row| row.len() == 8 && row[5].chars().all(|c| c.is_ascii_hexdigit()))
        );
        let hashes = rows
            .iter()
            .map(|row| row[5])
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(hashes.len(), rows.len());
        for (id, command) in [
            ("pp5_v1", "extract"),
            ("pp1_gbc_v1", "gb-banks"),
            ("pp12_v1", "nds-files"),
            ("pp_r_jp", "xci-files"),
        ] {
            assert!(rows.iter().any(|row| row[1] == id
                && row[6] == command
                && row[7].contains("fixture pending")));
        }
    }

    #[test]
    fn identifies_a_registered_rom_independently_of_its_filename() {
        let Some(source) = crate::test_assets::rom("pp4") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("renamed.bin");
        fs::copy(source, &path).unwrap();
        let unknown = dir.path().join("unregistered.gba");
        fs::write(&unknown, b"abc").unwrap();
        let result = identify(&[path, unknown]).unwrap();
        assert_eq!(result[0]["catalog"]["profile"], "pp4");
        assert_eq!(result[0]["catalog"]["revision"], 1);
        assert_eq!(result[0]["bytes"], 8 * 1024 * 1024);
        assert_eq!(
            result[1]["sha1"],
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert!(result[1]["catalog"].is_null());
    }

    #[test]
    fn extract_routes_ds_rom_and_rejects_gba_categories_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("fixture.NDS");
        let out = temp.path().join("out");
        let mut input = vec![0; 0x240];
        input[0x0c..0x10].copy_from_slice(b"VPTJ");
        for (offset, word) in [(0x48, 0x180u32), (0x4c, 8), (0x180, 0x200), (0x184, 0x203)] {
            input[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        input[0x200..0x203].copy_from_slice(b"raw");
        fs::write(&rom, &input).unwrap();
        let cli = |only| Cli {
            command: Command::Extract {
                rom: rom.clone(),
                out: out.clone(),
                only,
            },
        };
        let reject = execute(cli(Only::Scripts), |_, _, _, _| unreachable!()).unwrap_err();
        assert!(reject.to_string().contains("Nintendo DS"));
        assert!(!out.exists());
        execute(cli(Only::All), |_, _, _, _| unreachable!()).unwrap();
        let title = out.join("pp12_v0");
        let dump = fs::read_dir(title).unwrap().next().unwrap().unwrap().path();
        assert_eq!(fs::read(dump.join("0000.bin")).unwrap(), b"raw");
    }

    #[test]
    fn extracts_ds_without_a_filename_extension() {
        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("dump");
        let out = temp.path().join("out");
        let mut input = vec![0; 0x240];
        input[0x0c..0x10].copy_from_slice(b"AP8J");
        for (offset, word) in [(0x48, 0x180u32), (0x4c, 8), (0x180, 0x200), (0x184, 0x203)] {
            input[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        input[0x200..0x203].copy_from_slice(b"raw");
        fs::write(&rom, &input).unwrap();
        let cli = Cli {
            command: Command::Extract {
                rom,
                out: out.clone(),
                only: Only::All,
            },
        };
        execute(cli, |_, _, _, _| unreachable!()).unwrap();
        let dump = fs::read_dir(out.join("pp8_v0"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::read(dump.join("0000.bin")).unwrap(), b"raw");
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
    fn catalog_pp6_uses_its_own_output_folder() {
        let Some(source) = crate::test_assets::rom("pp6") else {
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("out");
        let mut rom = fs::read(source).unwrap();
        // Restore the original Thumb instruction in the existing parity fixture.
        rom[0x44246..0x44248].copy_from_slice(&[0x07, 0xda]);
        let source = temp.path().join("catalog.gba");
        fs::write(&source, &rom).unwrap();
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
                assert_eq!(bytes, rom);
                assert_eq!(game, Game::Pp6);
                assert_eq!(path, out.join("pp6_v0_clean/scripts"));
                called.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(called.get());
        assert!(!out.join("6").exists());
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
