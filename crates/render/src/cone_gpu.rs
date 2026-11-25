// The cone-traced cascades on the GPU (experimental, lighting key
// `cone_cascades`). Included into gpu.rs, as lighting_gpu.rs is: the passes
// use the lighting descriptor set, its layout and its push constant. The
// field (`cone_field.rs`) is built on a thread when the world or its glTF
// model changes or the eye leaves its clip volumes, uploaded over a few
// frames, and then every frame the passes in `shaders/cone_*.comp` run before
// compose, the light pass (`cone_light.comp`) first.

/// Words a coarse texel holds on the GPU (`cone_common.glsl` CONE_TEXEL): the
/// six ways' and the lamps' light and area, occupancy, seat, albedo, each
/// way's surface along its axis, and each way's sun share (kept by the light
/// pass; 0 here).
const CONE_TEXEL_WORDS: usize = 23;
/// Bytes of the field uploaded each frame.
const CONE_UPLOAD_STEP: usize = 64 << 20;
/// The first interval's reach in cascade-0 probe spacings.
const CONE_T0: f32 = 2.5;
/// Push flags: the merge and the shade weigh each probe by what passes to it.
const CONE_SEE_MERGE: u32 = 1;
const CONE_SEE_SHADE: u32 = 2;
/// Push flag, knockout only: seat probes on their pixels' mean
/// ([`crate::gate::CONE_FIELD_SEAT`]).
const CONE_PIXEL_SEAT: u32 = 4;
/// Knockout only: the share of the screen's probes the tables are sized for
/// ([`crate::gate::CONE_TABLE_CAP`]).
const CONE_KNOCKOUT_SHARE: u64 = 16;
/// Work-buffer layout, as `cone_common.glsl` has it.
const CONE_HEADER_WORDS: u64 = 256;
const CONE_HASH_WORDS: u64 = 10;
const CONE_PROBE_WORDS: u64 = 32;
const CONE_CONE_WORDS: u64 = 4;
/// Cascade 0's probes for each `probe_px` square of the screen. The voxel
/// layer with each pixel's eight neighbours measured 2.8 in the hall (floors
/// seen at a grazing angle add to it), so 6 leaves room; gate B fails if it
/// does not.
const CONE_PROBE_SLACK: u64 = 6;
/// Each cascade holds this share of the probes of the one below (measured
/// 1/3.1 with the neighbours spread: probes on a surface, at twice the spacing).
const CONE_CASCADE_SHARE: u64 = 3;
const CONE_MIN_CAP: u64 = 64;
/// Cascade-0 room for the world layer's probes (`cone_world.comp`): one a
/// surface face at 2 m within 8 m of the eye and 4 m within 16 m;
/// Sponza has about 7,500 such faces on screen or off.
const CONE_WORLD_CAP: u64 = 16_384;
/// Header words cleared each frame; the layout follows them.
const CONE_CLEARED: u64 = 64;
const CONE_LAYOUT_AT: u64 = 64;
const CONE_PIPES: usize = 11;
const CONE_PIPE_NAMES: [&str; CONE_PIPES] = [
    "cone-insert",
    "cone-compact",
    "cone-chain",
    "cone-trace",
    "cone-weights",
    "cone-merge",
    "cone-shade",
    "cone-lamps",
    "cone-spread",
    "cone-light",
    "cone-world",
];
/// Header words of last frame's eye and whether its probes are in the work
/// buffer (`cone_common.glsl` CONE_PREV_EYE), for the light pass.
const CONE_PREV_EYE_AT: u64 = 92;
/// The bounce's short history (Anthony, 25 Nov 2026): each frame a voxel's
/// light moves `1 - exp(-dt / CONE_BOUNCE_SECONDS)` of the way to what last
/// frame's probes and this frame's suns give it, so a bounce settles in a few
/// tenths of a second whatever the frame rate. The direct light does not wait.
const CONE_BOUNCE_SECONDS: f32 = 0.1;
/// Threads the light pass runs a fine brick and a coarse brick.
const CONE_BRICK_VOXELS: u64 = 512;
const CONE_PADDED_TEXELS: u64 = 1000;
/// Points on each lamp's sphere (`cone_lamps.comp`), and the field levels it
/// is written at: the most lamp-hash entries one lamp can fill.
const CONE_LAMP_POINTS: u64 = 256;
const CONE_LAMP_ENTRIES: u64 = CONE_LAMP_POINTS * 8;
const CONE_LAMP_WORDS: u64 = 5;

