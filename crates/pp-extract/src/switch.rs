mod chk;
pub(crate) mod exefs;
pub(crate) mod nca;
pub(crate) mod nso;
pub(crate) mod res00;
pub(crate) mod romfs;

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

const PP_R_SIZE: u64 = 7_985_954_816;
pub(crate) const PP_R_SHA1: &str = "df20e7fb1b8bbeaedb6a51a3888fb272ee1e59bd";

#[derive(Serialize)]
struct Entry {
    name: String,
    offset: u64,
    size: u64,
    hashed_size: u32,
    #[serde(skip)]
    hash: [u8; 32],
    #[serde(skip_serializing_if = "Option::is_none")]
    extracted_path: Option<String>,
}

#[derive(Serialize)]
struct Partition {
    entry: Entry,
    files: Vec<Entry>,
}

fn word(data: &[u8]) -> u32 {
    u32::from_le_bytes(data[..4].try_into().unwrap())
}

fn double_word(data: &[u8]) -> u64 {
    u64::from_le_bytes(data[..8].try_into().unwrap())
}

fn table(reader: &mut (impl Read + Seek), range: Range<u64>) -> Result<(Vec<u8>, Vec<Entry>)> {
    ensure!(
        range.end.saturating_sub(range.start) >= 16,
        "truncated HFS0 partition"
    );
    reader.seek(SeekFrom::Start(range.start))?;
    let mut first = [0; 16];
    reader.read_exact(&mut first)?;
    ensure!(&first[..4] == b"HFS0", "missing HFS0 signature");
    let count = u64::from(word(&first[4..]));
    let names_size = u64::from(word(&first[8..]));
    let names_at = 16 + count * 64;
    let size = names_at + names_size;
    ensure!(
        size <= range.end - range.start && size <= 1024 * 1024,
        "invalid HFS0 header size"
    );
    let data_start = range.start + size;
    let mut header = first.to_vec();
    header.resize(size as usize, 0);
    reader.read_exact(&mut header[16..])?;

    let names = &header[names_at as usize..];
    let mut entries = Vec::new();
    for record in header[16..names_at as usize].chunks_exact(64) {
        let offset = data_start
            .checked_add(double_word(record))
            .context("HFS0 file offset overflow")?;
        let size = double_word(&record[8..]);
        let hashed_size = word(&record[20..]);
        ensure!(
            offset <= range.end && size <= range.end - offset && u64::from(hashed_size) <= size,
            "HFS0 file outside partition"
        );
        let tail = names
            .get(word(&record[16..]) as usize..)
            .context("HFS0 filename outside table")?;
        let stop = tail
            .iter()
            .position(|&byte| byte == 0)
            .context("unterminated HFS0 filename")?;
        let name = std::str::from_utf8(&tail[..stop])
            .context("invalid HFS0 filename")?
            .to_owned();
        entries.push(Entry {
            name,
            offset,
            size,
            hashed_size,
            hash: record[32..64].try_into().unwrap(),
            extracted_path: None,
        });
    }
    Ok((header, entries))
}

fn partitions(reader: &mut (impl Read + Seek), file_size: u64) -> Result<Vec<Partition>> {
    let mut header = [0; 0x200];
    reader
        .read_exact(&mut header)
        .context("truncated XCI header")?;
    ensure!(
        &header[0x100..0x104] == b"HEAD",
        "missing XCI HEAD signature"
    );
    let start = double_word(&header[0x130..]);
    let declared = double_word(&header[0x138..]);
    ensure!(
        start >= 0x200 && start < file_size,
        "invalid XCI root offset"
    );
    let (root, entries) = table(reader, start..file_size)?;
    ensure!(
        declared == root.len() as u64 && entries.len() <= 4,
        "invalid XCI root header"
    );
    let digest = Sha256::digest(&root);
    ensure!(
        digest[..] == header[0x140..0x160],
        "XCI root header hash mismatch"
    );
    entries
        .into_iter()
        .map(|entry| {
            verify_entry_hash(reader, &entry)
                .with_context(|| format!("validating XCI partition {}", entry.name))?;
            let (_, files) = table(reader, entry.offset..entry.offset + entry.size)?;
            Ok(Partition { entry, files })
        })
        .collect()
}

