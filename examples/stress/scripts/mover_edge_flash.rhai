// A pixel on the bottom edge of a moving box face can land a hair under the
// floor. Its level-0 cell is the layer under the floor, and the voxel used to
// be seated at that cell's centre, 12 cm under the floor: it read 0.4 against
// about 3 for its neighbours, lived one frame, and darkened the pixels over
// it. Watch the side-face voxels around the nearest moving box for 200 frames
// and require none far below the median of its frame.
time(#{ fixed_dt: 0.0166667 });
wait(500);
let cam = status().camera.position;
let best = -1;
let bd = 1.0e9;
for o in objects() {
    let p = o.position;
    let d = (p[0] - cam[0]) * (p[0] - cam[0]) + (p[2] - cam[2]) * (p[2] - cam[2]);
    if d < bd && o.size < 3.0 {
        bd = d;
        best = o.index;
    }
}
let o = objects()[best];
let c = [o.position[0], 0.5, o.position[2]];
let dark = 0;
let seen = 0;
let i = 0;
while i < 200 {
    let v = level0_voxels(#{ around: c, radius: 2.0 });
    let lums = [];
    let side = [];
    for x in v.voxels {
        if x.axis != 2 && x.axis != 3 && x.slots > 0.5 {
            lums.push(0.2126 * x.radiance[0] + 0.7152 * x.radiance[1] + 0.0722 * x.radiance[2]);
            side.push(x);
        }
    }
    if lums.len() > 4 {
        let sorted = lums;
        sorted.sort();
        let median = sorted[sorted.len() / 2];
        seen += lums.len();
        for k in 0..lums.len() {
            if lums[k] < 0.3 * median {
                dark += 1;
                log(`frame ${i}: level ${side[k].level} axis ${side[k].axis} at ${side[k].position} reads ${lums[k]}, median ${median}`);
            }
        }
    }
    i += 1;
}
log(`side-face voxels seen ${seen}, far below their frame's median ${dark}`);
check_gt(seen, 1000, "the box's side faces hold level-0 voxels");
check_lt(dark, 1, "no side-face voxel is seated under the floor");