fn cone_dirs(theta: u32, c: u32) -> u64 {
    let t = u64::from(theta) << c;
    2 * t * t
}
/// Cascade sizes and word offsets in the work buffer, from the voxel layer's
/// cap: so many probes for the screen, each hash at most half full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ConeLayout {
    hash: [u64; 5],
    cap: [u64; 5],
    hash_at: [u64; 5],
    probes_at: [u64; 5],
    cones_at: [u64; 5],
    dirs: [u64; 5],
    count: u32,
    /// The lamp hash: entries (a power of two) and word offset.
    lamp_hash: u64,
    lamp_at: u64,
    words: u64,
}

fn cone_layout(theta: u32, count: u32, pixels: u64, probe_px: u32, lamps: u64) -> ConeLayout {
    let px = u64::from(probe_px.max(1));
    let base = pixels.div_ceil(px * px).max(1) * CONE_PROBE_SLACK;
    let count = count.min(5);
    let mut l = ConeLayout { count, ..ConeLayout::default() };
    let mut at = CONE_HEADER_WORDS;
    let mut cap = base + CONE_WORLD_CAP;
    for c in 0..count as usize {
        let k = u32::try_from(c).unwrap_or(0);
        l.cap[c] = cap.max(CONE_MIN_CAP);
        l.hash[c] = (2 * l.cap[c]).next_power_of_two();
        l.dirs[c] = cone_dirs(theta, k);
        l.hash_at[c] = at;
        at += l.hash[c] * CONE_HASH_WORDS;
        l.probes_at[c] = at;
        at += l.cap[c] * CONE_PROBE_WORDS;
        l.cones_at[c] = at;
        at += l.cap[c] * l.dirs[c] * CONE_CONE_WORDS;
        cap = cap.div_ceil(CONE_CASCADE_SHARE);
    }
    l.lamp_hash = (2 * lamps.max(1) * CONE_LAMP_ENTRIES).next_power_of_two();
    l.lamp_at = at;
    at += l.lamp_hash * CONE_LAMP_WORDS;
    l.words = at;
    l
}

impl ConeLayout {
    /// The header words the shaders read it from (`cone_common.glsl`).
    fn header(&self) -> Vec<u32> {
        let w = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
        (0..5)
            .flat_map(|c| [w(self.hash[c]), w(self.cap[c]), w(self.hash_at[c]), w(self.probes_at[c]), w(self.cones_at[c])])
            .chain([w(self.lamp_hash), w(self.lamp_at)])
            .collect()
    }
}

/// The renderer's cone-cascade state.
struct ConeGpu {
    pipes: [Handle; CONE_PIPES],
    table: Buffer,
    fine: Buffer,
    records: Buffer,
    coarse: Buffer,
    work: Buffer,
    /// The world, model, lamps and lamp radius the field was built for.
    key: u64,
    building: Option<(u64, [f32; 3], std::thread::JoinHandle<ConeBuilt>)>,
    /// The eye the resident field's clip volumes are about.
    clip_eye: [f32; 3],
    /// The resident field has its fine levels (not the coarse levels alone).
    has_fine: bool,
    /// Bytes still to upload: (buffer 0..3, bytes, cursor).
    pending: Vec<(usize, Vec<u8>, usize)>,
    /// A field built while another is resident (the clip volumes about a
    /// moved eye): its four buffers fill here while the resident field stays
    /// bound, and take its place before a frame once they are full.
    next: Vec<Buffer>,
    next_full: bool,
    /// Fine and coarse bricks of the field in use, and of the one filling.
    bricks: (u64, u64),
    next_bricks: (u64, u64),
    /// This frame's eye, and last frame's when its probes are still in the
    /// work buffer (the light pass reads their irradiance).
    eye: [f32; 3],
    prev_eye: Option<[f32; 3]>,
    /// When the light pass last ran (its blend steps by real frame time), and
    /// whether the field in use is new (its voxels take their light at once).
    lit_at: Option<std::time::Instant>,
    fresh: bool,
    /// Frames recorded: which share of the world layer is traced this frame.
    frames: u32,
    ready: bool,
    /// The work buffer's shape (theta, cascades, pixels, probe_px, lamps) and layout.
    work_shape: (u32, u32, u64, u32, u64),
    /// Knockout only ([`crate::gate::CONE_LAMP_FOLLOW`]): the lamps were
    /// written once and are kept.
    lamps_frozen: bool,
    /// Lights in the scene, suns included: the lamp pass's lamp count.
    scene_lights: u32,
    layout: ConeLayout,
    /// The last build's sizes, for the debug tools.
    report: String,
    /// Each solid's pose last frame, and the solids that moved within
    /// [`CONE_MOVER_SECONDS`]: the field leaves them out.
    solid_poses: Vec<[f32; 5]>,
    movers: std::collections::BTreeMap<usize, std::time::Instant>,
    /// The solids left out of the field in use, and of the one building.
    field_movers: std::collections::BTreeSet<usize>,
    building_movers: std::collections::BTreeSet<usize>,
}

