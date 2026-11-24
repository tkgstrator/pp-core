import sys, copy
sys.path.insert(0, __import__("os").path.dirname(__file__))
import numpy as np
from model import Driver
d = np.load(sys.argv[1]); pre = d['pre'].tobytes(); frames = d['frames']
IW, SI, IO = slice(0, 0x7f0), slice(0x7f0, 0x7f0 + 0x350), slice(0xb40, 0xb80)
drv = Driver(pre[IW], pre[SI], pre[IO]); drv.pending = int(sys.argv[2], 16)
cnt = lambda fr: int.from_bytes(fr[0xb80:0xb84].tobytes(), 'little')
nframes = int(sys.argv[3]) if len(sys.argv) > 3 else len(frames)
# readable bits of each IO halfword as mGBA keeps them
MASK = {0x60: 0x7f, 0x62: 0xffc0, 0x64: 0x4000, 0x68: 0xffc0, 0x6c: 0x4000, 0x70: 0xe0, 0x72: 0xe000,
        0x74: 0x4000, 0x78: 0xff00, 0x7c: 0x40ff, 0x80: 0xff77, 0x82: 0xff0f, 0x84: 0x80, 0x88: 0xffff}
CH = [2, 3, 4, 7, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27]
def diffs(drv, fr):
    out = []; iw = fr[IW]
    for s in range(6):
        a = drv.psg[s].b; r = iw[0x58 + s*0x84:0x58 + (s+1)*0x84]
        out += [(f'psg{s}', o, a[o], r[o]) for o in range(0x84) if a[o] != r[o]]
    for s in range(7):
        a = drv.pcm[s].b; r = iw[0x370 + s*0x80:0x370 + (s+1)*0x80]
        out += [(f'pcm{s}', o, a[o], r[o]) for o in range(0x80) if a[o] != r[o]]
        rc = fr[SI][0x50 + s*0x40:0x90 + s*0x40]; mc = drv.ch[s].b
        out += [(f'ch{s}', o, mc[o], rc[o]) for o in CH if mc[o] != rc[o]]
    for o, m in MASK.items():
        a = drv.io.u16(o) & m; r = (fr[IO][o-0x60] | fr[IO][o-0x5f] << 8) & m
        if a != r: out.append(('io', o, a, r))
    out += [('g', o, drv.g.b[o], iw[o]) for o in range(0x2a, 0x44) if drv.g.b[o] != iw[o]]
    return out
bad = {}; ok = 0; streak = {}; persist = {}
for i in range(nframes):
    fr = frames[i].tobytes(); C = cnt(frames[i])
    while drv.g.u32(4) < C: drv.tick()
    dd = diffs(drv, fr)
    if dd:  # snapshot may land inside the timer IRQ (track updated, counter not yet)
        nxt = copy.deepcopy(drv); nxt.tick()
        d2 = diffs(nxt, fr)
        if len(d2) < len(dd): dd = d2
    if not dd: ok += 1
    cur = set()
    for k in dd:
        bad.setdefault(k[:2], (i, k[2], k[3])); cur.add(k[:2])
        streak[k[:2]] = streak.get(k[:2], 0) + 1
        if streak[k[:2]] == 3: persist.setdefault(k[:2], (i - 2, k[2], k[3]))
    for k in list(streak):
        if k not in cur: del streak[k]
print(nframes, 'frames,', ok, 'exact; mismatching fields:', len(bad))
print('persistent (>=3 frames):', len(persist))
for k, v in sorted(persist.items(), key=lambda kv: kv[1][0])[:50]:
    print(k[0], hex(k[1]), 'frame', v[0], 'model', hex(v[1]), 'emu', hex(v[2]))
