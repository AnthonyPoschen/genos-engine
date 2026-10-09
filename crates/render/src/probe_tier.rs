//! Persistent world probe tier: sparse bricks of probes on a fixed world lattice.
//!
//! Probes sit at fixed world positions (`spacing` apart), grouped in bricks of
//! `BRICK`^3. A brick is allocated only when one of its probes is live: outside
//! every solid and within `reach` of a surface. Empty air and solid interiors cost
//! no memory and no rays. The window is a box around the camera snapped to whole
//! bricks, so moving the camera adds and drops whole bricks at its edges and never
//! moves a probe.

/// Probes along one brick edge.
pub const BRICK: i32 = 4;
/// Field texels per probe: three ambient cubes of six RGBA32F faces, then the probe's
/// position (the last texel, xyz: a probe moved out of a solid sits off its lattice
/// point). The first cube holds all the bounced light (the picture reads it; alpha
/// counts samples), then up to two bounces, then one. Every pass feeds the first cube
/// back at the hits, so each pass adds a bounce and the light converges to unlimited
/// bounces; white paint returns 0.8 of the light, so the series converges.
pub const PROBE_TEXELS: u32 = 19;
/// Bytes kept per probe.
pub const PROBE_BYTES: usize = PROBE_TEXELS as usize * 16;
/// Probes in one brick.
pub const BRICK_PROBES: u32 = (BRICK * BRICK * BRICK) as u32;
/// Bytes per window brick in the indirection table (a brick slot or none).
pub const INDIRECTION_BYTES: usize = 4;

/// Axis-aligned box a probe can see. A floor or roof is a box with no height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl SurfaceBox {
    fn inside(&self, p: [f32; 3]) -> bool {
        (0..3).all(|i| p[i] > self.min[i] && p[i] < self.max[i])
    }

    /// Distance from `p` to the nearest point on the box surface.
    fn surface_distance(&self, p: [f32; 3]) -> f32 {
        if self.inside(p) {
            (0..3)
                .map(|i| (p[i] - self.min[i]).min(self.max[i] - p[i]))
                .fold(f32::MAX, f32::min)
        } else {
            let mut sum = 0.0;
            for i in 0..3 {
                let d = (self.min[i] - p[i]).max(p[i] - self.max[i]).max(0.0);
                sum += d * d;
            }
            sum.sqrt()
        }
    }

    fn overlaps(&self, min: [f32; 3], max: [f32; 3]) -> bool {
        (0..3).all(|i| self.min[i] <= max[i] && self.max[i] >= min[i])
    }

    /// Where the ray from `origin` along `dir` enters the box: the distance and the axis
    /// of the face it enters through. None when it misses or starts inside.
    fn entry(&self, origin: [f32; 3], dir: [f32; 3]) -> Option<(f32, usize)> {
        let (mut t0, mut t1, mut axis) = (f32::MIN, f32::MAX, 0);
        for i in 0..3 {
            if dir[i].abs() < 1.0e-9 {
                if origin[i] < self.min[i] || origin[i] > self.max[i] {
                    return None;
                }
                continue;
            }
            let (a, b) = ((self.min[i] - origin[i]) / dir[i], (self.max[i] - origin[i]) / dir[i]);
            let (near, far) = (a.min(b), a.max(b));
            if near > t0 {
                t0 = near;
                axis = i;
            }
            t1 = t1.min(far);
        }
        (t0 <= t1 && t0 > 1.0e-4).then_some((t0, axis))
    }

    /// True when the segment `a`..`b` passes through the box.
    fn crosses(&self, a: [f32; 3], b: [f32; 3]) -> bool {
        let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
        for i in 0..3 {
            let d = b[i] - a[i];
            if d.abs() < 1.0e-9 {
                if a[i] < self.min[i] || a[i] > self.max[i] {
                    return false;
                }
                continue;
            }
            let (mut near, mut far) = ((self.min[i] - a[i]) / d, (self.max[i] - a[i]) / d);
            if near > far {
                std::mem::swap(&mut near, &mut far);
            }
            t0 = t0.max(near);
            t1 = t1.min(far);
            if t0 > t1 {
                return false;
            }
        }
        true
    }
}

/// Every surface in a scene as boxes: floor, walls, solids (a circle uses its square)
/// and the roof.
pub fn scene_boxes(scene: &genos_scene::Scene) -> Vec<SurfaceBox> {
    let f = &scene.floor;
    let (fx0, fx1) = (f.position.x - f.half_x, f.position.x + f.half_x);
    let (fz0, fz1) = (f.position.z - f.half_z, f.position.z + f.half_z);
    let mut out = vec![SurfaceBox { min: [fx0, 0.0, fz0], max: [fx1, 0.0, fz1] }];
    for wall in &scene.walls {
        out.push(SurfaceBox {
            min: [wall.position.x - wall.half_x, wall.base, wall.position.z - wall.half_z],
            max: [
                wall.position.x + wall.half_x,
                wall.base + wall.height,
                wall.position.z + wall.half_z,
            ],
        });
    }
    for solid in &scene.solids {
        // A turned square: the box around its corners.
        let half = crate::world::solid_reach(solid);
        out.push(SurfaceBox {
            min: [solid.position.x - half, 0.0, solid.position.z - half],
            max: [solid.position.x + half, solid.height, solid.position.z + half],
        });
    }
    if let Some(ceiling) = &scene.ceiling {
        out.push(SurfaceBox { min: [fx0, ceiling.height, fz0], max: [fx1, ceiling.height, fz1] });
    }
    out
}

/// Lattice and window of the tier.
#[derive(Clone, Copy, Debug)]
pub struct TierLayout {
    /// Metres between probes.
    pub spacing: f32,
    /// Half width of the window box around the camera, in metres.
    pub radius: f32,
    /// A probe is live when a surface is this close, in metres. sqrt(3) spacings
    /// keeps all eight probes around any surface point live.
    pub reach: f32,
}

impl TierLayout {
    pub fn new(spacing: f32, radius: f32) -> Self {
        Self { spacing, radius, reach: spacing * 3.0_f32.sqrt() }
    }

    fn brick_span(&self) -> f32 {
        self.spacing * BRICK as f32
    }

    /// Brick coordinates covering the window around `eye` (inclusive bounds).
    pub fn window(&self, eye: [f32; 3]) -> ([i32; 3], [i32; 3]) {
        let span = self.brick_span();
        let mut lo = eye.map(|e| ((e - self.radius) / span).floor() as i32);
        let hi = eye.map(|e| ((e + self.radius) / span).ceil() as i32 - 1);
        // A high camera puts the ground in the last metres of the window. The picture
        // fades the tier out there and shows the coarse world probes. Keep the ground
        // a full fade (4 m) inside the window.
        let ground = ((-4.0) / span).floor() as i32;
        if lo[1] > ground {
            lo[1] = ground;
        }
        (lo, hi)
    }

    /// World position of probe `local` (0..BRICK on each axis) in brick `brick`.
    pub fn probe_position(&self, brick: [i32; 3], local: [i32; 3]) -> [f32; 3] {
        let mut p = [0.0; 3];
        for i in 0..3 {
            p[i] = ((brick[i] * BRICK + local[i]) as f32 + 0.5) * self.spacing;
        }
        p
    }
}

/// The allocated part of the window.
#[derive(Clone, Debug, Default)]
pub struct BrickSet {
    /// Allocated bricks, in window order.
    pub bricks: Vec<[i32; 3]>,
    /// Live probes of each brick, one bit per probe at `(y * BRICK + z) * BRICK + x`.
    pub masks: Vec<u64>,
    /// Position of every probe of each brick, in the same order (lattice points for
    /// dead ones).
    pub positions: Vec<Vec<[f32; 3]>>,
    /// Live probes (outside solids, near a surface) inside the allocated bricks.
    pub live_probes: usize,
    /// Bricks in the window, allocated or not.
    pub window_bricks: usize,
}

impl BrickSet {
    pub fn allocated_probes(&self) -> usize {
        self.bricks.len() * (BRICK * BRICK * BRICK) as usize
    }

    pub fn window_probes(&self) -> usize {
        self.window_bricks * (BRICK * BRICK * BRICK) as usize
    }

    /// Probe payload plus the indirection table, in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.allocated_probes() * PROBE_BYTES + self.window_bricks * INDIRECTION_BYTES
    }
}

/// Least gap between a live probe and any surface, in metres. A probe on a face (a
/// wall face on a lattice plane) is not strictly inside, but its rays into that solid
/// start on the face: the ray test sees no entry and the rays come out on the far side
/// with the light there. The bounce rays skip hits nearer than 2 mm, so the gap must
/// be wider than that. 1 cm is far above float noise at the window edge (50 m).
pub const PROBE_CLEARANCE: f32 = 0.01;

/// True when the probe at `p` is outside every box, at least `PROBE_CLEARANCE` from
/// every surface, and within `reach` of one. Nothing stands below the ground, so a
/// probe under it is dead.
pub fn probe_live(boxes: &[SurfaceBox], p: [f32; 3], reach: f32) -> bool {
    if p[1] < 0.0 {
        return false;
    }
    let mut near = false;
    for b in boxes {
        if b.inside(p) || b.surface_distance(p) < PROBE_CLEARANCE {
            return false;
        }
        if !near && b.surface_distance(p) <= reach {
            near = true;
        }
    }
    near
}

/// Gap between a solid's face and a probe moved out of that solid, in spacings.
pub const RELOCATE_GAP: f32 = 0.1;

/// Where the probe of lattice point `p` sits, or none when it is dead. A lattice point
/// inside a solid (or touching one) would leave the band beside that solid with no
/// probe of its own, and the light there changes fastest: a solid hides the light
/// behind it from the floor at its foot. Such a probe moves straight out through the
/// nearest face (through each of two or three faces that are equally near, so a probe
/// in a corner lands outside the corner) to `RELOCATE_GAP` spacings past it. It stays
/// dead when that leaves its own cell (half a spacing on any axis) or lands in or
/// against another surface. This is the same physics as a probe on its lattice point,
/// measured where the light is.
pub fn place_probe(boxes: &[SurfaceBox], p: [f32; 3], layout: &TierLayout) -> Option<[f32; 3]> {
    if p[1] < 0.0 {
        return None;
    }
    let gap = RELOCATE_GAP * layout.spacing;
    let mut q = p;
    let solid = |b: &SurfaceBox| (0..3).all(|i| b.max[i] - b.min[i] > 1.0e-4);
    if let Some(b) = boxes
        .iter()
        .find(|b| solid(b) && (b.inside(p) || b.surface_distance(p) < PROBE_CLEARANCE))
    {
        // Exits: (axis, toward max, distance to that face). A box standing on the
        // ground has no way out below.
        let mut exits: Vec<(usize, bool, f32)> = Vec::new();
        for axis in 0..3 {
            if axis != 1 || b.min[1] > 0.0 {
                exits.push((axis, false, p[axis] - b.min[axis]));
            }
            exits.push((axis, true, b.max[axis] - p[axis]));
        }
        let least = exits.iter().map(|e| e.2).fold(f32::MAX, f32::min);
        let mut moved = [false; 3];
        for &(axis, up, d) in &exits {
            // Equal exits on one axis (the middle of a thin wall): the low side.
            if d <= least + 1.0e-3 && !moved[axis] {
                moved[axis] = true;
                q[axis] = if up { b.max[axis] + gap } else { b.min[axis] - gap };
            }
        }
        if (0..3).any(|i| (q[i] - p[i]).abs() > 0.5 * layout.spacing) {
            return None;
        }
    }
    probe_live(boxes, q, layout.reach).then_some(q)
}

/// Allocate the bricks of the window around `eye` that hold a live probe.
pub fn allocate(boxes: &[SurfaceBox], eye: [f32; 3], layout: &TierLayout) -> BrickSet {
    let (lo, hi) = layout.window(eye);
    let mut set = BrickSet::default();
    for by in lo[1]..=hi[1] {
        for bz in lo[2]..=hi[2] {
            for bx in lo[0]..=hi[0] {
                set.window_bricks += 1;
                let brick = [bx, by, bz];
                if let Some((mask, positions)) = place_brick(boxes, brick, layout) {
                    set.bricks.push(brick);
                    set.masks.push(mask);
                    set.positions.push(positions);
                    set.live_probes += mask.count_ones() as usize;
                }
            }
        }
    }
    set
}

/// Bounds of the surfaces that can make a probe of `brick` live, dead or moved.
fn brick_reach(brick: [i32; 3], layout: &TierLayout) -> ([f32; 3], [f32; 3]) {
    let span = layout.brick_span();
    (brick.map(|b| b as f32 * span - layout.reach), brick.map(|b| (b + 1) as f32 * span + layout.reach))
}

/// The live probes of `brick` and where each probe sits, or none when no probe is live.
fn place_brick(boxes: &[SurfaceBox], brick: [i32; 3], layout: &TierLayout) -> Option<(u64, Vec<[f32; 3]>)> {
    let (min, max) = brick_reach(brick, layout);
    let nearby: Vec<SurfaceBox> = boxes.iter().copied().filter(|b| b.overlaps(min, max)).collect();
    if nearby.is_empty() {
        return None;
    }
    let mut mask = 0u64;
    let mut positions = vec![[0.0f32; 3]; BRICK_PROBES as usize];
    for ly in 0..BRICK {
        for lz in 0..BRICK {
            for lx in 0..BRICK {
                let p = layout.probe_position(brick, [lx, ly, lz]);
                let bit = (ly * BRICK + lz) * BRICK + lx;
                positions[bit as usize] = p;
                if let Some(q) = place_probe(&nearby, p, layout) {
                    mask |= 1 << bit;
                    positions[bit as usize] = q;
                }
            }
        }
    }
    (mask != 0).then_some((mask, positions))
}

