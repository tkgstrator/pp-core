use anyhow::{Context, Result, ensure};
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

pub(crate) fn extract(
    nca: &Path,
    base_nca: Option<&Path>,
    keyset: &Path,
    hactool: &Path,
    out: &Path,
    decode_main: bool,
    decode_res00: bool,
) -> Result<()> {
    let nca = nca.canonicalize().context("resolving NCA input")?;
    let keyset = keyset.canonicalize().context("resolving hactool keyset")?;
    ensure!(
        nca.is_file() && keyset.is_file(),
        "NCA and keyset must be regular files"
    );
    let temp = tempfile::Builder::new().prefix("pp-nca-").tempdir()?;
    let exefs = temp.path().join("exefs.pfs0");
    let romfs = temp.path().join("romfs.bin");
    let sections = ["section0", "section1", "section2", "section3"];

    // Use separate arguments and discard hactool diagnostics, which can contain keys.
    let mut command = Command::new(hactool);
    command
        .args(["--extract", "--intype=nca", "--keyset"])
        .arg(&keyset)
        .arg(path_option("--exefs=", &exefs))
        .arg(path_option("--romfs=", &romfs));
    if let Some(base) = base_nca {
        let base = base.canonicalize().context("resolving base NCA")?;
        ensure!(
            base.is_file() && base != nca,
            "base NCA must be a separate regular file"
        );
        command
            .arg(path_option("--basenca=", &base))
            .arg("--appendsectypes");
        for section in sections {
            command.arg(path_option(
                &format!("--{section}="),
                &temp.path().join(section),
            ));
        }
    }
    let status = command
        .arg(&nca)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("starting hactool at {}", hactool.display()))?;
    ensure!(
        status.success(),
        "hactool failed to extract the NCA ({status})"
    );
    if base_nca.is_some() {
        let exefs = exefs.with_extension("pfs0.pfs0");
        let plain_romfs = romfs.with_extension("bin.romfs");
        let mut merged = sections
            .map(|section| temp.path().join(section).with_extension("romfs"))
            .into_iter()
            .filter(|path| path.is_file());
        let romfs = merged.next().unwrap_or(plain_romfs);
        ensure!(merged.next().is_none(), "multiple merged RomFS sections");
        export_sections(&exefs, &romfs, out, decode_main, decode_res00)
    } else {
        export_sections(&exefs, &romfs, out, decode_main, decode_res00)
    }
}

fn path_option(prefix: &str, path: &Path) -> OsString {
    let mut option = OsString::from(prefix);
    option.push(path);
    option
}

fn export_sections(
    exefs: &Path,
    romfs: &Path,
    out: &Path,
    decode_main: bool,
    decode_res00: bool,
) -> Result<()> {
    let has_exefs = exefs.is_file();
    let has_romfs = romfs.is_file();
    ensure!(
        has_exefs || has_romfs,
        "hactool produced neither ExeFS nor RomFS"
    );
    ensure!(
        !decode_main || has_exefs,
        "--decode-main requires an ExeFS section"
    );
    ensure!(
        !decode_res00 || has_romfs,
        "--decode-res00 requires a RomFS section"
    );
    if has_exefs {
        super::exefs::extract(exefs, &out.join("exefs"), decode_main)?;
    }
    if has_romfs {
        super::romfs::extract(romfs, &out.join("romfs"), decode_res00)?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn invokes_hactool_and_dispatches_its_pfs0_image() {
        let dir = tempfile::tempdir().unwrap();
        let nca = dir.path().join("sample with space.nca");
        let keyset = dir.path().join("test keys");
        let hactool = dir.path().join("stub-hactool");
        let mut fixture = vec![0; 40];
        fixture[..4].copy_from_slice(b"PFS0");
        fixture[4] = 1;
        fixture[8] = 8;
        fixture[24] = 3;
        fixture.extend_from_slice(b"main\0\0\0\0raw");
        std::fs::write(&nca, fixture).unwrap();
        std::fs::write(&keyset, []).unwrap();
        std::fs::write(
            &hactool,
            r#"#!/bin/sh
set -eu
[ "$1" = --extract ] && [ "$2" = --intype=nca ] && [ "$3" = --keyset ]
[ -f "$4" ] && [ "${5%%=*}" = --exefs ] && [ "${6%%=*}" = --romfs ]
if [ "${7%%=*}" = --basenca ]; then
  [ -f "${7#*=}" ] && [ "$8" = --appendsectypes ] && [ "$#" = 13 ]
  [ "${10%%=*}" = --section1 ]
  cp "${13}" "${5#*=}.pfs0"
  cp "${7#*=}" "${10#*=}.romfs"
else
  cp "$7" "${5#*=}"
fi
"#,
        )
        .unwrap();
        std::fs::set_permissions(&hactool, std::fs::Permissions::from_mode(0o700)).unwrap();
        let out = dir.path().join("out");
        extract(&nca, None, &keyset, &hactool, &out, false, false).unwrap();
        assert_eq!(
            std::fs::read(out.join("exefs/files/000000.bin")).unwrap(),
            b"raw"
        );
        let bad = dir.path().join("bad");
        assert!(extract(&nca, None, &keyset, &hactool, &bad, false, true).is_err());
        let base = dir.path().join("base program.nca");
        let mut romfs = vec![0; 0x78];
        for (offset, value) in [
            (0, 0x50u64),
            (0x18, 0x50),
            (0x20, 24),
            (0x38, 0x78),
            (0x48, 0x78),
        ] {
            romfs[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        for offset in [0x54, 0x58, 0x5c, 0x60] {
            romfs[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        std::fs::write(&base, romfs).unwrap();
        let patched = dir.path().join("patched");
        extract(&nca, Some(&base), &keyset, &hactool, &patched, false, false).unwrap();
        assert_eq!(
            std::fs::read(patched.join("exefs/files/000000.bin")).unwrap(),
            b"raw"
        );
        assert!(!bad.exists());
        assert!(patched.join("romfs/romfs-index.json").is_file());
    }
}