fn verify_entry_hash(reader: &mut (impl Read + Seek), entry: &Entry) -> Result<()> {
    if entry.hashed_size == 0 {
        return Ok(());
    }
    let mut buffer = [0; 64 * 1024];
    let mut hash = Sha256::new();
    reader.seek(SeekFrom::Start(entry.offset))?;
    let mut remaining = entry.hashed_size as usize;
    while remaining > 0 {
        let count = remaining.min(buffer.len());
        reader.read_exact(&mut buffer[..count])?;
        hash.update(&buffer[..count]);
        remaining -= count;
    }
    ensure!(
        hash.finalize()[..] == entry.hash,
        "HFS0 file header hash mismatch"
    );
    Ok(())
}

fn copy_entry(reader: &mut (impl Read + Seek), entry: &Entry, path: &Path) -> Result<()> {
    verify_entry_hash(reader, entry)?;
    reader.seek(SeekFrom::Start(entry.offset))?;
    let mut output = std::fs::File::create_new(path)?;
    let copied = std::io::copy(&mut reader.take(entry.size), &mut output);
    if !matches!(copied, Ok(size) if size == entry.size) {
        drop(output);
        std::fs::remove_file(path)?;
        ensure!(copied? == entry.size, "XCI file ended during copy");
    }
    Ok(())
}

fn copy_partition(
    reader: &mut (impl Read + Seek),
    partition: &mut Partition,
    root: &Path,
) -> Result<()> {
    let folder = root.join("partition-files");
    std::fs::create_dir(&folder).context("creating a new partition-files directory")?;
    for (id, entry) in partition.files.iter_mut().enumerate() {
        let name = format!("{id:04x}.bin");
        let path = folder.join(&name);
        copy_entry(reader, entry, &path)
            .with_context(|| format!("copying HFS0 file {}", entry.name))?;
        entry.extracted_path = Some(format!("partition-files/{name}"));
    }
    Ok(())
}

pub(crate) fn copy_secure_nca(path: &Path, name: &str, destination: &Path) -> Result<()> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let length = file.metadata()?.len();
    let partitions = partitions(&mut file, length)?;
    let mut matches = partitions.iter().filter(|p| p.entry.name == "secure");
    let secure = matches.next().context("XCI has no secure partition")?;
    ensure!(
        matches.next().is_none(),
        "XCI has multiple secure partitions"
    );
    let mut matches = secure
        .files
        .iter()
        .filter(|entry| entry.name.eq_ignore_ascii_case(name));
    let entry = matches
        .next()
        .with_context(|| format!("NCA {name:?} not found in XCI secure partition"))?;
    ensure!(
        matches.next().is_none(),
        "ambiguous NCA name in XCI secure partition"
    );
    ensure!(
        entry
            .name
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("nca")),
        "XCI entry is not an NCA"
    );
    copy_entry(&mut file, entry, destination)
        .with_context(|| format!("copying program NCA {}", entry.name))
}

