use anyhow::{Context, Result, ensure};

fn header(input: &[u8]) -> Option<(usize, usize)> {
    let ([kind @ (0x10 | 0x11 | 0x24 | 0x28 | 0x30), a, b, c], _) =
        input.split_first_chunk::<4>()?
    else {
        return None;
    };
    let size = u32::from_le_bytes([*a, *b, *c, 0]) as usize;
    if size == 0 && *kind == 0x11 {
        let field = input.get(4..8)?.try_into().ok()?;
        Some((8, u32::from_le_bytes(field) as usize))
    } else {
        Some((4, size))
    }
}

pub(super) fn recognized_suffix(suffix: &str) -> bool {
    ["lz", "lz10", "lz11", "rle", "rl", "huff", "huf"]
        .iter()
        .any(|known| suffix.eq_ignore_ascii_case(known))
}

pub(super) fn candidate(input: &[u8], name: Option<&str>) -> bool {
    let Some((offset, size)) = header(input) else {
        return false;
    };
    if name.is_some_and(|path| {
        path.rsplit_once('.')
            .is_some_and(|(_, suffix)| recognized_suffix(suffix))
    }) {
        return true;
    }
    if matches!(input[0], 0x24 | 0x28) {
        return size >= 4
            && huffman(&input[offset..], 4, input[0] & 15)
                .is_ok_and(|(prefix, _)| known_signature(&prefix));
    }
    // Check the stored prefix for a known signature before probing unnamed data.
    let Some((&flags, data)) = input.get(offset..).and_then(<[u8]>::split_first) else {
        return false;
    };
    let literal_prefix = if input[0] == 0x30 {
        (3..=127).contains(&flags)
    } else {
        flags & 0xf0 == 0
    };
    literal_prefix && known_signature(data)
}

fn known_signature(data: &[u8]) -> bool {
    matches!(
        data.get(..4),
        Some(
            b"NARC"
                | b"SDAT"
                | b"STRM"
                | b"SWAV"
                | b"SWAR"
                | b"SSEQ"
                | b"SSAR"
                | b"SBNK"
                | b"RGCN"
                | b"RLCN"
                | b"RCSN"
                | b"RECN"
                | b"RNAN"
                | b"RTFN"
                | b"MESG"
                | b"BMD0"
        )
    )
}

pub(super) fn decompress(input: &[u8], maximum: usize) -> Result<Vec<u8>> {
    decode(input, maximum, false).map(|(bytes, _)| bytes)
}

pub(super) fn probe(input: &[u8], maximum: usize) -> Option<Vec<u8>> {
    let (bytes, used) = decode(input, maximum, true).ok()?;
    let trailing = &input[used..];
    (trailing.is_empty()
        || (trailing.len() < 4
            && input.len().is_multiple_of(4)
            && trailing.iter().all(|&byte| byte == 0)))
    .then_some(bytes)
}

fn decode(input: &[u8], maximum: usize, exact: bool) -> Result<(Vec<u8>, usize)> {
    let (offset, size) = header(input).context("invalid Nintendo compression header")?;
    ensure!(
        size > 0 && size <= maximum,
        "Nintendo decompressed size outside output limit"
    );
    if input[0] == 0x30 {
        let (bytes, used) = rle(&input[offset..], size)?;
        return Ok((bytes, offset + used));
    }
    if matches!(input[0], 0x24 | 0x28) {
        let (bytes, used) = huffman(&input[offset..], size, input[0] & 15)?;
        return Ok((bytes, offset + used));
    }
    let lz11 = input[0] == 0x11;
    let mut data = input[offset..].iter().copied();
    let mut byte = || {
        data.next()
            .context("truncated Nintendo LZ stream")
            .map(usize::from)
    };
    let mut output = Vec::new();
    while output.len() < size {
        let flags = byte()?;
        for bit in (0..8).rev() {
            if flags & (1 << bit) == 0 {
                output.push(byte()? as u8);
            } else {
                let first = byte()?;
                let second = byte()?;
                let (length, upper, lower) = match (lz11, first >> 4) {
                    (true, 0) => (((first & 15) << 4 | second >> 4) + 0x11, second, byte()?),
                    (true, 1) => {
                        let third = byte()?;
                        (
                            ((first & 15) << 12 | second << 4 | third >> 4) + 0x111,
                            third,
                            byte()?,
                        )
                    }
                    (true, short) => (short + 1, first, second),
                    (false, short) => (short + 3, first, second),
                };
                let distance = ((upper & 15) << 8 | lower) + 1;
                ensure!(
                    distance <= output.len(),
                    "Nintendo LZ reference before output start"
                );
                let remaining = size - output.len();
                ensure!(
                    !exact || length <= remaining,
                    "Nintendo LZ block exceeds declared output size"
                );
                for _ in 0..length.min(remaining) {
                    output.push(output[output.len() - distance]);
                }
            }
            if output.len() == size {
                break;
            }
        }
    }
    Ok((output, input.len() - data.len()))
}

