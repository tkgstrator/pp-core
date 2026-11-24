//! ROM bytes by GBA address, from a pp-extract sound blob or a whole ROM.
use std::cell::Cell;

pub struct Rom {
    /// (ROM offset, bytes), sorted by offset.
    chunks: Vec<(u32, Vec<u8>)>,
    last: Cell<usize>,
}

impl Rom {
    /// A `PP6S` blob (crates/pp-extract `pp6_sound_blob`).
    pub fn from_blob(blob: &[u8]) -> Option<Rom> {
        let word = |at: usize| {
            blob.get(at..at + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        };
        if blob.get(..4)? != b"PP6S" {
            return None;
        }
        let mut chunks = Vec::new();
        let mut at = 8;
        for _ in 0..word(4)? {
            let (addr, len) = (word(at)? & 0x01ff_ffff, word(at + 4)? as usize);
            chunks.push((addr, blob.get(at + 8..at + 8 + len)?.to_vec()));
            at += 8 + len.next_multiple_of(4);
        }
        chunks.sort_by_key(|c| c.0);
        Some(Rom {
            chunks,
            last: Cell::new(0),
        })
    }

    /// A whole cartridge image (native tools and tests).
    pub fn from_image(rom: &[u8]) -> Rom {
        Rom {
            chunks: vec![(0, rom.to_vec())],
            last: Cell::new(0),
        }
    }

    /// The bytes from `addr` to the end of its chunk (empty if not present).
    pub fn bytes(&self, addr: u32) -> &[u8] {
        let off = addr & 0x01ff_ffff;
        let hit = |i: usize| {
            let (start, data) = &self.chunks[i];
            (off >= *start && off < start + data.len() as u32)
                .then(|| &data[(off - start) as usize..])
        };
        if let Some(b) = hit(self.last.get()) {
            return b;
        }
        let i = self.chunks.partition_point(|c| c.0 <= off);
        if i > 0
            && let Some(b) = hit(i - 1)
        {
            self.last.set(i - 1);
            return b;
        }
        &[]
    }

    pub fn r8(&self, a: u32) -> u32 {
        self.bytes(a).first().copied().unwrap_or(0) as u32
    }
    pub fn r16(&self, a: u32) -> u32 {
        self.r8(a) | self.r8(a + 1) << 8
    }
    pub fn r32(&self, a: u32) -> u32 {
        self.r16(a) | self.r16(a + 2) << 16
    }
}