impl ConeGpu {
    fn new() -> Self {
        Self {
            pipes: [std::ptr::null_mut(); CONE_PIPES],
            table: Buffer::empty(),
            fine: Buffer::empty(),
            records: Buffer::empty(),
            coarse: Buffer::empty(),
            work: Buffer::empty(),
            key: 0,
            building: None,
            clip_eye: [0.0; 3],
            has_fine: false,
            pending: Vec::new(),
            next: Vec::new(),
            next_full: false,
            bricks: (0, 0),
            next_bricks: (0, 0),
            eye: [0.0; 3],
            prev_eye: None,
            lit_at: None,
            fresh: true,
            frames: 0,
            ready: false,
            work_shape: (0, 0, 0, 0, 0),
            lamps_frozen: false,
            scene_lights: 0,
            layout: ConeLayout::default(),
            report: String::new(),
            solid_poses: Vec::new(),
            movers: std::collections::BTreeMap::new(),
            field_movers: std::collections::BTreeSet::new(),
            building_movers: std::collections::BTreeSet::new(),
        }
    }
}

/// What a cone-cascade field depends on: the world and its glTF model. The
/// lamps are written each frame (`cone_lamps.comp`), so a lamp that moves or
/// changes does not rebuild it.
fn cone_world_key(world: &World) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    world.identity().hash(&mut h);
    world.gltf_surface.as_ref().map(|g| g.key()).hash(&mut h);
    h.finish()
}

/// The scene's point lamps (a sun is not a sphere in the field).
fn cone_point_lamps(scene: &genos_scene::Scene) -> u64 {
    (0..scene.light_len())
        .filter(|&i| {
            let d = scene.light(i).direction;
            d.x == 0.0 && d.y == 0.0 && d.z == 0.0
        })
        .count() as u64
}

fn cone_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The field's four buffers as bytes: the table (head, levels and clip
/// volumes first), the
/// fine headers, the fine records, and the coarse texels
/// ([`CONE_TEXEL_WORDS`] each).
fn cone_field_bytes(field: &crate::cone_field::ConeField) -> [Vec<u8>; 4] {
    let mut table: Vec<u32> = vec![field.atlas[0], field.atlas[1], field.atlas[2], field.coarse_bricks];
    for level in &field.levels {
        table.extend(level.origin.map(|v| u32::from_ne_bytes(v.to_ne_bytes())));
        table.push(level.offset);
        table.extend(level.dims);
        table.push(0);
    }
    for level in &field.levels {
        table.extend(level.clip.map(|v| u32::from_ne_bytes(v.to_ne_bytes())));
    }
    // info: where the brick origins start in `table`, and the brick counts.
    let entries = u32::try_from(field.table.len()).unwrap_or(u32::MAX);
    let fine_count = u32::try_from(field.fine_origins.len()).unwrap_or(u32::MAX);
    let coarse_count = u32::try_from(field.coarse_origins.len()).unwrap_or(u32::MAX);
    table.extend([entries, entries + 4 * fine_count, fine_count, coarse_count]);
    table.extend_from_slice(&field.table);
    for origin in field.fine_origins.iter().chain(&field.coarse_origins) {
        table.extend(origin.map(|v| u32::from_ne_bytes(v.to_ne_bytes())));
    }
    let texels = field.occupancy.len();
    let w = CONE_TEXEL_WORDS;
    let mut coarse = vec![0u32; texels * w];
    for i in 0..texels {
        for d in 0..7 {
            let f = &field.faces[d][i * 4..i * 4 + 4];
            coarse[i * w + 2 * d] = u32::from(f[0]) | (u32::from(f[1]) << 16);
            coarse[i * w + 2 * d + 1] = u32::from(f[2]) | (u32::from(f[3]) << 16);
        }
        coarse[i * w + 14] = u32::from(field.occupancy[i] != 0);
        coarse[i * w + 15] = field.coarse_seats[i];
        let a = &field.coarse_albedo[i * 3..i * 3 + 3];
        coarse[i * w + 16] = u32::from(a[0]) | (u32::from(a[1]) << 16);
        coarse[i * w + 17] = u32::from(a[2]);
        let f = &field.coarse_planes[i * 6..i * 6 + 6];
        for k in 0..3 {
            coarse[i * w + 18 + k] = u32::from(f[2 * k]) | (u32::from(f[2 * k + 1]) << 16);
        }
    }
    [cone_bytes(&table), cone_bytes(&field.fine_bricks), cone_bytes(&field.fine_voxels), cone_bytes(&coarse)]
}

