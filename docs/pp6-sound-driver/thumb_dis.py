# Thumb disassembler with literal-pool annotation. usage: dis.py start end
import sys, struct
from capstone import *
rom=open('/Users/devonly/Developer/biccame-life/assets/roms/pp6_v0_clean.gba','rb').read()
md=Cs(CS_ARCH_ARM, CS_MODE_THUMB); md.skipdata=True; md.detail=False
a=int(sys.argv[1],16)&0xffffff; b=int(sys.argv[2],16)&0xffffff
for i in md.disasm(rom[a:b],0x8000000+a):
    s=f'{i.address:08x} {i.mnemonic} {i.op_str}'
    if i.mnemonic=='ldr' and 'pc' in i.op_str:
        off=int(i.op_str.split('#')[1].rstrip(']'),16)
        la=((i.address+4)&~3)+off
        s+=f'   ; ={struct.unpack_from("<I",rom,la-0x8000000)[0]:#x}'
    print(s)
