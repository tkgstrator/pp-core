use super::{double_word, word};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Serialize)]
struct Entry {
    name: String,
    offset: u64,
    size: u64,
    path: String,
}

fn scan(input: &mut impl Read, length: u64) -> Result<Vec<Entry>> {
    let mut header = [0; 16];
    input
        .read_exact(&mut header)
        .context("reading PFS0 header")?;
    ensure!(header.starts_with(b"PFS0"), "expected a PFS0 image");
    let count = u64::from(word(&header[4..]));
    let names_size = u64::from(word(&header[8..]));
    let records_size = count * 24;
    let data_start = 16 + records_size + names_size;
    ensure!(
        data_start <= length && data_start <= 4 * 1024 * 1024,
        "invalid PFS0 metadata size"
    );
    let mut metadata = vec![0; (records_size + names_size) as usize];
    input.read_exact(&mut metadata)?;
    let (records, names) = metadata.split_at(records_size as usize);
    let mut total = 0u64;
    let mut name_budget = 4 * 1024 * 1024;
    records
        .chunks_exact(24)
        .enumerate()
        .map(|(id, record)| {
            let offset = data_start
                .checked_add(double_word(record))
                .context("PFS0 offset overflow")?;
            let size = double_word(&record[8..]);
            ensure!(
                offset <= length && size <= length - offset,
                "PFS0 entry {id} outside image"
            );
            total = total
                .checked_add(size)
                .context("PFS0 total size overflow")?;
            ensure!(
                total <= length - data_start,
                "PFS0 entry sizes exceed image size"
            );
            let name = names
                .get(word(&record[16..]) as usize..)
                .context("PFS0 filename offset outside table")?;
            let end = name[..name.len().min(name_budget)]
                .iter()
                .position(|&b| b == 0)
                .context("unterminated PFS0 filename or name limit exceeded")?;
            name_budget -= end + 1;
            let name = std::str::from_utf8(&name[..end]).context("invalid PFS0 filename")?;
            ensure!(!name.is_empty(), "empty PFS0 filename");
            Ok(Entry {
                name: name.into(),
                offset,
                size,
                path: format!("files/{id:06x}.bin"),
            })
        })
        .collect()
}

pub(crate) fn index_nsp(image: &Path, out: &Path) -> Result<()> {
    let mut input =
        std::fs::File::open(image).with_context(|| format!("opening {}", image.display()))?;
    let length = input.metadata()?.len();
    let files: Vec<_> = scan(&mut input, length)?
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "name": entry.name,
                "offset": entry.offset,
                "size": entry.size,
            })
        })
        .collect();
    std::fs::create_dir_all(out)?;
    let path = out.join("nsp-index.json");
    let json = serde_json::json!({ "size": length, "files": files });
    std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
    println!("{}", path.display());
    Ok(())
}

pub(crate) fn copy_nsp_nca(image: &Path, name: &str, out: &Path) -> Result<()> {
    ensure!(
        name.rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("nca")),
        "NSP content entry must have an .nca extension"
    );
    let mut input =
        std::fs::File::open(image).with_context(|| format!("opening {}", image.display()))?;
    let length = input.metadata()?.len();
    let entries = scan(&mut input, length)?;
    let mut matches = entries
        .iter()
        .filter(|entry| entry.name.eq_ignore_ascii_case(name));
    let entry = matches
        .next()
        .with_context(|| format!("NCA {name:?} not found in NSP"))?;
    ensure!(matches.next().is_none(), "ambiguous NCA name in NSP");
    copy_entry(&mut input, entry, out)
}

fn copy_entry(input: &mut std::fs::File, entry: &Entry, path: &Path) -> Result<()> {
    input.seek(SeekFrom::Start(entry.offset))?;
    let mut file = std::fs::File::create_new(path)?;
    let result = std::io::copy(&mut input.take(entry.size), &mut file);
    if !matches!(result, Ok(size) if size == entry.size) {
        drop(file);
        std::fs::remove_file(path)?;
        ensure!(result? == entry.size, "PFS0 image ended during copy");
    }
    Ok(())
}

pub(crate) fn extract(image: &Path, out: &Path, decode_main: bool) -> Result<()> {
    let mut input =
        std::fs::File::open(image).with_context(|| format!("opening {}", image.display()))?;
    let length = input.metadata()?.len();
    let entries = scan(&mut input, length)?;
    let main = if decode_main {
        let mut candidates = entries.iter().filter(|entry| entry.name == "main");
        let main = candidates
            .next()
            .context("PFS0 contains no main executable")?;
        ensure!(
            candidates.next().is_none(),
            "PFS0 contains multiple main executables"
        );
        Some(main)
    } else {
        None
    };

    std::fs::create_dir_all(out)?;
    std::fs::create_dir(out.join("files")).context("creating a new ExeFS files directory")?;
    for entry in &entries {
        copy_entry(&mut input, entry, &out.join(&entry.path))?;
    }
    let index = std::fs::File::create(out.join("exefs-index.json"))?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(index), &entries)?;
    if let Some(main) = main {
        super::nso::extract(&out.join(&main.path), &out.join("main"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_only_the_named_nca_from_an_nsp() {
        let names = b"program.nca\0control.nca\0";
        let mut image = vec![0; 16 + 2 * 24];
        image[..4].copy_from_slice(b"PFS0");
        image[4] = 2;
        image[8] = names.len() as u8;
        image[24] = 3;
        image[40] = 3;
        image[48] = 4;
        image[56] = 12;
        image.extend_from_slice(names);
        image.extend_from_slice(b"oldtest");
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("update.nsp");
        let out = dir.path().join("selected.nca");
        std::fs::write(&source, image).unwrap();
        let index = dir.path().join("index");
        index_nsp(&source, &index).unwrap();
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(index.join("nsp-index.json")).unwrap()).unwrap();
        assert_eq!(metadata["files"][1]["name"], "control.nca");
        assert_eq!(
            metadata["files"][1]["offset"],
            16 + 2 * 24 + names.len() + 3
        );
        assert_eq!(metadata["files"][1]["size"], 4);
        assert_eq!(std::fs::read_dir(index).unwrap().count(), 1);
        assert!(copy_nsp_nca(&source, "missing.nca", &out).is_err());
        assert!(!out.exists());
        copy_nsp_nca(&source, "CONTROL.NCA", &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"test");
    }

    #[test]
    fn extracts_and_decodes_main_from_pfs0() {
        let mut main = vec![0; 0x100];
        main[..4].copy_from_slice(b"NSO0");
        main[0x20..0x24].copy_from_slice(&0x100u32.to_le_bytes());
        main[0x28] = 4;
        main.extend_from_slice(b"test");
        let mut pfs = vec![0; 40];
        pfs[..4].copy_from_slice(b"PFS0");
        pfs[4] = 1;
        pfs[8] = 8;
        pfs[24..32].copy_from_slice(&(main.len() as u64).to_le_bytes());
        pfs.extend_from_slice(b"main\0\0\0\0");
        pfs.extend(main);
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("exefs.pfs0");
        std::fs::write(&source, &pfs).unwrap();
        let out = dir.path().join("out");
        extract(&source, &out, true).unwrap();
        assert_eq!(std::fs::read(out.join("main/rodata.bin")).unwrap(), b"test");
        assert!(out.join("exefs-index.json").is_file());
        pfs[24] = 0xff;
        std::fs::write(&source, pfs).unwrap();
        let bad = dir.path().join("bad");
        assert!(extract(&source, &bad, true).is_err());
        assert!(!bad.exists());
    }
}
