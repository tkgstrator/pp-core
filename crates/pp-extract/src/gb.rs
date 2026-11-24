mod dialogue;
pub(crate) use dialogue::{export_dialogue, find_dialogue};

use anyhow::{Context, Result, ensure};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::fmt::Write;
use std::path::Path;

const BANK_SIZE: usize = 0x4000;
pub(crate) const PROFILES: &[(&str, &str)] = &[
    ("pp1_gbc_v0", "f796f9cb259646555f0d429b9337ac6b423755a3"),
    ("pp1_gbc_v1", "ae145f2de0d53c19b973c71baaf3f2cca2ca395a"),
    ("pp2_gbc_v0", "5c0ba404cf30296dceeb427a54e3644415c9e8db"),
];

fn read_rom(rom: &Path) -> Result<(&'static str, String, Vec<u8>)> {
    let data = std::fs::read(rom).with_context(|| format!("reading ROM {}", rom.display()))?;
    let sha1 = format!("{:x}", Sha1::digest(&data));
    let profile = profile_for_hash(&sha1)
        .with_context(|| format!("unrecognized Power Pro Kun Pocket GB/GBC dump, SHA-1 {sha1}"))?;
    Ok((profile, sha1, data))
}

fn profile_for_hash(sha1: &str) -> Option<&'static str> {
    PROFILES
        .iter()
        .find_map(|&(profile, hash)| (hash == sha1).then_some(profile))
}

pub(crate) fn detect(data: &[u8]) -> Option<(&'static str, String)> {
    if data.len() != 2 * 1024 * 1024 {
        return None;
    }
    let sha1 = format!("{:x}", Sha1::digest(data));
    Some((profile_for_hash(&sha1)?, sha1))
}

pub(crate) fn extract(rom: &Path, out: &Path) -> Result<()> {
    let (profile, sha1, data) = read_rom(rom)?;
    extract_bytes(&data, out, profile, &sha1)
}

pub(crate) fn extract_bytes(data: &[u8], out: &Path, profile: &str, sha1: &str) -> Result<()> {
    let root = out.join(profile);
    write_banks(data, &root, sha1)?;
    println!("{profile}: {}", root.display());
    Ok(())
}

pub(crate) fn parse_offset(value: &str) -> Result<usize, String> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        usize::from_str_radix(hex, 16)
    } else {
        value.parse()
    }
    .map_err(|error| error.to_string())
}

pub(crate) fn export_tiles(rom: &Path, out: &Path, offset: usize, count: usize) -> Result<()> {
    let (profile, sha1, data) = read_rom(rom)?;
    let image = tile_sheet(&data, offset, count)?;
    let folder = out.join(profile).join("tile-previews");
    let filename = format!("{offset:06x}-{count}");
    crate::gba::save_png(&image, &folder.join(format!("{filename}.png")))?;
    let index = serde_json::json!({
        "sha1": sha1,
        "rom_offset": offset,
        "tile_count": count,
        "tile_size_bytes": 16,
        "tiles_per_row": count.min(16),
    });
    std::fs::write(
        folder.join(format!("{filename}.json")),
        serde_json::to_string_pretty(&index)?,
    )?;
    println!("{profile}: {}", folder.display());
    Ok(())
}

fn tile_sheet(rom: &[u8], offset: usize, count: usize) -> Result<image::RgbaImage> {
    ensure!(
        (1..=65536).contains(&count),
        "GB tile count outside preview limit"
    );
    let tiles = rom
        .get(offset..)
        .and_then(|rest| rest.get(..count * 16))
        .context("GB tile range outside ROM")?;
    let columns = count.min(16);
    let rows = count.div_ceil(columns);
    Ok(image::RgbaImage::from_fn(
        (columns * 8) as u32,
        (rows * 8) as u32,
        |x, y| {
            let (x, y) = (x as usize, y as usize);
            let tile = (y / 8) * columns + x / 8;
            if tile >= count {
                return image::Rgba([0; 4]);
            }
            let row = tile * 16 + (y % 8) * 2;
            let bit = 7 - (x % 8);
            let color = ((tiles[row] >> bit) & 1) | (((tiles[row + 1] >> bit) & 1) << 1);
            let pixel = 255 - color * 85;
            image::Rgba([pixel, pixel, pixel, 255])
        },
    ))
}

