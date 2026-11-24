import numpy as np, glob
tot=ok=0; bad=[]
for f in sorted(glob.glob('all/*.npz'))[:60]:
    fr=np.load(f)['frames']; SI=0x7f0
    mv=int(fr[0][SI+7])
    for i in range(len(fr)-1):
        for s in range(7):
            a=fr[i][SI+0x50+s*0x40:SI+0x90+s*0x40].astype(int); b=fr[i+1][SI+0x50+s*0x40:SI+0x90+s*0x40].astype(int)
            st=a[0]
            if st==0 or b[0]&0x80 or (a[0]&0x40)!=(b[0]&0x40) and not (b[0]==0): continue
            if a[2]!=b[2] or a[3]!=b[3] or a[4:8].tolist()!=b[4:8].tolist(): continue
            env=a[9]; s2=st
            if st&0x80:
                if st&0x40: s2=0
                else:
                    s2=3|(b[0]&0x10); env=0
                    env+=a[4]
                    if env>=0xff: env=0xff; s2-=1
            elif st&0x40:
                if st&4:
                    continue
                env=env*a[7]>>8
                if env<=a[0xc]:
                    env=a[0xc]
                    if env==0: s2=0
                    else: s2|=4
            elif st&3==2:
                env=env*a[5]>>8
                if env<=a[6]:
                    env=a[6]
                    if env==0: s2=0
                    else: s2-=1
            elif st&3==3:
                env+=a[4]
                if env>=0xff: env=0xff; s2-=1
            if s2==0:
                pred=(0,None,None,None)
            else:
                v=(mv+1)*env>>4
                pred=(s2,env,a[2]*v>>8,a[3]*v>>8)
            got=(b[0],b[9],b[0xa],b[0xb]) if s2 else (b[0],None,None,None)
            tot+=1
            if pred==got: ok+=1
            elif len(bad)<8: bad.append((f,i,s,st,pred,got))
print(tot,ok); print(*bad,sep='\n')
