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
/// Field texels per probe: three ambient cubes of six RGBA32F faces. The first holds
/// light that bounced up to three times (the picture reads it; alpha counts samples),
/// then up to twice, then once (hits read those to build the next order). The engine
/// stops at three bounces like the reference path tracer: with white walls of albedo 1
/// a closed room has no finite answer for unlimited bounces.
pub const PROBE_TEXELS: u32 = 18;
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
            min: [wall.position.x - wall.half_x, 0.0, wall.position.z - wall.half_z],
            max: [wall.position.x + wall.half_x, wall.height, wall.position.z + wall.half_z],
        });
    }
    for solid in &scene.solids {
        let half = solid.size * 0.5;
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
        let lo = eye.map(|e| ((e - self.radius) / span).floor() as i32);
        let hi = eye.map(|e| ((e + self.radius) / span).ceil() as i32 - 1);
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

/// True when the probe at `p` is outside every box and within `reach` of a surface.
/// Nothing stands below the ground, so a probe under it is dead.
pub fn probe_live(boxes: &[SurfaceBox], p: [f32; 3], reach: f32) -> bool {
    if p[1] < 0.0 {
        return false;
    }
    let mut near = false;
    for b in boxes {
        if b.inside(p) {
            return false;
        }
        if !near && b.surface_distance(p) <= reach {
            near = true;
        }
    }
    near
}

/// Allocate the bricks of the window around `eye` that hold a live probe.
pub fn allocate(boxes: &[SurfaceBox], eye: [f32; 3], layout: &TierLayout) -> BrickSet {
    let (lo, hi) = layout.window(eye);
    let span = layout.brick_span();
    let mut set = BrickSet::default();
    for by in lo[1]..=hi[1] {
        for bz in lo[2]..=hi[2] {
            for bx in lo[0]..=hi[0] {
                set.window_bricks += 1;
                let brick = [bx, by, bz];
                let min = brick.map(|b| b as f32 * span - layout.reach);
                let max = brick.map(|b| (b + 1) as f32 * span + layout.reach);
                let nearby: Vec<SurfaceBox> =
                    boxes.iter().copied().filter(|b| b.overlaps(min, max)).collect();
                if nearby.is_empty() {
                    continue;
                }
                let mut mask = 0u64;
                for ly in 0..BRICK {
                    for lz in 0..BRICK {
                        for lx in 0..BRICK {
                            let p = layout.probe_position(brick, [lx, ly, lz]);
                            if probe_live(&nearby, p, layout.reach) {
                                mask |= 1 << ((ly * BRICK + lz) * BRICK + lx);
                            }
                        }
                    }
                }
                if mask != 0 {
                    set.bricks.push(brick);
                    set.masks.push(mask);
                    set.live_probes += mask.count_ones() as usize;
                }
            }
        }
    }
    set
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
/// Room reserved for the slot table and the work list.
const TIER_TABLE_ROOM: u32 = 2048;
/// One texel per work item: slot, rays, 1 = start over, pass seed.
pub const TIER_WORK: u32 = TIER_SLOTS + TIER_TABLE_ROOM * 2;
/// Probe cubes, `PROBE_TEXELS` per probe and `BRICK_PROBES` per slot.
pub const TIER_PROBES: u32 = TIER_INFO + 40960;
/// End of the tier, the size of the light field in texels.
pub const TIER_END: u32 = TIER_PROBES + TIER_SLOT_CAP * BRICK_PROBES * PROBE_TEXELS;

const _: () = assert!(TIER_SLOT_CAP <= TIER_TABLE_ROOM && TIER_WORK + TIER_TABLE_ROOM <= TIER_PROBES);

/// Rays a probe takes on its first pass and after a change: quick and noisy.
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
}

#[derive(Clone, Debug)]
struct Slot {
    brick: [i32; 3],
    mask: u64,
    /// Lit at least once, so the picture may read it.
    filled: bool,
    samples: u32,
    /// Light generation the stored light belongs to.
    clean_gen: u64,
    /// Passes that started over for `clean_gen`. Bounce order k reads order k - 1 at
    /// the hits, so after a change the third such pass is the first with no light
    /// left from before; until then a pass replaces instead of averaging.
    restarts: u32,
}

/// Passes that start over after a change, one per bounce order.
pub const RESTART_PASSES: u32 = 3;

/// One brick of work in a build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TierItem {
    pub slot: u32,
    pub brick: [i32; 3],
    pub rays: u32,
    pub reset: bool,
    pub gen: u64,
    pub live: u32,
    /// For a reset: restarts this slot had done for `gen` when the item was issued.
    pub stage: u32,
}

