# Needs the mGBA libretro harness (HARNESS env, default /tmp/bic-transition/harness.py).
# Per video frame trace of driver state for one BGM: python trace.py ID frames out.npz
exec(open(__import__('os').environ.get('HARNESS','/tmp/bic-transition/harness.py')).read())
import sys, numpy as np
def run(n,key=-1):
    state['key']=key
    for _ in range(n): lib.retro_run()
run(650)
for key in [3,8,8]: run(3,key); run(90)
run(60)
iw=maps[0x3000000]; mem=(c.c_uint8*iw[1]).from_address(iw[0])
io=maps[0x4000000]; reg=(c.c_uint8*io[1]).from_address(io[0])
bid=int(sys.argv[1],16); N=int(sys.argv[2])
# stop whatever plays: let menu BGM keep going; just request
cnt=lambda: int.from_bytes(bytes(mem[0x164:0x168]),'little')
c0=cnt()
pre=np.frombuffer(bytes(mem[0x160:0x160+0x7f0])+bytes(mem[0x5700:0x5700+0x350])+bytes(reg[0x60:0xa0])+cnt().to_bytes(4,'little'),dtype=np.uint8)
n=mem[0x169]; mem[0x16a+2*n]=bid&0xff; mem[0x16b+2*n]=bid>>8; mem[0x169]=n+1
frames=[]
for f in range(N):
    lib.retro_run()
    frames.append(np.frombuffer(bytes(mem[0x160:0x160+0x7f0])+bytes(mem[0x5700:0x5700+0x350])+bytes(reg[0x60:0xa0])+cnt().to_bytes(4,'little'),dtype=np.uint8))
np.savez(sys.argv[3], c0=c0, pre=pre, frames=np.array(frames))
print('c0',c0, 'audio', sum(len(a) for a in audio))
