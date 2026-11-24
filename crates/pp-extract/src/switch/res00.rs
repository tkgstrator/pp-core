use super::word;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, btree_map::Entry};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

fn resource_key(name: &[u8]) -> u32 {
    const SHIFTS: [u32; 16] = [5, 6, 4, 5, 6, 5, 4, 5, 5, 6, 6, 5, 6, 4, 6, 4];
    let basename = name
        .rsplit(|&byte| matches!(byte, b'/' | b'\\'))
        .next()
        .unwrap_or_default();
    let seed =
        basename
            .iter()
            .rev()
            .enumerate()
            .fold(basename.len() as u32, |key, (index, byte)| {
                key.wrapping_add((index as u32 + 1) * u32::from(byte.to_ascii_uppercase()))
            });
    let mut key = seed;
    let mut shift = 0;
    for (index, &byte) in name.iter().take(basename.len()).enumerate() {
        let byte = if byte == b'/' {
            b'\\'
        } else {
            byte.to_ascii_uppercase()
        };
        key = key.wrapping_add(u32::from(byte).wrapping_sub(0x30) << shift);
        let step = SHIFTS[(seed.wrapping_add(index as u32) & 15) as usize];
        shift = (shift + step) % 29;
    }
    key
}

fn transform_resource(bytes: &mut [u8], mut seed: u32) -> Result<()> {
    ensure!(
        bytes.len().is_multiple_of(4),
        "RES00 encrypted input has a partial word"
    );
    let mut table = [0u32; 64];
    for bit in 0..32 {
        let mut bits = 0;
        for (index, word) in table.iter_mut().enumerate() {
            if index % 31 == 0 {
                seed = seed.wrapping_mul(5).wrapping_add(0xb1ae_f645) >> 1;
                bits = seed;
            }
            *word |= (bits & 1) << bit;
            bits >>= 1;
        }
    }
    for (index, bytes) in bytes.chunks_exact_mut(4).enumerate() {
        let slot = index & 63;
        table[slot] ^= table[(slot + 3) & 63];
        let original = word(bytes) ^ table[(slot + 1) & 63];
        bytes.copy_from_slice(&original.to_le_bytes());
    }
    Ok(())
}

#[derive(Serialize)]
struct Resource {
    id: usize,
    name: String,
    flags: u8,
    size_field: u32,
    rdb_offset: Option<u64>,
    reference_id: Option<u32>,
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stored_alias_of: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decoded_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    strings_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    textures_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fonts_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decode_error: Option<String>,
}

#[derive(Serialize)]
struct PatchIndex {
    header_words: [u32; 2],
    ranges: Vec<PatchRange>,
}

