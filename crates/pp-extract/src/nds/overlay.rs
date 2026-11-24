use super::le_word;
use anyhow::{Context, Result, ensure};
use std::collections::HashSet;
use std::fmt::Write;
use std::ops::Range;
use std::path::Path;

pub(super) fn decompress(data: &[u8], expected: Option<usize>) -> Result<Vec<u8>> {
    let footer = data
        .get(data.len().saturating_sub(8)..)
        .filter(|data| data.len() == 8)
        .context("missing overlay BLZ footer")?;
    let span = le_word(footer);
    let header_len = span >> 24;
    let compressed_len = span & 0xff_ffff;
    let extra = le_word(&footer[4..]);
    ensure!(
        header_len >= 8 && compressed_len >= header_len && compressed_len <= data.len(),
        "invalid overlay BLZ lengths"
    );
    let total = data
        .len()
        .checked_add(extra)
        .context("overlay BLZ size overflow")?;
    ensure!(
        extra != 0 && total <= 64 * 1024 * 1024,
        "BLZ expanded size outside supported range"
    );
    ensure!(
        expected.is_none_or(|size| size == total),
        "overlay BLZ size differs from RAM size"
    );
    let prefix = data.len() - compressed_len;
    let payload_end = data.len() - header_len;
    ensure!(
        data[payload_end..data.len() - 8]
            .iter()
            .all(|&byte| byte == 0xff),
        "invalid overlay BLZ padding"
    );

    let mut input = data[prefix..payload_end].iter().rev().copied();
    let mut next = || input.next().context("truncated overlay BLZ stream");
    let mut decoded = Vec::new();
    let target = total - prefix;
    while decoded.len() < target {
        let flags = next()?;
        for bit in (0..8).rev() {
            if flags & (1 << bit) == 0 {
                decoded.push(next()?);
            } else {
                let hi = usize::from(next()?);
                let lo = usize::from(next()?);
                let length = (hi >> 4) + 3;
                let distance = ((hi & 15) << 8 | lo) + 3;
                ensure!(
                    distance <= decoded.len() && length <= target - decoded.len(),
                    "invalid overlay BLZ reference"
                );
                for _ in 0..length {
                    decoded.push(decoded[decoded.len() - distance]);
                }
            }
            if decoded.len() == target {
                break;
            }
        }
    }
    decoded.extend(data[..prefix].iter().rev());
    decoded.reverse();
    Ok(decoded)
}

pub(super) fn export(rom: &[u8], files: &[Range<usize>], root: &Path) -> Result<()> {
    for (cpu, field) in [("arm9", 0x50), ("arm7", 0x58)] {
        let offset = le_word(&rom[field..]);
        let size = le_word(&rom[field + 4..]);
        if size == 0 {
            continue;
        }
        let end = offset
            .checked_add(size)
            .context("overlay table range overflow")?;
        ensure!(
            offset >= 0x160 && size.is_multiple_of(32),
            "invalid {cpu} overlay table"
        );
        let table = rom.get(offset..end).context("overlay table outside ROM")?;
        let out = root.join("overlays").join(cpu);
        std::fs::create_dir_all(&out)?;
        let mut index = String::from(concat!(
            "overlay_id\tfile_id\tram_address\tram_size\tbss_size\t",
            "init_start\tinit_end\tflags\tstored_size\tpath\tstatus\n"
        ));
        let mut ids = HashSet::new();
        for record in table.chunks_exact(32) {
            let id = le_word(record);
            ensure!(ids.insert(id), "duplicate {cpu} overlay ID {id}");
            let ram = le_word(&record[4..]);
            let size = le_word(&record[8..]);
            let bss = le_word(&record[12..]);
            let init_start = le_word(&record[16..]);
            let init_end = le_word(&record[20..]);
            let file = le_word(&record[24..]);
            let field = le_word(&record[28..]);
            let flags = field >> 24;
            let stored = field & 0xff_ffff;
            let mut name = String::new();
            let mut status = "missing_file";
            if let Some(range) = files.get(file) {
                let raw = &rom[range.clone()];
                let output = if flags & 1 == 0 {
                    Ok(raw.to_vec())
                } else {
                    let packed = raw
                        .get(..stored)
                        .context("overlay compressed size outside FAT file");
                    packed.and_then(|data| decompress(data, Some(size)))
                };
                match output {
                    Ok(bytes) => {
                        status = if flags & 1 != 0 {
                            "decompressed"
                        } else {
                            "uncompressed"
                        };
                        name = format!("{id:04x}.bin");
                        std::fs::write(out.join(&name), bytes)?;
                    }
                    Err(error) => {
                        status = "invalid_blz";
                        eprintln!("{cpu} overlay {id}: {error:#}");
                    }
                }
            }
            writeln!(
                index,
                "{id}\t{file}\t{ram:08x}\t{size}\t{bss}\t{init_start:08x}\t{init_end:08x}\t{flags:02x}\t{stored}\t{name}\t{status}"
            )?;
        }
        std::fs::write(out.join("index.tsv"), index)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_overlay_using_its_fat_id_and_blz_trailer() {
        let mut rom = vec![0u8; 0x240];
        rom[0x50..0x54].copy_from_slice(&0x180u32.to_le_bytes());
        rom[0x54] = 32;
        rom[0x180] = 9;
        rom[0x188] = 21;
        rom[0x19c..0x1a0].copy_from_slice(&0x0100_000eu32.to_le_bytes());
        let packed = [0, 0xf0, b'A', b'A', b'A', 0x10, 14, 0, 0, 8, 7, 0, 0, 0];
        rom[0x200..0x20e].copy_from_slice(&packed);
        let out = tempfile::tempdir().unwrap();
        let ranges = std::iter::once(0x200..0x20e).collect::<Vec<_>>();
        export(&rom, &ranges, out.path()).unwrap();
        let folder = out.path().join("overlays/arm9");
        assert_eq!(std::fs::read(folder.join("0009.bin")).unwrap(), [b'A'; 21]);
        assert!(
            std::fs::read_to_string(folder.join("index.tsv"))
                .unwrap()
                .contains("\t0009.bin\tdecompressed")
        );
        assert!(decompress(&packed, Some(22)).is_err());
    }
}
