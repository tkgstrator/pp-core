# Reference model of PP6's Konami sound sequencer (BGM path), byte-exact on
# the driver's own IWRAM structs. Used to verify docs/pp6-sound-driver.md.
import struct

ROM = open('/Users/devonly/Developer/biccame-life/assets/roms/pp6_v0_clean.gba', 'rb').read()
def r8(a): return ROM[a & 0xffffff]
def r16(a): return struct.unpack_from('<H', ROM, a & 0xffffff)[0]
def r32(a): return struct.unpack_from('<I', ROM, a & 0xffffff)[0]
def s8(v): v &= 0xff; return v - 256 if v & 0x80 else v
def s16(v): v &= 0xffff; return v - 65536 if v & 0x8000 else v

BGM_TABLE = 0x08135c0c          # 12-byte descriptors, indexed by id
SE_TABLE = 0x0813a568
PERIODS = 0x080fbab4            # u16[128] GB period per semitone
PERIOD_FRAC = 0x080fbbb4        # u8[128]  period delta per semitone (x frac/256)
NOISE = 0x080fbc34              # u8[128]  NR43 per semitone
LFO2 = 0x080fbcb4               # u8[129]  random-ish bytes (u16 LE window)
WAVES = 0x080fbd34              # 16 bytes per wave
WAVE_VOL = 0x0813d36c           # u16[5] NR32 values
VIB = 0x0813d376                # u8[16]
PAN = 0x0813d386                # u8[21]
DRUM_STREAMS = 0x0855e574       # u32[] percussion subtrack streams (pitch-0x50)
PCM_DRUMS = 0x0855e318          # (u16 sample, u16 pitch) per pitch-0x50
SAMPLE_BANKS = 0x0855e2a0       # u32 -> u32[] offsets from 0x08722da8
TRANSPOSE = 0x0855e2a8          # s16 per bank-0 sample
SCALE = 0x08554408; FREQS = 0x085544bc
MIXER_MASTER = 14

# mGBA read masks for the GB sound registers the driver reads back
READ_MASK = {0x62: 0xffc0, 0x68: 0xffc0, 0x72: 0xe000, 0x78: 0xff00,
             0x64: 0x4000, 0x6c: 0x4000, 0x74: 0x4000, 0x7c: 0x4000, 0x70: 0x00e0}

class Struct:
    def __init__(self, data): self.b = bytearray(data)
    def u8(self, o): return self.b[o]
    def u16(self, o): return self.b[o] | self.b[o+1] << 8
    def u32(self, o): return struct.unpack_from('<I', self.b, o)[0]
    def w8(self, o, v): self.b[o] = v & 0xff
    def w16(self, o, v): struct.pack_into('<H', self.b, o, v & 0xffff)
    def w32(self, o, v): struct.pack_into('<I', self.b, o, v & 0xffffffff)

def midi_key_to_freq(wav, key, fine):
    if key > 178: key, fine = 178, 255
    def val(k):
        x = r8(SCALE + k); return r32(FREQS + (x & 15) * 4) >> (x >> 4)
    v1, v2 = val(key), val(key + 1)
    return (r32(wav + 4) * ((v1 + (((v2 - v1) & 0xffffffff) * (fine << 24) >> 32)) & 0xffffffff)) >> 32