/// Margin, in metres, around a moved solid inside which every brick relights.
const GEOMETRY_MARGIN: f32 = 1.0;
/// Bounce reach of a moved solid, in its own sizes past the margin. A diffuse face of
/// size s seen from d away covers about s^2 / (pi d^2) of the sky: past two sizes its
/// light, or the light it hid, is under a tenth of what a face beside it gets.
const GEOMETRY_SIZES: f32 = 2.0;
/// How far a directional light's shadow of a moved solid is followed, in metres.
const SUN_SHADOW: f32 = 80.0;

/// Walls and solids, not the floor or roof sheets. The eye is outside and far when it
/// is beyond this hull and can see the whole of it. Interior animation then does not
/// relight the probes: the outside picture stays the sun, the sky and the building.
fn building_hull(boxes: &[SurfaceBox]) -> Option<SurfaceBox> {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut any = false;
    for b in boxes {
        if b.max[1] - b.min[1] < 0.4 {
            continue;
        }
        any = true;
        for i in 0..3 {
            lo[i] = lo[i].min(b.min[i]);
            hi[i] = hi[i].max(b.max[i]);
        }
    }
    any.then_some(SurfaceBox { min: lo, max: hi })
}

/// True when `eye` is outside `hull` and far enough to see all of it.
fn outside_far(eye: [f32; 3], hull: &SurfaceBox) -> bool {
    let inside_xz = (0..3).step_by(2).all(|i| eye[i] > hull.min[i] && eye[i] < hull.max[i]);
    let inside_y = eye[1] > hull.min[1] - 0.5 && eye[1] < hull.max[1] + 1.5;
    if inside_xz && inside_y {
        return false;
    }
    let center = [0, 1, 2].map(|i| 0.5 * (hull.min[i] + hull.max[i]));
    let extent = (hull.max[0] - hull.min[0]).max(hull.max[2] - hull.min[2]) * 0.5;
    let horiz = ((eye[0] - center[0]).powi(2) + (eye[2] - center[2]).powi(2)).sqrt();
    horiz > extent + 4.0 || eye[1] > hull.max[1] + 4.0
}

/// A box or a point lamp that sits inside the hull, not on the outer shell.
fn interior_point(p: [f32; 3], hull: &SurfaceBox) -> bool {
    (0..3).all(|i| p[i] > hull.min[i] + 0.3 && p[i] < hull.max[i] - 0.3) && p[1] < hull.max[1]
}

fn interior_box(b: &SurfaceBox, hull: &SurfaceBox) -> bool {
    let c = [0, 1, 2].map(|i| 0.5 * (b.min[i] + b.max[i]));
    interior_point(c, hull)
}

/// True when moving the solids `moved` (their old and new places) can change the light
/// in the brick `lo`..`hi`: it is within their bounce reach, or a light that reaches it
/// passes one of them on the way (their shadow fell or now falls on it).
fn geometry_reaches(moved: &[SurfaceBox], lights: &[TierLight], lo: [f32; 3], hi: [f32; 3]) -> bool {
    let center = [0, 1, 2].map(|i| 0.5 * (lo[i] + hi[i]));
    let half = 0.5 * (0..3).map(|i| (hi[i] - lo[i]).powi(2)).sum::<f32>().sqrt();
    for m in moved {
        let size = (0..3).map(|i| m.max[i] - m.min[i]).fold(0.0_f32, f32::max);
        let reach = GEOMETRY_MARGIN + GEOMETRY_SIZES * size;
        let gap2: f32 = (0..3)
            .map(|i| (m.min[i] - hi[i]).max(lo[i] - m.max[i]).max(0.0).powi(2))
            .sum();
        if gap2 <= reach * reach {
            return true;
        }
        let m_center = [0, 1, 2].map(|i| 0.5 * (m.min[i] + m.max[i]));
        let m_half = 0.5 * (0..3).map(|i| (m.max[i] - m.min[i]).powi(2)).sum::<f32>().sqrt();
        for light in lights.iter().filter(|l| l.reaches(lo, hi)) {
            // The rays from the light to the brick fill a cone (a cylinder for a
            // directional light); at the solid it is as wide as the brick times the
            // solid's share of the way there.
            let (to, share) = if light.directional {
                let len = light.pos.iter().map(|v| v * v).sum::<f32>().sqrt().max(1.0e-6);
                ([0, 1, 2].map(|i| center[i] - light.pos[i] / len * SUN_SHADOW), 1.0)
            } else {
                let dist = |p: [f32; 3]| (0..3).map(|i| (p[i] - light.pos[i]).powi(2)).sum::<f32>().sqrt();
                (light.pos, ((dist(m_center) + m_half) / dist(center).max(1.0e-6)).min(1.0))
            };
            let grow = half * share;
            let grown = SurfaceBox { min: m.min.map(|v| v - grow), max: m.max.map(|v| v + grow) };
            if grown.crosses(center, to) {
                return true;
            }
        }
    }
    false
}

/// The boxes of `old` that `new` lacks, then those of `new` that `old` lacks: the old
/// and new places of everything that moved, appeared or went.
fn moved_boxes(old: &[SurfaceBox], new: &[SurfaceBox]) -> Vec<SurfaceBox> {
    let key = |b: &SurfaceBox| [b.min, b.max].map(|v| v.map(f32::to_bits));
    let mut count: std::collections::HashMap<_, i32> = std::collections::HashMap::new();
    for b in old {
        *count.entry(key(b)).or_default() += 1;
    }
    for b in new {
        *count.entry(key(b)).or_default() -= 1;
    }
    let mut out = Vec::new();
    for (list, sign) in [(old, 1), (new, -1)] {
        for b in list {
            let left = count.get_mut(&key(b)).expect("counted");
            if *left * sign > 0 {
                *left -= sign;
                out.push(*b);
            }
        }
    }
    out
}


// ---- Field layout ----------------------------------------------------------
// The tier lives after the world probes in the light field (texel = 16 bytes).
// light.comp and scene.frag (tier.glsl) mirror these offsets.

/// First tier texel. x, y, z: low window brick. w: probe spacing (0 = no tier).
pub const TIER_INFO: u32 = crate::field::FIELD_COPY;
/// x, y, z: window size in bricks. w: window half width in metres.
pub const TIER_DIMS: u32 = TIER_INFO + 1;
/// One texel per window brick. x: brick slot, or -1 for none or not lit yet.
pub const TIER_INDIR: u32 = TIER_DIMS + 1;
pub const TIER_INDIR_CAP: u32 = 32768;
/// Two texels per slot: brick coordinates (w = 1), then the live mask as four
/// 16-bit integers.
pub const TIER_SLOTS: u32 = TIER_INDIR + TIER_INDIR_CAP;
/// Bricks the tier can hold. Past it the farthest bricks drop to the world probes.
pub const TIER_SLOT_CAP: u32 = 1024;
/// Room reserved for the slot table.
const TIER_TABLE_ROOM: u32 = 2048;
/// Work list, `WORK_TEXELS` per item: slot, rays, blend mode (0 averages, 1 replaces, 2 + h
/// keeps h rays of history), pass seed; then the
/// position of each of the brick's probes.
pub const TIER_WORK: u32 = TIER_SLOTS + TIER_TABLE_ROOM * 2;
pub const WORK_TEXELS: u32 = 1 + BRICK_PROBES;
/// Probe cubes, `PROBE_TEXELS` per probe and `BRICK_PROBES` per slot.
pub const TIER_PROBES: u32 = TIER_INFO + 106496;
/// End of the tier, the size of the light field in texels.
pub const TIER_END: u32 = TIER_PROBES + TIER_SLOT_CAP * BRICK_PROBES * PROBE_TEXELS;

const _: () = assert!(TIER_SLOT_CAP <= TIER_TABLE_ROOM && TIER_WORK + TIER_SLOT_CAP * WORK_TEXELS <= TIER_PROBES);

/// Rays a probe takes on its brick's first pass: quick and noisy.
pub const FIRST_RAYS: u32 = 16;
/// Rays per refine pass.
pub const REFINE_RAYS: u32 = 64;
/// Samples at which a probe stops refining.
pub const TARGET_SAMPLES: u32 = 512;
/// A lamp that moves this far (metres) marks the tier for relighting. Direct light is
/// per pixel and follows every move; only the bounce waits for this.
pub const LAMP_MOVE: f32 = 0.1;
/// Relative change in lamp colour or strength that marks the tier for relighting.
pub const LAMP_TINT: f32 = 0.02;

/// One light as the tier sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TierLight {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub directional: bool,
}

impl TierLight {
    /// True when this light's irradiance can reach the box `lo`..`hi` above
    /// [`LAMP_REACH_IRRADIANCE`]. A directional light reaches everything.
    fn reaches(&self, lo: [f32; 3], hi: [f32; 3]) -> bool {
        if self.directional {
            return true;
        }
        let strength = self.color.iter().fold(0.0_f32, |m, c| m.max(c.abs()));
        let reach2 = LAMP_UNIT * strength / LAMP_REACH_IRRADIANCE;
        let d2: f32 = (0..3).map(|i| (self.pos[i] - self.pos[i].clamp(lo[i], hi[i])).powi(2)).sum();
        d2 <= reach2
    }

    /// True when `self` differs from `old` enough to relight the bounce.
    fn moved_from(&self, old: &TierLight) -> bool {
        if self.directional != old.directional {
            return true;
        }
        let d: f32 = (0..3).map(|i| (self.pos[i] - old.pos[i]).powi(2)).sum::<f32>().sqrt();
        let limit = if self.directional { 0.01 } else { LAMP_MOVE };
        if d >= limit {
            return true;
        }
        let scale = old.color.iter().fold(1.0e-3_f32, |m, c| m.max(c.abs()));
        (0..3).any(|i| (self.color[i] - old.color[i]).abs() > LAMP_TINT * scale)
    }

    /// True when a point lamp's strength at least doubles or halves.
    /// A move of the same lamp is not this: the direct term follows the move.
    fn strength_flip(&self, old: &TierLight) -> bool {
        if self.directional || old.directional {
            return false;
        }
        let a = self.color.iter().fold(0.0f32, |m, c| m.max(c.abs()));
        let b = old.color.iter().fold(0.0f32, |m, c| m.max(c.abs()));
        let hi = a.max(b);
        hi > 1.0e-3 && a.min(b) * 2.0 < hi
    }
}

#[derive(Clone, Debug)]
struct Slot {
    brick: [i32; 3],
    mask: u64,
    positions: Vec<[f32; 3]>,
    /// Lit at least once, so the picture may read it.
    filled: bool,
    samples: u32,
    /// Change passes still due (see [`CHANGE_PASSES`]).
    change_left: u32,
    /// The stored light is replaced in one pass. A sun or the sky appeared or disappeared.
    replace: bool,
    /// The replace follows a light that went out. The first pass drops the old bounce
    /// and the result keeps a heavy weight. A light that came on keeps the old bounce
    /// so the new direct can build on it.
    darken: bool,
    /// Probes (bits of `mask`) that already ran the pass under way. A pass larger than
    /// a build's budget runs a share of the probes per build; it counts once all have.
    done: u64,
    /// When the brick last finished an update (all its change passes), or was made.
    since: std::time::Instant,
    /// Change passes since the picture last took the brick's light: after
    /// [`CHANGE_PASSES`] it is a whole update, even with changes still coming.
    run: u32,
}

impl Slot {
    /// A change reached the brick: it takes its change passes again.
    fn restart(&mut self) {
        self.change_left = CHANGE_PASSES;
        self.replace = false;
        self.darken = false;
        self.done = 0;
    }

    /// A sun, the sky, or a point lamp appeared or disappeared. One pass replaces the
    /// stored light. `darken` is a light that went out.
    fn restart_wide(&mut self, darken: bool) {
        self.change_left = 1;
        self.replace = true;
        self.darken = darken;
        self.done = 0;
    }
}

/// Passes a brick takes after a light or geometry change that reaches it. The stored
/// light is kept, never cleared: each pass moves it a fixed share of the way to the
/// new light ([`CHANGE_HISTORY`]) and feeds the stored light back at its hits, so the
/// direct change lands on the first pass and each pass adds a bounce. Every brick the
/// change reaches takes pass k before any takes pass k + 1, so neighbours agree and
/// the change spreads evenly instead of brick by brick. The schedule is kept per slot
/// and only ever blends, so further light layers per probe can take the same passes.
/// Averaging refine passes follow:
/// they keep all the history, so the share of each new pass falls from there on and
/// the noise of the fast passes averages away without holding up the visible change.
pub const CHANGE_PASSES: u32 = 6;
/// Rays per change pass.
pub const CHANGE_RAYS: u32 = 256;
/// History, in rays, a change pass keeps: a third of its own rays, so a pass takes
/// three quarters of the way to its new estimate. The rest of the old light fades by
/// a quarter of itself per pass; the remaining error shrinks by (1 - 0.75 (1 - r)) per
/// pass for a scene that returns r of its light per bounce.
pub const CHANGE_HISTORY: u32 = CHANGE_RAYS / 3;
/// Fewest rays when a lamp switches. Four rays left the night shadow black.
/// A sun change uses [`FLUSH_RAYS_MAX`] instead.
const FLUSH_RAYS_MIN: u64 = 16;
/// Most rays in that pass. More rays than this do not fit the tenth of a second.
const FLUSH_RAYS_MAX: u64 = 64;
/// Probe rays that pass may spend, across every pass. The picture waits for it.
const FLUSH_RAY_BUDGET: u64 = 48_000;
/// Passes in that build. Each pass reads the one before it and adds a bounce.
const FLUSH_PASSES: u32 = 4;
/// Sun-gone passes: the first drops the old bounce, then one pass per bounce
/// the open yard needs. The shadow beside a wall reads a farther floor probe
/// in the picture; more gather passes did not raise it.
const NIGHT_HOPS: u32 = 8;
/// Probe rays one night pass may spend. The pass has to finish inside the
/// live picture's tenth of a second, with the passes that follow it.
const NIGHT_RAY_BUDGET: u64 = 200_000;
/// Irradiance below which a lamp no longer reaches a brick: a change to that lamp
/// leaves the brick alone. A unit lamp reaches about 270 m.
pub const LAMP_REACH_IRRADIANCE: f32 = 1.0e-3;
/// Irradiance of a unit lamp at 1 m (light.comp LAMP_UNIT).
const LAMP_UNIT: f32 = 72.0;
/// Scheduling class of the averaging refine passes, after every change pass.
const REFINE_CLASS: u32 = CHANGE_PASSES + 1;

