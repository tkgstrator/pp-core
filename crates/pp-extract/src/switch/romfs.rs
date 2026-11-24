use super::{double_word, word};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const NONE: u32 = u32::MAX;

#[derive(Serialize)]
struct FileEntry {
    rom_path: String,
    offset: u64,
    size: u64,
    path: String,
}

fn metadata(reader: &mut (impl Read + Seek), offset: u64, size: u64, end: u64) -> Result<Vec<u8>> {
    ensure!(
        offset >= 0x50 && offset <= end && size <= end - offset && size <= 64 * 1024 * 1024,
        "RomFS metadata outside image or size limit"
    );
    reader.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; size as usize];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn entry(table: &[u8], offset: u32, header_size: usize) -> Result<(&[u8], &str)> {
    ensure!(offset.is_multiple_of(4), "unaligned RomFS entry");
    let tail = table
        .get(offset as usize..)
        .context("RomFS entry outside table")?;
    let header = tail.get(..header_size).context("truncated RomFS entry")?;
    let count = word(&header[header_size - 4..]) as usize;
    let name = tail[header_size..]
        .get(..count)
        .context("RomFS name outside table")?;
    let name = std::str::from_utf8(name).context("invalid RomFS UTF-8 name")?;
    ensure!(
        name != "." && name != ".." && !name.contains(['/', '\\', '\0']),
        "invalid RomFS name"
    );
    Ok((header, name))
}

fn scan(reader: &mut (impl Read + Seek), image_size: u64) -> Result<Vec<FileEntry>> {
    let mut header = [0; 0x50];
    reader.read_exact(&mut header)?;
    ensure!(
        double_word(&header) == 0x50,
        "invalid Switch RomFS header size"
    );
    let dirs = metadata(
        reader,
        double_word(&header[0x18..]),
        double_word(&header[0x20..]),
        image_size,
    )?;
    let files = metadata(
        reader,
        double_word(&header[0x38..]),
        double_word(&header[0x40..]),
        image_size,
    )?;
    let data = double_word(&header[0x48..]);
    ensure!(
        (0x50..=image_size).contains(&data),
        "RomFS data outside image"
    );

    let mut queue = VecDeque::from([(0, NONE, String::new())]);
    let mut seen_dirs = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut entries = Vec::new();
    let mut total = 0u64;
    while let Some((dir, parent, prefix)) = queue.pop_front() {
        ensure!(seen_dirs.insert(dir), "repeated RomFS directory");
        let (record, name) = entry(&dirs, dir, 24)?;
        ensure!(
            (dir == 0 && name.is_empty() && word(&record[4..]) == NONE)
                || (dir != 0 && !name.is_empty() && word(record) == parent),
            "invalid RomFS directory parent or name"
        );
        let path = if dir == 0 {
            prefix.clone()
        } else {
            format!("{prefix}{name}/")
        };
        ensure!(path.len() <= 4096, "RomFS path exceeds length limit");
        let sibling = word(&record[4..]);
        if sibling != NONE {
            queue.push_back((sibling, parent, prefix));
        }
        let child = word(&record[8..]);
        if child != NONE {
            queue.push_back((child, dir, path.clone()));
        }
        let mut file = word(&record[12..]);
        while file != NONE {
            ensure!(seen_files.insert(file), "repeated RomFS file entry");
            let (record, name) = entry(&files, file, 32)?;
            ensure!(
                word(record) == dir && !name.is_empty(),
                "invalid RomFS file parent or name"
            );
            let offset = data
                .checked_add(double_word(&record[8..]))
                .context("RomFS file offset overflow")?;
            let size = double_word(&record[16..]);
            ensure!(
                offset <= image_size && size <= image_size - offset,
                "RomFS file outside image"
            );
            total = total
                .checked_add(size)
                .context("RomFS total file size overflow")?;
            ensure!(total <= image_size, "RomFS files exceed image size");
            ensure!(
                path.len() + name.len() <= 4096,
                "RomFS path exceeds length limit"
            );
            entries.push(FileEntry {
                rom_path: format!("{path}{name}"),
                offset,
                size,
                path: format!("files/{:06x}.bin", entries.len()),
            });
            file = word(&record[4..]);
        }
    }
    Ok(entries)
}