pub(crate) fn index(path: &Path, out: &Path, copy_name: Option<&str>) -> Result<()> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let size = file.metadata()?.len();
    let mut partitions = partitions(&mut file, size)?;
    let sha1 = if size == PP_R_SIZE {
        file.seek(SeekFrom::Start(0))?;
        let mut hash = Sha1::new();
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        Some(format!("{:x}", hash.finalize()))
    } else {
        None
    };
    let profile = (sha1.as_deref() == Some(PP_R_SHA1)).then_some("pp_r_jp");
    if let Some(name) = copy_name {
        let partition = partitions
            .iter_mut()
            .find(|item| item.entry.name == name)
            .with_context(|| format!("XCI partition {name:?} is absent"))?;
        std::fs::create_dir_all(out)?;
        copy_partition(&mut file, partition, out)?;
    }
    let index = serde_json::json!({
        "size": size,
        "catalog_sha1": sha1,
        "profile": profile,
        "partitions": partitions,
    });
    std::fs::create_dir_all(out)?;
    let path = out.join("xci-index.json");
    std::fs::write(&path, serde_json::to_string_pretty(&index)?)?;
    println!("{}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn partition(name: &str, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 80];
        bytes[..4].copy_from_slice(b"HFS0");
        bytes[4] = 1;
        bytes[8..12].copy_from_slice(&(name.len() as u32 + 1).to_le_bytes());
        bytes[24..32].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.push(0);
        bytes.extend(payload);
        bytes
    }

    #[test]
    fn verifies_available_retail_cartridge_index() {
        let Some(assets) = crate::test_assets::root() else {
            return;
        };
        let mut verified = 0;
        for entry in std::fs::read_dir(assets.join("roms")).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("xci"))
                || entry.metadata().unwrap().len() != PP_R_SIZE
            {
                continue;
            }
            let out = tempfile::tempdir().unwrap();
            index(&path, out.path(), None).unwrap();
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(out.path().join("xci-index.json")).unwrap())
                    .unwrap();
            if value["catalog_sha1"] != PP_R_SHA1 {
                continue;
            }
            assert_eq!(value["profile"], "pp_r_jp");
            let secure = value["partitions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|partition| partition["entry"]["name"] == "secure")
                .unwrap();
            let start = secure["entry"]["offset"].as_u64().unwrap();
            let end = start + secure["entry"]["size"].as_u64().unwrap();
            let files = secure["files"].as_array().unwrap();
            assert!(!files.is_empty());
            assert!(
                files
                    .iter()
                    .any(|file| file["name"].as_str().unwrap().ends_with(".nca"))
            );
            for file in files {
                let offset = file["offset"].as_u64().unwrap();
                let size = file["size"].as_u64().unwrap();
                assert!(start <= offset && offset + size <= end);
            }
            verified += 1;
        }
        eprintln!("verified {verified} local Power Pro Kun Pocket R XCI dumps");
    }

    #[test]
    fn indexes_nested_files_without_reading_their_payloads() {
        let child = partition("app.nca", b"raw");
        let mut root = partition("secure", &child);
        let root_size = root.len() - child.len();
        let child_header = &child[..child.len() - 3];
        root[36..40].copy_from_slice(&(child_header.len() as u32).to_le_bytes());
        root[48..80].copy_from_slice(&Sha256::digest(child_header));
        let mut image = vec![0; 0x200];
        image[0x100..0x104].copy_from_slice(b"HEAD");
        image[0x130..0x138].copy_from_slice(&0x200u64.to_le_bytes());
        image[0x138..0x140].copy_from_slice(&(root_size as u64).to_le_bytes());
        image[0x140..0x160].copy_from_slice(&Sha256::digest(&root[..root_size]));
        image.extend(root);
        let size = image.len() as u64;
        let mut reader = std::io::Cursor::new(image);
        let mut output = partitions(&mut reader, size).unwrap();
        assert_eq!(output[0].entry.name, "secure");
        let entry = &output[0].files[0];
        assert_eq!(entry.name, "app.nca");
        assert_eq!((entry.offset, entry.size), (size - 3, 3));
        let mut damaged = reader.get_ref().clone();
        damaged[0x200 + root_size + 12] ^= 1;
        let error = partitions(&mut std::io::Cursor::new(damaged), size)
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("HFS0 file header hash mismatch"));
        let out = tempfile::tempdir().unwrap();
        copy_partition(&mut reader, &mut output[0], out.path()).unwrap();
        assert_eq!(
            std::fs::read(out.path().join("partition-files/0000.bin")).unwrap(),
            b"raw"
        );
        assert_eq!(
            output[0].files[0].extracted_path.as_deref(),
            Some("partition-files/0000.bin")
        );
        assert!(copy_partition(&mut reader, &mut output[0], out.path()).is_err());
        output[0].files[0].hashed_size = 3;
        let wrong_hash = tempfile::tempdir().unwrap();
        assert!(copy_partition(&mut reader, &mut output[0], wrong_hash.path()).is_err());
        assert!(!wrong_hash.path().join("partition-files/0000.bin").exists());
        let cartridge = out.path().join("game.xci");
        std::fs::write(&cartridge, reader.get_ref()).unwrap();
        let program = out.path().join("program.nca");
        copy_secure_nca(&cartridge, "APP.NCA", &program).unwrap();
        assert_eq!(std::fs::read(program).unwrap(), b"raw");
        let missing = out.path().join("missing.nca");
        assert!(copy_secure_nca(&cartridge, "missing.nca", &missing).is_err());
        assert!(!missing.exists());
        reader.get_mut()[0x140] ^= 1;
        reader.set_position(0);
        assert!(partitions(&mut reader, size).is_err());
    }
}
