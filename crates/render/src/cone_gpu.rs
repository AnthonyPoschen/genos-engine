// The cone-traced cascades on the GPU (experimental, lighting key
// `cone_cascades`). Included into gpu.rs, as lighting_gpu.rs is: the passes
// use the lighting descriptor set, its layout and its push constant. The
// field (`cone_field.rs`) is built on a thread when the world, its glTF model
// or its lamps change, uploaded over a few frames, and then every frame the
// seven passes in `shaders/cone_*.comp` run before compose.

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
const CONE_HASH_WORDS: u64 = 9;
const CONE_PROBE_WORDS: u64 = 32;
const CONE_CONE_WORDS: u64 = 4;
/// Cascade 0's probes for each `probe_px` square of the screen. The voxel
/// layer measured 3.0 to 3.5 (floors seen at a grazing angle, voxels split
/// between levels), so 5 leaves room; gate B fails if it does not.
const CONE_PROBE_SLACK: u64 = 5;
/// Each cascade holds this share of the probes of the one below (measured
/// 1/3.4: probes on a surface, at twice the spacing).
const CONE_CASCADE_SHARE: u64 = 3;
const CONE_MIN_CAP: u64 = 64;
/// Header words cleared each frame; the layout follows them.
const CONE_CLEARED: u64 = 64;
const CONE_LAYOUT_AT: u64 = 64;
const CONE_PIPES: usize = 8;
const CONE_PIPE_NAMES: [&str; CONE_PIPES] = [
    "cone-insert",
    "cone-compact",
    "cone-chain",
    "cone-trace",
    "cone-weights",
    "cone-merge",
    "cone-shade",
    "cone-lamps",
];
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
    let mut cap = base;
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
    building: Option<(u64, std::thread::JoinHandle<crate::cone_field::ConeField>)>,
    /// Bytes still to upload: (buffer 0..3, bytes, cursor).
    pending: Vec<(usize, Vec<u8>, usize)>,
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
            pending: Vec::new(),
            ready: false,
            work_shape: (0, 0, 0, 0, 0),
            lamps_frozen: false,
            scene_lights: 0,
            layout: ConeLayout::default(),
            report: String::new(),
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

/// The field's four buffers as bytes: the table (head and levels first), the
/// fine headers, the fine records, and the coarse texels (16 words each).
fn cone_field_bytes(field: &crate::cone_field::ConeField) -> [Vec<u8>; 4] {
    let mut table: Vec<u32> = vec![field.atlas[0], field.atlas[1], field.atlas[2], field.coarse_bricks];
    for level in &field.levels {
        table.extend(level.origin.map(|v| u32::from_ne_bytes(v.to_ne_bytes())));
        table.push(level.offset);
        table.extend(level.dims);
        table.push(0);
    }
    table.extend_from_slice(&field.table);
    let texels = field.occupancy.len();
    let mut coarse = vec![0u32; texels * 16];
    for i in 0..texels {
        for d in 0..7 {
            let f = &field.faces[d][i * 4..i * 4 + 4];
            coarse[i * 16 + 2 * d] = u32::from(f[0]) | (u32::from(f[1]) << 16);
            coarse[i * 16 + 2 * d + 1] = u32::from(f[2]) | (u32::from(f[3]) << 16);
        }
        coarse[i * 16 + 14] = u32::from(field.occupancy[i] != 0);
        coarse[i * 16 + 15] = field.coarse_seats[i];
    }
    [cone_bytes(&table), cone_bytes(&field.fine_bricks), cone_bytes(&field.fine_voxels), cone_bytes(&coarse)]
}