fn write_banks(data: &[u8], root: &Path, sha1: &str) -> Result<()> {
    ensure!(
        data.len() >= BANK_SIZE && data.len().is_multiple_of(BANK_SIZE),
        "GB/GBC ROM has a partial bank"
    );
    let banks = root.join("banks");
    std::fs::create_dir_all(&banks)?;
    let mut index = String::from("bank\trom_offset\tlength\tpath\n");
    for (id, bank) in data.chunks_exact(BANK_SIZE).enumerate() {
        let filename = format!("{id:03x}.bin");
        std::fs::write(banks.join(&filename), bank)?;
        writeln!(
            index,
            "{id}\t{:06x}\t{BANK_SIZE}\tbanks/{filename}",
            id * BANK_SIZE
        )?;
    }
    std::fs::write(root.join("index.tsv"), index)?;
    let manifest = serde_json::json!({
        "sha1": sha1,
        "sha256": format!("{:x}", Sha256::digest(data)),
        "size": data.len(),
        "bank_size": BANK_SIZE,
        "bank_count": data.len() / BANK_SIZE,
        "header": {
            "cgb_flag": data[0x143],
            "sgb_flag": data[0x146],
            "cartridge_type": data[0x147],
            "rom_size_code": data[0x148],
            "ram_size_code": data[0x149],
            "revision": data[0x14c],
        },
    });
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_game_boy_bitplanes_at_an_explicit_offset() {
        let mut data = [0; 19];
        data[3] = 0xa0;
        data[4] = 0x60;
        let image = tile_sheet(&data, 3, 1).unwrap();
        assert_eq!(image.dimensions(), (8, 8));
        let intensities: Vec<_> = (0..4).map(|x| image.get_pixel(x, 0).0[0]).collect();
        assert_eq!(intensities, [170, 85, 0, 255]);
        assert!(tile_sheet(&data, 4, 1).is_err());
        assert_eq!(parse_offset("0x4000"), Ok(0x4000));
    }

    #[test]
    fn verifies_available_retail_dumps_through_extract_command() {
        let Some(assets) = crate::test_assets::root() else {
            return;
        };
        use clap::Parser;

        let out = tempfile::tempdir().unwrap();
        let mut checked = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(assets.join("roms")).unwrap() {
            let path = entry.unwrap().path();
            if !path.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("gb") || ext.eq_ignore_ascii_case("gbc")
            }) {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let Some((profile, hash)) = detect(&bytes) else {
                continue;
            };
            if !checked.insert(profile) {
                continue;
            }
            let args = [
                std::ffi::OsStr::new("ppextractor"),
                std::ffi::OsStr::new("extract"),
                path.as_os_str(),
                std::ffi::OsStr::new("--out"),
                out.path().as_os_str(),
            ];
            crate::cli::execute(crate::cli::Cli::parse_from(args), |_, _, _, _| {
                panic!("GB cartridge routed to the GBA extractor")
            })
            .unwrap();
            let root = out.path().join(profile);
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap())
                    .unwrap();
            assert_eq!(manifest["sha1"], hash);
            for (id, bank) in bytes.chunks_exact(BANK_SIZE).enumerate() {
                assert_eq!(
                    std::fs::read(root.join(format!("banks/{id:03x}.bin"))).unwrap(),
                    bank,
                    "{profile} bank {id}"
                );
            }
        }
        eprintln!("verified GB/GBC retail dumps: {checked:?}");
    }

    #[test]
    fn exports_byte_exact_banks_and_rejects_unregistered_roms() {
        let temp = tempfile::tempdir().unwrap();
        let rom = temp.path().join("fixture.gbc");
        let output = temp.path().join("out");
        let mut data = vec![0; 2 * BANK_SIZE];
        data[BANK_SIZE..].fill(0xa5);
        data[0x14c] = 1;
        std::fs::write(&rom, &data).unwrap();
        assert!(
            extract(&rom, &output)
                .unwrap_err()
                .to_string()
                .contains("unrecognized")
        );
        assert!(!output.exists());
        write_banks(&data, &output, "fixture").unwrap();
        for (id, bank) in data.chunks_exact(BANK_SIZE).enumerate() {
            assert_eq!(
                std::fs::read(output.join(format!("banks/{id:03x}.bin"))).unwrap(),
                bank
            );
        }
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(metadata["bank_count"], 2);
        assert_eq!(metadata["header"]["revision"], 1);
    }
}
