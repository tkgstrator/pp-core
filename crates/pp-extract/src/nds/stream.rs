use super::{adpcm_samples, graphics::section, le_half, le_word, write_loop_metadata};
use anyhow::{Context, Result, ensure};
use std::path::Path;

pub(super) fn export(file: &[u8], root: &Path, id: usize) -> Result<()> {
    let head = section(file, b"HEAD")?;
    let head = head.get(..40).context("truncated STRM HEAD block")?;
    let codec = head[0];
    let bytes_per_sample = match codec {
        0 => 1,
        1 | 2 => 2,
        codec => anyhow::bail!("unsupported STRM codec {codec}"),
    };
    let channels = usize::from(head[2]);
    let rate = le_half(&head[4..]);
    ensure!(
        (1..=16).contains(&channels) && rate != 0,
        "invalid STRM channels or sample rate"
    );

    let blocks = le_word(&head[20..]);
    let frames = le_word(&head[12..]);
    let normal = (le_word(&head[24..]), le_word(&head[28..]));
    let mut last = (le_word(&head[32..]), le_word(&head[36..]));
    let adpcm_bytes = |samples: usize| samples.div_ceil(2) + 4;
    // Some single-block streams leave the predictor out of the stored size.
    if codec == 2 && blocks == 1 && last.0 < adpcm_bytes(last.1) {
        last.0 = last
            .0
            .checked_add(4)
            .context("STRM predictor size overflow")?;
    }
    let data = section(file, b"DATA")?;
    ensure!(
        blocks > 0 && frames > 0 && blocks <= data.len() / channels,
        "invalid STRM block count"
    );

    let block_size = |(stored, samples): (usize, usize)| -> Result<usize> {
        let enough = if codec == 2 {
            stored >= adpcm_bytes(samples)
        } else {
            samples <= stored / bytes_per_sample
        };
        ensure!(samples > 0 && enough, "invalid STRM block sample count");
        let bytes = stored
            .checked_mul(channels)
            .context("STRM block size overflow")?;
        let aligned = bytes
            .checked_add(3)
            .context("STRM block alignment overflow")?
            & !3;
        Ok(aligned)
    };
    let normal_size = if blocks > 1 { block_size(normal)? } else { 0 };
    let last_size = block_size(last)?;
    let prefix = (blocks - 1)
        .checked_mul(normal_size)
        .context("STRM block count overflow")?;
    ensure!(
        prefix
            .checked_add(last_size)
            .is_some_and(|size| size <= data.len()),
        "STRM blocks outside DATA"
    );
    ensure!(
        (blocks - 1)
            .checked_mul(normal.1)
            .and_then(|size| size.checked_add(last.1))
            == Some(frames),
        "STRM frame count does not match blocks"
    );

    let out = root.join("wav");
    std::fs::create_dir_all(&out)?;
    let path = out.join(format!("{id:04x}.wav"));
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: channels as u16,
            sample_rate: rate as u32,
            bits_per_sample: (bytes_per_sample * 8) as u16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for block in 0..blocks {
        let (stride, samples) = if block == blocks - 1 { last } else { normal };
        let start = block * normal_size;
        if codec == 2 {
            let mut decoders: Vec<_> = data[start..start + channels * stride]
                .chunks_exact(stride)
                .map(adpcm_samples)
                .collect();
            for _ in 0..samples {
                for decoder in &mut decoders {
                    writer.write_sample(decoder.next().context("truncated STRM ADPCM samples")?)?;
                }
            }
            continue;
        }
        for frame in 0..samples {
            for channel in 0..channels {
                let at = start + channel * stride + frame * bytes_per_sample;
                if bytes_per_sample == 1 {
                    writer.write_sample(data[at] as i8)?;
                } else {
                    writer.write_sample(i16::from_le_bytes([data[at], data[at + 1]]))?;
                }
            }
        }
    }
    writer.finalize()?;
    if head[1] != 0 {
        write_loop_metadata(&path, Some(le_word(&head[8..])), frames, rate as u32)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adpcm_fixture(blocks: u8, last_bytes: u8, samples: &[u8]) -> Vec<u8> {
        let mut file = b"STRM\xff\xfe\x00\x01\0\0\0\0\x10\0\x02\0".to_vec();
        file.extend(b"HEAD");
        file.extend(48u32.to_le_bytes());
        let mut head = [0; 40];
        head[0] = 2;
        head[2] = 2;
        head[4..6].copy_from_slice(&8000u16.to_le_bytes());
        head[12] = blocks * 2;
        head[20] = blocks;
        head[24] = 5;
        head[28] = 2;
        head[32] = last_bytes;
        head[36] = 2;
        file.extend(head);
        file.extend(b"DATA");
        file.extend((8 + samples.len() as u32).to_le_bytes());
        file.extend(samples);
        let size = file.len() as u32;
        file[8..12].copy_from_slice(&size.to_le_bytes());
        file
    }

    #[test]
    fn exports_interleaved_adpcm_and_resets_each_block_predictor() {
        let out = tempfile::tempdir().unwrap();
        let samples = [
            0, 0, 0, 0, 0xf7, 100, 0, 0, 0, 0, 0, 0, 200, 0, 0, 0, 0, 44, 1, 0, 0, 0, 0, 0,
        ];
        export(&adpcm_fixture(2, 5, &samples), out.path(), 1).unwrap();
        export(&adpcm_fixture(1, 1, &samples[..12]), out.path(), 2).unwrap();
        for (id, expected) in [
            (1, &[11, 100, -19, 100, 200, 300, 200, 300][..]),
            (2, &[11, 100, -19, 100][..]),
        ] {
            let mut wav =
                hound::WavReader::open(out.path().join(format!("wav/{id:04x}.wav"))).unwrap();
            assert_eq!(wav.spec().channels, 2);
            assert_eq!(wav.spec().bits_per_sample, 16);
            assert_eq!(
                wav.samples::<i16>()
                    .collect::<hound::Result<Vec<_>>>()
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn exports_stereo_pcm_with_block_padding() {
        let mut file = b"STRM\xff\xfe\x00\x01\0\0\0\0\x10\0\x02\0".to_vec();
        file.extend(b"HEAD");
        file.extend(48u32.to_le_bytes());
        let mut head = [0; 40];
        head[2] = 2;
        head[4..6].copy_from_slice(&8000u16.to_le_bytes());
        head[12] = 4;
        head[20] = 2;
        head[24] = 3;
        head[28] = 3;
        head[32] = 1;
        head[36] = 1;
        file.extend(head);
        file.extend(b"DATA");
        file.extend(20u32.to_le_bytes());
        file.extend([1, 2, 3, 11, 12, 13, 0, 0, 4, 14, 0, 0]);
        let size = file.len() as u32;
        file[8..12].copy_from_slice(&size.to_le_bytes());

        let out = tempfile::tempdir().unwrap();
        export(&file, out.path(), 3).unwrap();
        let mut wav = hound::WavReader::open(out.path().join("wav/0003.wav")).unwrap();
        assert_eq!(wav.spec().channels, 2);
        assert_eq!(wav.spec().sample_rate, 8000);
        let samples: Vec<_> = wav.samples::<i8>().collect::<hound::Result<_>>().unwrap();
        assert_eq!(samples, [1, 11, 2, 12, 3, 13, 4, 14]);

        file[36] = 5;
        assert!(export(&file, out.path(), 4).is_err());
        assert!(!out.path().join("wav/0004.wav").exists());
    }
    #[test]
    fn reports_the_stream_loop_in_frames_per_channel() {
        let dir = tempfile::tempdir().unwrap();
        let samples = [0; 12];
        let mut file = adpcm_fixture(1, 5, &samples);
        file[25] = 1;
        file[32] = 1;
        export(&file, dir.path(), 3).unwrap();
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("wav/0003.loop.json")).unwrap())
                .unwrap();
        assert_eq!(metadata["loop_start_frame"], 1);
        assert_eq!(metadata["loop_end_frame_exclusive"], 2);
    }
}