/// The field for a scene, built off the frame thread: its surfaces. The lamps
/// are not in it; they are written each frame.
fn cone_build(scene: &genos_scene::Scene, gltf: Option<&crate::gltf_surface::GltfSurface>) -> crate::cone_field::ConeField {
    // The engine's paint reflectance on the analytic shapes, as compose has it.
    const PAINT: f32 = 0.8;
    let points = crate::cone_field::surface_points(scene, gltf, PAINT);
    let mut voxels = crate::cone_field::FieldVoxels::default();
    for (at, normal, area, _) in &points {
        voxels.add_surface(*at, *normal, *area, [0.0; 3]);
    }
    voxels.brick()
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
    fn sync_cone(&mut self, world: &World) -> Result<(), String> {
        let cone = self.lighting.settings.cone;
        if !cone.enabled {
            return Ok(());
        }
        let key = cone_world_key(world);
        // One build at a time. A moving lamp changes the key every frame, and a
        // thread for each frame's key held a field each and ran the machine out
        // of memory in a minute. The next build starts when this one is done,
        // from the world as it is then, so a moving lamp lags by a build.
        if self.cone.key != key && self.cone.building.is_none() {
            let scene = world.scene.clone();
            let gltf = world.gltf_surface.clone();
            let handle = std::thread::Builder::new()
                .name("cone-field".into())
                .spawn(move || cone_build(&scene, gltf.as_deref()))
                .map_err(|e| format!("cone field thread: {e}"))?;
            self.cone.building = Some((key, handle));
        }
        if self.cone.building.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            if let Some((k, handle)) = self.cone.building.take() {
                let field = handle.join().map_err(|_| "cone field thread panicked".to_string())?;
                self.cone_take_field(k, &field)?;
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
            if self.lighting.ready {
                self.write_lighting_sets()?;
            }
        }
        if self.cone.pipes[0].is_null() && self.lighting.ready {
            let codes: [&[u8]; CONE_PIPES] = [CONE_INSERT_SPV, CONE_COMPACT_SPV, CONE_CHAIN_SPV, CONE_TRACE_SPV, CONE_WEIGHTS_SPV, CONE_MERGE_SPV, CONE_SHADE_SPV, CONE_LAMPS_SPV];
            for (k, code) in codes.iter().enumerate() {
                self.cone.pipes[k] = self.make_compute_pipe(code, self.lighting.layout, CONE_PIPE_NAMES[k])?;
            }
        }
        Ok(())
    }

    /// A built field: new buffers, and its bytes queued for upload.
    fn cone_take_field(&mut self, key: u64, field: &crate::cone_field::ConeField) -> Result<(), String> {
        let bytes = cone_field_bytes(field);
        self.cone.report = format!(
            "{} points, {} voxels, {} fine bricks, {} fine records, {} coarse bricks, {} MiB",
            field.points,
            field.voxels,
            field.fine_bricks.len() / crate::cone_field::FINE_HEADER,
            field.fine_voxels.len() / crate::cone_field::FINE_RECORD,
            field.coarse_bricks,
            bytes.iter().map(Vec::len).sum::<usize>() >> 20
        );
        self.idle_device();
        let share = self.light_families();
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
                let dst = match *k {
                    0 => self.cone.table.buffer,
                    1 => self.cone.fine.buffer,
                    2 => self.cone.records.buffer,
                    _ => self.cone.coarse.buffer,
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
            self.cone.ready = true;
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
            return;
        }
        let (w, h) = (self.lighting.dims[0], self.lighting.dims[1]);
        let (theta, count) = (cone.theta, cone.count);
        let seat = if crate::gate::guard(crate::gate::CONE_FIELD_SEAT) { 0 } else { CONE_PIXEL_SEAT };
        let flags = CONE_SEE_MERGE | CONE_SEE_SHADE | seat;
        let radius = cone.lamp_radius.to_bits();
        let push = |c: u32| [w, h, 0, 0, c, theta, count, cone.probe_px, flags, CONE_T0.to_bits(), radius, 0];
        // Knockout only: the lamps written once and kept, as when the field held them.
        let write_lamps = crate::gate::guard(crate::gate::CONE_LAMP_FOLLOW) || !self.cone.lamps_frozen;
        let layout = self.cone.layout;
        let header = layout.header();
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
        let line = |n: u64| -> (u32, u32) {
            let groups = n.div_ceil(64);
            let x = groups.min(65_535);
            (u32::try_from(x).unwrap_or(1), u32::try_from(groups.div_ceil(x.max(1))).unwrap_or(1))
        };
        if write_lamps {
            self.cone_dispatch(7, push(0), line(u64::from(self.cone.scene_lights) * CONE_LAMP_POINTS));
            compute(self);
            self.cone.lamps_frozen = true;
        }
        self.cone_dispatch(0, push(0), pixels);
        compute(self);
        for c in 0..count {
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
        let stride = CONE_PROBE_WORDS as usize;
        Ok(w.chunks_exact(stride)
            .map(|r| crate::ConeProbe {
                key: r[0],
                lod: r[1],
                cell: [r[4] as i32, r[5] as i32, r[6] as i32],
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
        self.cone.ready = false;
        self.cone.key = 0;
        self.cone.pending.clear();
    }
}