/// One brick's pass, or a share of its probes, in a build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TierItem {
    pub slot: u32,
    pub brick: [i32; 3],
    pub rays: u32,
    pub reset: bool,
    /// The live probes (bits of the brick's mask) this item runs: all of them, or the
    /// share of a pass that fits what is left of the budget.
    pub probes: u64,
    /// The build runs its items in rounds, one after another: a brick on screen takes
    /// all its change passes in one build, pass k in round k.
    pub round: u32,
    /// For a change pass: the history, in rays, the pass keeps. `None` averages into
    /// all of it (refine) or, with `reset`, replaces it (a brick's first pass).
    pub history: Option<u32>,
}

/// Everything one light build needs from the tier.
#[derive(Clone, Debug, Default)]
pub struct TierBatch {
    pub id: u64,
    /// Texels from `TIER_INFO` up to `TIER_WORK` plus `WORK_TEXELS` per item.
    pub texels: Vec<[f32; 4]>,
    pub items: Vec<TierItem>,
    /// Slots below this index may hold light. The build copies them forward.
    pub used_slots: u32,
    /// Probe rays this batch casts (live probes times rays).
    pub probe_rays: u64,
    /// Items in each round; `items` holds round 0's first, then round 1's, and so on.
    pub rounds: Vec<u32>,
}

/// The camera, for on-screen importance: the bricks whose light it sees go first, and
/// changing ones take all their change passes at once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TierCamera {
    pub eye: [f32; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    /// Tangents of the half view angles across and up.
    pub tan_x: f32,
    pub tan_y: f32,
}

/// Rays across the view each frame (columns, rows) for what the camera sees. The
/// grid turns by a sixteenth of a cell each frame, so 16 frames cover the screen on a
/// grid four times as fine.
const LOOK_COLS: u32 = 32;
const LOOK_ROWS: u32 = 18;
/// Share of a brick's seen count kept from one frame to the next: a brick stays seen
/// for a few frames after the rays last met it.
const LOOK_KEEP: f32 = 0.75;
/// Seen count above which a brick is in view.
const LOOK_SEEN: f32 = 0.05;

/// How the tier ranks its work and how far bricks hold still. Live: set it with
/// `Renderer::set_tier_weights`; `GENOS_TIER_WEIGHTS=near=4,feed=0.5,...` sets it at
/// start. A brick's priority is its share of the screen (the view rays that met it)
/// times `near / (near + distance)` times `1 + age / stale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TierWeights {
    /// Metres: a brick this far from the camera ranks half as high as one at it.
    pub near: f32,
    /// Share of its most-seen neighbour's count a brick weighs: the light it feeds them.
    pub feed: f32,
    /// The view rays reach this share of the screen's half widths past its edges, so
    /// bricks the camera is about to turn to weigh something.
    pub margin: f32,
    /// What a ray past the screen's edge counts.
    pub edge: f32,
    /// Seconds after its last finished update in which a brick's priority doubles
    /// (only a brick the picture uses).
    pub stale: f32,
    /// Metres from the camera beyond which a brick skips small finished updates.
    pub far: f32,
    /// The largest change (a share of the light) a brick `far` metres beyond `far`
    /// skips: it grows from 0 at `far` to this at twice `far`, so far light never
    /// creeps by small steps.
    pub far_skip: f32,
}

impl Default for TierWeights {
    fn default() -> Self {
        Self { near: 4.0, feed: 0.5, margin: 0.25, edge: 0.25, stale: 0.25, far: 8.0, far_skip: 0.03 }
    }
}

impl TierWeights {
    /// `name=value` pairs, comma-separated, over the defaults.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut out = Self::default();
        for pair in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (name, value) = pair.split_once('=').ok_or(format!("tier weight {pair} is not name=value"))?;
            let value: f32 = value.trim().parse().map_err(|_| format!("tier weight {pair} is not a number"))?;
            let field = match name.trim() {
                "near" => &mut out.near,
                "feed" => &mut out.feed,
                "margin" => &mut out.margin,
                "edge" => &mut out.edge,
                "stale" => &mut out.stale,
                "far" => &mut out.far,
                "far_skip" => &mut out.far_skip,
                other => return Err(format!("no tier weight {other}")),
            };
            *field = value;
        }
        Ok(out)
    }
}

impl std::fmt::Display for TierWeights {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "near={} feed={} margin={} edge={} stale={} far={} far_skip={}",
            self.near, self.feed, self.margin, self.edge, self.stale, self.far, self.far_skip
        )
    }
}

/// Counts for reports.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TierStats {
    pub window_bricks: usize,
    pub bricks: usize,
    pub live_probes: usize,
    pub filled_bricks: usize,
    pub pending_bricks: usize,
    /// Bricks still taking their first or a change pass: the visible part of a change.
    /// Refining after that only averages noise away.
    pub changing_bricks: usize,
    pub dropped_bricks: usize,
    /// Changing bricks on screen in the last batch: they took all their change passes
    /// in it (as far as the budget reached).
    pub critical_bricks: usize,
    /// Bricks whose light the camera sees, and how many of those have no work left.
    pub seen_bricks: usize,
    pub seen_settled: usize,
    /// CPU microseconds the last batch took to pick its work, on-screen test included.
    pub batch_us: u32,
    /// Bricks left unlit because the camera was outside and far, and the change was
    /// a box or a lamp inside the building.
    pub interior_skipped: u64,
}

/// The persistent tier on the CPU: which brick lives in which slot, what each slot
/// still needs, and how much of that fits in one build.
#[derive(Clone, Debug)]
pub struct TierState {
    pub layout: TierLayout,
    /// Keys of the boxes and of the surface colours last seen.
    geometry: Option<u64>,
    materials: Option<u64>,
    boxes: Vec<SurfaceBox>,
    window: Option<([i32; 3], [i32; 3])>,
    slots: Vec<Option<Slot>>,
    by_brick: std::collections::HashMap<[i32; 3], u32>,
    free: Vec<u32>,
    lights: Vec<TierLight>,
    light_gen: u64,
    /// The last batch's on-screen changing bricks and its CPU time (TierStats).
    critical: usize,
    batch_us: u32,
    next_batch: u64,
    dropped: usize,
    live: usize,
    /// GPU milliseconds per probe ray, learned from timed builds ([`Self::note_time`]).
    /// None before the first.
    ms_per_ray: Option<f64>,
    /// The last budget a batch was cut to: only a build that used most of its budget
    /// teaches the per-ray cost.
    last_budget: Option<u64>,
    /// Camera distance, in metres, at which the picture leaves the fine probes for
    /// the world volume. [`Self::fit_depth`] moves it so the gather meets its budget.
    fine_depth: f32,
    /// The directional lights changed count on the last update: the sun or the sky
    /// came or went. The replacing pass then covers every brick on screen.
    sun_wide: bool,
    /// The sun, the sky, or a point lamp switched on or off on the last update.
    /// The world volume has to be rebuilt. A moved box does not.
    world_now: bool,
    /// The sun or the sky went out on the last update. The first pass of that
    /// build drops the old bounce. The picture keeps up with the rebuild.
    sun_drop: bool,
    /// Jacobi passes still due after the sun went out: one drop, then one pass
    /// per bounce the ray trace keeps. Each pass is its own frame so it reads
    /// the pass before it.
    night_left: u32,
    /// The last night pass has been scheduled. Commit then stops refine, so
    /// later passes do not add bounces the ray trace does not count.
    night_freeze: bool,
    /// The last batch was a flush that is allowed to run long. It must not
    /// shrink the fine shell.
    long_build: bool,
    /// See [`TierStats::interior_skipped`].
    interior_skipped: u64,
    /// Per slot: how much of the view reads the brick's light ([`Self::look`]).
    seen: Vec<f32>,
    look_frame: u32,
    pub weights: TierWeights,
}

/// How far one slower build moves the per-ray cost toward itself.
const COST_RISE: f64 = 0.05;

/// Probe rays a build needs before it can show that rays got dearer. A probe is one
/// workgroup, so a build of fewer probes leaves most of a GPU idle: they run side by
/// side in the time of one, and its time is the latency of a probe and the wait for
/// the queue, not the cost of its rays. Learning from it shrank the budget to one
/// probe a build in busy scenes.
const FILL_RAYS: u64 = 32 * CHANGE_RAYS as u64;

/// A build shares the GPU with the picture, and that only ever makes it look slower:
/// the fastest builds show what a ray costs. A faster sample is taken at once; a
/// slower one moves the cost a little, so a lasting change (a heavier scene, lower
/// clocks) still comes through within some tens of builds while a frame that held the
/// GPU does not shrink the budget.
fn learn_cost(old: f64, sample: f64) -> f64 {
    if sample < old {
        sample
    } else if sample > old * 4.0 {
        // A full build that costs several times the learned price closes half the gap.
        // Waiting on the small step left a 30 ms gather priced as 1.5 ms.
        old + (sample - old) * 0.5
    } else {
        old + (sample - old) * COST_RISE
    }
}

impl Default for TierState {
    fn default() -> Self {
        Self::new(TierLayout::new(1.0, 50.0))
    }
}

impl TierState {
    pub fn new(layout: TierLayout) -> Self {
        Self {
            layout,
            geometry: None,
            materials: None,
            boxes: Vec::new(),
            window: None,
            slots: Vec::new(),
            by_brick: std::collections::HashMap::new(),
            free: Vec::new(),
            lights: Vec::new(),
            light_gen: 0,
            critical: 0,
            batch_us: 0,
            next_batch: 1,
            dropped: 0,
            live: 0,
            ms_per_ray: None,
            last_budget: None,
            fine_depth: 16.0,
            sun_wide: false,
            world_now: false,
            sun_drop: false,
            night_left: 0,
            night_freeze: false,
            long_build: false,
            interior_skipped: 0,
            seen: Vec::new(),
            weights: TierWeights::default(),
            look_frame: 0,
        }
    }

    /// Light generation. It moves on when a light or the geometry changes.
    pub fn light_gen(&self) -> u64 {
        self.light_gen
    }

    /// True when the last update added or removed the sun or the sky.
    pub fn sun_wide(&self) -> bool {
        self.sun_wide
    }

    /// The world volume should be rebuilt from this update.
    pub fn world_now(&self) -> bool {
        self.world_now
            || self.slots.iter().flatten().any(|slot| slot.replace && slot.change_left > 0)
    }

    /// True when the last update removed the sun or the sky.
    pub fn sun_drop(&self) -> bool {
        self.sun_drop
    }

