#!/usr/bin/env python3
# wiggle_steps.py PORT LABEL -> wiggle the yaw left and right in small steps; at EVERY step (while moving, one frame per step)
# save the bounce picture and read the level-0 voxels around a world point on the vault. Prints per step: voxel count, how many
# voxels kept their key from the step before, how many of those kept their seat, and how much the kept ones' light changed.
import json,subprocess,sys,math
port,label=sys.argv[1],sys.argv[2]
def c(name,arg=None):
    return json.loads(subprocess.run(['curl','-s','-m','120','-X','POST',f'localhost:{port}/cmd/{name}','-d',json.dumps(arg or {})],capture_output=True,text=True).stdout)
st=c('scene.status'); cam=st['camera']; p,yaw,pitch=cam['position'],cam['yaw'],cam['pitch']
probe=json.loads(sys.argv[3]) if len(sys.argv)>3 else None
if probe is None:
    # a point 3.2 m ahead and 2.4 m up: the vault at the top of the picture
    probe=[p[0]+3.2*math.sin(yaw), p[1]+2.4, p[2]-3.2*math.cos(yaw)]
c('scene.camera',{"position":p,"yaw":yaw,"pitch":pitch}); c('scene.wait_settled',{"timeout_frames":600})
c('scene.time',{"pause":True})
prev=None; rows=[]
import os
STEP=float(os.environ.get("STEP","0.006"))
steps=[STEP*k for k in list(range(0,13))+list(range(11,-13,-1))+list(range(-11,1))]
for i,dy in enumerate(steps):
    if os.environ.get("MODE")=="strafe":
        c('scene.camera',{"position":[p[0]+dy*math.cos(yaw),p[1],p[2]+dy*math.sin(yaw)],"yaw":yaw,"pitch":pitch})
    elif os.environ.get("MODE")=="walk":
        c('scene.camera',{"position":[p[0]+dy*math.sin(yaw),p[1],p[2]-dy*math.cos(yaw)],"yaw":yaw,"pitch":pitch})
    else:
        c('scene.camera',{"position":p,"yaw":yaw+dy,"pitch":pitch})
    c('scene.step',{"frames":1})
    c('capture.capture',{"path":f"{label}/step-{i:03d}.png","mode":"bounce","width":847,"height":688})
    vox={ (tuple(v['cell']),v['level'],v['axis']):(v['position'],sum(v['radiance'])/3,v['slots']) for v in c('lighting.level0_voxels',{"around":probe,"radius":1.0})['voxels']}
    if prev is not None:
        kept=[k for k in vox if k in prev]
        seat=[k for k in kept if max(abs(a-b) for a,b in zip(vox[k][0],prev[k][0]))<1e-4]
        rel=[abs(vox[k][1]-prev[k][1])/max(vox[k][1],prev[k][1],1e-6) for k in kept]
        big=sum(1 for r in rel if r>0.1)
        lv=sorted(set(k[1] for k in vox))
        rows.append((i,round(dy,3),len(vox),len(kept),len(seat),big,round(sum(rel)/max(len(rel),1),3),min(v[2] for v in vox.values()) if vox else 0,lv))
    prev=vox
c('scene.time',{"pause":False}); c('scene.camera',{"position":p,"yaw":yaw,"pitch":pitch})
print('probe',[round(x,2) for x in probe])
print('step dyaw voxels kept_key kept_seat changed>10% mean_rel_change min_slots levels')
for r in rows: print(*r)
tot=sum(r[3] for r in rows); print('SUM kept_key',tot,'kept_seat',sum(r[4] for r in rows),'changed>10%',sum(r[5] for r in rows),'new voxels',sum(r[2]-r[3] for r in rows))
