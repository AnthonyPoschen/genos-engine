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
/// Work-buffer layout, as `cone_common.glsl` has it.
const CONE_HEADER_WORDS: u64 = 256;
const CONE_HASH_WORDS: u64 = 9;
const CONE_PROBE_WORDS: u64 = 32;
const CONE_CONE_WORDS: u64 = 4;
const CONE_CONES_CAP: u64 = 1 << 21;
const CONE_PIPES: usize = 7;
const CONE_PIPE_NAMES: [&str; CONE_PIPES] = [
    "cone-insert",
    "cone-compact",
    "cone-chain",
    "cone-trace",
    "cone-weights",
    "cone-merge",
    "cone-shade",
];

fn cone_dirs(theta: u32, c: u32) -> u64 {
    let t = u64::from(theta) << c;
    2 * t * t
}
fn cone_hash_size(c: u32) -> u64 {
    65_536 >> c
}
fn cone_cap(theta: u32, c: u32) -> u64 {
    (cone_hash_size(c) / 2).min(CONE_CONES_CAP / cone_dirs(theta, c))
}
/// Word offset of cascade `c`'s hash.
fn cone_hash_at(theta: u32, c: u32) -> u64 {
    let mut at = CONE_HEADER_WORDS;
    for k in 0..c {
        at += cone_hash_size(k) * CONE_HASH_WORDS + cone_cap(theta, k) * CONE_PROBE_WORDS + cone_cap(theta, k) * cone_dirs(theta, k) * CONE_CONE_WORDS;
    }
    at
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
    /// The work buffer's theta and cascade count.
    work_shape: (u32, u32),
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
            work_shape: (0, 0),
            report: String::new(),
        }
    }
}

/// What a cone-cascade field depends on: the world, its glTF model, its point
/// lamps and their sphere's radius.
fn cone_world_key(world: &World, radius: f32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    world.identity().hash(&mut h);
    world.gltf_surface.as_ref().map(|g| g.key()).hash(&mut h);
    radius.to_bits().hash(&mut h);
    let scene = &world.scene;
    for i in 0..scene.light_len() {
        let l = scene.light(i);
        for v in [l.position.x, l.position.y, l.position.z, l.color[0], l.color[1], l.color[2], l.direction.x, l.direction.y, l.direction.z] {
            v.to_bits().hash(&mut h);
        }
    }
    h.finish()
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
    }
    [cone_bytes(&table), cone_bytes(&field.fine_bricks), cone_bytes(&field.fine_voxels), cone_bytes(&coarse)]
}

/// The field for a scene, built off the frame thread.
fn cone_build(scene: &genos_scene::Scene, gltf: Option<&crate::gltf_surface::GltfSurface>, radius: f32) -> crate::cone_field::ConeField {
    // The engine's paint reflectance on the analytic shapes, as compose has it.
    const PAINT: f32 = 0.8;
    let points = crate::cone_field::surface_points(scene, gltf, PAINT);
    let mut voxels = crate::cone_field::FieldVoxels::default();
    for (at, normal, area, _) in &points {
        voxels.add_surface(*at, *normal, *area, [0.0; 3]);
    }
    voxels.add_scene_lamps(scene, radius);
    voxels.brick()
}

/// Cone mode runs only the cascades: the old pipeline's direct pass, shadows,
/// GI and ambient fill are off, and compose takes the cascades' light as the
/// surface's emitted light. The settings the user wrote stay as they are.
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
}

impl Gpu {
    /// Build, upload and size the cone field for this world when cone mode is on.
    fn sync_cone(&mut self, world: &World) -> Result<(), String> {
        let cone = self.lighting.settings.cone;
        if !cone.enabled {
            return Ok(());
        }
        let key = cone_world_key(world, cone.lamp_radius);
        // One build at a time. A moving lamp changes the key every frame, and a
        // thread for each frame's key held a field each and ran the machine out
        // of memory in a minute. The next build starts when this one is done,
        // from the world as it is then, so a moving lamp lags by a build.
        if self.cone.key != key && self.cone.building.is_none() {
            let scene = world.scene.clone();
            let gltf = world.gltf_surface.clone();
            let radius = cone.lamp_radius;
            let handle = std::thread::Builder::new()
                .name("cone-field".into())
                .spawn(move || cone_build(&scene, gltf.as_deref(), radius))
                .map_err(|e| format!("cone field thread: {e}"))?;
            self.cone.building = Some((key, handle));
        }
        if self.cone.building.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            if let Some((k, handle)) = self.cone.building.take() {
                let field = handle.join().map_err(|_| "cone field thread panicked".to_string())?;
                self.cone_take_field(k, &field)?;
            }
        }
        let shape = (cone.theta, cone.count);
        if self.cone.work_shape != shape || self.cone.work.buffer.is_null() {
            let words = cone_hash_at(cone.theta, cone.count);
            self.idle_device();
            let mut old = std::mem::replace(&mut self.cone.work, Buffer::empty());
            self.destroy_buffer(&mut old);
            let share = self.light_families();
            self.cone.work = self.make_buffer_queues(words * 4, 0x20 | 0x2, Memory::Device, &share)?;
            self.cone.work_shape = shape;
            if self.lighting.ready {
                self.write_lighting_sets()?;
            }
        }
        if self.cone.pipes[0].is_null() && self.lighting.ready {
            let codes: [&[u8]; CONE_PIPES] = [CONE_INSERT_SPV, CONE_COMPACT_SPV, CONE_CHAIN_SPV, CONE_TRACE_SPV, CONE_WEIGHTS_SPV, CONE_MERGE_SPV, CONE_SHADE_SPV];
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
        if !cone.enabled || !self.cone.ready || self.cone.pipes[0].is_null() || self.cone.work.buffer.is_null() || self.cone.work_shape != (cone.theta, cone.count) {
            return;
        }
        let (w, h) = (self.lighting.dims[0], self.lighting.dims[1]);
        let (theta, count) = (cone.theta, cone.count);
        let flags = CONE_SEE_MERGE | CONE_SEE_SHADE;
        let push = |c: u32| [w, h, 0, 0, c, theta, count, cone.probe_px, flags, CONE_T0.to_bits(), 0, 0];
        unsafe {
            (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, 0, CONE_HEADER_WORDS * 4, 0);
            for c in 0..count {
                (self.fns.cmd_fill_buffer)(self.cmd, self.cone.work.buffer, cone_hash_at(theta, c) * 4, cone_hash_size(c) * CONE_HASH_WORDS * 4, 0);
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
        self.cone_dispatch(0, push(0), pixels);
        compute(self);
        for c in 0..count {
            self.cone_dispatch(1, push(c), line(cone_hash_size(c)));
            compute(self);
        }
        for c in 1..count {
            self.cone_dispatch(2, push(c), line(cone_cap(theta, c)));
            compute(self);
        }
        for c in 0..count {
            self.cone_dispatch(3, push(c), line(cone_cap(theta, c) * cone_dirs(theta, c)));
            compute(self);
        }
        for c in 0..count.saturating_sub(1) {
            self.cone_dispatch(4, push(c), line(cone_cap(theta, c)));
        }
        compute(self);
        for c in (0..count.saturating_sub(1)).rev() {
            self.cone_dispatch(5, push(c), line(cone_cap(theta, c) * cone_dirs(theta, c)));
            compute(self);
        }
        self.cone_dispatch(6, push(0), pixels);
        compute(self);
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