/// The field for a scene around the eye, built off the frame thread: its
/// surfaces, the fine levels in clip volumes about `eye`. The lamps are not
/// in it; they are written each frame.
/// Seconds a solid stays a mover after it last moved.
const CONE_MOVER_SECONDS: f32 = 2.0;

/// The scene without the solids `drop` names.
fn cone_without_solids(scene: &genos_scene::Scene, drop: &std::collections::BTreeSet<usize>) -> genos_scene::Scene {
    if drop.is_empty() {
        return scene.clone();
    }
    let walls = scene.walls().cloned().collect();
    let solids = scene.solids().cloned().enumerate().filter(|(i, _)| !drop.contains(i)).map(|(_, s)| s).collect();
    scene.replace_parts(walls, solids, scene.lights_cloned())
}

fn cone_build(scene: &genos_scene::Scene, gltf: Option<&crate::gltf_surface::GltfSurface>, eye: [f32; 3], fine: bool) -> ConeBuilt {
    let started = std::time::Instant::now();
    let field = crate::cone_field::FieldVoxels::around_eye(scene, gltf, eye, fine).brick();
    let bytes = cone_field_bytes(&field);
    let report = format!(
        "{} points, {} voxels, {} fine bricks, {} fine records, {} coarse bricks, {} MiB, built in {:.2} s",
        field.points,
        field.voxels,
        field.fine_bricks.len() / crate::cone_field::FINE_HEADER,
        field.fine_voxels.len() / crate::cone_field::FINE_RECORD,
        field.coarse_bricks,
        bytes.iter().map(Vec::len).sum::<usize>() >> 20,
        started.elapsed().as_secs_f64()
    );
    let bricks = (u64::try_from(field.fine_origins.len()).unwrap_or(u64::MAX), u64::try_from(field.coarse_origins.len()).unwrap_or(u64::MAX));
    ConeBuilt { bytes, report: if fine { report } else { format!("coarse levels only: {report}") }, fine, bricks }
}

/// A built field as the GPU takes it, packed on the build thread.
struct ConeBuilt {
    bytes: [Vec<u8>; 4],
    report: String,
    /// False for the coarse levels alone ([`cone_build`]).
    fine: bool,
    /// Fine and coarse bricks: the light pass's threads.
    bricks: (u64, u64),
}

/// Whether the eye has moved far enough from the clip volumes' centre that
/// they are built again around it.
fn cone_clip_stale(built: [f32; 3], eye: [f32; 3]) -> bool {
    (0..3).any(|a| (built[a] - eye[a]).abs() > crate::cone_field::CLIP_MOVE)
}

/// Cone mode runs only the cascades: the old pipeline's direct pass, shadows,
/// GI, ambient fill, denoiser and compose reuse are off, and compose takes the
/// cascades' light as the surface's emitted light. The settings the user wrote stay as they are.
fn cone_effective(settings: &mut genos_scene::LightingSettings) {
    if !settings.cone.enabled {
        return;
    }
    settings.direct.enabled = false;
    settings.restir_di = false;
    settings.gi.enabled = false;
    settings.world_cache.enabled = false;
    settings.bounce.count = 0;
    settings.ambient.mode = genos_scene::AmbientMode::Off;
    settings.emissive = true;
    // Nothing carries over between frames in this mode: the denoiser's
    // history and compose reuse would hold an old frame's light.
    settings.denoiser.enabled = false;
    settings.compose_reuse = false;
}

impl Gpu {
    /// Build, upload and size the cone field for this world when cone mode is on.
    /// Note which solids moved since last frame; the solids that moved
    /// within [`CONE_MOVER_SECONDS`].
    fn note_cone_movers(&mut self, scene: &genos_scene::Scene) -> std::collections::BTreeSet<usize> {
        let now = std::time::Instant::now();
        let poses: Vec<[f32; 5]> = scene.solids().map(|s| [s.position.x, s.position.y, s.position.z, s.yaw, s.size + s.height]).collect();
        if poses.len() == self.cone.solid_poses.len() {
            for (i, (a, b)) in poses.iter().zip(&self.cone.solid_poses).enumerate() {
                if a != b {
                    self.cone.movers.insert(i, now);
                }
            }
        } else {
            self.cone.movers.clear();
        }
        self.cone.solid_poses = poses;
        self.cone.movers.retain(|_, at| now.duration_since(*at).as_secs_f32() < CONE_MOVER_SECONDS);
        self.cone.movers.keys().copied().collect()
    }

