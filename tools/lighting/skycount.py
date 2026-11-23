#!/usr/bin/env python3
# skycount.py PORT [x y z yaw pitch] -> share of level-0 voxels in view whose colour is the sky's (blue over red above 0.6 of an indoor surface)
import json,subprocess,sys,math
port=sys.argv[1]
def c(name,arg=None):
    return json.loads(subprocess.run(['curl','-s','-m','120','-X','POST',f'localhost:{port}/cmd/{name}','-d',json.dumps(arg or {})],capture_output=True,text=True).stdout)
if len(sys.argv)>6:
    x,y,z,yaw,pitch=map(float,sys.argv[2:7]); c('scene.camera',{"position":[x,y,z],"yaw":yaw,"pitch":pitch}); c('scene.wait_settled',{"timeout_frames":900})
cam=c('scene.status')['camera']; p,yaw,pitch=cam['position'],cam['yaw'],cam['pitch']
f=[math.sin(yaw)*math.cos(pitch), math.sin(pitch), -math.cos(yaw)*math.cos(pitch)]
seen={}
for d in (2,3,4,5,6):
    for dy in (-1.5,-0.5,0.5):
        for sx in (-1.5,0,1.5):
            q=[p[0]+f[0]*d+sx*math.cos(yaw), p[1]+f[1]*d+dy, p[2]+f[2]*d+sx*math.sin(yaw)]
            for v in c('lighting.level0_voxels',{"around":q,"radius":1.3})['voxels']:
                seen[(tuple(v['cell']),v['level'],v['axis'])]=v
br=[v['radiance'][2]/max(v['radiance'][0],1e-6) for v in seen.values()]
print('pose',[round(v,2) for v in p],round(yaw,2),'voxels',len(br),'sky-coloured %d (%.3f)'%(sum(1 for b in br if b>0.6),sum(1 for b in br if b>0.6)/max(len(br),1)))