fn decode_res00(entries: &[FileEntry], out: &Path) -> Result<()> {
    use super::res00::PatchArchive;
    #[derive(Default)]
    struct Archives<'a> {
        indexes: Vec<&'a FileEntry>,
        base: Vec<&'a FileEntry>,
        patch: Vec<&'a FileEntry>,
    }
    let mut pairs = BTreeMap::<_, Archives<'_>>::new();
    for entry in entries {
        let (parent, name) = entry
            .rom_path
            .rsplit_once('/')
            .unwrap_or(("", &entry.rom_path));
        if name.eq_ignore_ascii_case("RES00.RDI") {
            pairs.entry(parent).or_default().indexes.push(entry);
        } else if name.eq_ignore_ascii_case("RES00.RDB") {
            pairs.entry(parent).or_default().base.push(entry);
        } else if name.eq_ignore_ascii_case("RES11.RDB") {
            pairs.entry(parent).or_default().patch.push(entry);
        }
    }

    let mut report = Vec::new();
    let mut decoded = 0;
    for (parent, group) in pairs {
        if let ([index], [archive]) = (group.indexes.as_slice(), group.base.as_slice()) {
            // Output names derive from the extracted entry number, never from the ROM path.
            let file = Path::new(&index.path)
                .file_stem()
                .context("missing RomFS entry ID")?;
            let destination = Path::new("res00").join(file);
            let patch = match group.patch.as_slice() {
                [patch] => Some(*patch),
                _ => None,
            };
            let patch_path = patch.map(|file| out.join(&file.path));
            let result = super::res00::extract(
                &out.join(&index.path),
                &out.join(&archive.path),
                &out.join(&destination),
                true,
                patch_path
                    .as_deref()
                    .map_or(PatchArchive::Absent, PatchArchive::Optional),
            );
            let (error, patch_error) = match result {
                Ok(patch_error) => {
                    decoded += 1;
                    (None, patch_error)
                }
                Err(error) => (Some(format!("{error:#}")), None),
            };
            let patch_error = if group.patch.len() > 1 {
                Some("ambiguous RES11.RDB archive".into())
            } else {
                patch_error
            };
            report.push(serde_json::json!({
                "rom_directory": parent,
                "rdi": index.rom_path,
                "rdb": archive.rom_path,
                "output": destination,
                "patch_rdb": patch.map(|file| &file.rom_path),
                "patch_error": patch_error,
                "error": error,
            }));
        } else {
            report.push(serde_json::json!({
                "rom_directory": parent,
                "error": "missing or ambiguous RES00 pair",
            }));
        }
    }
    let report_file = std::fs::File::create(out.join("res00-pairs.json"))?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(report_file), &report)?;
    ensure!(
        decoded > 0,
        "no RES00 pair could be processed; see res00-pairs.json"
    );
    Ok(())
}