/// Everything one light build needs from the tier.
#[derive(Clone, Debug, Default)]
pub struct TierBatch {
    pub id: u64,
    /// Texels from `TIER_INFO` up to `TIER_WORK` plus one per item.
    pub texels: Vec<[f32; 4]>,
    pub items: Vec<TierItem>,
    /// Slots below this index may hold light. The build copies them forward.
    pub used_slots: u32,
    /// Probe rays this batch casts (live probes times rays).
    pub probe_rays: u64,
}

/// Counts for reports.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TierStats {
    pub window_bricks: usize,
    pub bricks: usize,
    pub live_probes: usize,
    pub filled_bricks: usize,
    pub pending_bricks: usize,
    pub dropped_bricks: usize,
}

/// The persistent tier on the CPU: which brick lives in which slot, what each slot
/// still needs, and how much of that fits in one build.
#[derive(Clone, Debug)]
pub struct TierState {
    pub layout: TierLayout,
    geometry: Option<u64>,
    boxes: Vec<SurfaceBox>,
    window: Option<([i32; 3], [i32; 3])>,
    slots: Vec<Option<Slot>>,
    by_brick: std::collections::HashMap<[i32; 3], u32>,
    free: Vec<u32>,
    lights: Vec<TierLight>,
    light_gen: u64,
    next_batch: u64,
    dropped: usize,
    live: usize,
    /// GPU milliseconds per probe ray, learned from the last builds.
    ms_per_ray: f64,
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
            boxes: Vec::new(),
            window: None,
            slots: Vec::new(),
            by_brick: std::collections::HashMap::new(),
            free: Vec::new(),
            lights: Vec::new(),
            light_gen: 0,
            next_batch: 1,
            dropped: 0,
            live: 0,
            // A first guess (lavapipe, a room). The first timed build replaces it.
            ms_per_ray: 2.0e-4,
        }
    }

    /// Light generation. It moves on when a light or the geometry changes.
    pub fn light_gen(&self) -> u64 {
        self.light_gen
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
        let geometry = boxes_key(&boxes) ^ materials.rotate_left(17);
        let mut realloc = false;
        if self.geometry != Some(geometry) {
            self.geometry = Some(geometry);
            self.boxes = boxes;
            realloc = true;
            self.light_gen += 1;
            changed = true;
        }
        let window = self.layout.window(eye);
        if self.window != Some(window) {
            self.window = Some(window);
            realloc = true;
        }
        if realloc {
            changed |= self.reallocate(eye);
        }
        let relight = lights.len() != self.lights.len()
            || lights.iter().zip(&self.lights).any(|(new, old)| new.moved_from(old));
        if relight {
            self.lights = lights.to_vec();
            self.light_gen += 1;
            changed = true;
        }
        changed
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
        for (brick, mask) in set.bricks.iter().zip(&set.masks) {
            if let Some(&slot) = self.by_brick.get(brick) {
                let entry = self.slots[slot as usize].as_mut().expect("mapped slot");
                if entry.mask != *mask {
                    // A probe came alive or died: light the whole brick again.
                    entry.mask = *mask;
                    entry.filled = false;
                    entry.samples = 0;
                    entry.restarts = 0;
                    changed = true;
                }
                continue;
            }
            let slot = match self.free.pop() {
                Some(slot) => slot,
                None => {
                    self.slots.push(None);
                    (self.slots.len() - 1) as u32
                }
            };
            self.slots[slot as usize] =
                Some(Slot {
                brick: *brick,
                mask: *mask,
                filled: false,
                samples: 0,
                clean_gen: 0,
                restarts: 0,
            });
            self.by_brick.insert(*brick, slot);
            changed = true;
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
            ..TierStats::default()
        };
        for slot in self.slots.iter().flatten() {
            stats.bricks += 1;
            stats.live_probes += slot.mask.count_ones() as usize;
            if slot.filled {
                stats.filled_bricks += 1;
            }
            if self.class(slot).is_some() {
                stats.pending_bricks += 1;
            }
        }
        stats
    }

    /// Restarts done for the current light, or none.
    fn restarts(&self, slot: &Slot) -> u32 {
        if slot.filled && slot.clean_gen == self.light_gen {
            slot.restarts
        } else {
            0
        }
    }

    /// 0 to 2: restart passes still due (the number done so far). 3: refining.
    fn class(&self, slot: &Slot) -> Option<u32> {
        let restarts = self.restarts(slot);
        if restarts < RESTART_PASSES {
            Some(restarts)
        } else if slot.samples < TARGET_SAMPLES {
            Some(RESTART_PASSES)
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
                    let local = [bit % BRICK, bit / (BRICK * BRICK), (bit / BRICK) % BRICK];
                    out.push(self.layout.probe_position(slot.brick, local));
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

    /// Probe rays that fit in `ms` of GPU time.
    pub fn budget_rays(&self, ms: f64) -> u64 {
        (ms / self.ms_per_ray.max(1.0e-9)).max(1.0) as u64
    }

    /// Learn the cost of a probe ray from a timed build.
    pub fn note_time(&mut self, probe_rays: u64, ms: f64) {
        if probe_rays < 256 || !(ms > 0.0) {
            return;
        }
        let sample = ms / probe_rays as f64;
        self.ms_per_ray = self.ms_per_ray * 0.5 + sample * 0.5;
    }

    pub fn ms_per_ray(&self) -> f64 {
        self.ms_per_ray
    }

    /// The next build's work, up to `budget` probe rays (`None`: everything that is
    /// due). Every brick's first restart comes before any brick's second, and all
    /// restarts before refining; nearest first inside each. `first` is the ray count
    /// of a restart pass.
    pub fn batch(&mut self, eye: [f32; 3], budget: Option<u64>, first: u32) -> TierBatch {
        let span = self.layout.brick_span();
        let mut due: Vec<(u32, f32, [i32; 3], u32)> = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(slot) = slot else { continue };
            let Some(class) = self.class(slot) else { continue };
            let d: f32 =
                (0..3).map(|i| ((slot.brick[i] as f32 + 0.5) * span - eye[i]).powi(2)).sum();
            due.push((class, d, slot.brick, index as u32));
        }
        due.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut items = Vec::new();
        let mut spent = 0u64;
        for (class, _, brick, index) in due {
            let slot = self.slots[index as usize].as_ref().expect("due slot");
            let live = slot.mask.count_ones();
            let (rays, reset) =
                if class == RESTART_PASSES { (REFINE_RAYS, false) } else { (first, true) };
            let cost = u64::from(live) * u64::from(rays);
            if let Some(limit) = budget {
                if !items.is_empty() && spent + cost > limit {
                    break;
                }
            }
            spent += cost;
            items.push(TierItem {
                slot: index,
                brick,
                rays,
                reset,
                gen: self.light_gen,
                live,
                stage: class.min(RESTART_PASSES),
            });
            if items.len() >= TIER_SLOT_CAP as usize {
                break;
            }
        }
        let id = self.next_batch;
        self.next_batch += 1;
        let texels = self.texels(&items, id);
        let used_slots = self
            .slots
            .iter()
            .rposition(|s| s.is_some())
            .map(|i| i as u32 + 1)
            .unwrap_or(0);
        TierBatch { id, texels, items, used_slots, probe_rays: spent }
    }

    fn texels(&self, items: &[TierItem], id: u64) -> Vec<[f32; 4]> {
        let count = (TIER_WORK - TIER_INFO) as usize + items.len();
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
        out[1] = [dims[0] as f32, dims[1] as f32, dims[2] as f32, self.layout.radius];
        let indir = (TIER_INDIR - TIER_INFO) as usize;
        for texel in &mut out[indir..indir + cells as usize] {
            *texel = [-1.0, 0.0, 0.0, 0.0];
        }
        let in_batch: std::collections::HashSet<u32> = items.iter().map(|i| i.slot).collect();
        let slots = (TIER_SLOTS - TIER_INFO) as usize;
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(slot) = slot else { continue };
            let b = slot.brick;
            out[slots + index * 2] = [b[0] as f32, b[1] as f32, b[2] as f32, 1.0];
            out[slots + index * 2 + 1] = [0, 1, 2, 3].map(|k| ((slot.mask >> (16 * k)) & 0xffff) as f32);
            // The picture reads a brick once a build has lit it.
            if slot.filled || in_batch.contains(&(index as u32)) {
                let rel = [b[0] - lo[0], b[1] - lo[1], b[2] - lo[2]];
                let cell = ((rel[1] * dims[2] + rel[2]) * dims[0] + rel[0]) as usize;
                out[indir + cell] = [index as f32, 1.0, 0.0, 0.0];
            }
        }
        let work = (TIER_WORK - TIER_INFO) as usize;
        let seed = (id % 4096) as f32;
        for (k, item) in items.iter().enumerate() {
            out[work + k] = [item.slot as f32, item.rays as f32, if item.reset { 1.0 } else { 0.0 }, seed];
        }
        out
    }

    /// The build that carries `batch` has started. Its light counts from now on.
    pub fn commit(&mut self, batch: &TierBatch) {
        for item in &batch.items {
            let Some(Some(slot)) = self.slots.get_mut(item.slot as usize) else { continue };
            if slot.brick != item.brick {
                continue;
            }
            if item.reset {
                if slot.filled && slot.clean_gen == item.gen {
                    slot.restarts = slot.restarts.max(item.stage + 1);
                } else {
                    slot.restarts = 1;
                }
                slot.filled = true;
                slot.samples = item.rays;
                slot.clean_gen = item.gen;
            } else {
                slot.samples = (slot.samples + item.rays).min(TARGET_SAMPLES);
            }
        }
    }
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
    fn moving_the_camera_keeps_probe_positions() {
        let layout = TierLayout::new(1.0, 50.0);
        let floor = SurfaceBox { min: [-80.0, 0.0, -80.0], max: [80.0, 0.0, 80.0] };
        let a = allocate(&[floor], [0.0, 1.7, 0.0], &layout);
        let b = allocate(&[floor], [1.3, 1.7, 0.0], &layout);
        let shared = a.bricks.iter().filter(|x| b.bricks.contains(x)).count();
        assert!(shared >= a.bricks.len() - 25 * 2, "a small move keeps nearly every brick");
    }

    fn room() -> Vec<SurfaceBox> {
        vec![
            SurfaceBox { min: [-5.0, 0.0, -5.0], max: [5.0, 0.0, 5.0] },
            SurfaceBox { min: [-5.0, 0.0, -5.2], max: [5.0, 3.0, -5.0] },
        ]
    }

    #[test]
    fn a_converged_tier_has_no_work() {
        let mut tier = TierState::default();
        let lights = [TierLight { pos: [0.0, 2.5, 0.0], color: [1.0; 3], directional: false }];
        tier.update(room(), 0, [0.0, 1.7, 0.0], &lights);
        assert!(tier.has_work());
        for _ in 0..16 {
            let batch = tier.batch([0.0, 1.7, 0.0], None, REFINE_RAYS);
            assert!(batch.items.iter().all(|i| i.reset) || batch.items.iter().all(|i| !i.reset));
            if batch.items.is_empty() {
                break;
            }
            tier.commit(&batch);
        }
        assert!(!tier.has_work());
        // A small lamp move keeps the light. A real move relights every brick.
        let nudged = [TierLight { pos: [0.05, 2.5, 0.0], ..lights[0] }];
        assert!(!tier.update(room(), 0, [0.0, 1.7, 0.0], &nudged));
        let moved = [TierLight { pos: [1.0, 2.5, 0.0], ..lights[0] }];
        assert!(tier.update(room(), 0, [0.0, 1.7, 0.0], &moved));
        let batch = tier.batch([0.0, 1.7, 0.0], None, FIRST_RAYS);
        assert!(batch.items.iter().all(|i| i.reset && i.rays == FIRST_RAYS));
        assert_eq!(batch.items.len(), tier.stats().bricks);
    }

    #[test]
    fn a_budget_takes_the_nearest_bricks_first() {
        let mut tier = TierState::default();
        tier.update(room(), 0, [4.5, 1.7, 4.5], &[]);
        let batch = tier.batch([4.5, 1.7, 4.5], Some(1), FIRST_RAYS);
        assert_eq!(batch.items.len(), 1);
        assert_eq!(batch.items[0].brick, [1, 0, 1]);
        // Only lit bricks are readable.
        let indir = (TIER_INDIR - TIER_INFO) as usize;
        let readable = batch.texels[indir..indir + 26 * 26 * 26].iter().filter(|t| t[0] >= 0.0).count();
        assert_eq!(readable, 1);
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