    fn sync_cone(&mut self, world: &World, eye: [f32; 3]) -> Result<(), String> {
        let cone = self.lighting.settings.cone;
        if !cone.enabled {
            return Ok(());
        }
        self.cone_swap_next()?;
        self.cone.eye = eye;
        let key = cone_world_key(world);
        // One build at a time: each holds a field, and a thread a frame once
        // ran the machine out of memory in a minute.
        // The fine levels are clip volumes about the eye: a new world, or an
        // eye that has left the middle of the volumes, builds them again
        // (the field in use stays until the new one is resident).
        // A new world first gets its coarse levels alone, which build in a
        // fraction of the time, so cone mode lights at once; the fine levels
        // follow in the next build.
        let new_world = self.cone.key != key;
        // A moving solid is left out of the field: the field is built on a
        // thread now and then, and held a mover where it stood then, so the
        // pixels on the mover started their cones inside its old copy and
        // went black (Anthony, 25 Nov 2026: the moving green box). A solid
        // that starts to move builds the field again without it.
        let movers = self.note_cone_movers(&world.scene);
        let mover_held = !movers.is_subset(&self.cone.field_movers);
        let stale = new_world || mover_held || !self.cone.has_fine || cone_clip_stale(self.cone.clip_eye, eye);
        if stale && self.cone.building.is_none() {
            let scene = cone_without_solids(&world.scene, &movers);
            self.cone.building_movers = movers;
            let gltf = world.gltf_surface.clone();
            let fine = !new_world;
            let handle = std::thread::Builder::new()
                .name("cone-field".into())
                .spawn(move || cone_build(&scene, gltf.as_deref(), eye, fine))
                .map_err(|e| format!("cone field thread: {e}"))?;
            self.cone.building = Some((key, eye, handle));
        }
        if self.cone.building.as_ref().is_some_and(|(_, _, h)| h.is_finished()) {
            if let Some((k, at, handle)) = self.cone.building.take() {
                let built = handle.join().map_err(|_| "cone field thread panicked".to_string())?;
                let fine = built.fine;
                self.cone_take_field(k, built)?;
                self.cone.clip_eye = at;
                self.cone.has_fine = fine;
                self.cone.field_movers = std::mem::take(&mut self.cone.building_movers);
            }
        }
        let pixels = u64::from(self.lighting.dims[0]) * u64::from(self.lighting.dims[1]);
        // Room for at least 16 lamps, so lamps added in play rarely resize it.
        let lamps = cone_point_lamps(&world.scene).max(16).next_power_of_two();
        self.cone.scene_lights = u32::try_from(world.scene.light_len()).unwrap_or(u32::MAX);
        let shape = (cone.theta, cone.count, pixels, cone.probe_px, lamps);
        if pixels > 0 && (self.cone.work_shape != shape || self.cone.work.buffer.is_null()) {
            let sized = if crate::gate::guard(crate::gate::CONE_TABLE_CAP) { pixels } else { pixels / CONE_KNOCKOUT_SHARE };
            let layout = cone_layout(cone.theta, cone.count, sized, cone.probe_px, lamps);
            let words = layout.words;
            self.idle_device();
            let mut old = std::mem::replace(&mut self.cone.work, Buffer::empty());
            self.destroy_buffer(&mut old);
            let share = self.light_families();
            self.cone.work = self.make_buffer_queues(words * 4, 0x20 | 0x2, Memory::Device, &share)?;
            self.cone.work_shape = shape;
            self.cone.layout = layout;
            self.cone.prev_eye = None;
            if self.lighting.ready {
                self.write_lighting_sets()?;
            }
        }
        if self.cone.pipes[0].is_null() && self.lighting.ready {
            let codes: [&[u8]; CONE_PIPES] = [CONE_INSERT_SPV, CONE_COMPACT_SPV, CONE_CHAIN_SPV, CONE_TRACE_SPV, CONE_WEIGHTS_SPV, CONE_MERGE_SPV, CONE_SHADE_SPV, CONE_LAMPS_SPV, CONE_SPREAD_SPV, CONE_LIGHT_SPV, CONE_WORLD_SPV];
            for (k, code) in codes.iter().enumerate() {
                self.cone.pipes[k] = self.make_compute_pipe(code, self.lighting.layout, CONE_PIPE_NAMES[k])?;
            }
        }
        Ok(())
    }