fn huffman(data: &[u8], size: usize, depth: u8) -> Result<(Vec<u8>, usize)> {
    let size_byte = *data.first().context("missing Nintendo Huffman tree size")?;
    let tree_len = (usize::from(size_byte) + 1) * 2;
    let (tree, stream) = data
        .split_at_checked(tree_len)
        .context("truncated Nintendo Huffman tree")?;
    let mut output = Vec::new();
    // Tree indices include the length byte so child pairs start on even offsets.
    let mut node = 1;
    let mut low = None;
    for (index, bytes) in stream.chunks_exact(4).enumerate() {
        let word = u32::from_le_bytes(bytes.try_into().unwrap());
        for shift in (0..32).rev() {
            let bit = ((word >> shift) & 1) as usize;
            let descriptor = tree[node];
            let child = (node & !1) + usize::from(descriptor & 0x3f) * 2 + 2 + bit;
            let &value = tree
                .get(child)
                .context("Nintendo Huffman node outside tree")?;
            if descriptor & (0x80 >> bit) == 0 {
                node = child;
                continue;
            }
            node = 1;
            if depth == 8 {
                output.push(value);
            } else {
                let value = value & 15;
                if let Some(first) = low.take() {
                    output.push(first | (value << 4));
                } else {
                    low = Some(value);
                }
            }
            if output.len() == size {
                return Ok((output, tree_len + (index + 1) * 4));
            }
        }
    }
    anyhow::bail!("truncated Nintendo Huffman bitstream")
}

fn rle(mut data: &[u8], size: usize) -> Result<(Vec<u8>, usize)> {
    let input_len = data.len();
    let mut output = Vec::new();
    while output.len() < size {
        let (&control, rest) = data
            .split_first()
            .context("truncated Nintendo RLE control")?;
        let repeat = control & 0x80 != 0;
        let count = usize::from(control & 0x7f) + if repeat { 3 } else { 1 };
        ensure!(
            count <= size - output.len(),
            "Nintendo RLE block exceeds declared output size"
        );
        if repeat {
            let (&value, rest) = rest
                .split_first()
                .context("truncated Nintendo RLE repeat")?;
            output.extend(std::iter::repeat_n(value, count));
            data = rest;
        } else {
            let (literal, rest) = rest
                .split_at_checked(count)
                .context("truncated Nintendo RLE literal")?;
            output.extend_from_slice(literal);
            data = rest;
        }
    }
    Ok((output, input_len - data.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_only_complete_streams_with_optional_alignment_padding() {
        let raw = b"\x10\x03\0\0\0\x01\x02\xfa";
        assert_eq!(probe(raw, 16).unwrap(), b"\x01\x02\xfa");
        assert_eq!(probe(b"\x10\x02\0\0\0\x01\x02\0", 16).unwrap(), b"\x01\x02");
        assert!(probe(b"\x10\x02\0\0\0\x01\x02\xff", 16).is_none());
        assert!(probe(b"\x10\x03\0\0\x40A\x10\0", 16).is_none());
        assert!(probe(&raw[..7], 16).is_none());
    }

    #[test]
    fn decodes_huffman_words_and_low_nibble_first() {
        let packed = b"\x28\x04\0\0\x03\0\xc0\xc1NARC\0\0\0\x1b";
        assert!(candidate(packed, None));
        assert_eq!(decompress(packed, 4).unwrap(), b"NARC");
        let packed = b"\x24\x04\0\0\x01\xc0\x01\x02\0\0\0\x55";
        assert!(candidate(packed, Some("data.huff")));
        assert_eq!(decompress(packed, 4).unwrap(), b"!!!!");
        assert!(decompress(b"\x28\x04\0\0\x01\xff\x01\x02\0\0\0\0", 4).is_err());
        assert!(decompress(&packed[..packed.len() - 1], 4).is_err());
    }

    #[test]
    fn recognizes_and_decodes_rle_without_guessing_from_the_type_byte() {
        let packed = b"\x30\x08\0\0\x03NARC\x81!";
        assert!(candidate(packed, None));
        assert!(!candidate(b"\x30\x08\0\0\x03junk\x81!", None));
        assert!(candidate(b"\x30\x08\0\0\x03junk\x81!", Some("art.RLE")));
        assert_eq!(decompress(packed, 8).unwrap(), b"NARC!!!!");
        assert!(decompress(packed, 7).is_err());
        assert!(decompress(&packed[..packed.len() - 1], 8).is_err());
        assert!(decompress(b"\x30\x03\0\0\x81!", 8).is_err());
    }

    #[test]
    fn reads_overlapping_tokens_in_all_length_forms() {
        for (input, count) in [
            (&[0x10, 6, 0, 0, 0x40, b'A', 0x20, 0][..], 6),
            (&[0x11, 6, 0, 0, 0x40, b'A', 0x40, 0][..], 6),
            (&[0x11, 18, 0, 0, 0x40, b'A', 0, 0, 0][..], 18),
            (
                &[0x11, 0, 0, 0, 18, 1, 0, 0, 0x40, b'A', 0x10, 0, 0, 0][..],
                274,
            ),
        ] {
            assert_eq!(decompress(input, 300).unwrap(), vec![b'A'; count]);
        }
        assert!(decompress(&[0x10, 3, 0, 0, 0x80, 0, 0], 300).is_err());
        assert!(decompress(&[0x10, 3, 0, 0, 0], 300).is_err());
    }
}
