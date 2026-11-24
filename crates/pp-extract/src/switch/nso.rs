use super::word;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const HEADER_SIZE: usize = 0x100;
const SIZE_LIMIT: usize = 256 * 1024 * 1024;

// Layout and flags match the NSO0 header described by hactool.
// https://github.com/SciresM/hactool/blob/master/nso.h
pub(crate) fn extract(path: &Path, out: &Path) -> Result<()> {
    let mut input =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let length = input.metadata()?.len();
    let mut header = [0; HEADER_SIZE];
    input
        .read_exact(&mut header)
        .context("reading NSO0 header")?;
    ensure!(
        header.starts_with(b"NSO0"),
        "expected a decrypted NSO0 executable"
    );

    let flags = word(&header[12..]);
    let mut segments = Vec::new();
    let mut total = 0usize;
    for (id, name) in ["text", "rodata", "data"].into_iter().enumerate() {
        let fields = &header[0x10 + id * 16..];
        let offset = u64::from(word(fields));
        let address = word(&fields[4..]);
        let size = word(&fields[8..]) as usize;
        let compressed = flags & (1 << id) != 0;
        let stored = if compressed {
            word(&header[0x60 + id * 4..]) as usize
        } else {
            size
        };
        total = total
            .checked_add(size)
            .context("NSO0 segment size overflow")?;
        ensure!(
            total <= SIZE_LIMIT && stored <= SIZE_LIMIT,
            "NSO0 segment sizes exceed limit"
        );
        ensure!(
            (size == 0 && stored == 0)
                || (offset >= HEADER_SIZE as u64
                    && offset <= length
                    && stored as u64 <= length - offset),
            "NSO0 {name} segment outside file"
        );
        let mut bytes = vec![0; stored];
        if stored > 0 {
            input.seek(SeekFrom::Start(offset))?;
            input
                .read_exact(&mut bytes)
                .with_context(|| format!("reading NSO0 {name}"))?;
        }
        if compressed {
            let mut expanded = vec![0; size];
            let decoded = lz4_flex::block::decompress_into(&bytes, &mut expanded)
                .with_context(|| format!("decoding NSO0 {name} LZ4 block"))?;
            ensure!(
                decoded == size,
                "NSO0 {name} expanded size differs from header"
            );
            bytes = expanded;
        }
        let hash = Sha256::digest(&bytes);
        let hash_offset = 0xa0 + 32 * id;
        let has_hash = flags & (1 << (id + 3)) != 0;
        if has_hash {
            ensure!(
                hash[..] == header[hash_offset..hash_offset + 32],
                "NSO0 {name} SHA-256 mismatch"
            );
        }
        let path = format!("{name}.bin");
        let metadata = serde_json::json!({
            "path": path,
            "rom_offset": offset,
            "memory_offset": address,
            "size": size,
            "stored_size": stored,
            "compressed": compressed,
            "sha256": format!("{hash:x}"),
            "hash_verified": has_hash,
        });
        segments.push((path, bytes, metadata));
    }

    std::fs::create_dir_all(out)?;
    let mut metadata = Vec::new();
    for (path, bytes, entry) in segments {
        std::fs::write(out.join(path), bytes)?;
        metadata.push(entry);
    }
    let manifest = serde_json::json!({
        "flags": flags,
        "bss_size": word(&header[0x3c..]),
        "segments": metadata,
    });
    std::fs::write(out.join("nso.json"), serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_compressed_rodata_and_requires_correct_hash() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("main");
        let mut nso = vec![0; HEADER_SIZE];
        nso[..4].copy_from_slice(b"NSO0");
        nso[12] = 0x12;
        nso[0x20..0x24].copy_from_slice(&0x100u32.to_le_bytes());
        nso[0x24..0x28].copy_from_slice(&0x2000u32.to_le_bytes());
        nso[0x28] = 4;
        nso[0x64] = 5;
        nso[0xc0..0xe0].copy_from_slice(&Sha256::digest(b"text"));
        nso.extend_from_slice(b"\x40text");
        std::fs::write(&source, &nso).unwrap();
        let out = dir.path().join("out");
        extract(&source, &out).unwrap();
        assert_eq!(std::fs::read(out.join("rodata.bin")).unwrap(), b"text");
        assert!(
            std::fs::read_to_string(out.join("nso.json"))
                .unwrap()
                .contains("8192")
        );
        nso[0xc0] ^= 1;
        std::fs::write(&source, nso).unwrap();
        let bad = dir.path().join("bad");
        assert!(
            extract(&source, &bad)
                .unwrap_err()
                .to_string()
                .contains("SHA-256 mismatch")
        );
        assert!(!bad.exists());
    }
}