class Driver:
    def __init__(self, iwram, soundinfo, io):
        self.g = Struct(iwram[0:0x58])
        self.psg = [Struct(iwram[0x58 + i*0x84:0x58 + (i+1)*0x84]) for i in range(6)]
        self.pcm = [Struct(iwram[0x370 + i*0x80:0x370 + (i+1)*0x80]) for i in range(7)]
        self.ch = [Struct(soundinfo[0x50 + i*0x40:0x90 + i*0x40]) for i in range(7)]
        self.io = Struct(bytes(0x60) + bytes(io))   # index by IO offset
        self.cur = None; self.cursor = 0; self.chan = None
        self.writes = []
    # ---- IO helpers (address -> offset) ----
    def ioread(self, a):
        o = a - 0x4000000
        if not (0 <= o < 0xa0): return 0            # dummy (control track) regs live in IWRAM
        return self.io.u16(o) & READ_MASK.get(o, 0xffff)
    def iowrite(self, a, v):
        o = a - 0x4000000
        if not (0 <= o < 0xa0): return
        self.io.w16(o, v); self.writes.append((o, v & 0xffff))
    def byte(self):
        v = r8(self.cursor); self.cursor += 1; return v

    # ================= request / start =================
    def start_bgm(self, sid):
        e = BGM_TABLE + sid * 12
        flags = r32(e); prio = r16(e + 4) & 0xff
        lo, hi = flags & 0xff, flags & 0x8000ff00
        n = bin(lo & 0x3f).count('1') + bin((hi >> 8) & 0x7f).count('1')
        if n > 1:
            ptrs = []
            for bit in [0, 1, 2, 3, 4, 8, 9, 10, 11, 12, 13, 14]:
                if flags & (1 << bit): ptrs.append(r32(r32(e + 8) + 4 * len(ptrs)))
        else:
            ptrs = [r32(e + 8)]
        # 0x800fdcc(0): reset PSG master, stop all BGM tracks
        self.iowrite(0x4000080, 0); self.g.w16(0x3e, 0x77); self.iowrite(0x4000080, 0x77)
        self.g.w8(0x2d, 0)
        for s in range(6):
            if self.psg[s].u8(0x10) == 1: self.psg_kill(s)
        for s in range(7):
            if self.pcm[s].u8(0x13) == 1: self.pcm_kill(s)
        req = dict(id=sid, prio=prio, cat=1, delay=0, param=0)
        k = 0
        mask = 0
        for bit, slot, en in [(1, 0, 0x1100), (2, 1, 0x2200), (4, 2, 0x4400), (0x10, 3, 0x8800), (8, 5, 0x8800)]:
            if lo & bit:
                if bit == 1: self.iowrite(0x4000060, 8)
                self.psg_init(slot, req, ptrs[k]); k += 1; mask |= en
        if mask:
            self.g.w16(0x3e, self.g.u16(0x3e) | mask); self.iowrite(0x4000080, self.g.u16(0x3e))
        for i in range(7):
            if hi & (0x100 << i): self.pcm_init(i, req, ptrs[k]); k += 1

    def psg_kill(self, s):  # 0x800fe84
        t = self.psg[s]; t.w16(0x12, 0); t.w8(0x14, 0); t.w16(0x76, 0); t.w16(0x74, 0); t.w8(0x10, 0)
        self.silence(s)
    def silence(self, s):  # 0x800feb8
        t = self.psg[s]
        if t.u16(0) == 0: t = self.psg[5]
        self.iowrite(t.u32(4), 0)
        if t.u16(0) & 0xb00: self.iowrite(t.u32(8), 0x8000)
    def pcm_kill(self, s):  # 0x8010958
        t = self.pcm[s]; t.w16(0, 0); t.w8(2, 0); t.w16(0x68, 0); t.w16(0x6a, 0); t.w16(0x6c, 0); t.w8(0x13, 0)
        self.ch[s].w8(7, 0); self.ch[s].w8(0, self.ch[s].u8(0) | 0x40)

    def psg_init(self, s, q, ptr):  # 0x800fc50
        t = self.psg[s]
        t.w16(0x12, q['id']); t.w8(0x14, q['prio']); t.w8(0x16, q['delay']); t.w32(0x18, ptr); t.w8(0x10, q['cat'])
        t.w16(0x1c, 0x80); t.w8(0x1e, 0x80); t.w8(0x1f, 1); t.w32(0x78, 0); t.w16(0x20, t.u16(0)); t.w8(0x22, q['param'])
        t.w16(0x26, 0); t.w16(0x28, 0)
        for o in range(0x2a, 0x30): t.w8(o, 0)
        t.w8(0x23, 0); t.w8(0x5b, 0); t.w8(0x5a, 0); t.w16(0x5e, 0); t.w16(0x5c, 0)
    def pcm_init(self, s, q, ptr):  # 0x800fcd0
        t = self.pcm[s]
        t.w16(0, q['id']); t.w8(2, q['prio']); t.w8(4, q['delay']); t.w32(8, ptr); t.w8(0x13, q['cat'])
        t.w16(0xc, 0x80); t.w8(0xe, 0x80); t.w8(0xf, 1); t.w16(0x10, 10); t.w8(0x12, 1); t.w16(0x16, 0); t.w16(0x18, 0)
        t.w8(0x1a, 0xff); t.w8(0x1b, 0); t.w8(0x1c, 0); t.w8(0x1d, 0); t.w8(0x1e, 0xff); t.w8(0x1f, 0); t.w8(0x14, 0); t.w8(0x15, 0)
        for o in (0x68, 0x6a, 0x72, 0x74, 0x6c, 0x76, 0x78, 0x7a, 0x4c, 0x4a, 0x50, 0x4e): t.w16(o, 0)

    # ================= main timer tick (0x80103f4) =================
    def tick(self):
        g = self.g; phase = g.u32(4) & 3
        if phase == 0:
            if getattr(self, 'pending', None) is not None: self.start_bgm(self.pending); self.pending = None
            for s in range(3): self.pcm_slot(s)
        elif phase == 1:
            for s in range(3, 7): self.pcm_slot(s)
        elif phase == 2:
            for s in range(3): self.psg_slot(s)
        else:
            for s in range(3, 6): self.psg_slot(s)
        g.w32(4, g.u32(4) + 1)
    def psg_slot(self, s):
        t = self.psg[s]
        if t.u16(0x12) == 0: return
        if t.u8(0x16): t.w8(0x16, t.u8(0x16) - 1); return
        self.cur = t; self.psg_tick()
    def pcm_slot(self, s):
        t = self.pcm[s]
        if t.u16(0) == 0: return
        if t.u8(4): t.w8(4, t.u8(4) - 1); return
        self.cur = t; self.chan = self.ch[s]; self.pcm_tick()

    # ================= PSG =================
    def psg_tick(self):  # 0x8010f18
        g, t = self.g, self.cur
        g.w8(0x3b, 0); g.w16(0x3c, 0)
        g.w8(0x3a, 1 if t.u8(0x15) == 0 and self.psg[4].u16(0x12) else 0)
        add = t.u8(0x1e) + (g.u8(0x32) if t.u8(0x10) == 1 else 0)
        ph = (t.u16(0x1c) + add) & 0xffff; t.w16(0x1c, ph)
        if ph <= 0xff:
            self.wave_release(); self.psg_freq_update(); return
        t.w16(0x1c, ph - 0x100); t.w8(0x1f, t.u8(0x1f) - 1)
        if t.u8(0x1f): self.psg_step(); return
        self.cursor = t.u32(0x18); self.psg_interp(); t.w32(0x18, self.cursor)

    def psg_step(self):  # 0x801131c
        g, t = self.g, self.cur
        if t.u8(0x70): return
        if t.u8(0x2b):
            t.w8(0x2b, t.u8(0x2b) - 1)
            if t.u8(0x2b) == 0:
                t.w16(0x38, t.u16(0x38) + (s8(t.u8(0x36)) << 8)); g.w8(0x3b, 1)
        if t.u8(0x3c):
            t.w8(0x3c, t.u8(0x3c) - 1)
            if t.u8(0x3c) == 0:
                if t.u8(0x15) == 2:
                    if t.u8(0x2f) == 0 and t.u8(0x31) == 0:
                        t.w8(0x70, 1); self.soundcnt(0); return
                    t.w8(0x31, t.u8(0x2f)); self.wave_release()
                else:
                    if g.u8(0x3a): return
                    a = t.u32(4)
                    if t.u8(0x2f): v = (t.u8(0x30) << 12) | (t.u8(0x2f) << 8) | self.ioread(a)
                    else: v = self.ioread(a) & 0xc0
                    self.iowrite(a, v); g.w16(0x3c, 0x8000)
        self.psg_freq_update()

    def wave_release(self):  # 0x8011410
        t = self.cur
        if t.u8(0x15) != 2 or t.u8(0x70) or not t.u8(0x31): return
        t.w8(0x31, t.u8(0x31) - 1)
        if t.u8(0x31) or not t.u8(0x30): return
        t.w8(0x30, t.u8(0x30) - 1); self.set_vol(t.u8(0x30))
        if t.u8(0x30): t.w8(0x31, t.u8(0x2f))
        else: t.w8(0x70, 1)

    def set_vol(self, v):  # 0x80111e4
        g, t = self.g, self.cur
        if t.u8(0x15) == 2:
            v = s8(v)
            if v > 4 or v < 0: v = 4
            self.iowrite(t.u32(4), r16(WAVE_VOL + 2 * v)); self.soundcnt(t.u16(0x20)); return
        a = t.u32(4)
        x = self.ioread(a) & 0xc0
        if t.u8(0x2e) & 0x40: x |= t.u8(0x2e) << 8
        else: x |= (v & 0xff) << 12
        self.iowrite(a, x); g.w16(0x3c, 0x8000)

    def soundcnt(self, x):  # 0x80112e8
        g, t = self.g, self.cur
        if g.u8(0x3a): return
        v = (g.u16(0x3e) & ~t.u16(0)) | x; g.w16(0x3e, v); self.iowrite(0x4000080, v)

    def freq_write(self, f):  # 0x801125c
        g, t = self.g, self.cur
        frac, semi = f & 0xff, (f >> 8) & 0x7f
        if t.u8(0x15) == 5:
            self.iowrite(t.u32(8), r8(NOISE + semi) | t.u8(0x23) | g.u16(0x3c)); return
        if frac: frac = (r8(PERIOD_FRAC + semi) * frac) >> 8
        self.iowrite(t.u32(8), ((r16(PERIODS + 2 * semi) + frac) & 0x7ff) | g.u16(0x3c))
        a = t.u32(4); self.iowrite(a, self.ioread(a) | t.u8(0x23))

    def psg_freq_update(self):  # 0x8011480
        g, t = self.g, self.cur
        f = t.u16(0x38)
        if t.u8(0x2b):
            t.w16(0x34, t.u16(0x34) + t.u16(0x32)); f = (f + t.u16(0x34)) & 0xffff; g.w8(0x3b, 1)
        if t.u8(0x2c):
            self.vibrato(t, 0x3d, 0x3e, 0x42, 0x44, 0x45, 0x46, 0x2c, 0x40); f = (f + t.u16(0x40)) & 0xffff
        if t.u8(0x2d):
            self.lfo2(t, 0x4c, 0x2d, 0x47, 0x4a, 0x48); f = (f + t.u16(0x48)) & 0xffff
        if not g.u8(0x3a) and (g.u8(0x3b) or g.u16(0x3c)): self.freq_write(f)

    def vibrato(self, t, ramp, scale, acc, speed, pos, step, depth, out):  # 0x80116bc / 0x80125f8
        g = self.g
        if (g.u32(4) & 0xc) == 0 and t.u8(ramp) and t.u16(scale):
            t.w16(scale, t.u16(scale) + t.u8(ramp))
            if t.u16(scale) > 0xff: t.w16(scale, 0)
        t.w16(acc, t.u16(acc) + t.u8(speed))
        if t.u16(acc) <= 0xff: return
        t.w16(acc, t.u16(acc) & 0xff)
        d = (t.u8(depth) * t.u16(scale)) >> 8 if t.u16(scale) else t.u8(depth)
        t.w8(pos, t.u8(pos) + t.u8(step))
        x = r8(VIB + (t.u8(pos) & 15)) * d
        t.w16(out, -x if t.u8(pos) & 0x10 else x); g.w8(0x3b, 1)

    def lfo2(self, t, acc, speed, idx, mask, out):  # 0x8011640 / 0x80126ac
        t.w16(acc, t.u16(acc) + t.u8(speed))
        if t.u16(acc) <= 0xff: return
        t.w16(acc, t.u16(acc) & 0xff); t.w8(idx, (t.u8(idx) + 1) & 0x7f)
        i = t.u8(idx); t.w16(out, (r8(LFO2 + i) | r8(LFO2 + i + 1) << 8) & t.u16(mask)); self.g.w8(0x3b, 1)

    def d1(self, b):  # 0x8011518
        g, t = self.g, self.cur
        t.w8(0x24, b)
        if g.u8(0x3a): t.w8(0x23, b); return
        k = t.u8(0x15)
        if k in (0, 1, 4):
            t.w8(0x23, b); a = t.u32(4)
            self.iowrite(a, self.ioread(a) & 0xff3f); self.iowrite(a, self.ioread(a) | b)
        elif k == 2:
            self.iowrite(0x4000070, 0x40)
            for i in range(8): self.iowrite(0x4000090 + 2 * i, r16(WAVES + b * 16 + 2 * i))
            g.w16(0x3e, g.u16(0x3e) & 0xbbff); self.iowrite(0x4000080, g.u16(0x3e))
            self.iowrite(0x4000070, 0x80)
            a = t.u32(8); self.iowrite(a, self.ioread(a) | 0x8000)
        elif k == 5:
            t.w8(0x23, 8 if b else 0)

    def pan(self, b):  # 0x8011b74 body
        t = self.cur
        if b == 1: t.w16(0x20, t.u16(0) & 0x0f00)
        elif b == 2: t.w16(0x20, t.u16(0) & 0xf000)
        elif b == 3: t.w16(0x20, t.u16(0))
        self.soundcnt(t.u16(0x20))

    def psg_stop(self):  # 0x8011780 (BGM-relevant part)
        t = self.cur; t.w16(0x12, 0); t.w8(0x14, 0); t.w8(0x30, 0)
        k = t.u8(0x15)
        if k == 4: return  # SE sq1 restoring BGM sq1: not modelled
        if self.g.u8(0x3a): return
        if k == 3 and self.psg[5].u16(0x12): return
        self.silence(k)

    def loop_mark(self, t, s, cur_o, cnt_o, vol_o, pit_o, pw):
        t.w32(cur_o + 4 * s, self.cursor); t.w8(cnt_o + s, 0)
        if pw == 1: t.w8(vol_o + s, 0)
        else: t.w16(vol_o + 2 * s, 0)
        t.w16(pit_o + 2 * s, 0)

    def psg_interp(self):  # 0x8010fc0
        g, t = self.g, self.cur
        t.w8(0x2b, 0)
        while True:
            op = self.byte()
            if op < 0x80: break
            self.psg_op(op)
            if op in (0xd9, 0xc0, 0xe3): return
        t.w8(0x31, 0); t.w8(0x70, 0)
        if not op & 0x10: t.w8(0x3b, self.byte())
        t.w8(0x3c, t.u8(0x3b))
        if not op & 0x20: t.w8(0x3a, self.byte())
        t.w8(0x1f, t.u8(0x3a))
        if not op & 0x40: t.w8(0x37, self.byte())
        nib = op & 15
        if t.u8(0x15) == 3: self.drum(nib); return
        v = s8(nib - t.u8(0x2a) - t.u8(0x22)); v = s8(v + t.u8(0x5a)); v = s8(v + t.u8(0x5b))
        if v < 0: v = 0; t.w8(0x70, 1)
        elif v > 15: v = 15
        t.w8(0x30, v)
        if not g.u8(0x3a): self.set_vol(v)
        if t.u8(0x2c):
            t.w16(0x3e, t.u8(0x3d)); t.w16(0x42, 0); t.w8(0x45, 0); t.w16(0x40, 0)
        f = (s8(t.u8(0x37)) << 8) + t.u16(0x26) + t.u16(0x28) + t.u16(0x5c) + t.u16(0x5e)
        t.w16(0x38, f)
        if not g.u8(0x3a): self.freq_write(f & 0xffff)

    def drum(self, nib):  # 0x8011164
        g, t = self.g, self.cur
        if self.psg[5].u16(0x12) > 0x3f or t.u8(0x37) < 0x50: return
        t.w8(0x70, 1)
        g.w16(0x3e, g.u16(0x3e) | 0x8800); self.iowrite(0x4000080, g.u16(0x3e))
        q = dict(id=0xf, prio=0, cat=2, delay=0, param=nib)
        self.psg_init(5, q, r32(DRUM_STREAMS + 4 * (t.u8(0x37) - 0x50)))

    def psg_op(self, op):
        g, t = self.g, self.cur
        if 0x80 <= op <= 0x83: self.d1((op - 0x80) << 6)
        elif 0x84 <= op <= 0x93: self.d1(op - 0x84)
        elif op == 0x94: t.w8(0x23, 0)
        elif op == 0x95: t.w8(0x23, 8)
        elif 0x96 <= op <= 0xa5: t.w8(0x2a, op - 0x96)
        elif 0xa6 <= op <= 0xad: t.w8(0x2f, op - 0xa6)
        elif op == 0xae: t.w16(0x20, t.u16(0) & 0xf000); self.soundcnt(t.u16(0x20))
        elif op == 0xaf: t.w16(0x20, t.u16(0) & 0x0f00); self.soundcnt(t.u16(0x20))
        elif op == 0xb0: t.w16(0x20, t.u16(0)); self.soundcnt(t.u16(0x20))
        elif op in (0xc1, 0xc2): self.loop_mark(t, op - 0xc1, 0x50, 0x58, 0x5a, 0x5c, 1)
        elif op in (0xf3, 0xf4): self.repeat(t, op - 0xf3, 0x50, 0x58, 0x5a, 0x5c, 15)
        elif op == 0xc3: t.w32(0x60, self.cursor)
        elif op == 0xc6: self.cursor = t.u32(0x60)
        elif op == 0xc4: t.w8(0x64, 0); t.w32(0x68, self.cursor)
        elif op == 0xc5: self.volta(t, 0x64, 0x68, 0x6c)
        elif op == 0xe4: pass
        elif op == 0xd0: t.w8(0x1e, self.byte())
        elif op == 0xd1: self.d1(self.byte())
        elif op == 0xdc: self.byte()
        elif op == 0xd2: t.w8(0x2a, self.byte())
        elif op == 0xd3: self.pan(self.byte())
        elif op == 0xd4: t.w16(0x26, self.byte() << 8)
        elif op == 0xd5: t.w16(0x28, s8(self.byte()) << 2)
        elif op == 0xd6: t.w8(0x2f, self.byte())
        elif op == 0xda: t.w8(0x2e, self.byte())
        elif op == 0xe0: t.w8(0x2a, self.byte()); self.d1(self.byte())
        elif op == 0xf0: t.w8(0x2a, self.byte()); self.d1(self.byte()); self.pan(self.byte())
        elif op == 0xdb:
            b = self.byte()
            if t.u16(0) & 0x1100:
                if t.u8(0x15): self.iowrite(0x4000060, b)
                else:
                    g.w16(0x38, b)
                    if not g.u8(0x3a): self.iowrite(0x4000060, b)
        elif op == 0xe1: t.w8(0x2b, self.byte()); t.w8(0x36, self.byte())
        elif op == 0xe2: t.w16(0x32, self.byte() | self.byte() << 8); t.w16(0x34, 0)
        elif op == 0xf1: self.vib_set(t, 0x3d, 0x2c, 0x44, 0x46)
        elif op == 0xd8: t.w8(0x3d, self.byte())
        elif op == 0xf2:
            t.w8(0x2d, self.byte()); hi = self.byte(); t.w16(0x4a, hi << 8 | self.byte()); t.w16(0x4c, 0); t.w8(0x47, 0)
        elif op in (0xf5, 0xf6): self.cursor += 3
        elif op == 0xe3:
            b = self.byte(); t.w8(0x3a, b); t.w8(0x1f, b); b = self.byte(); t.w8(0x3b, b); t.w8(0x3c, b)
        elif op == 0xd9:
            t.w8(0x70, 1); b = self.byte(); t.w8(0x3a, b); t.w8(0x1f, b)
            if not g.u8(0x3a): self.silence(t.u8(0x15))
        else: self.psg_stop()

    def vib_set(self, t, ramp, depth, speed, step):
        t.w8(ramp, self.byte()); b = self.byte(); t.w8(depth, self.byte())
        if not t.u8(depth): return
        if b == 0xff: t.w8(speed, b); t.w8(step, 8)
        elif b & 0x80: t.w8(speed, b); t.w8(step, 4)
        elif b > 0x3f: t.w8(speed, b << 1); t.w8(step, 2)
        else: t.w8(speed, b << 2); t.w8(step, 1)

    def repeat(self, t, s, cur_o, cnt_o, vol_o, pit_o, bound):
        n = self.byte()
        if n:
            t.w8(cnt_o + s, t.u8(cnt_o + s) + 1)
            if t.u8(cnt_o + s) == n:
                self.cursor += 2
                if bound == 15: t.w8(vol_o + s, 0)
                else: t.w16(vol_o + 2 * s, 0)
                t.w16(pit_o + 2 * s, 0); return
        dv = s8(self.byte())
        if dv:
            if bound == 15:
                v = s8(t.u8(vol_o + s) + dv); t.w8(vol_o + s, max(-15, min(15, v)))
            else:
                v = s16(t.u16(vol_o + 2 * s) + dv); t.w16(vol_o + 2 * s, max(-255, min(255, v)))
        dp = s8(self.byte())
        if dp: t.w16(pit_o + 2 * s, t.u16(pit_o + 2 * s) + (dp << 3))
        self.cursor = t.u32(cur_o + 4 * s)

    def volta(self, t, st, a, ret):
        s = t.u8(st)
        if s == 0: t.w8(st, 1)
        elif s == 1: t.w8(st, 2); t.w32(ret, self.cursor); self.cursor = t.u32(a)
        elif s == 2: t.w8(st, 1); self.cursor = t.u32(ret)

    # ================= PCM =================
    def pcm_tick(self):  # 0x8011f9c
        g, t = self.g, self.cur
        g.w8(0x3b, 0); g.w16(0x3c, 0)
        self.pcm_fades()
        add = t.u8(0xe) + (g.u8(0x32) if t.u8(0x13) == 1 else 0)
        ph = (t.u16(0xc) + add) & 0xffff; t.w16(0xc, ph)
        if ph <= 0xff: self.pcm_freq_update(); return
        t.w16(0xc, ph - 0x100); t.w8(0xf, t.u8(0xf) - 1)
        if t.u8(0xf): self.pcm_step()
        else:
            self.cursor = t.u32(8); self.pcm_interp(); t.w32(8, self.cursor)
        self.pcm_volume(t.u8(0x25))

    def pcm_fades(self):  # 0x8012038..0x80120fc (inactive in normal play)
        t = self.cur
        if t.u16(0x68):
            t.w16(0x68, t.u16(0x68) - 1)
            if t.u16(0x68) == 0:
                t.w16(0x68, t.u16(0x66))
                if t.u16(0x6a): t.w16(0x6a, t.u16(0x6a) - 1)
        if t.u16(0x70):
            t.w16(0x70, t.u16(0x70) - 1)
            if t.u16(0x70) == 0:
                t.w16(0x70, t.u16(0x6e))
                if t.u16(0x72): t.w16(0x72, t.u16(0x72) - 1); t.w16(0x74, t.u16(0x74) + 1)
                else: t.w16(0x70, 0)
        if t.u16(0x78):
            t.w16(0x78, t.u16(0x78) - 1)
            if t.u16(0x78) == 0:
                t.w16(0x78, t.u16(0x76)); lim = 10 if t.u16(0) == 0x6a else 6
                if t.u16(0x7a) >= lim: t.w16(0x7a, t.u16(0x7a) - 1)
                else: t.w16(0x78, 0)
        if t.u8(0x7d):
            t.w8(0x7d, t.u8(0x7d) - 1)
            if t.u8(0x7d) == 0:
                t.w8(0x7d, t.u8(0x7c))
                if t.u8(0x7e): t.w8(0x7e, t.u8(0x7e) - 1); t.w8(0x7f, t.u8(0x7f) + 1)
                else: t.w8(0x7d, 0)

    def pcm_step(self):  # 0x80124a0
        g, t, c = self.g, self.cur, self.chan
        if t.u8(0x64): return
        if t.u8(0x1b):
            t.w8(0x1b, t.u8(0x1b) - 1)
            if t.u8(0x1b) == 0: t.w16(0x28, t.u16(0x28) + (s8(t.u8(0x24)) << 8)); g.w8(0x3b, 1)
        if t.u8(0x2d):
            t.w8(0x2d, t.u8(0x2d) - 1)
            if t.u8(0x2d) == 0: c.w8(7, t.u8(0x1f)); c.w8(0, c.u8(0) | 0x40)
        self.pcm_freq_update()

    def pcm_freq_update(self):  # 0x8012520
        g, t = self.g, self.cur
        f = t.u16(0x28)
        if t.u8(0x1b):
            t.w16(0x22, t.u16(0x22) + t.u16(0x20)); f = (f + t.u16(0x22)) & 0xffff; g.w8(0x3b, 1)
        if t.u8(0x1c):
            self.vibrato(t, 0x2e, 0x30, 0x34, 0x36, 0x37, 0x38, 0x1c, 0x32); f = (f + t.u16(0x32)) & 0xffff
        if t.u8(0x1d):
            self.lfo2(t, 0x3e, 0x1d, 0x39, 0x3c, 0x3a); f = (f + t.u16(0x3a)) & 0xffff
        if g.u8(0x3b): self.set_freq(f)

    def set_freq(self, f):  # 0x8012470
        x = (f + 0x1800) & 0xffff
        self.chan.w32(0x20, midi_key_to_freq(self.chan.u32(0x24), (x >> 8) & 0x7f, x & 0xff))

    def pcm_volume(self, v):  # 0x8012370
        t, c = self.cur, self.chan
        p = t.u16(0x10)
        out = []
        for idx in (p, 20 - p):
            x = (v * r8(PAN + idx)) >> 8
            if t.u16(0x68):
                x = (x * t.u16(0x6a)) >> 4
                if t.u16(0x6a) == 0: self.pcm_kill(t.u8(3)); return
            if t.u16(0x70): x = (x * t.u16(0x74)) >> 4
            if t.u16(0x7a): x = (x * t.u16(0x7a)) >> 4
            if t.u8(0x7e): x = (x * t.u8(0x7f)) >> 4
            out.append(x & 0xff)
        c.w8(3, out[0]); c.w8(2, out[1])

    def sample(self, local):  # 0x801259c
        t, c = self.cur, self.chan
        t.w8(0x14, local)
        c.w32(0x24, r32(r32(SAMPLE_BANKS + 4 * t.u8(0x15)) + 4 * local) + 0x08722da8)
        t.w16(0x2a, r16(TRANSPOSE + 2 * t.u8(0x14)) if t.u8(0x15) == 0 else 0)

    def pcm_interp(self):  # 0x8012150
        g, t, c = self.g, self.cur, self.chan
        t.w8(0x1b, 0)
        while True:
            op = self.byte()
            if op < 0x80: break
            self.pcm_op(op)
            if op in (0xd9, 0xc0, 0xe3): return
        if t.u8(0x64): retrig = 1; t.w8(0x64, 0)
        else: retrig = t.u8(0x12)
        if not op & 0x10: t.w8(0x12, self.byte())
        t.w8(0x2d, t.u8(0x12))
        if not op & 0x20: t.w8(0x2c, self.byte())
        t.w8(0xf, t.u8(0x2c))
        if not op & 0x40: t.w8(0x26, self.byte())
        v = ((op & 15) * 8 + s16(t.u16(0x4a)) + s16(t.u16(0x4c))) * t.u8(0x1a)
        if v < 0: v = 0; t.w8(0x64, 1)
        elif v > 0x7fff: v = 0x7fff
        t.w8(0x25, v >> 7)
        if t.u8(0x26) > 0x4f:
            i = t.u8(0x26) - 0x50
            t.w8(0x14, r16(PCM_DRUMS + 4 * i)); t.w16(0x2a, r16(PCM_DRUMS + 4 * i + 2))
            c.w32(0x24, r32(r32(SAMPLE_BANKS) + 4 * t.u8(0x14)) + 0x08722da8); t.w8(0x26, 0x18)
        elif t.u8(0x1c):
            t.w16(0x30, t.u8(0x2e)); t.w16(0x34, 0); t.w8(0x37, 0); t.w16(0x32, 0)
        f = (s8(t.u8(0x26)) << 8) + t.u16(0x2a) + t.u16(0x16) + t.u16(0x18) + t.u16(0x4e) + t.u16(0x50)
        t.w16(0x28, f); self.set_freq(f & 0xffff)
        if retrig:
            c.w8(4, t.u8(0x1e)); c.w8(5, 0xff); c.w8(6, 0xff); c.w8(7, 0xff); c.w8(0, 0x80)

    def pcm_op(self, op):
        g, t, c = self.g, self.cur, self.chan
        if op in (0xc1, 0xc2): self.loop_mark(t, op - 0xc1, 0x40, 0x48, 0x4a, 0x4e, 2)
        elif op in (0xf3, 0xf4): self.repeat(t, op - 0xf3, 0x40, 0x48, 0x4a, 0x4e, 255)
        elif op == 0xc3: t.w32(0x54, self.cursor)
        elif op == 0xc6: self.cursor = t.u32(0x54)
        elif op == 0xc4: t.w8(0x58, 0); t.w32(0x5c, self.cursor)
        elif op == 0xc5: self.volta(t, 0x58, 0x5c, 0x60)
        elif op in (0xe4, 0xf5, 0xf6): pass
        elif op == 0xd0: t.w8(0xe, self.byte())
        elif op == 0xdc: t.w8(0x15, self.byte())
        elif op == 0xd1: self.sample(self.byte())
        elif 0x80 <= op <= 0x83: self.sample(0)
        elif 0x84 <= op <= 0x93: self.sample(5)
        elif op == 0xd2: t.w8(0x1a, self.byte())
        elif 0x96 <= op <= 0xa5: t.w8(0x1a, (15 - (op - 0x96)) << 4)
        elif op == 0xd3: t.w16(0x10, self.byte())
        elif op == 0xae: t.w16(0x10, 0)
        elif op == 0xaf: t.w16(0x10, 20)
        elif op == 0xb0: t.w16(0x10, 10)
        elif op == 0xd4: t.w16(0x16, self.byte() << 8)
        elif op == 0xd5: t.w16(0x18, s8(self.byte()) << 2)
        elif op == 0xd6: b = self.byte(); t.w8(0x1f, b + 0xdf if b else 0)
        elif 0xa6 <= op <= 0xad: t.w8(0x1f, op + 0x5a)
        elif op == 0xda:
            b = self.byte(); t.w8(0x1e, ((b << 1) | 1) & 0xff if b > 0x77 else b)
        elif op == 0xe0: t.w8(0x1a, self.byte()); self.sample(self.byte())
        elif op == 0xf0: t.w8(0x1a, self.byte()); self.sample(self.byte()); t.w16(0x10, self.byte())
        elif op == 0xe1: t.w8(0x1b, self.byte()); t.w8(0x24, self.byte())
        elif op == 0xe2: t.w16(0x20, self.byte() | self.byte() << 8); t.w16(0x22, 0)
        elif op == 0xf1: self.vib_set(t, 0x2e, 0x1c, 0x36, 0x38)
        elif op == 0xd8: t.w8(0x2e, self.byte())
        elif op == 0xf2:
            t.w8(0x1d, self.byte()); hi = self.byte(); t.w16(0x3c, hi << 8 | self.byte()); t.w16(0x3e, 0); t.w8(0x39, 0)
        elif op == 0xe3:
            b = self.byte(); t.w8(0x2c, b); t.w8(0xf, b); b = self.byte(); t.w8(0x12, b); t.w8(0x2d, b)
        elif op == 0xd9:
            t.w8(0x64, 1); b = self.byte(); t.w8(0x2c, b); t.w8(0xf, b); c.w8(7, 0); c.w8(0, c.u8(0) | 0x40)
        else: self.pcm_kill(t.u8(3))