    /// A built field: new buffers, and its bytes queued for upload.
    fn cone_take_field(&mut self, key: u64, built: ConeBuilt) -> Result<(), String> {
        let ConeBuilt { bytes, report, bricks, .. } = built;
        self.cone.report = report;
        let share = self.light_families();
        if self.cone.ready && self.cone.next.is_empty() {
            // A field is resident and lighting: fill the new one beside it.
            for b in &bytes {
                let size = u64::try_from(b.len().max(16)).unwrap_or(16);
                let made = self.make_buffer_queues(size, 0x20 | 0x2, Memory::Device, &share)?;
                self.cone.next.push(made);
            }
            self.cone.pending = bytes.into_iter().enumerate().map(|(k, b)| (k, b, 0)).collect();
            self.cone.next_full = false;
            self.cone.next_bricks = bricks;
            self.cone.key = key;
            return Ok(());
        }
        self.idle_device();
        for (k, b) in bytes.iter().enumerate() {
            let target = match k {
                0 => &mut self.cone.table,
                1 => &mut self.cone.fine,
                2 => &mut self.cone.records,
                _ => &mut self.cone.coarse,
            };
            let mut old = std::mem::replace(target, Buffer::empty());
            self.destroy_buffer(&mut old);
            let size = u64::try_from(b.len().max(16)).unwrap_or(16);
            let made = self.make_buffer_queues(size, 0x20 | 0x2, Memory::Device, &share)?;
            match k {
                0 => self.cone.table = made,
                1 => self.cone.fine = made,
                2 => self.cone.records = made,
                _ => self.cone.coarse = made,
            }
        }
        self.cone.pending = bytes.into_iter().enumerate().map(|(k, b)| (k, b, 0)).collect();
        self.cone.bricks = bricks;
        self.cone.fresh = true;
        self.cone.ready = false;
        self.cone.lamps_frozen = false;
        self.cone.key = key;
        if self.lighting.ready {
            self.write_lighting_sets()?;
        }
        Ok(())
    }

    /// Record the next part of the field's upload into this frame.
    fn cone_upload(&mut self) -> Result<(), String> {
        if self.cone.pending.is_empty() {
            return Ok(());
        }
        let mut budget = CONE_UPLOAD_STEP;
        let cmd = self.cmd;
        let mut pending = std::mem::take(&mut self.cone.pending);
        for (k, bytes, cursor) in &mut pending {
            if budget == 0 {
                break;
            }
            let end = (*cursor + budget).min(bytes.len());
            if end > *cursor {
                let dst = match (self.cone.next.get(*k), *k) {
                    (Some(next), _) => next.buffer,
                    (None, 0) => self.cone.table.buffer,
                    (None, 1) => self.cone.fine.buffer,
                    (None, 2) => self.cone.records.buffer,
                    (None, _) => self.cone.coarse.buffer,
                };
                self.write_device(cmd, dst, u64::try_from(*cursor).unwrap_or(0), &bytes[*cursor..end])?;
                budget -= end - *cursor;
                *cursor = end;
            }
        }
        pending.retain(|(_, bytes, cursor)| *cursor < bytes.len());
        self.cone.pending = pending;
        if self.cone.pending.is_empty() {
            self.memory_barrier(0x1000, 0x800, 0x1000, 0x20);
            if self.cone.next.is_empty() {
                self.cone.ready = true;
            } else {
                self.cone.next_full = true;
            }
        }
        Ok(())
    }

    /// Put a filled next field in the resident one's place, before a frame
    /// is recorded (the lighting sets are written again).
    fn cone_swap_next(&mut self) -> Result<(), String> {
        if !self.cone.next_full {
            return Ok(());
        }
        self.idle_device();
        let mut next = std::mem::take(&mut self.cone.next);
        for (k, made) in next.drain(..).enumerate() {
            let target = match k {
                0 => &mut self.cone.table,
                1 => &mut self.cone.fine,
                2 => &mut self.cone.records,
                _ => &mut self.cone.coarse,
            };
            let mut old = std::mem::replace(target, made);
            self.destroy_buffer(&mut old);
        }
        self.cone.next_full = false;
        self.cone.bricks = self.cone.next_bricks;
        self.cone.fresh = true;
        self.cone.lamps_frozen = false;
        if self.lighting.ready {
            self.write_lighting_sets()?;
        }
        Ok(())
    }