#[derive(Serialize)]
struct PatchRange {
    rdb_offset: u64,
    size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

fn resources(index: &[u8], rdb_size: u64) -> Result<(Vec<Resource>, Option<PatchIndex>)> {
    let header = index.get(..56).context("truncated RDI2 header")?;
    ensure!(&header[..4] == b"RDI2", "expected a plaintext RDI2 index");
    ensure!(
        u16::from_le_bytes([header[4], header[5]]) == 1
            && matches!(u16::from_le_bytes([header[6], header[7]]), 0 | 2)
            && word(&header[8..]) as usize == index.len(),
        "unsupported RDI2 header or size"
    );
    // The patch-bearing RDI2 layout adds two words before the name offsets.
    // See UNPACK_RES00.py in masagrator/NXGameScripts.
    let patch_header = if word(&header[12..]) == 2 {
        Some(index.get(56..64).context("truncated RDI2 patch header")?)
    } else {
        None
    };
    let entries_start = if patch_header.is_some() { 64 } else { 56 };
    let count = word(&header[16..]) as usize;
    ensure!(
        word(&header[20..]) as usize == count,
        "RDI2 entry counts differ"
    );
    ensure!(
        count <= (index.len() - entries_start) / 13 && count <= 262144,
        "invalid RDI2 file count"
    );
    let names_end = entries_start + count * 4;
    let records_end = names_end + count * 9;
    let patch_entries = word(&header[36..]) as usize;
    ensure!(
        patch_entries <= (index.len() - records_end) / 8,
        "truncated RDI2 auxiliary table"
    );
    let patch_table_end = records_end + patch_entries * 8;
    let names = &index[patch_table_end..];
    let patch = patch_header.map(|extension| PatchIndex {
        header_words: [word(extension), word(&extension[4..])],
        ranges: index[records_end..patch_table_end]
            .chunks_exact(8)
            .map(|record| PatchRange {
                rdb_offset: u64::from(word(record)) * 0x200,
                size: word(&record[4..]),
                path: None,
            })
            .collect(),
    });
    let mut budget = 16 * 1024 * 1024;
    let mut ranges = BTreeMap::new();
    let mut total = 0u64;
    let entries = index[entries_start..names_end]
        .chunks_exact(4)
        .zip(index[names_end..records_end].chunks_exact(9))
        .enumerate()
        .map(|(id, (name_offset, entry))| {
            let tail = names
                .get(word(name_offset) as usize..)
                .context("RDI2 name outside table")?;
            let len = tail[..tail.len().min(budget).min(4096)]
                .iter()
                .position(|&byte| byte == 0)
                .context("unterminated RDI2 name or name limit exceeded")?;
            budget -= len + 1;
            let name = String::from_utf8_lossy(&tail[..len]).into_owned();
            let position = word(entry);
            let size_field = word(&entry[4..]);
            let flags = entry[8];
            let stored = matches!(flags, 0 | 0x20);
            let mut owner = None;
            let rdb_offset = if stored {
                let offset = u64::from(position) * 0x200;
                let size = u64::from(size_field);
                ensure!(
                    offset <= rdb_size && size <= rdb_size - offset,
                    "RDI2 resource {id} outside RDB"
                );
                owner = match ranges.entry((offset, size)) {
                    Entry::Occupied(entry) => Some(*entry.get()),
                    Entry::Vacant(entry) => {
                        entry.insert(id);
                        total = total
                            .checked_add(size)
                            .context("RDI2 total stored size overflow")?;
                        ensure!(total <= rdb_size, "RDI2 resource sizes exceed RDB length");
                        None
                    }
                };
                Some(offset)
            } else {
                None
            };
            Ok(Resource {
                id,
                name,
                flags,
                size_field,
                rdb_offset,
                reference_id: (!stored).then_some(position),
                path: stored.then(|| format!("chunks/{:06x}.bin", owner.unwrap_or(id))),
                stored_alias_of: owner,
                decoded_path: None,
                strings_path: None,
                textures_path: None,
                fonts_path: None,
                decode_error: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((entries, patch))
}

const CHUNK_LIMIT: usize = 64 * 1024 * 1024;

fn inflate(input: &[u8], zlib_header: bool) -> Result<Vec<u8>> {
    use flate2::{Decompress, FlushDecompress, Status};
    let mut decoder = Decompress::new(zlib_header);
    let mut buffer = [0u8; 32768];
    let mut decoded = Vec::new();
    loop {
        let (read, written) = (decoder.total_in(), decoder.total_out());
        let status =
            decoder.decompress(&input[read as usize..], &mut buffer, FlushDecompress::None)?;
        let output = (decoder.total_out() - written) as usize;
        ensure!(
            decoded.len() + output <= CHUNK_LIMIT,
            "RES00 inflated payload exceeds size limit"
        );
        decoded.extend_from_slice(&buffer[..output]);
        if status == Status::StreamEnd {
            return Ok(decoded);
        }
        ensure!(
            output != 0 || decoder.total_in() != read,
            "truncated RES00 compressed payload"
        );
    }
}

fn decode_chunk(mut bytes: Vec<u8>, name: &str, flags: u8) -> Result<Vec<u8>> {
    ensure!(
        name.is_ascii() && !name.is_empty(),
        "unsupported RES00 chunk filename encoding"
    );
    // RES00 encrypts complete little-endian words. The raw archive copy
    // retains any trailing bytes outside those words.
    bytes.truncate(bytes.len() / 4 * 4);
    transform_resource(
        &mut bytes,
        resource_key(b"RES00.RDI") ^ resource_key(name.as_bytes()),
    )?;
    let header = bytes.get(..32).context("truncated RES00 chunk header")?;
    let name_bytes = &header[..24];
    let end = name_bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(name_bytes.len());
    let basename = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = basename.rsplit_once('.').map_or(basename, |(stem, _)| stem);
    ensure!(
        name_bytes[..end].eq_ignore_ascii_case(basename.as_bytes())
            || name_bytes[..end].eq_ignore_ascii_case(stem.as_bytes()),
        "decoded RES00 chunk name differs from RDI2 entry"
    );
    if flags == 0x20 {
        let payload = inflate(&bytes[32..], true).or_else(|_| inflate(&bytes[32..], false))?;
        bytes.truncate(32);
        bytes.extend(payload);
    }
    Ok(bytes)
}

fn copy_range(source: &mut std::fs::File, offset: u64, size: u64, path: &Path) -> Result<()> {
    source.seek(SeekFrom::Start(offset))?;
    let mut file = std::fs::File::create_new(path)?;
    let copied = std::io::copy(&mut source.take(size), &mut file);
    drop(file);
    if !matches!(copied, Ok(count) if count == size) {
        std::fs::remove_file(path)?;
        ensure!(copied? == size, "RDB archive ended during copy");
    }
    Ok(())
}

fn assign_patch_paths(patch: &mut PatchIndex, length: u64) -> Result<()> {
    let mut paths = BTreeMap::new();
    let mut total = 0u64;
    for (id, range) in patch.ranges.iter_mut().enumerate() {
        let size = u64::from(range.size);
        ensure!(
            range.rdb_offset <= length && size <= length - range.rdb_offset,
            "RDI2 patch range {id} outside patch RDB"
        );
        match paths.entry((range.rdb_offset, size)) {
            Entry::Occupied(entry) => {
                range.path = Some(format!("patch-chunks/{:06x}.bin", entry.get()))
            }
            Entry::Vacant(entry) => {
                entry.insert(id);
                total = total
                    .checked_add(size)
                    .context("RDI2 patch size overflow")?;
                ensure!(total <= length, "RDI2 patch ranges exceed patch RDB length");
                range.path = Some(format!("patch-chunks/{id:06x}.bin"));
            }
        }
    }
    Ok(())
}

pub(crate) enum PatchArchive<'a> {
    Required(&'a Path),
    Optional(&'a Path),
    Absent,
}

pub(crate) fn extract(
    index: &Path,
    rdb: &Path,
    out: &Path,
    decode_chunks: bool,
    patch_data: PatchArchive<'_>,
) -> Result<Option<String>> {
    let mut index = std::fs::File::open(index).context("opening RDI2 index")?;
    ensure!(
        index.metadata()?.len() <= 64 * 1024 * 1024,
        "RDI2 index exceeds size limit"
    );
    let mut bytes = Vec::new();
    index.read_to_end(&mut bytes)?;
    if !bytes.starts_with(b"RDI2") {
        transform_resource(&mut bytes, resource_key(b"RES00.RDI"))?;
        ensure!(
            bytes.starts_with(b"RDI2"),
            "unrecognized RES00 index encoding"
        );
    }
    let mut rdb = std::fs::File::open(rdb).context("opening RDB archive")?;
    let (mut entries, mut patch) = resources(&bytes, rdb.metadata()?.len())?;
    let locate_patch = |path: &Path, patch: &mut Option<PatchIndex>| -> Result<std::fs::File> {
        let patch = patch.as_mut().context("RDI2 index has no patch table")?;
        let archive = std::fs::File::open(path).context("opening patch RDB archive")?;
        assign_patch_paths(patch, archive.metadata()?.len())?;
        Ok(archive)
    };
    let mut patch_error = None;
    let mut patch_rdb = match patch_data {
        PatchArchive::Absent => None,
        PatchArchive::Required(path) => Some(locate_patch(path, &mut patch)?),
        PatchArchive::Optional(path) => match locate_patch(path, &mut patch) {
            Ok(archive) => Some(archive),
            Err(error) => {
                if let Some(patch) = &mut patch {
                    for range in &mut patch.ranges {
                        range.path = None;
                    }
                }
                patch_error = Some(format!("{error:#}"));
                None
            }
        },
    };
    std::fs::create_dir_all(out)?;
    std::fs::create_dir(out.join("chunks")).context("creating a new RDB chunks directory")?;
    for entry in &mut entries {
        if let (Some(offset), Some(path)) = (entry.rdb_offset, &entry.path) {
            let path = out.join(path);
            let size = u64::from(entry.size_field);
            if entry.stored_alias_of.is_none() {
                copy_range(&mut rdb, offset, size, &path)?;
            }
            if decode_chunks {
                let decode = || -> Result<Vec<u8>> {
                    ensure!(size <= CHUNK_LIMIT as u64, "RES00 chunk exceeds size limit");
                    decode_chunk(std::fs::read(&path)?, &entry.name, entry.flags)
                };
                match decode() {
                    Ok(decoded) => {
                        let name = format!("decoded/{:06x}.bin", entry.id);
                        std::fs::create_dir_all(out.join("decoded"))?;
                        std::fs::write(out.join(&name), &decoded)?;
                        let strings = super::chk::strings(&decoded);
                        if !strings.is_empty() {
                            let path = format!("strings/{:06x}.json", entry.id);
                            std::fs::create_dir_all(out.join("strings"))?;
                            let file = std::fs::File::create(out.join(&path))?;
                            serde_json::to_writer_pretty(std::io::BufWriter::new(file), &strings)?;
                            entry.strings_path = Some(path);
                        }
                        if let Some(surfaces) = super::chk::surfaces(&decoded)
                            && !surfaces.is_empty()
                        {
                            let folder = format!("textures/{:06x}", entry.id);
                            std::fs::create_dir_all(out.join(&folder))?;
                            let mut index = Vec::new();
                            for (id, surface) in surfaces.iter().enumerate() {
                                let file = format!("{id:04x}.bin");
                                let start = surface.data_offset;
                                let bytes = &decoded[start..start + surface.size];
                                std::fs::write(out.join(&folder).join(&file), bytes)?;
                                let mut record =
                                    serde_json::json!({ "path": file, "surface": surface });
                                match surface.preview(bytes) {
                                    Ok(image) => {
                                        let png = format!("{id:04x}.png");
                                        image.save(out.join(&folder).join(&png))?;
                                        record["png"] = serde_json::json!(png);
                                    }
                                    Err(error) => {
                                        record["preview_error"] =
                                            serde_json::json!(format!("{error:#}"));
                                    }
                                }
                                index.push(record);
                            }
                            let path = format!("{folder}/index.json");
                            let file = std::fs::File::create(out.join(&path))?;
                            serde_json::to_writer_pretty(std::io::BufWriter::new(file), &index)?;
                            entry.textures_path = Some(path);
                        }
                        entry.fonts_path = super::chk::export_fonts(&decoded, out, entry.id)?;
                        entry.decoded_path = Some(name);
                    }
                    Err(error) => {
                        eprintln!("RES00 resource {}: {error:#}", entry.id);
                        entry.decode_error = Some(format!("{error:#}"));
                    }
                }
            }
        }
    }
    if let Some(patch) = patch {
        if let Some(archive) = &mut patch_rdb {
            std::fs::create_dir(out.join("patch-chunks"))?;
            let mut copied = std::collections::HashSet::new();
            for range in &patch.ranges {
                let path = range.path.as_deref().context("missing patch output path")?;
                if copied.insert(path) {
                    copy_range(
                        archive,
                        range.rdb_offset,
                        u64::from(range.size),
                        &out.join(path),
                    )?;
                }
            }
        }
        let file = std::fs::File::create(out.join("res00-patch-index.json"))?;
        serde_json::to_writer_pretty(std::io::BufWriter::new(file), &patch)?;
    }
    let index = std::fs::File::create(out.join("res00-index.json"))?;
    serde_json::to_writer_pretty(std::io::BufWriter::new(index), &entries)?;
    Ok(patch_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_published_res00_cipher_vectors() {
        assert_eq!(resource_key(b"RES00.RDI"), 0xf006_f0c9);
        assert_eq!(resource_key(b"CESAWARN.CHK"), 0x04fc_aff2);
        assert_eq!(resource_key(b"dir/Test.bin"), 0x1636_5969);
        let mut reference = [
            0xb4, 0xc9, 0xc7, 0x74, 0x0a, 0x8d, 0xd5, 0x88, 0x3d, 0x30, 0x3d, 0xab, 0x9b, 0xc3,
            0x94, 0xed,
        ];
        transform_resource(&mut reference, resource_key(b"RES00.RDI")).unwrap();
        assert_eq!(&reference, b"RDI2\0\0\0\0\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn decrypts_and_inflates_named_chunks_with_both_deflate_wrappers() {
        use std::io::Write;
        let payload = b"baseball resource";
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        zlib.write_all(payload).unwrap();
        let mut raw =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        raw.write_all(payload).unwrap();
        for data in [zlib.finish().unwrap(), raw.finish().unwrap()] {
            let mut chunk = vec![0; 32];
            chunk[..8].copy_from_slice(b"test.chk");
            chunk.extend(data);
            chunk.resize(chunk.len().next_multiple_of(4), 0);
            transform_resource(
                &mut chunk,
                resource_key(b"RES00.RDI") ^ resource_key(b"test.chk"),
            )
            .unwrap();
            assert_eq!(
                &decode_chunk(chunk, "test.chk", 0x20).unwrap()[32..],
                payload
            );
        }
        assert!(inflate(&[0x78, 0x9c, 0x00], true).is_err());
    }

    #[test]
    fn extracts_png_and_raw_texture_from_encrypted_res00() {
        use std::io::Write;
        let mut chk = vec![0; 32 + 0xe0 + 512];
        chk[..4].copy_from_slice(b"CHK ");
        for (offset, value) in [(16, 32u32), (20, 32 + 0xe0), (24, 1), (28, 512)] {
            chk[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        let descriptor = &mut chk[32..32 + 0xe0];
        descriptor[8] = 0xe0;
        descriptor[16..24].copy_from_slice(b"NX  SUR ");
        descriptor[32..37].copy_from_slice(b"x.tsr");
        descriptor[64] = 4;
        descriptor[68] = 4;
        descriptor[0x54] = 0x20;
        descriptor[0x71] = 2;
        // The first block of a single Tegra GOB starts at byte zero.
        chk[32 + 0xe0..34 + 0xe0].copy_from_slice(&0x001fu16.to_le_bytes());

        let mut compress =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        compress.write_all(&chk).unwrap();
        let mut chunk = vec![0; 32];
        chunk[..8].copy_from_slice(b"test.chk");
        chunk.extend(compress.finish().unwrap());
        chunk.resize(chunk.len().next_multiple_of(4), 0);
        transform_resource(
            &mut chunk,
            resource_key(b"RES00.RDI") ^ resource_key(b"test.chk"),
        )
        .unwrap();

        let mut index = vec![0; 56];
        index[..4].copy_from_slice(b"RDI2");
        index[4] = 1;
        index[16] = 1;
        index[20] = 1;
        index.extend(0u32.to_le_bytes()); // Name offset
        index.extend(0u32.to_le_bytes()); // RDB sector
        index.extend((chunk.len() as u32).to_le_bytes());
        index.push(0x20);
        index.extend(b"test.chk\0");
        index.resize(index.len().next_multiple_of(4), 0);
        let index_size = index.len() as u32;
        index[8..12].copy_from_slice(&index_size.to_le_bytes());
        transform_resource(&mut index, resource_key(b"RES00.RDI")).unwrap();

        let temp = tempfile::tempdir().unwrap();
        let idx = temp.path().join("RES00.RDI");
        let rdb = temp.path().join("RES00.RDB");
        let out = temp.path().join("out");
        std::fs::write(&idx, index).unwrap();
        std::fs::write(&rdb, chunk).unwrap();
        extract(&idx, &rdb, &out, true, PatchArchive::Absent).unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("res00-index.json")).unwrap()).unwrap();
        let index = out.join(manifest[0]["textures_path"].as_str().unwrap());
        let texture: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
        let folder = index.parent().unwrap();
        let image = image::open(folder.join(texture[0]["png"].as_str().unwrap()))
            .unwrap()
            .into_rgba8();
        assert_eq!(image.dimensions(), (4, 4));
        assert!(image.pixels().all(|pixel| pixel.0 == [0, 0, 255, 255]));
        assert_eq!(
            std::fs::read(folder.join(texture[0]["path"].as_str().unwrap())).unwrap(),
            &chk[32 + 0xe0..]
        );
    }

    #[test]
    fn preserves_partial_words_in_the_raw_archive_copy() {
        let mut index = vec![0; 56];
        index[..4].copy_from_slice(b"RDI2");
        index[4] = 1;
        index[16] = 1;
        index[20] = 1;
        index.extend(0u32.to_le_bytes());
        index.extend([0, 0, 0, 0, 38, 0, 0, 0, 0]);
        index.extend(b"test.chk\0");
        let size = index.len() as u32;
        index[8..12].copy_from_slice(&size.to_le_bytes());

        let mut expected = vec![0; 32];
        expected[..8].copy_from_slice(b"test.chk");
        expected.extend(b"data");
        let mut raw = expected.clone();
        transform_resource(
            &mut raw,
            resource_key(b"RES00.RDI") ^ resource_key(b"test.chk"),
        )
        .unwrap();
        raw.extend([0xab, 0xcd]);

        let temp = tempfile::tempdir().unwrap();
        let idx = temp.path().join("RES00.RDI");
        let rdb = temp.path().join("RES00.RDB");
        let out = temp.path().join("out");
        std::fs::write(&idx, index).unwrap();
        std::fs::write(&rdb, &raw).unwrap();
        extract(&idx, &rdb, &out, true, PatchArchive::Absent).unwrap();
        assert_eq!(std::fs::read(out.join("chunks/000000.bin")).unwrap(), raw);
        assert_eq!(
            std::fs::read(out.join("decoded/000000.bin")).unwrap(),
            expected
        );
    }

    #[test]
    fn copies_shared_ranges_once_and_preserves_each_label() {
        let mut index = vec![0; 56];
        index[..4].copy_from_slice(b"RDI2");
        index[4] = 1;
        index[16] = 2;
        index[20] = 2;
        index.extend(0u32.to_le_bytes());
        index.extend(4u32.to_le_bytes());
        let record = [0, 0, 0, 0, 3, 0, 0, 0, 0];
        index.extend(record);
        index.extend(record);
        index.extend(b"one\0two\0");
        let size = index.len() as u32;
        index[8..12].copy_from_slice(&size.to_le_bytes());

        let temp = tempfile::tempdir().unwrap();
        let idx = temp.path().join("data.rdi");
        let rdb = temp.path().join("data.rdb");
        let out = temp.path().join("out");
        std::fs::write(&idx, index).unwrap();
        std::fs::write(&rdb, b"raw").unwrap();
        extract(&idx, &rdb, &out, false, PatchArchive::Absent).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("res00-index.json")).unwrap()).unwrap();
        assert_eq!(manifest[0]["path"], manifest[1]["path"]);
        assert_eq!(manifest[1]["name"], "two");
        assert_eq!(manifest[1]["stored_alias_of"], 0);
        assert_eq!(std::fs::read_dir(out.join("chunks")).unwrap().count(), 1);
    }

    #[test]
    fn distinguishes_rdb_ranges_from_reference_ids() {
        let mut index = vec![0; 56];
        index[..4].copy_from_slice(b"RDI2");
        index[4] = 1;
        index[16] = 2;
        index[20] = 2;
        index.extend(0u32.to_le_bytes());
        index.extend(4u32.to_le_bytes());
        index.extend([1, 0, 0, 0, 3, 0, 0, 0, 0x20]);
        index.extend([5, 0, 0, 0, 0, 0, 0, 0, 0x40]);
        index.extend(b"one\0two\0");
        let size = index.len() as u32;
        index[8..12].copy_from_slice(&size.to_le_bytes());
        let (entries, patch) = resources(&index, 515).unwrap();
        assert!(patch.is_none());
        assert_eq!(entries[0].name, "one");
        assert_eq!(entries[0].rdb_offset, Some(512));
        assert_eq!(entries[1].reference_id, Some(5));
        assert!(resources(&index, 514).is_err());

        let temp = tempfile::tempdir().unwrap();
        let idx = temp.path().join("data.rdi");
        let rdb = temp.path().join("data.rdb");
        let out = temp.path().join("out");
        std::fs::write(&idx, index).unwrap();
        let mut data = vec![0; 512];
        data.extend(b"raw");
        std::fs::write(&rdb, data).unwrap();
        extract(&idx, &rdb, &out, false, PatchArchive::Absent).unwrap();
        assert_eq!(
            std::fs::read(out.join("chunks/000000.bin")).unwrap(),
            b"raw"
        );
        assert!(!out.join("chunks/000001.bin").exists());
    }
    #[test]
    fn extracts_base_entries_from_a_patch_index_and_preserves_patch_ranges() {
        let mut index = vec![0; 64];
        index[..4].copy_from_slice(b"RDI2");
        index[4] = 1;
        index[12] = 2;
        index[16] = 2;
        index[20] = 2;
        index[36] = 1;
        index[56] = 9;
        index.extend(0u32.to_le_bytes());
        index.extend(4u32.to_le_bytes());
        index.extend([0, 0, 0, 0, 3, 0, 0, 0, 0]);
        index.extend([0, 0, 0, 0, 0, 0, 0, 0, 0x40]);
        index.extend([7, 0, 0, 0, 32, 0, 0, 0]);
        index.extend(b"one\0two\0");
        let size = index.len() as u32;
        index[8..12].copy_from_slice(&size.to_le_bytes());
        let temp = tempfile::tempdir().unwrap();
        let idx = temp.path().join("data.rdi");
        let rdb = temp.path().join("data.rdb");
        let out = temp.path().join("out");
        std::fs::write(&idx, index).unwrap();
        std::fs::write(&rdb, b"raw").unwrap();
        extract(&idx, &rdb, &out, false, PatchArchive::Absent).unwrap();
        assert_eq!(
            std::fs::read(out.join("chunks/000000.bin")).unwrap(),
            b"raw"
        );
        assert!(!out.join("chunks/000001.bin").exists());
        let patch: serde_json::Value =
            serde_json::from_slice(&std::fs::read(out.join("res00-patch-index.json")).unwrap())
                .unwrap();
        assert_eq!(patch["header_words"][0], 9);
        assert_eq!(patch["ranges"][0]["rdb_offset"], 7 * 512);
        assert_eq!(patch["ranges"][0]["size"], 32);
        let patch_rdb = temp.path().join("RES11.RDB");
        std::fs::write(&patch_rdb, [0]).unwrap();
        let with_patch = temp.path().join("with-patch");
        assert!(
            extract(
                &idx,
                &rdb,
                &with_patch,
                false,
                PatchArchive::Required(&patch_rdb)
            )
            .is_err()
        );
        assert!(!with_patch.exists());
        let mut data = vec![0; 7 * 512];
        data.extend([0xa5; 32]);
        std::fs::write(&patch_rdb, data).unwrap();
        extract(
            &idx,
            &rdb,
            &with_patch,
            false,
            PatchArchive::Required(&patch_rdb),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(with_patch.join("patch-chunks/000000.bin")).unwrap(),
            [0xa5; 32]
        );
    }
}