    /// Follow the scene, the camera and the lights. `materials` keys the surface
    /// colours. True when the bricks or the light generation changed.
    pub fn update(
        &mut self,
        boxes: Vec<SurfaceBox>,
        materials: u64,
        eye: [f32; 3],
        lights: &[TierLight],
    ) -> bool {
        let mut changed = false;
        let geometry = boxes_key(&boxes);
        // What a geometry change reaches: nothing, the bricks near the boxes that moved
        // (`Some`), or every brick (`None`: new surface colours).
        let mut reached: Option<Vec<SurfaceBox>> = Some(Vec::new());
        if self.materials != Some(materials) {
            reached = None;
            self.materials = Some(materials);
            self.light_gen += 1;
            changed = true;
        }
        let mut moved = Vec::new();
        if self.geometry != Some(geometry) {
            self.geometry = Some(geometry);
            moved = moved_boxes(&self.boxes, &boxes);
            self.boxes = boxes;
            if let Some(list) = reached.as_mut() {
                list.extend_from_slice(&moved);
            }
            self.light_gen += 1;
            changed = true;
        }
        let window = self.layout.window(eye);
        // Outside and far: a box or a lamp inside the building does not relight
        // probes. The sun and the sky still do. The outside picture stays the
        // static building.
        let hull = building_hull(&self.boxes);
        let far = hull.as_ref().is_some_and(|hull| outside_far(eye, hull));
        if far {
            if let Some(hull) = hull.as_ref() {
                let before = moved.len();
                moved.retain(|b| !interior_box(b, hull));
                self.interior_skipped += (before - moved.len()) as u64;
                if let Some(list) = reached.as_mut() {
                    list.retain(|b| !interior_box(b, hull));
                }
            }
        }
        if self.window != Some(window) {
            self.window = Some(window);
            changed |= self.reallocate(eye);
        } else if !moved.is_empty() {
            changed |= self.reallocate_near(&moved, eye);
        }
        // Lamps that changed, where they were and where they are now.
        // A sun or the sky coming or going is one replacing pass. A drift of colour is not:
        // the running sun would replace every probe several times a second.
        let old_dirs = self.lights.iter().filter(|light| light.directional).count();
        let new_dirs = lights.iter().filter(|light| light.directional).count();
        let wide = old_dirs != new_dirs;
        self.sun_wide = wide;
        let mut reach: Vec<TierLight> = Vec::new();
        // Point lamps that switched on or off. Their bricks take one replacing pass.
        let mut flips: Vec<TierLight> = Vec::new();
        let mut went_dark = new_dirs < old_dirs;
        // A running sun drifts. A jump, such as a time skip, replaces the field.
        let mut sun_jump = false;
        for i in 0..lights.len().max(self.lights.len()) {
            match (self.lights.get(i), lights.get(i)) {
                (Some(old), Some(new)) if !new.moved_from(old) => {}
                (old, new) => {
                    if let (Some(old), Some(new)) = (old, new) {
                        if new.directional && old.directional {
                            let d: f32 = (0..3)
                                .map(|k| (new.pos[k] - old.pos[k]).powi(2))
                                .sum::<f32>()
                                .sqrt();
                            if d > 0.15 {
                                sun_jump = true;
                            }
                        }
                        if new.strength_flip(old) {
                            flips.extend([*old, *new]);
                            let now = new.color.iter().fold(0.0f32, |m, c| m.max(c.abs()));
                            let was = old.color.iter().fold(0.0f32, |m, c| m.max(c.abs()));
                            if now * 2.0 < was {
                                went_dark = true;
                            }
                        }
                    }
                    reach.extend(old.into_iter().chain(new).copied());
                }
            }
        }
        if sun_jump {
            self.sun_wide = true;
        }
        // A jump that keeps the sun is not a drop. Only the light going out is.
        // One drop, then one pass per bounce the reference counts.
        self.sun_drop = self.sun_wide && went_dark;
        // A moved box changes the generation too. The world volume only has to
        // move when the sun, the sky, or a point lamp switches.
        self.world_now = self.sun_wide || !flips.is_empty();
        if self.sun_drop {
            self.night_left = NIGHT_HOPS;
        }
        if far {
            if let Some(hull) = building_hull(&self.boxes) {
                let before = reach.len();
                reach.retain(|light| light.directional || !interior_point(light.pos, &hull));
                flips.retain(|light| light.directional || !interior_point(light.pos, &hull));
                self.interior_skipped += (before - reach.len()) as u64;
            }
        }
        if !reach.is_empty() {
            self.lights = lights.to_vec();
            self.light_gen += 1;
            changed = true;
        }
        let geometry_reach = |lo, hi| match &reached {
            None => true,
            Some(list) => !list.is_empty() && geometry_reaches(list, &self.lights, lo, hi),
        };
        let span = self.layout.brick_span();
        let mut relit = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(slot) = slot.as_ref().filter(|s| s.filled) else { continue };
            let lo = slot.brick.map(|b| b as f32 * span);
            let hi = slot.brick.map(|b| (b + 1) as f32 * span);
            if geometry_reach(lo, hi) || reach.iter().any(|l| l.reaches(lo, hi)) {
                relit.push(index);
            }
        }
        for index in relit {
            let brick = self.slots[index].as_ref().expect("relit slot").brick;
            let lo = brick.map(|b| b as f32 * span);
            let hi = brick.map(|b| (b + 1) as f32 * span);
            let slot = self.slots[index].as_mut().expect("relit slot");
            if wide || sun_jump || flips.iter().any(|light| light.reaches(lo, hi)) {
                slot.restart_wide(went_dark);
            } else {
                slot.restart();
            }
        }
        changed
    }

    /// Place the probes again in the bricks the boxes `moved` can reach, after only
    /// they moved: the rest of the window keeps its probes, so dragging a solid does
    /// not walk the whole window every frame.
    fn reallocate_near(&mut self, moved: &[SurfaceBox], eye: [f32; 3]) -> bool {
        let Some((lo, hi)) = self.window else { return false };
        let span = self.layout.brick_span();
        let reach = self.layout.reach;
        let mut changed = false;
        for m in moved {
            let first = [0, 1, 2].map(|i| (((m.min[i] - reach) / span).floor() as i32).max(lo[i]));
            let last = [0, 1, 2].map(|i| (((m.max[i] + reach) / span).floor() as i32).min(hi[i]));
            for by in first[1]..=last[1] {
                for bz in first[2]..=last[2] {
                    for bx in first[0]..=last[0] {
                        let brick = [bx, by, bz];
                        match place_brick(&self.boxes, brick, &self.layout) {
                            Some((mask, positions)) => {
                                if !self.by_brick.contains_key(&brick)
                                    && self.by_brick.len() >= TIER_SLOT_CAP as usize
                                {
                                    // Full: the nearest bricks win, as a whole placement picks.
                                    return self.reallocate(eye) || changed;
                                }
                                changed |= self.place(brick, mask, positions);
                            }
                            None => {
                                if let Some(slot) = self.by_brick.remove(&brick) {
                                    self.slots[slot as usize] = None;
                                    self.free.push(slot);
                                    self.free.sort_unstable_by(|a, b| b.cmp(a));
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        self.live = self.slots.iter().flatten().map(|s| s.mask.count_ones() as usize).sum();
        changed
    }

    /// Put `brick`'s probes in its slot, or in a new one. A brick whose probes came
    /// alive, died or moved keeps its light and takes the change passes: a probe that
    /// moved starts from the light it had a fraction of a spacing away, a new one from
    /// nothing (the picture skips it until its first pass), and a dead one is cleared
    /// by the next pass. Returns true when anything changed.
    fn place(&mut self, brick: [i32; 3], mask: u64, positions: Vec<[f32; 3]>) -> bool {
        if let Some(&slot) = self.by_brick.get(&brick) {
            let entry = self.slots[slot as usize].as_mut().expect("mapped slot");
            if entry.mask == mask && entry.positions == positions {
                return false;
            }
            entry.mask = mask;
            entry.positions = positions;
            entry.done = 0;
            if entry.filled {
                entry.restart();
            }
            return true;
        }
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                self.slots.push(None);
                (self.slots.len() - 1) as u32
            }
        };
        if let Some(seen) = self.seen.get_mut(slot as usize) {
            *seen = 0.0;
        }
        self.slots[slot as usize] = Some(Slot {
            brick,
            mask,
            positions,
            filled: false,
            samples: 0,
            change_left: 0,
            replace: false,
            darken: false,
            done: 0,
            since: std::time::Instant::now(),
            run: 0,
        });
        self.by_brick.insert(brick, slot);
        true
    }

    fn reallocate(&mut self, eye: [f32; 3]) -> bool {
        let mut set = allocate(&self.boxes, eye, &self.layout);
        self.dropped = 0;
        if set.bricks.len() > TIER_SLOT_CAP as usize {
            // Keep the nearest bricks. Ties go by coordinates, so the same view keeps
            // the same bricks whatever came before.
            let span = self.layout.brick_span();
            let mut order: Vec<usize> = (0..set.bricks.len()).collect();
            let dist = |b: [i32; 3]| -> f32 {
                (0..3).map(|i| ((b[i] as f32 + 0.5) * span - eye[i]).powi(2)).sum()
            };
            order.sort_by(|&a, &b| {
                dist(set.bricks[a])
                    .total_cmp(&dist(set.bricks[b]))
                    .then(set.bricks[a].cmp(&set.bricks[b]))
            });
            order.truncate(TIER_SLOT_CAP as usize);
            order.sort_unstable();
            self.dropped = set.bricks.len() - order.len();
            set.bricks = order.iter().map(|&i| set.bricks[i]).collect();
            set.masks = order.iter().map(|&i| set.masks[i]).collect();
            set.positions = order.iter().map(|&i| set.positions[i].clone()).collect();
        }
        let keep: std::collections::HashMap<[i32; 3], u64> =
            set.bricks.iter().copied().zip(set.masks.iter().copied()).collect();
        let mut changed = false;
        // Free the bricks that left.
        let gone: Vec<[i32; 3]> =
            self.by_brick.keys().filter(|b| !keep.contains_key(*b)).copied().collect();
        for brick in gone {
            if let Some(slot) = self.by_brick.remove(&brick) {
                self.slots[slot as usize] = None;
                self.free.push(slot);
                changed = true;
            }
        }
        // Lowest free slot first, so the used span stays short.
        self.free.sort_unstable_by(|a, b| b.cmp(a));
        for ((brick, mask), positions) in set.bricks.iter().zip(&set.masks).zip(set.positions) {
            changed |= self.place(*brick, *mask, positions);
        }
        self.live = set.live_probes;
        changed
    }

    pub fn stats(&self) -> TierStats {
        let window_bricks = self
            .window
            .map(|(lo, hi)| (0..3).map(|i| (hi[i] - lo[i] + 1).max(0) as usize).product())
            .unwrap_or(0);
        let mut stats = TierStats {
            window_bricks,
            dropped_bricks: self.dropped,
            critical_bricks: self.critical,
            batch_us: self.batch_us,
            interior_skipped: self.interior_skipped,
            ..TierStats::default()
        };
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(slot) = slot else { continue };
            if self.in_view(index) {
                stats.seen_bricks += 1;
                stats.seen_settled += usize::from(self.class(slot).is_none());
            }
            stats.bricks += 1;
            stats.live_probes += slot.mask.count_ones() as usize;
            if slot.filled {
                stats.filled_bricks += 1;
            }
            if let Some(class) = self.class(slot) {
                stats.pending_bricks += 1;
                if class < REFINE_CLASS {
                    stats.changing_bricks += 1;
                }
            }
        }
        stats
    }

    /// Each slot's weight in the picture: the view rays that met it (about the pixels
    /// it lights, hidden faces and everything off screen counting nothing), plus
    /// [`LOOK_FEED`] of the most any neighbour brick was met, since a pass reads its
    /// neighbours' light and their light reaches the view in one bounce. Zero: the
    /// picture does not use the brick now.
    fn weights(&self) -> Vec<f32> {
        let mut weight: Vec<f32> = (0..self.slots.len()).map(|i| self.seen.get(i).copied().unwrap_or(0.0)).collect();
        let fed: Vec<(usize, f32)> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let brick = slot.as_ref()?.brick;
                let most = (0..27)
                    .filter(|&k| k != 13)
                    .filter_map(|k| {
                        let o = [k % 3 - 1, k / 3 % 3 - 1, k / 9 - 1];
                        let near = self.by_brick.get(&[0, 1, 2].map(|i| brick[i] + o[i]))?;
                        self.seen.get(*near as usize).copied()
                    })
                    .fold(0.0f32, f32::max);
                (most > LOOK_SEEN).then_some((index, self.weights.feed * most))
            })
            .collect();
        for (index, feed) in fed {
            weight[index] += feed;
        }
        for w in &mut weight {
            if *w <= LOOK_SEEN {
                *w = 0.0;
            }
        }
        weight
    }

    fn in_view(&self, slot: usize) -> bool {
        self.seen.get(slot).is_some_and(|&s| s > LOOK_SEEN)
    }

    /// What the camera sees: rays across the view meet the nearest surface, and each
    /// hit counts toward the bricks of the probes the picture reads there (around the
    /// point lifted half a spacing off the face). Faces hidden behind others and
    /// everything off screen count nothing. The counts fade by [`LOOK_KEEP`] a frame.
    pub fn look(&mut self, camera: &TierCamera) {
        self.seen.resize(self.slots.len(), 0.0);
        for seen in &mut self.seen {
            *seen *= LOOK_KEEP;
        }
        const BAYER: [u32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
        let turn = BAYER[(self.look_frame % 16) as usize];
        self.look_frame = self.look_frame.wrapping_add(1);
        let (ox, oy) = (((turn % 4) as f32 + 0.5) / 4.0, ((turn / 4) as f32 + 0.5) / 4.0);
        let spacing = self.layout.spacing;
        for row in 0..LOOK_ROWS {
            for col in 0..LOOK_COLS {
                let reach = 1.0 + self.weights.margin.max(0.0);
                let fx = ((col as f32 + ox) / LOOK_COLS as f32 * 2.0 - 1.0) * reach;
                let fy = (1.0 - (row as f32 + oy) / LOOK_ROWS as f32 * 2.0) * reach;
                let count = if fx.abs() > 1.0 || fy.abs() > 1.0 { self.weights.edge } else { 1.0 };
                let (sx, sy) = (fx * camera.tan_x, fy * camera.tan_y);
                let d = [0, 1, 2].map(|i| camera.forward[i] + camera.right[i] * sx + camera.up[i] * sy);
                let len = d.iter().map(|v| v * v).sum::<f32>().sqrt().max(1.0e-6);
                let dir = d.map(|v| v / len);
                let Some((t, axis)) = self.boxes.iter().filter_map(|b| b.entry(camera.eye, dir)).min_by(|a, b| a.0.total_cmp(&b.0))
                else {
                    continue;
                };
                let mut q = [0, 1, 2].map(|i| camera.eye[i] + dir[i] * t);
                q[axis] -= dir[axis].signum() * 0.5 * spacing;
                let cell = q.map(|v| (v / spacing - 0.5).floor() as i32);
                let mut bricks: Vec<[i32; 3]> = Vec::with_capacity(8);
                for corner in 0..8 {
                    let c = [0, 1, 2].map(|i| cell[i] + (corner >> i & 1));
                    let brick = c.map(|v| v.div_euclid(BRICK));
                    if !bricks.contains(&brick) {
                        bricks.push(brick);
                    }
                }
                for brick in bricks {
                    if let Some(&slot) = self.by_brick.get(&brick) {
                        self.seen[slot as usize] += count;
                    }
                }
            }
        }
    }

    /// 0: the brick's first pass. 1 + k: change pass k is due. REFINE_CLASS: refining.
    /// None: converged, nothing to do.
    fn class(&self, slot: &Slot) -> Option<u32> {
        if !slot.filled {
            Some(0)
        } else if slot.change_left > 0 {
            Some(1 + CHANGE_PASSES - slot.change_left.min(CHANGE_PASSES))
        } else if slot.samples < TARGET_SAMPLES {
            Some(REFINE_CLASS)
        } else {
            None
        }
    }

    /// Positions of the live probes in lit bricks, for the debug view.
    pub fn lit_probes(&self) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        for slot in self.slots.iter().flatten().filter(|s| s.filled) {
            for bit in 0..BRICK_PROBES as i32 {
                if slot.mask & (1 << bit) != 0 {
                    out.push(slot.positions[bit as usize]);
                }
            }
        }
        out
    }

    /// Corners of the allocated bricks, for the debug view.
    pub fn brick_boxes(&self) -> Vec<([f32; 3], [f32; 3])> {
        let span = self.layout.brick_span();
        self.slots
            .iter()
            .flatten()
            .map(|s| (s.brick.map(|b| b as f32 * span), s.brick.map(|b| (b + 1) as f32 * span)))
            .collect()
    }

    pub fn has_work(&self) -> bool {
        self.slots.iter().flatten().any(|s| self.class(s).is_some())
    }

    /// Probe rays for a build of `ms` GPU milliseconds. Before the first timed build
    /// everything due fits, and that build measures the GPU.
    /// Metres at which the picture leaves the fine probes for the world volume.
    pub fn fine_depth(&self) -> f32 {
        self.fine_depth
    }

    /// Move the fine shell so a gather of `measured_ms` meets `target_ms`.
    /// A slow build pulls the shell in. A fast build lets it reach farther.
    pub fn fit_depth(&mut self, measured_ms: f64, target_ms: f64) {
        if !(measured_ms > 0.0) || !(target_ms > 0.0) {
            return;
        }
        if measured_ms > target_ms * 1.4 {
            self.fine_depth = (self.fine_depth * 0.8).max(4.0);
        } else if measured_ms < target_ms * 0.55 {
            self.fine_depth = (self.fine_depth * 1.15).min(48.0);
        }
    }

    /// A sun or lamp flush is allowed to run long. The fine shell stays put.
    pub fn long_build(&self) -> bool {
        self.long_build
    }

    pub fn budget_rays(&self, ms: f64) -> u64 {
        match self.ms_per_ray {
            Some(per_ray) => (ms / per_ray.max(1.0e-9)).max(1.0) as u64,
            None => u64::MAX,
        }
    }

    /// Learn the per-ray cost from a timed build. A faster build than the cost predicts
    /// always teaches. A slower one teaches only when it used most of its budget and
    /// had at least [`FILL_RAYS`]: a small build cannot fill the GPU, so its rays look
    /// dearer than they are, and learning from it would shrink the next build and make
    /// it look dearer still.
    pub fn note_time(&mut self, probe_rays: u64, ms: f64) {
        if probe_rays < 256 || !(ms > 0.0) {
            return;
        }
        let sample = ms / probe_rays as f64;
        let full = probe_rays >= FILL_RAYS
            && self.last_budget.is_none_or(|budget| probe_rays.saturating_mul(2) >= budget);
        self.ms_per_ray = match self.ms_per_ray {
            None => Some(sample),
            Some(per_ray) if sample < per_ray || full => Some(learn_cost(per_ray, sample)),
            keep => keep,
        };
    }

    /// The next build's work, up to `budget` probe rays (`None`: everything that is
    /// due). First passes come first, then change passes (every brick's pass k before
    /// any brick's pass k + 1), then refining; nearest first inside each. `first` is
    /// the ray count of a brick's first pass.
    pub fn batch(&mut self, eye: [f32; 3], budget: Option<u64>, first: u32) -> TierBatch {
        self.batch_seen(eye, None, budget, first)
    }

    /// `batch`, with the camera: a changing brick it sees takes all its remaining
    /// change passes in this build, one per round, so what is on screen converges in
    /// one build and what is off screen follows a pass a build. Bricks still waiting
    /// for their first light go first, then the bricks on screen, nearest first.
    pub fn batch_seen(
        &mut self,
        eye: [f32; 3],
        camera: Option<&TierCamera>,
        budget: Option<u64>,
        first: u32,
    ) -> TierBatch {
        let started = std::time::Instant::now();
        // fit_depth runs before this batch and can leave a 4 m shell. From
        // outside, the yard is tens of metres away and would read the coarse
        // world probes. Pin the shell after that shrink.
        if building_hull(&self.boxes).as_ref().is_some_and(|hull| outside_far(eye, hull)) {
            self.fine_depth = self.fine_depth.max(48.0);
        }
        if let Some(camera) = camera {
            self.look(camera);
        }
        let span = self.layout.brick_span();
        let weight = self.weights();
        // A replacing pass stores a short ray set and then holds it. A brick the
        // camera does not see takes a normal update instead, so that short set is
        // not what the picture shows when the camera turns toward the brick.
        // A night hop has to cross the building, including bricks off screen.
        // Downgrading those bricks would take them out of the hop.
        let off_screen: Vec<usize> = if self.night_left > 0 {
            Vec::new()
        } else {
            self.slots
                .iter()
                .enumerate()
                .filter_map(|(index, slot)| {
                    let slot = slot.as_ref()?;
                    // A lamp that went out keeps its short result only on screen. Off screen,
                    // that hold would leave the yard dark after the lamp comes back.
                    // A lamp that came on still replaces off-screen bricks, in this build.
                    (slot.replace && slot.darken && weight.get(index).copied().unwrap_or(0.0) <= 0.0)
                        .then_some(index)
                })
                .collect()
        };
        for index in off_screen {
            if let Some(slot) = self.slots[index].as_mut() {
                slot.restart();
            }
        }
        // (order, weight, class key, distance, brick, slot). Order 0: a brick the
        // picture uses that waits for its first light (a cheap pass, and until it lands
        // the picture falls back to the world probes). 1: one the picture uses with a
        // change under way (its light is wrong), by weight. 2: one the picture uses that
        // refines (its light is right but noisy). 3 to 5: the same for bricks it does not
        // use; they only get what is left. Inside a weight bucket the brick nearest done
        // goes first, so it finishes.
        let mut due: Vec<(u32, i32, u32, f32, [i32; 3], u32)> = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(slot) = slot else { continue };
            let Some(class) = self.class(slot) else { continue };
            let center = [0, 1, 2].map(|i| (slot.brick[i] as f32 + 0.5) * span);
            let d: f32 = (0..3).map(|i| (center[i] - eye[i]).powi(2)).sum();
            // Share of the screen, times nearness, times staleness: a brick the picture
            // uses rises the longer since its last finished update, so under load it
            // is refreshed in turn. A brick the picture does not use stays at zero.
            let near = self.weights.near.max(1.0e-3);
            let stale = 1.0 + slot.since.elapsed().as_secs_f32() / self.weights.stale.max(1.0e-3);
            let depth = d.sqrt();
            let w = weight[index] * near / (near + depth) * stale;
            // Off-screen bricks wait. On-screen bricks run even when the surface
            // is far, so a sun change still lands on the ground in view.
            let order = 3 * u32::from(w <= 0.0) + match class {
                0 => 0,
                REFINE_CLASS => 2,
                _ => 1,
            };
            // Weight buckets a factor of two apart, so the jittered counts do not
            // reorder bricks of about the same weight from one build to the next.
            let bucket = if w > 0.0 { w.log2().floor() as i32 } else { i32::MIN };
            // Change passes before refining; among change passes, the furthest along.
            let key = if class >= REFINE_CLASS { u32::MAX } else { CHANGE_PASSES - class.min(CHANGE_PASSES) };
            due.push((order, bucket, key, d, slot.brick, index as u32));
        }
        due.sort_by(|a, b| {
            a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)).then(a.3.total_cmp(&b.3)).then(a.4.cmp(&b.4))
        });
        // A sun or the sky came or went. Every on-screen brick it reaches replaces its
        // light in this build, so the next picture is the new light. Off-screen bricks
        // wait until the camera sees them. A colour drift still uses the short passes
        // below, or the nearest brick would take the whole budget.
        let wide = budget.is_some()
            && due.iter().any(|&(_, _, _, _, _, index)| {
                weight[index as usize] > 0.0
                    && self.slots[index as usize].as_ref().is_some_and(|slot| slot.replace && slot.change_left > 0)
            });
        let spread = !wide && budget.is_some() && due.len() > 48;
        let mut change_rays: u32 = if spread { 32 } else { CHANGE_RAYS };
        let change_history: u32 = if spread { 4 } else { CHANGE_HISTORY };
        // Each brick in that order takes its next pass, as far as the budget goes: the
        // bricks that matter most, and those furthest along, run while the rest wait,
        // and one pass of many bricks keeps the GPU full (a round of a few bricks
        // leaves most of it idle). A pass that does not fit runs the share of its
        // probes that does, lowest first, and the batch ends there: no build passes its
        // budget by more than one probe's rays. What is left deepens the bricks picked,
        // in the same order: each takes every change pass it has left, as rounds of one
        // pass each, before the next takes more. With room for everything (a settling
        // draw) every brick runs all its passes together.
        // (slot, passes, weight, probes of the first pass)
        let mut items = Vec::new();
        let mut critical = 0;
        let mut spent = 0u64;
        let night_hop = self.night_left > 0;
        if night_hop {
            self.push_night_hop(&mut items, &mut spent);
        }
        let mut picked: Vec<(u32, u32, f32, u64)> = Vec::new();
        if wide && !night_hop {
            let mut chosen: Vec<(u32, u64, f32)> = Vec::new();
            let mut probes_n = 0u64;
            for &(_, _, _, _, _, index) in &due {
                let Some(slot) = self.slots[index as usize].as_ref() else { continue };
                if !slot.replace || slot.change_left == 0 {
                    continue;
                }
                // Off-screen bricks take the replace when a lamp comes back, so the
                // yard is the new light before the camera turns toward it.
                if weight[index as usize] <= 0.0 && slot.darken {
                    continue;
                }
                let left = slot.mask & !slot.done;
                if left == 0 {
                    continue;
                }
                probes_n += u64::from(left.count_ones());
                chosen.push((index, left, weight[index as usize]));
            }
            // A lamp that went out takes two extra passes so a dark face can
            // catch the bounce the floor already holds. A lamp that comes back
            // stays at four; six passes there left the hall short.
            let went_out = chosen.iter().any(|(index, _, _)| {
                self.slots[*index as usize].as_ref().is_some_and(|slot| slot.darken)
            });
            let flush_passes = if self.sun_wide || went_out { FLUSH_PASSES + 2 } else { FLUSH_PASSES };
            let pass_budget = FLUSH_RAY_BUDGET / u64::from(flush_passes);
            // A lamp that comes back uses the full ray set. Sixteen rays left the
            // box face short of the trace on some gathers. A lamp that goes out
            // stays on the short set: more rays there made the hall too bright.
            let least = if self.sun_wide || !went_out {
                FLUSH_RAYS_MAX
            } else {
                FLUSH_RAYS_MIN
            };
            let most = FLUSH_RAYS_MAX;
            let mut rays = if probes_n == 0 { most } else { pass_budget / probes_n.max(1) };
            rays = rays.clamp(least, most);
            change_rays = rays as u32;
            for (index, left, w) in chosen {
                spent += u64::from(left.count_ones()) * rays * u64::from(flush_passes);
                picked.push((index, flush_passes, w, left));
            }
        } else if !night_hop {
            for &(_, _, _, _, _, index) in &due {
                if spread && weight[index as usize] <= 0.0 {
                    break;
                }
                let slot = self.slots[index as usize].as_ref().expect("due slot");
                let rays = u64::from(match self.class(slot).expect("due class") {
                    0 => first,
                    REFINE_CLASS => REFINE_RAYS,
                    _ => change_rays,
                });
                let left = slot.mask & !slot.done;
                let mut probes = left;
                if let Some(limit) = budget {
                    let room = (limit.saturating_sub(spent) / rays) as u32;
                    let fit = if picked.is_empty() { room.max(1) } else { room };
                    if fit == 0 {
                        break;
                    }
                    probes = lowest_bits(left, fit);
                }
                spent += u64::from(probes.count_ones()) * rays;
                picked.push((index, 1, weight[index as usize], probes));
                if probes != left || picked.len() >= TIER_SLOT_CAP as usize {
                    break;
                }
            }
            let mut count = picked.len();
            for entry in &mut picked {
                let slot = self.slots[entry.0 as usize].as_ref().expect("due slot");
                let class = self.class(slot).expect("due class");
                // A brick deepens once this batch finishes its pass under way: the next
                // passes run all its probes.
                if entry.3 != slot.mask & !slot.done || class == 0 || class >= REFINE_CLASS {
                    continue;
                }
                let whole = u64::from(slot.mask.count_ones()) * u64::from(change_rays);
                let mut extra = if spread {
                    0
                } else {
                    slot.change_left.min(CHANGE_PASSES).saturating_sub(1)
                };
                extra = extra.min((TIER_SLOT_CAP as usize).saturating_sub(count) as u32);
                if let Some(limit) = budget {
                    extra = extra.min((limit.saturating_sub(spent) / whole.max(1)) as u32);
                }
                entry.1 += extra;
                spent += u64::from(extra) * whole;
                count += extra as usize;
            }
        }
        if !night_hop {
            for &(index, passes, w, probes) in &picked {
                let slot = self.slots[index as usize].as_ref().expect("due slot");
                let class = self.class(slot).expect("due class");
                let (rays, reset, history) = match class {
                    0 => (first, true, None),
                    REFINE_CLASS => (REFINE_RAYS, false, None),
                    _ if slot.replace && slot.darken => (change_rays, false, Some(0)),
                    _ if slot.replace => (change_rays, false, Some(1)),
                    // The first pass after a move replaces the stale light. Blending the
                    // old value left the new box face short of the ray trace.
                    _ if slot.change_left == CHANGE_PASSES => (CHANGE_RAYS, false, Some(3)),
                    _ => (change_rays, false, Some(change_history)),
                };
                if w > 0.0 && class < REFINE_CLASS {
                    critical += 1;
                }
                for round in 0..passes {
                    let probes = if round == 0 { probes } else { slot.mask };
                    let history = if history == Some(0) && round >= 4 {
                        // The floor is already the lamp-off light. Later passes
                        // are for a face that is still dark, such as the box.
                        Some(8)
                    } else if history == Some(3) && round > 0 {
                        // Only the first pass after a move replaces. Later passes blend.
                        Some(change_history)
                    } else {
                        history
                    };
                    items.push(TierItem {
                        slot: index,
                        brick: slot.brick,
                        rays,
                        reset,
                        probes,
                        round,
                        history,
                    });
                }
            }
        }
        items.truncate(TIER_SLOT_CAP as usize);
        // Round by round; inside a round, the order picked above.
        items.sort_by_key(|item| item.round);
        let mut rounds = Vec::new();
        for item in &items {
            if rounds.len() <= item.round as usize {
                rounds.resize(item.round as usize + 1, 0);
            }
            rounds[item.round as usize] += 1;
        }
        let id = self.next_batch;
        self.next_batch += 1;
        let texels = self.texels(&items, id, eye);
        let used_slots = self
            .slots
            .iter()
            .rposition(|s| s.is_some())
            .map(|i| i as u32 + 1)
            .unwrap_or(0);
        self.critical = critical;
        // A forced flush spends more than a frame budget. Teaching that time
        // as the ray price shrinks later frames, and the outside shadow then
        // stops short of the ray trace.
        self.long_build = night_hop || wide;
        self.last_budget = if self.long_build { Some(u64::MAX) } else { budget };
        self.batch_us = started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
        TierBatch { id, texels, items, used_slots, probe_rays: spent, rounds }
    }

    /// One sun-gone pass over every filled brick. The first pass drops the old
    /// bounce. Each pass after reads that result and adds one bounce. The ray
    /// trace stops at six bounces, so this does too.
    fn push_night_hop(&mut self, items: &mut Vec<TierItem>, spent: &mut u64) {
        let dropping = self.night_left == NIGHT_HOPS;
        // The window follows a far camera, so most probes are empty ground.
        // The night bounce that reaches the yard lives on the building and the
        // ground beside it. Spending the rays there is what makes six bounces
        // fit in the tenth of a second.
        let hull = building_hull(&self.boxes);
        let span = self.layout.brick_span();
        let mut probes_n = 0u64;
        let mut chosen = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(slot) = slot.as_ref() else { continue };
            if !slot.filled || slot.mask == 0 {
                continue;
            }
            if !dropping {
                if let Some(hull) = hull.as_ref() {
                    let center = [0, 1, 2].map(|i| (slot.brick[i] as f32 + 0.5) * span);
                    let near = (0..3).all(|i| center[i] >= hull.min[i] - 8.0 && center[i] <= hull.max[i] + 8.0);
                    if !near {
                        continue;
                    }
                }
            }
            probes_n += u64::from(slot.mask.count_ones());
            chosen.push(index as u32);
        }
        // The drop only has to erase the sun. Later passes spend the rays on
        // the building, where the lamp bounce is.
        let (least, most) = if dropping { (16u64, 32u64) } else { (64, 256) };
        let rays = if probes_n == 0 {
            most as u32
        } else {
            (NIGHT_RAY_BUDGET / probes_n).clamp(least, most) as u32
        };
        // 7: the last pass lets a dark floor probe keep most of a lit floor
        // probe two metres away. The doorway light dies in that gap otherwise.
        let history = if dropping {
            Some(2)
        } else if self.night_left == 1 {
            Some(7)
        } else {
            Some(1)
        };
        // The batch holds 1024 bricks. Ground at the yards goes first so the
        // open yard and the shadow are not the ones cut off.
        chosen.sort_by_key(|index| {
            let brick = self.slots[*index as usize].as_ref().expect("night brick").brick;
            (brick[1], -(brick[0].abs() + brick[2].abs()))
        });
        for index in chosen {
            let slot = self.slots[index as usize].as_ref().expect("night brick");
            *spent += u64::from(slot.mask.count_ones()) * u64::from(rays);
            items.push(TierItem {
                slot: index,
                brick: slot.brick,
                rays,
                reset: false,
                probes: slot.mask,
                round: 0,
                history,
            });
        }
        self.night_left = self.night_left.saturating_sub(1);
        if self.night_left == 0 {
            self.night_freeze = true;
        }
    }

    fn texels(&self, items: &[TierItem], id: u64, eye: [f32; 3]) -> Vec<[f32; 4]> {
        let count = (TIER_WORK - TIER_INFO) as usize + items.len() * WORK_TEXELS as usize;
        let mut out = vec![[0.0f32; 4]; count];
        let Some((lo, hi)) = self.window else {
            return out;
        };
        let dims = [hi[0] - lo[0] + 1, hi[1] - lo[1] + 1, hi[2] - lo[2] + 1];
        let cells = (dims[0] * dims[1] * dims[2]) as u32;
        if cells > TIER_INDIR_CAP {
            // The window does not fit the indirection table: no tier this build.
            return out;
        }
        out[0] = [lo[0] as f32, lo[1] as f32, lo[2] as f32, self.layout.spacing];
        out[1] = [dims[0] as f32, dims[1] as f32, dims[2] as f32, self.fine_depth];
        let indir = (TIER_INDIR - TIER_INFO) as usize;
        for texel in &mut out[indir..indir + cells as usize] {
            *texel = [-1.0, 0.0, 0.0, 0.0];
        }
        // Bricks whose first pass this batch finishes: the picture reads them from now.
        let lit_now: std::collections::HashSet<u32> = items
            .iter()
            .filter(|i| {
                i.reset
                    && self.slots[i.slot as usize].as_ref().is_some_and(|s| (s.done | i.probes) & s.mask == s.mask)
            })
            .map(|i| i.slot)
            .collect();
        let slots = (TIER_SLOTS - TIER_INFO) as usize;
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(slot) = slot else { continue };
            let b = slot.brick;
            // w: 0 while the brick's update is under way, so the picture keeps blending
            // to the last whole one and never takes a value part way through. 1 + s
            // once it is whole: the picture takes it, unless it changes the light by
            // less than s (a far brick skips small steps).
            let show = if self.whole_after(index as u32, slot, items) {
                let center = [0, 1, 2].map(|i| (b[i] as f32 + 0.5) * self.layout.brick_span());
                let d = (0..3).map(|i| (center[i] - eye[i]).powi(2)).sum::<f32>().sqrt();
                let far = self.weights.far.max(1.0e-3);
                1.0 + self.weights.far_skip.max(0.0) * ((d - far) / far).clamp(0.0, 1.0)
            } else {
                0.0
            };
            out[slots + index * 2] = [b[0] as f32, b[1] as f32, b[2] as f32, show];
            out[slots + index * 2 + 1] = [0, 1, 2, 3].map(|k| ((slot.mask >> (16 * k)) & 0xffff) as f32);
            // The picture reads a brick once a build has lit all of it.
            if slot.filled || lit_now.contains(&(index as u32)) {
                let rel = [b[0] - lo[0], b[1] - lo[1], b[2] - lo[2]];
                let cell = ((rel[1] * dims[2] + rel[2]) * dims[0] + rel[0]) as usize;
                out[indir + cell] = [index as f32, 1.0, 0.0, 0.0];
            }
        }
        let work = (TIER_WORK - TIER_INFO) as usize;
        for (k, item) in items.iter().enumerate() {
            let at = work + k * WORK_TEXELS as usize;
            // z: 0 averages into the history, 1 replaces it, 2 + h keeps h rays of it.
            let mode = match (item.reset, item.history) {
                (true, _) => 1.0,
                (false, Some(h)) => 2.0 + h as f32,
                (false, None) => 0.0,
            };
            // Every pass of a sun change turns the ray set. One set misses a
            // narrow opening on every hop, so the shadow stays dark. Other
            // passes share the build seed: the history blend still damps a
            // moving lamp.
            let spin = if self.sun_wide { u64::from(item.round) } else { 0 };
            let seed = ((id + spin) % 4096) as f32;
            out[at] = [item.slot as f32, item.rays as f32, mode, seed];
            if let Some(Some(slot)) = self.slots.get(item.slot as usize) {
                // w = 1: the probe runs in this item. The pass's first share also takes
                // the dead probes, which a first pass clears.
                let dead = if slot.done == 0 { !slot.mask } else { 0 };
                for (i, p) in slot.positions.iter().enumerate() {
                    let runs = (item.probes | dead) >> i & 1 != 0;
                    out[at + 1 + i] = [p[0], p[1], p[2], if runs { 1.0 } else { 0.0 }];
                }
            }
        }
        out
    }

    /// Whether `slot`'s (at `index`) light is a whole update once `items` run: no pass
    /// part way through, and a change finished in them, refining reached
    /// [`TARGET_SAMPLES`], or [`CHANGE_PASSES`] change passes landed since the picture
    /// last took it (so a brick that changes all the time still updates). The
    /// bookkeeping of [`TierState::commit`], without changing anything.
    fn whole_after(&self, index: u32, slot: &Slot, items: &[TierItem]) -> bool {
        let (mut left, mut done, mut samples, mut run) = (slot.change_left, slot.done, slot.samples, slot.run);
        let mut changed = false;
        for item in items.iter().filter(|item| item.slot == index) {
            done |= item.probes & slot.mask;
            if done != slot.mask {
                continue;
            }
            done = 0;
            if item.reset {
                left = CHANGE_PASSES;
                samples = item.rays;
            } else if let Some(history) = item.history {
                left = left.saturating_sub(1);
                run += 1;
                samples = (samples.min(history) + item.rays).min(TARGET_SAMPLES);
                changed |= left == 0;
            } else {
                samples = (samples + item.rays).min(TARGET_SAMPLES);
            }
        }
        done == 0 && (run >= CHANGE_PASSES || left == 0 && (changed || samples >= TARGET_SAMPLES))
    }

    /// The build that carries `batch` has started. Its light counts from now on.
    pub fn commit(&mut self, batch: &TierBatch) {
        // The bricks the picture takes from this build: their update counts from now.
        let mut wholes: Vec<u32> = batch.items.iter().map(|item| item.slot).collect();
        wholes.sort_unstable();
        wholes.dedup();
        wholes.retain(|&index| {
            self.slots.get(index as usize).and_then(Option::as_ref).is_some_and(|slot| {
                batch.items.iter().any(|i| i.slot == index && i.brick == slot.brick)
                    && self.whole_after(index, slot, &batch.items)
            })
        });
        for item in &batch.items {
            let Some(Some(slot)) = self.slots.get_mut(item.slot as usize) else { continue };
            if slot.brick != item.brick {
                continue;
            }
            slot.done |= item.probes & slot.mask;
            if slot.done != slot.mask {
                // A share of the pass: it counts once every probe has run it.
                continue;
            }
            slot.done = 0;
            if item.reset {
                // A new brick converges its bounces like a change does.
                slot.filled = true;
                slot.samples = item.rays;
                slot.change_left = CHANGE_PASSES;
            } else if let Some(history) = item.history {
                // The shader keeps at most `history` of the stored rays.
                slot.change_left = slot.change_left.saturating_sub(1);
                if slot.change_left == 0 {
                    slot.replace = false;
                    slot.darken = false;
                }
                slot.run += 1;
                slot.samples = (slot.samples.min(history) + item.rays).min(TARGET_SAMPLES);
            } else {
                slot.samples = (slot.samples + item.rays).min(TARGET_SAMPLES);
            }
        }
        for index in wholes {
            if let Some(Some(slot)) = self.slots.get_mut(index as usize) {
                slot.run = 0;
                slot.since = std::time::Instant::now();
            }
        }
        if self.night_freeze {
            self.night_freeze = false;
            for slot in self.slots.iter_mut().flatten() {
                slot.samples = TARGET_SAMPLES;
                slot.change_left = 0;
                slot.replace = false;
                slot.darken = false;
            }
        }
    }
}