pub(crate) fn extract(image: &Path, out: &Path, unpack_res00: bool) -> Result<()> {
    let mut source =
        std::fs::File::open(image).with_context(|| format!("opening {}", image.display()))?;
    let size = source.metadata()?.len();
    let entries = scan(&mut source, size)?;
    std::fs::create_dir_all(out)?;
    std::fs::create_dir(out.join("files")).context("creating a new RomFS files directory")?;
    for entry in &entries {
        source.seek(SeekFrom::Start(entry.offset))?;
        let path = out.join(&entry.path);
        let mut file = std::fs::File::create_new(&path)?;
        let copied = std::io::copy(&mut (&mut source).take(entry.size), &mut file);
        if !matches!(copied, Ok(size) if size == entry.size) {
            drop(file);
            std::fs::remove_file(path)?;
            ensure!(copied? == entry.size, "RomFS image ended during copy");
        }
    }
    let index = std::fs::File::create(out.join("romfs-index.json"))?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(index), &entries)?;
    if unpack_res00 {
        decode_res00(&entries, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_only_unique_res00_pairs_from_the_same_directory() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path();
        std::fs::create_dir(out.join("files")).unwrap();
        let mut rdi = vec![0; 56];
        rdi[..4].copy_from_slice(b"RDI2");
        rdi[4] = 1;
        rdi[8] = 56;
        let files = [
            ("data/res00.rdi", &rdi[..]),
            ("data/RES00.RDB", &[][..]),
            ("other/RES00.RDI", &rdi[..]),
        ];
        let entries: Vec<_> = files
            .into_iter()
            .enumerate()
            .map(|(id, (name, bytes))| {
                let path = format!("files/{id:06x}.bin");
                std::fs::write(out.join(&path), bytes).unwrap();
                FileEntry {
                    rom_path: name.into(),
                    offset: 0,
                    size: bytes.len() as u64,
                    path,
                }
            })
            .collect();
        decode_res00(&entries, out).unwrap();
        assert!(out.join("res00/000000/res00-index.json").is_file());
        assert!(!out.join("res00/000002").exists());
        assert!(
            std::fs::read_to_string(out.join("res00-pairs.json"))
                .unwrap()
                .contains("missing or ambiguous RES00 pair")
        );
    }

    #[test]
    fn finds_sibling_patch_and_still_extracts_base_when_patch_is_invalid() {
        let temp = tempfile::tempdir().unwrap();
        let mut rdi = vec![0; 64];
        rdi[..4].copy_from_slice(b"RDI2");
        rdi[4] = 1;
        rdi[12] = 2;
        rdi[36] = 1;
        rdi.extend_from_slice(&[0, 0, 0, 0, 2, 0, 0, 0]);
        rdi[8] = 72;

        let entries = [
            ("data/res00.rdi", &rdi[..]),
            ("data/res00.rdb", &[][..]),
            ("data/res11.rdb", b"ok"),
        ]
        .into_iter()
        .enumerate()
        .map(|(id, (name, data))| {
            (
                FileEntry {
                    rom_path: name.into(),
                    offset: 0,
                    size: data.len() as u64,
                    path: format!("files/{id:06x}.bin"),
                },
                data,
            )
        })
        .collect::<Vec<_>>();
        for valid in [false, true] {
            let out = temp.path().join(valid.to_string());
            std::fs::create_dir_all(out.join("files")).unwrap();
            for (entry, bytes) in &entries {
                let bytes: &[u8] = if entry.rom_path.ends_with("res11.rdb") && !valid {
                    &[]
                } else {
                    bytes
                };
                std::fs::write(out.join(&entry.path), bytes).unwrap();
            }
            let files = entries
                .iter()
                .map(|(entry, _)| FileEntry {
                    rom_path: entry.rom_path.clone(),
                    offset: entry.offset,
                    size: entry.size,
                    path: entry.path.clone(),
                })
                .collect::<Vec<_>>();
            decode_res00(&files, &out).unwrap();
            let dest = out.join("res00/000000");
            assert!(dest.join("res00-index.json").is_file());
            let report: serde_json::Value =
                serde_json::from_slice(&std::fs::read(out.join("res00-pairs.json")).unwrap())
                    .unwrap();
            assert_eq!(report[0]["patch_rdb"], "data/res11.rdb");
            assert_eq!(report[0]["patch_error"].is_null(), valid);
            if valid {
                assert_eq!(
                    std::fs::read(dest.join("patch-chunks/000000.bin")).unwrap(),
                    b"ok"
                );
            }
        }
    }

    #[test]
    fn extracts_nested_romfs_file_and_rejects_a_cycle() {
        let temp = tempfile::tempdir().unwrap();
        let mut image = vec![0u8; 0x180];
        for (offset, value) in [
            (0, 0x50u64),
            (0x18, 0x80),
            (0x20, 56),
            (0x38, 0xc0),
            (0x40, 48),
            (0x48, 0x100),
        ] {
            image[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [
            (0x84, NONE),
            (0x88, 24),
            (0x8c, NONE),
            (0x9c, NONE),
            (0xa0, NONE),
            (0xac, 3),
            (0xc0, 24),
            (0xc4, NONE),
            (0xd0, 3),
            (0xdc, 5),
        ] {
            image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        image[0xb0..0xb3].copy_from_slice(b"img");
        image[0xe0..0xe5].copy_from_slice(b"a.bin");
        image[0x100..0x103].copy_from_slice(b"raw");
        let source = temp.path().join("fixture.romfs");
        std::fs::write(&source, &image).unwrap();
        let output = temp.path().join("out");
        extract(&source, &output, false).unwrap();
        assert_eq!(
            std::fs::read(output.join("files/000000.bin")).unwrap(),
            b"raw"
        );
        assert!(
            std::fs::read_to_string(output.join("romfs-index.json"))
                .unwrap()
                .contains("img/a.bin")
        );
        image[0x9c..0xa0].copy_from_slice(&24u32.to_le_bytes());
        let size = image.len() as u64;
        assert!(scan(&mut std::io::Cursor::new(image), size).is_err());
    }
}
