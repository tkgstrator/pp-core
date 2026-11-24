//! Render PP6 sound natively, for comparing with emulator recordings:
//!   cargo run --release -p pp-sound --example render -- DATA OUT.wav bgm|se ID SECONDS
//! DATA is a pp-extract sound blob or a whole (possibly patched) ROM image.
//! The WAV is 32,768 Hz stereo in mGBA's output units (48 per DAC step).
use pp_sound::{NATIVE_RATE, Player, Rom};
use std::io::Write;
use std::rc::Rc;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    assert!(a.len() == 6, "usage: render DATA OUT.wav bgm|se ID SECONDS");
    let data = std::fs::read(&a[1]).expect("read data");
    let rom = Rom::from_blob(&data).unwrap_or_else(|| Rom::from_image(&data));
    let mut player = Player::new(Rc::new(rom));
    // PP_TIMING=timer,counter,vblank,latency (cycles) lines up with an mGBA recording.
    if let Ok(t) = std::env::var("PP_TIMING") {
        let v: Vec<u64> = t.split(',').map(|x| x.parse().expect("timing")).collect();
        player.set_timing(v[0], v[1] as u32, v[2], v[3]);
    }
    let id = u32::from_str_radix(&a[4], 16).expect("hex id");
    if a[3] == "se" {
        player.play_se(id)
    } else {
        player.play_bgm(id)
    }
    let frames = (a[5].parse::<f64>().expect("seconds") * NATIVE_RATE as f64) as usize;
    let mut pcm = Vec::with_capacity(frames * 4);
    for _ in 0..frames {
        let (l, r) = player.next_dac();
        pcm.extend(((l * 48) as i16).to_le_bytes());
        pcm.extend(((r * 48) as i16).to_le_bytes());
    }
    let mut f = std::fs::File::create(&a[2]).expect("create wav");
    let header = |f: &mut std::fs::File| -> std::io::Result<()> {
        f.write_all(b"RIFF")?;
        f.write_all(&(36 + pcm.len() as u32).to_le_bytes())?;
        f.write_all(b"WAVEfmt ")?;
        f.write_all(&16u32.to_le_bytes())?;
        f.write_all(&[1, 0, 2, 0])?;
        f.write_all(&NATIVE_RATE.to_le_bytes())?;
        f.write_all(&(NATIVE_RATE * 4).to_le_bytes())?;
        f.write_all(&[4, 0, 16, 0])?;
        f.write_all(b"data")?;
        f.write_all(&(pcm.len() as u32).to_le_bytes())?;
        f.write_all(&pcm)
    };
    header(&mut f).expect("write wav");
}
