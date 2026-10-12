import re,sys,numpy as np
# chg.py RUN_DIR... : S78S change counters (lightingchg lines), mean per frame over f300-600
N=['keptWrite','keptChanged','kD<.2%','kD<1%','kD<5%','kD>5%','kSrcTDLreuse','kSrcTDLcheck','kSrcLamps','recChanged','keptHeldTol',
   'gRun','gHeld','gDirty','gMoved','gPicks','texWrite','tD<.2%','tD<1%','tD<5%','tD>5%','kbFine','kbCoarse','kbNone','kbParent','kbNbr','kcFine','kcCoarse','kcNone','kcParent','kcNbr']
for r in sys.argv[1:]:
    L=open(f'{r}/run.log').read().splitlines()
    C=np.array([list(map(int,re.search(r'c \[(.*)\]',l).group(1).split(','))) for l in L if l.startswith('lightingchg frame')],float)
    if len(C)==0: print(r,'no lightingchg'); continue
    m=C[300:600].mean(0) if len(C)>=600 else C[len(C)//2:].mean(0)
    print(r.rstrip('/').split('/')[-1], ' '.join(f'{n} {m[i]:.0f}' for i,n in enumerate(N)))