/// The lowest `n` set bits of `bits`.
fn lowest_bits(bits: u64, n: u32) -> u64 {
    let mut out = 0u64;
    let mut rest = bits;
    for _ in 0..n.min(bits.count_ones()) {
        let low = rest & rest.wrapping_neg();
        out |= low;
        rest &= !low;
    }
    out
}

fn boxes_key(boxes: &[SurfaceBox]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    boxes.len().hash(&mut hasher);
    for b in boxes {
        for v in b.min.iter().chain(b.max.iter()) {
            v.to_bits().hash(&mut hasher);
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_air_allocates_nothing() {
        let layout = TierLayout::new(1.0, 50.0);
        let set = allocate(&[], [0.0, 1.7, 0.0], &layout);
        assert!(set.bricks.is_empty());
        // The whole 50 m on each side is covered, so the snapped window spans 26 bricks.
        assert_eq!(set.window_bricks, 26 * 26 * 26);
    }

    #[test]
    fn a_floor_allocates_one_layer_of_bricks() {
        let layout = TierLayout::new(1.0, 50.0);
        let floor = SurfaceBox { min: [-8.0, 0.0, -8.0], max: [8.0, 0.0, 8.0] };
        let set = allocate(&[floor], [0.0, 1.7, 0.0], &layout);
        // Probes at y 0.5 and 1.5 are within sqrt(3) m. Nothing is below the ground.
        let ys: std::collections::BTreeSet<i32> = set.bricks.iter().map(|b| b[1]).collect();
        assert_eq!(ys.into_iter().collect::<Vec<_>>(), vec![0]);
        assert!(set.memory_bytes() < 2 * 1024 * 1024);
    }

    #[test]
    fn a_solid_interior_is_dead() {
        let layout = TierLayout::new(1.0, 50.0);
        let block = SurfaceBox { min: [-10.0, -10.0, -10.0], max: [10.0, 10.0, 10.0] };
        assert!(!probe_live(&[block], [0.5, 0.5, 0.5], layout.reach));
        assert!(probe_live(&[block], [10.5, 0.5, 0.5], layout.reach));
        let set = allocate(&[block], [0.0, 0.0, 0.0], &layout);
        assert!(set.bricks.iter().all(|b| b.iter().any(|&c| c <= -3 || c >= 2)));
    }

    #[test]
    fn a_probe_on_a_face_is_dead() {
        // A wall whose face lies on the lattice plane x = 1.5.
        let wall = SurfaceBox { min: [1.5, 0.0, -4.0], max: [1.9, 5.0, 4.0] };
        assert!(!probe_live(&[wall], [1.5, 0.5, 0.5], 1.7));
        assert!(!probe_live(&[wall], [1.495, 0.5, 0.5], 1.7));
        assert!(probe_live(&[wall], [1.48, 0.5, 0.5], 1.7));
        // Two crossing walls: inside either one is dead, the open corner is live.
        let cross = SurfaceBox { min: [-0.3, 0.0, 1.8], max: [3.7, 5.0, 2.2] };
        assert!(!probe_live(&[wall, cross], [1.7, 0.5, 2.0], 1.7));
        assert!(probe_live(&[wall, cross], [1.0, 0.5, 1.5], 1.7));
    }

    #[test]
    fn a_probe_inside_a_solid_moves_out_beside_it() {
        let layout = TierLayout::new(1.0, 50.0);
        let floor = SurfaceBox { min: [-8.0, 0.0, -8.0], max: [8.0, 0.0, 8.0] };
        let block = SurfaceBox { min: [-0.75, 0.0, -0.75], max: [0.75, 1.2, 0.75] };
        let boxes = [floor, block];
        // Equally near two sides: out through both, past the corner.
        let q = place_probe(&boxes, [0.5, 0.5, -0.5], &layout).expect("moved out");
        assert!((q[0] - 0.85).abs() < 1.0e-5 && (q[2] + 0.85).abs() < 1.0e-5 && q[1] == 0.5);
        // Nearer one side: out through that side only.
        let q = place_probe(&boxes, [0.5, 0.5, -0.6], &layout).expect("moved out");
        assert!((q[2] + 0.85).abs() < 1.0e-5 && q[0] == 0.5);
        // The middle of a wide solid is too far from any side to leave its cell.
        let big = SurfaceBox { min: [-3.0, 0.0, -3.0], max: [3.0, 1.2, 3.0] };
        assert!(place_probe(&[floor, big], [0.5, 0.5, 0.5], &layout).is_none());
        // A probe on the lattice outside every solid stays put.
        assert_eq!(place_probe(&boxes, [1.5, 0.5, 1.5], &layout), Some([1.5, 0.5, 1.5]));
    }

    #[test]
    fn moving_the_camera_keeps_probe_positions() {
        let layout = TierLayout::new(1.0, 50.0);
        let floor = SurfaceBox { min: [-80.0, 0.0, -80.0], max: [80.0, 0.0, 80.0] };
        let a = allocate(&[floor], [0.0, 1.7, 0.0], &layout);
        let b = allocate(&[floor], [1.3, 1.7, 0.0], &layout);
        let shared = a.bricks.iter().filter(|x| b.bricks.contains(x)).count();
        assert!(shared >= a.bricks.len() - 25 * 2, "a small move keeps nearly every brick");
    }

    /// A wide hall: a floor 40 m across and one wall, and a 1 m crate at `x`.
    fn hall(x: f32) -> Vec<SurfaceBox> {
        vec![
            SurfaceBox { min: [-20.0, 0.0, -20.0], max: [20.0, 0.0, 20.0] },
            SurfaceBox { min: [-20.0, 0.0, -20.2], max: [20.0, 3.0, -20.0] },
            SurfaceBox { min: [x - 0.5, 0.0, -0.5], max: [x + 0.5, 1.0, 0.5] },
        ]
    }

    fn settle(tier: &mut TierState, eye: [f32; 3]) {
        while tier.has_work() {
            let batch = tier.batch(eye, None, REFINE_RAYS);
            tier.commit(&batch);
        }
    }

    #[test]
    fn a_moved_solid_relights_only_the_bricks_it_reaches() {
        let eye = [0.0, 1.7, 4.0];
        let lamp = TierLight { pos: [0.0, 2.5, 3.0], color: [1.0; 3], directional: false };
        let mut tier = TierState::default();
        tier.update(hall(0.0), 0, eye, &[lamp]);
        settle(&mut tier, eye);
        let bricks = tier.stats().bricks;
        tier.update(hall(0.6), 0, eye, &[lamp]);
        let stats = tier.stats();
        // Every brick stays readable: none goes back to waiting for its first light.
        assert_eq!(stats.filled_bricks, stats.bricks);
        assert!(stats.changing_bricks > 0);
        assert!(stats.changing_bricks * 4 < bricks, "{} of {bricks} bricks relight", stats.changing_bricks);
        // The bricks around the crate relight first; a far corner of the hall does not.
        let batch = tier.batch(eye, Some(u64::from(CHANGE_RAYS)), REFINE_RAYS);
        let near = batch.items[0].brick.map(|b| (b as f32 + 0.5) * tier.layout.brick_span());
        assert!(near[0].abs() < 5.0 && near[2].abs() < 5.0, "{near:?}");
        let far = tier.by_brick[&[4, 0, 4]] as usize;
        assert_eq!(tier.slots[far].as_ref().unwrap().change_left, 0);
        // Placing only near the crate gives the probes a whole placement gives.
        let whole = allocate(&hall(0.6), eye, &tier.layout);
        for ((brick, mask), positions) in whole.bricks.iter().zip(&whole.masks).zip(&whole.positions) {
            let slot = tier.slots[tier.by_brick[brick] as usize].as_ref().unwrap();
            assert_eq!((slot.mask, &slot.positions), (*mask, positions), "{brick:?}");
        }
        assert_eq!(whole.bricks.len(), tier.stats().bricks);
    }

    #[test]
    fn the_view_takes_the_budget_before_hidden_bricks() {
        // A hall split by a wall at x = 0. The camera stands at x = -6 and looks away
        // from the wall, down -x: the bricks behind it and past the wall are hidden,
        // many of them nearer than the far end of the view.
        let mut boxes = hall(30.0);
        boxes.push(SurfaceBox { min: [0.0, 0.0, -20.0], max: [0.2, 3.0, 20.0] });
        let eye = [-6.0, 1.7, 0.0];
        let camera = TierCamera {
            eye,
            right: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            forward: [-1.0, 0.0, 0.0],
            tan_x: 0.7,
            tan_y: 0.4,
        };
        let lamp = TierLight { pos: [3.0, 2.5, 0.0], color: [1.0; 3], directional: false };
        let mut tier = TierState::default();
        tier.update(boxes.clone(), 0, eye, &[lamp]);
        settle(&mut tier, eye);
        for _ in 0..16 {
            tier.look(&camera);
        }
        let seen = tier.stats().seen_bricks;
        assert!(seen > 0 && seen * 3 < tier.stats().bricks, "{seen} of {} bricks seen", tier.stats().bricks);
        // Every brick in view lies ahead of the camera, on its side of the wall.
        for (index, slot) in tier.slots.iter().enumerate() {
            let slot = slot.as_ref().unwrap();
            if tier.in_view(index) {
                assert!(slot.brick[0] * BRICK <= -2, "{:?} is behind the camera", slot.brick);
            }
        }
        // The lamp moves; a budget of a few passes goes to the bricks the picture uses
        // (in view, at its edge, or feeding it), none behind the wall.
        let moved = TierLight { pos: [3.0, 2.5, 1.0], ..lamp };
        tier.update(boxes, 0, eye, &[moved]);
        let budget = 4 * u64::from(BRICK_PROBES) * u64::from(CHANGE_RAYS);
        let batch = tier.batch_seen(eye, Some(&camera), Some(budget), FIRST_RAYS);
        assert!(!batch.items.is_empty());
        let weight = tier.weights();
        for item in &batch.items {
            assert!(weight[item.slot as usize] > 0.0, "{:?} is not used by the picture", item.brick);
            assert!(item.brick[0] * BRICK < 0, "{:?} is behind the wall", item.brick);
        }
    }

    #[test]
    fn an_outside_camera_leaves_interior_motion_out_of_the_probes() {
        let floor = SurfaceBox { min: [-8.0, 0.0, -8.0], max: [8.0, 0.0, 8.0] };
        let wall = SurfaceBox { min: [-6.0, 0.0, -6.0], max: [6.0, 3.0, -5.6] };
        let east = SurfaceBox { min: [5.6, 0.0, -6.0], max: [6.0, 3.0, 6.0] };
        let west = SurfaceBox { min: [-6.0, 0.0, -6.0], max: [-5.6, 3.0, 6.0] };
        let south = SurfaceBox { min: [-6.0, 0.0, 5.6], max: [6.0, 3.0, 6.0] };
        let mut crate_at = SurfaceBox { min: [-0.5, 0.0, -0.5], max: [0.5, 1.0, 0.5] };
        let lamp = TierLight { pos: [0.0, 2.2, 0.0], color: [1.0; 3], directional: false };
        let sun = TierLight { pos: [0.2, -0.6, -0.8], color: [1.0; 3], directional: true };
        let boxes = vec![floor, wall, east, west, south, crate_at];
        let far_eye = [24.0, 18.0, 24.0];
        let mut tier = TierState::default();
        tier.update(boxes.clone(), 1, far_eye, &[lamp, sun]);
        while tier.has_work() {
            let batch = tier.batch(far_eye, None, REFINE_RAYS);
            tier.commit(&batch);
        }
        assert_eq!(tier.stats().changing_bricks, 0);
        crate_at.min[0] += 0.4;
        crate_at.max[0] += 0.4;
        let mut moved_boxes = boxes.clone();
        moved_boxes[5] = crate_at;
        let before = tier.stats().interior_skipped;
        tier.update(moved_boxes.clone(), 1, far_eye, &[lamp, sun]);
        assert!(tier.stats().interior_skipped > before, "interior move was not skipped");
        assert_eq!(tier.stats().changing_bricks, 0, "interior move relit probes from outside");
        // The same kind of move from inside the room does relight.
        let eye = [0.0, 1.6, 0.0];
        let mut inside = TierState::default();
        inside.update(boxes.clone(), 1, eye, &[lamp, sun]);
        while inside.has_work() {
            let batch = inside.batch(eye, None, REFINE_RAYS);
            inside.commit(&batch);
        }
        inside.update(moved_boxes, 1, eye, &[lamp, sun]);
        assert!(inside.stats().changing_bricks > 0, "an interior camera ignored the crate");
    }

    #[test]
    fn a_lamp_shadow_of_a_moved_solid_reaches_far_bricks() {
        // A low lamp beside the crate throws its shadow far across the floor.
        let lamp = TierLight { pos: [-2.0, 0.6, 0.0], color: [4.0; 3], directional: false };
        let moved = [hall(0.0)[2], hall(0.6)[2]];
        let span = 4.0;
        let behind = ([12.0, 0.0, -2.0], [12.0 + span, span, -2.0 + span]);
        let aside = ([-2.0, 0.0, 12.0], [-2.0 + span, span, 12.0 + span]);
        assert!(geometry_reaches(&moved, &[lamp], behind.0, behind.1));
        assert!(!geometry_reaches(&moved, &[lamp], aside.0, aside.1));
    }

    fn room() -> Vec<SurfaceBox> {
        vec![
            SurfaceBox { min: [-5.0, 0.0, -5.0], max: [5.0, 0.0, 5.0] },
            SurfaceBox { min: [-5.0, 0.0, -5.2], max: [5.0, 3.0, -5.0] },
        ]
    }

    #[test]
    fn bricks_on_screen_take_every_change_pass_in_one_build() {
        let eye = [0.0, 1.7, 0.0];
        let lamp = TierLight { pos: [0.0, 2.5, 0.0], color: [1.0; 3], directional: false };
        let mut tier = TierState::default();
        tier.update(room(), 0, eye, &[lamp]);
        while tier.has_work() {
            let batch = tier.batch(eye, None, REFINE_RAYS);
            tier.commit(&batch);
        }
        let moved = TierLight { pos: [1.0, 2.0, -1.0], ..lamp };
        tier.update(room(), 0, eye, &[moved]);
        // Looking along -z at the wall: the bricks behind the eye are off screen.
        let camera = TierCamera {
            eye,
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            tan_x: 0.75,
            tan_y: 0.58,
        };
        let changing: Vec<u32> = (0..tier.slots.len() as u32)
            .filter(|&i| tier.slots[i as usize].as_ref().is_some_and(|s| s.filled && s.change_left > 0))
            .collect();
        let pass = |i: u32| u64::from(tier.slots[i as usize].as_ref().unwrap().mask.count_ones()) * u64::from(CHANGE_RAYS);
        let one_each: u64 = changing.iter().map(|&i| pass(i)).sum();
        let biggest = changing.iter().map(|&i| pass(i)).max().unwrap();
        // A budget of a few passes goes to bricks the picture uses, none to the others.
        let few = tier.batch_seen(eye, Some(&camera), Some(biggest * 3), FIRST_RAYS);
        let weight = tier.weights();
        assert!(few.items.iter().all(|item| weight[item.slot as usize] > 0.0));
        // Room for a pass each and the rest of the passes of about one brick.
        let budget = one_each + biggest * u64::from(CHANGE_PASSES - 1);
        let batch = tier.batch_seen(eye, Some(&camera), Some(budget), FIRST_RAYS);
        assert_eq!(batch.rounds.len(), CHANGE_PASSES as usize);
        assert_eq!(batch.rounds.iter().sum::<u32>() as usize, batch.items.len());
        assert!(batch.items.windows(2).all(|w| w[0].round <= w[1].round));
        let mut passes = std::collections::HashMap::new();
        for item in &batch.items {
            *passes.entry(item.slot).or_insert(0u32) += 1;
        }
        assert_eq!(passes.len(), changing.len(), "every changing brick takes a pass");
        // The rest of the budget finishes the bricks the picture uses most first.
        let deep: Vec<u32> = passes.iter().filter(|(_, &n)| n == CHANGE_PASSES).map(|(&s, _)| s).collect();
        assert!(!deep.is_empty());
        for slot in &deep {
            let brick = tier.slots[*slot as usize].as_ref().unwrap().brick;
            assert!(weight[*slot as usize] > 0.0, "a brick out of view went deep: {brick:?}");
        }
        assert!(tier.stats().critical_bricks > 0);
        tier.commit(&batch);
        assert_eq!(tier.stats().changing_bricks, changing.len() - deep.len());

        // With room for everything, every changing brick runs all its passes together.
        tier.update(room(), 0, eye, &[lamp]);
        let all = tier.batch_seen(eye, Some(&camera), None, FIRST_RAYS);
        tier.commit(&all);
        assert_eq!(tier.stats().changing_bricks, 0);
        // A tiny budget still runs one brick's pass.
        tier.update(room(), 0, eye, &[moved]);
        let small = tier.batch_seen(eye, Some(&camera), Some(1), FIRST_RAYS);
        assert_eq!(small.items.len(), 1);
    }

    #[test]
    fn the_picture_takes_a_brick_only_once_its_change_is_whole() {
        let eye = [0.0, 1.7, 0.0];
        let lamp = TierLight { pos: [0.0, 2.5, 0.0], color: [1.0; 3], directional: false };
        let mut tier = TierState::default();
        tier.update(room(), 0, eye, &[lamp]);
        while tier.has_work() {
            let batch = tier.batch(eye, None, REFINE_RAYS);
            tier.commit(&batch);
        }
        tier.update(room(), 0, eye, &[TierLight { pos: [1.0, 2.0, -1.0], ..lamp }]);
        let shown = |batch: &TierBatch, slot: u32| batch.texels[(TIER_SLOTS - TIER_INFO) as usize + slot as usize * 2][3];
        // One pass of one brick: under way, so the picture holds it.
        let one = tier.batch(eye, Some(u64::from(CHANGE_RAYS)), FIRST_RAYS);
        let slot = one.items[0].slot;
        assert_eq!(shown(&one, slot), 0.0);
        tier.commit(&one);
        // Every pass of every brick: all whole, the picture takes them.
        let all = tier.batch(eye, None, FIRST_RAYS);
        assert!(all.items.iter().all(|item| shown(&all, item.slot) >= 1.0));
    }

    #[test]
    fn tier_weights_parse_over_the_defaults() {
        let weights = TierWeights::parse("near=2, stale=1").unwrap();
        assert_eq!(weights, TierWeights { near: 2.0, stale: 1.0, ..TierWeights::default() });
        assert_eq!(TierWeights::parse(&TierWeights::default().to_string().replace(' ', ",")).unwrap(), TierWeights::default());
        assert!(TierWeights::parse("nearness=2").is_err());
        assert!(TierWeights::parse("near=x").is_err());
    }

    #[test]
    fn a_converged_tier_has_no_work() {
        let mut tier = TierState::default();
        let lights = [TierLight { pos: [0.0, 2.5, 0.0], color: [1.0; 3], directional: false }];
        tier.update(room(), 0, [0.0, 1.7, 0.0], &lights);
        assert!(tier.has_work());
        for _ in 0..64 {
            let batch = tier.batch([0.0, 1.7, 0.0], None, REFINE_RAYS);
            assert!(batch.items.iter().all(|i| i.reset) || batch.items.iter().all(|i| !i.reset));
            if batch.items.is_empty() {
                break;
            }
            tier.commit(&batch);
        }
        assert!(!tier.has_work());
        // A small lamp move keeps the light. A real move blends every brick it reaches
        // toward the new light, all of them in each pass, and never clears one.
        let nudged = [TierLight { pos: [0.05, 2.5, 0.0], ..lights[0] }];
        assert!(!tier.update(room(), 0, [0.0, 1.7, 0.0], &nudged));
        let moved = [TierLight { pos: [1.0, 2.5, 0.0], ..lights[0] }];
        assert!(tier.update(room(), 0, [0.0, 1.7, 0.0], &moved));
        // One brick fits the budget, yet a pass still waits for every brick.
        let batch = tier.batch([0.0, 1.7, 0.0], Some(1), FIRST_RAYS);
        assert_eq!(batch.items.len(), 1);
        // Without a budget every brick takes all its change passes, one round each.
        let batch = tier.batch([0.0, 1.7, 0.0], None, FIRST_RAYS);
        assert_eq!(batch.items.len(), tier.stats().bricks * CHANGE_PASSES as usize);
        assert_eq!(batch.rounds, vec![tier.stats().bricks as u32; CHANGE_PASSES as usize]);
        assert!(batch.items.iter().all(|i| {
            !i.reset
                && if i.round == 0 { i.history == Some(3) } else { i.history == Some(CHANGE_HISTORY) }
        }));
        tier.commit(&batch);
        let batch = tier.batch([0.0, 1.7, 0.0], None, FIRST_RAYS);
        assert!(batch.items.iter().all(|i| !i.reset && i.history.is_none() && i.rays == REFINE_RAYS));
    }

    #[test]
    fn small_builds_do_not_shrink_the_budget() {
        let mut tier = TierState::default();
        assert_eq!(tier.budget_rays(1.5), u64::MAX);
        // A full build measures the GPU: 200k rays in 40 ms.
        tier.note_time(200_000, 40.0);
        let first = tier.budget_rays(1.5);
        assert_eq!(first, 7500);
        tier.last_budget = Some(first);
        // Builds with little work cannot fill the GPU: their rays look dear.
        for _ in 0..40 {
            tier.note_time(1024, 1.0);
        }
        assert_eq!(tier.budget_rays(1.5), first);
        // A budget of one probe: each build is all of it, but one probe is the latency
        // of a workgroup and a wait for the queue, so it does not shrink the budget.
        tier.last_budget = Some(u64::from(CHANGE_RAYS));
        for _ in 0..40 {
            tier.note_time(u64::from(CHANGE_RAYS), 28.0);
        }
        assert_eq!(tier.budget_rays(1.5), first);
        // A fast GPU: the budget grows to fit 1.5 ms.
        let mut fast = TierState::default();
        fast.note_time(300_000, 0.6);
        assert!(fast.budget_rays(1.5) >= 700_000);
    }

    #[test]
    fn the_fine_shell_follows_the_gpu_budget() {
        let mut tier = TierState::default();
        assert!((tier.fine_depth() - 16.0).abs() < 1.0e-4);
        tier.fit_depth(8.0, 1.5);
        assert!(tier.fine_depth() < 16.0);
        let pulled = tier.fine_depth();
        tier.fit_depth(0.4, 1.5);
        assert!(tier.fine_depth() > pulled);
        // A full build priced several times too cheap closes half the gap.
        let mut cost = TierState::default();
        cost.note_time(200_000, 40.0);
        cost.last_budget = Some(200_000);
        cost.note_time(200_000, 200.0);
        assert!(cost.budget_rays(1.5) < 4_000);
    }

    #[test]
    fn a_pass_larger_than_the_budget_runs_in_shares() {
        let eye = [0.0, 1.7, 0.0];
        let lamp = TierLight { pos: [0.0, 2.5, 0.0], color: [1.0; 3], directional: false };
        let mut tier = TierState::default();
        tier.update(room(), 0, eye, &[lamp]);
        // A budget of ten probes' first rays: under a brick of the floor.
        let budget = 10 * u64::from(FIRST_RAYS);
        let mut builds = 0;
        while tier.stats().filled_bricks < tier.stats().bricks {
            let batch = tier.batch(eye, Some(budget), FIRST_RAYS);
            assert!(batch.probe_rays <= budget, "{} rays over {budget}", batch.probe_rays);
            assert!(!batch.items.is_empty());
            // A brick shows once all its probes have their first light.
            let indir = (TIER_INDIR - TIER_INFO) as usize;
            let readable = batch.texels[indir..indir + 26 * 26 * 26].iter().filter(|t| t[0] >= 0.0).count();
            tier.commit(&batch);
            assert_eq!(readable, tier.stats().filled_bricks);
            builds += 1;
            assert!(builds < 10_000);
        }
        let live = tier.stats().live_probes as u64;
        assert!(builds as u64 >= live / 10, "{builds} builds for {live} probes");
        // Change passes too: a brick's pass runs in shares, and every brick converges.
        let moved = TierLight { pos: [1.0, 2.5, 0.0], ..lamp };
        tier.update(room(), 0, eye, &[moved]);
        let budget = 10 * u64::from(CHANGE_RAYS);
        let changing = tier.stats().changing_bricks;
        let mut passes = 0;
        while tier.stats().changing_bricks > 0 {
            let batch = tier.batch(eye, Some(budget), FIRST_RAYS);
            assert!(batch.probe_rays <= budget);
            tier.commit(&batch);
            passes += 1;
            assert!(passes < 100_000);
        }
        assert!(passes > changing);
    }

    #[test]
    fn builds_slowed_by_the_picture_do_not_shrink_the_budget() {
        let mut tier = TierState::default();
        tier.note_time(200_000, 40.0);
        let clean = tier.budget_rays(1.5);
        // Most builds run beside the picture and take three times as long; some run alone.
        // Each slower build moves the cost a little and the next one alone resets it, so
        // the budget dips between them and never drifts down.
        let mut lows = Vec::new();
        for k in 0..80 {
            let ms = if k % 4 == 0 { 1.5 } else { 4.5 };
            tier.note_time(clean, ms);
            if k % 4 == 3 {
                lows.push(tier.budget_rays(1.5));
            }
        }
        assert!(lows.iter().all(|&low| low * 10 >= clean * 7), "{lows:?} of {clean}");
        assert_eq!(lows.first(), lows.last());
        // A GPU that really got slower (full builds take twice as long) is followed.
        let mut slow = TierState::default();
        slow.note_time(200_000, 40.0);
        for _ in 0..80 {
            slow.note_time(200_000, 80.0);
        }
        let slower = slow.budget_rays(1.5);
        assert!(slower * 10 <= clean * 6, "{slower} of {clean}");
    }

    #[test]
    fn a_dim_lamp_far_away_leaves_the_tier_alone() {
        let mut tier = TierState::default();
        let lamp = TierLight { pos: [0.0, 2.5, 0.0], color: [1.0; 3], directional: false };
        let candle = TierLight { pos: [400.0, 1.0, 0.0], color: [0.001; 3], directional: false };
        tier.update(room(), 0, [0.0, 1.7, 0.0], &[lamp, candle]);
        while tier.has_work() {
            let batch = tier.batch([0.0, 1.7, 0.0], None, REFINE_RAYS);
            tier.commit(&batch);
        }
        let moved = TierLight { pos: [401.0, 1.0, 0.0], ..candle };
        tier.update(room(), 0, [0.0, 1.7, 0.0], &[lamp, moved]);
        assert!(!tier.has_work());
    }

    #[test]
    fn a_budget_takes_the_nearest_bricks_first() {
        let mut tier = TierState::default();
        tier.update(room(), 0, [4.5, 1.7, 4.5], &[]);
        let batch = tier.batch([4.5, 1.7, 4.5], Some(1), FIRST_RAYS);
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].brick, [1, 0, 1]);
        // The budget holds one probe's rays: the brick takes a share of its first pass.
        assert_eq!(batch.items[0].probes.count_ones(), 1);
        // Only bricks lit all through are readable.
        let indir = (TIER_INDIR - TIER_INFO) as usize;
        let readable = |batch: &TierBatch| batch.texels[indir..indir + 26 * 26 * 26].iter().filter(|t| t[0] >= 0.0).count();
        assert_eq!(readable(&batch), 0);
        let slot = tier.by_brick[&[1, 0, 1]];
        let live = tier.slots[slot as usize].as_ref().unwrap().mask.count_ones();
        let whole = tier.batch([4.5, 1.7, 4.5], Some(u64::from(FIRST_RAYS * live)), FIRST_RAYS);
        assert_eq!(whole.items[0].brick, [1, 0, 1]);
        assert_eq!(readable(&whole), 1);
    }

    #[test]
    fn a_walk_keeps_slots() {
        let mut tier = TierState::default();
        tier.update(room(), 0, [0.0, 1.7, 0.0], &[]);
        let before = tier.by_brick.clone();
        tier.update(room(), 0, [4.1, 1.7, 0.0], &[]);
        for (brick, slot) in &tier.by_brick {
            if let Some(old) = before.get(brick) {
                assert_eq!(old, slot);
            }
        }
    }
}
