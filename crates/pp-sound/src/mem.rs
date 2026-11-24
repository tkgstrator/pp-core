//! Byte-addressed driver structs, mirroring the IWRAM layouts in
//! docs/pp6-sound-driver.md so offsets read like the disassembly.
#[derive(Clone)]
pub struct Mem(pub Vec<u8>);

impl Mem {
    pub fn new(size: usize) -> Mem {
        Mem(vec![0; size])
    }
    pub fn u8(&self, o: usize) -> i64 {
        self.0[o] as i64
    }
    pub fn u16(&self, o: usize) -> i64 {
        (self.0[o] as i64) | (self.0[o + 1] as i64) << 8
    }
    pub fn u32(&self, o: usize) -> i64 {
        self.u16(o) | self.u16(o + 2) << 16
    }
    pub fn w8(&mut self, o: usize, v: i64) {
        self.0[o] = v as u8;
    }
    pub fn w16(&mut self, o: usize, v: i64) {
        self.0[o..o + 2].copy_from_slice(&(v as u16).to_le_bytes());
    }
    pub fn w32(&mut self, o: usize, v: i64) {
        self.0[o..o + 4].copy_from_slice(&(v as u32).to_le_bytes());
    }
}

pub fn s8(v: i64) -> i64 {
    v as u8 as i8 as i64
}
pub fn s16(v: i64) -> i64 {
    v as u16 as i16 as i64
}