    fn cone_dispatch(&self, pipe: usize, push: [u32; 12], groups: (u32, u32)) {
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, self.cone.pipes[pipe]);
            (self.fns.cmd_push)(self.cmd, self.lighting.layout, 0x20, 0, LIGHTING_PC_BYTES, push.as_ptr().cast::<c_void>());
            (self.fns.cmd_dispatch)(self.cmd, groups.0.max(1), groups.1.max(1), 1);
        }
    }

    /// The cascades for this frame, before compose: probes, chains, cones,
    /// merge and shade. Does nothing until the field is resident.
    fn record_cone(&mut self) {
        let cone = self.lighting.settings.cone;
        if !cone.enabled || !self.cone.ready || self.cone.pipes[0].is_null() || self.cone.work.buffer.is_null() || self.cone.work_shape.0 != cone.theta || self.cone.work_shape.1 != cone.count || self.cone.work_shape.3 != cone.probe_px {
            self.cone.prev_eye = None;
            return;
        }
        let (w, h) = (self.lighting.dims[0], self.lighting.dims[1]);
        let (theta, count) = (cone.theta, cone.count);
        let seat = if crate::gate::guard(crate::gate::CONE_FIELD_SEAT) { 0 } else { CONE_PIXEL_SEAT };
        let flags = CONE_SEE_MERGE | CONE_SEE_SHADE | seat;
        let radius = cone.lamp_radius.to_bits();
        let finest = cone.finest_probe_level();
        let push = |c: u32| [w, h, finest, 0, c, theta, count, cone.probe_px, flags, CONE_T0.to_bits(), radius, 0];
        // Knockout only: the lamps written once and kept, as when the field held them.
        let write_lamps = crate::gate::guard(crate::gate::CONE_LAMP_FOLLOW) || !self.cone.lamps_frozen;
        let layout = self.cone.layout;
        let header = layout.header();
        let line = |n: u64| -> (u32, u32) {
            let groups = n.div_ceil(64);
            let x = groups.min(65_535);
            (u32::try_from(x).unwrap_or(1), u32::try_from(groups.div_ceil(x.max(1))).unwrap_or(1))
        };
        // The light pass first: it lights the field's voxels from this
        // frame's lamps and last frame's probes, which are still in the work
        // buffer, keyed from last frame's eye.
        self.memory_barrier(0x800, 0x1000, 0x20 | 0x40, 0x1000);
        let prev = self.cone.prev_eye.map_or([0u32; 4], |e| [e[0].to_bits(), e[1].to_bits(), e[2].to_bits(), 1]);
        unsafe {
            (self.fns.cmd_update_buffer)(self.cmd, self.cone.work.buffer, CONE_PREV_EYE_AT * 4, 16, prev.as_ptr().cast::<c_void>());
        }
        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
        let (fine_bricks, coarse_bricks) = self.cone.bricks;
        let now = std::time::Instant::now();
        let dt = self.cone.lit_at.map_or(1.0, |t| now.duration_since(t).as_secs_f32().min(1.0));
        let fresh = self.cone.fresh;
        let toward = if fresh { 1.0 } else { 1.0 - (-dt / CONE_BOUNCE_SECONDS).exp() };
        self.cone.lit_at = Some(now);
        self.cone.fresh = false;
        let mut light_push = push(0);
        light_push[11] = toward.to_bits();
        // The frame's count for the sun shares' turns; bit 31 marches every
        // face (a new field).
        light_push[4] = (self.cone.frames & 0x7fff_ffff) | if fresh { 0x8000_0000 } else { 0 };
        self.cone_dispatch(9, light_push, line(fine_bricks * CONE_BRICK_VOXELS + coarse_bricks * CONE_PADDED_TEXELS));
        // The clears below are transfers: order them after every compute read
        // and write of the work buffer before them, the light pass's included;
        // the field's new light is read by the passes after them.
        self.memory_barrier(0x800, 0x1000 | 0x800, 0x20 | 0x40, 0x1000 | 0x20);
        unsafe {
            (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, 0, CONE_CLEARED * 4, 0);
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.cone.work.buffer,
                CONE_LAYOUT_AT * 4,
                (header.len() * 4) as u64,
                header.as_ptr().cast::<c_void>(),
            );
            for c in 0..count as usize {
                (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, layout.hash_at[c] * 4, layout.hash[c] * CONE_HASH_WORDS * 4, 0);
            }
            if write_lamps {
                (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, layout.lamp_at * 4, layout.lamp_hash * CONE_LAMP_WORDS * 4, 0);
            }
        }
        let compute = |gpu: &Self| gpu.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
        let pixels = (w.div_ceil(8), h.div_ceil(8));
        if write_lamps {
            self.cone_dispatch(7, push(0), line(u64::from(self.cone.scene_lights) * CONE_LAMP_POINTS));
            compute(self);
            self.cone.lamps_frozen = true;
        }
        // Cascade 0 from the pixels, then each cascade from the one below it
        // (its compact first, so its probes are numbered).
        self.cone_dispatch(0, push(0), pixels);
        compute(self);
        // The world layer: probes on the surfaces around the eye no pixel covers.
        let mut world_push = push(0);
        world_push[11] = self.cone.frames;
        self.cone.frames = self.cone.frames.wrapping_add(1);
        self.cone_dispatch(10, world_push, line(coarse_bricks * CONE_PADDED_TEXELS));
        compute(self);
        for c in 0..count {
            if c > 0 {
                self.cone_dispatch(8, push(c), line(layout.cap[c as usize - 1]));
                compute(self);
            }
            self.cone_dispatch(1, push(c), line(layout.hash[c as usize]));
            compute(self);
        }
        for c in 1..count {
            self.cone_dispatch(2, push(c), line(layout.cap[c as usize]));
            compute(self);
        }
        for c in 0..count {
            self.cone_dispatch(3, push(c), line(layout.cap[c as usize] * layout.dirs[c as usize]));
            compute(self);
        }
        for c in 0..count.saturating_sub(1) {
            self.cone_dispatch(4, push(c), line(layout.cap[c as usize]));
        }
        compute(self);
        for c in (0..count.saturating_sub(1)).rev() {
            self.cone_dispatch(5, push(c), line(layout.cap[c as usize] * layout.dirs[c as usize]));
            compute(self);
        }
        self.cone_dispatch(6, push(0), pixels);
        compute(self);
        self.cone.prev_eye = Some(self.cone.eye);
    }

    /// Last frame's counts for each cascade. Waits for that frame. Empty
    /// when cone mode has not run.
    pub(crate) fn cone_stats(&mut self) -> Result<Vec<crate::ConeCascadeStats>, String> {
        let layout = self.cone.layout;
        let count = layout.count;
        if self.cone.work.buffer.is_null() || count == 0 {
            return Ok(Vec::new());
        }
        self.idle_device();
        let raw = self.copy_device_range(self.cone.work.buffer, 0, 48 * 4)?;
        let w = u32_words(&raw);
        let at = |i: usize| w.get(i).copied().unwrap_or(0);
        Ok((0..count)
            .map(|c| {
                let i = c as usize;
                let cap = u32::try_from(layout.cap[i]).unwrap_or(u32::MAX);
                let probes = at(i);
                crate::ConeCascadeStats {
                    probes,
                    cap,
                    dropped_compact: at(8 + i),
                    dropped_insert: at(32 + i),
                    cones_traced: at(16 + i),
                    cones_closed: at(24 + i),
                    seat_fallbacks: at(40 + i),
                }
            })
            .collect())
    }

    /// Lamp points that found no lamp-hash slot last frame. Waits for that frame.
    pub(crate) fn cone_lamp_drops(&mut self) -> Result<u32, String> {
        if self.cone.work.buffer.is_null() {
            return Ok(0);
        }
        self.idle_device();
        let raw = self.copy_device_range(self.cone.work.buffer, 48 * 4, 4)?;
        Ok(u32_words(&raw).first().copied().unwrap_or(0))
    }

    /// Cascade `c`'s probes as last frame wrote them. Waits for that frame.
    pub(crate) fn cone_probes(&mut self, c: u32) -> Result<Vec<crate::ConeProbe>, String> {
        let layout = self.cone.layout;
        if self.cone.work.buffer.is_null() || c >= layout.count {
            return Ok(Vec::new());
        }
        self.idle_device();
        let head = u32_words(&self.copy_device_range(self.cone.work.buffer, 0, 8 * 4)?);
        let kept = u64::from(head.get(c as usize).copied().unwrap_or(0)).min(layout.cap[c as usize]);
        if kept == 0 {
            return Ok(Vec::new());
        }
        let at = layout.probes_at[c as usize];
        let raw = self.copy_device_range(self.cone.work.buffer, at * 4, kept * CONE_PROBE_WORDS * 4)?;
        let w = u32_words(&raw);
        let stride = usize::try_from(CONE_PROBE_WORDS).unwrap_or(32);
        Ok(w.chunks_exact(stride)
            .map(|r| crate::ConeProbe {
                key: r[0],
                face: (r[0] >> 24) & 7,
                lod: r[1],
                cell: [r[4].cast_signed(), r[5].cast_signed(), r[6].cast_signed()],
                pixels: r[7],
                seat: [f32::from_bits(r[8]), f32::from_bits(r[9]), f32::from_bits(r[10])],
            })
            .collect())
    }

    fn destroy_cone(&mut self) {
        for pipe in &mut self.cone.pipes {
            if !pipe.is_null() {
                unsafe { (self.fns.destroy_pipeline)(self.device, *pipe, std::ptr::null()) };
                *pipe = std::ptr::null_mut();
            }
        }
        for k in 0..5 {
            let mut old = std::mem::replace(
                match k {
                    0 => &mut self.cone.table,
                    1 => &mut self.cone.fine,
                    2 => &mut self.cone.records,
                    3 => &mut self.cone.coarse,
                    _ => &mut self.cone.work,
                },
                Buffer::empty(),
            );
            self.destroy_buffer(&mut old);
        }
        let mut next = std::mem::take(&mut self.cone.next);
        for buffer in &mut next {
            self.destroy_buffer(buffer);
        }
        self.cone.next_full = false;
        self.cone.ready = false;
        self.cone.key = 0;
        self.cone.has_fine = false;
        self.cone.pending.clear();
    }
}
