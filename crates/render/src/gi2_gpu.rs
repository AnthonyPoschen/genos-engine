// GI v2 frame path (docs/systems/gi-v2-design.md), included into gpu.rs.
//
// With GI v2 on, the frame rasters a G-buffer (gbuffer.frag, same vertices and
// depth prepass) instead of the old shading, then runs small compute passes, each
// with at most one trace site:
//
//   compact  list the light cache's live patches
//   place    one screen probe per tile on the G-buffer surface
//   trace    the probes' rays through the tracer (analytic shapes + mesh SDFs)
//   light    direct light with shadow rays at every pixel and every probe ray hit
//   gather   ray radiance to irradiance SH per probe
//   filter   each probe's SH averaged with its matching 3 x 3 neighbours
//   cache    relight a batch of light cache patches (gi2_cache.glsl): every bounce
//            after the first, read where probe and cache rays hit
//   compose  direct + probe irradiance per pixel, tone curve, dither
//
// The result goes back into the colour image, so anti-aliasing, overlays, readback
// and present work as before. The old path (scene.frag, light.comp) is untouched and
// is what runs with GI v2 off: `GENOS_GI=v2` at start, or `Renderer::set_gi_v2`.

/// Screen probe tile edge in pixels (Medium and above in the design).
pub(crate) const GI2_TILE: u32 = 16;
/// Rays per screen probe (Low in the design; a square for the stratified pattern).
pub(crate) const GI2_RAYS: u32 = 64;
/// Light cache sizes; gi2_cache_slots.glsl has the same numbers.
const GI2_CACHE_SLOTS: u64 = 262144;
const GI2_CACHE_BATCH: u32 = 16384;
const GI2_CACHE_RAYS: u32 = 16;
/// GI v2 counts as settled once the scene block (camera included) has stayed the
/// same for this many frames and a full sweep of the light cache changed no patch
/// by more than GI2_SETTLE_CHANGE.
const GI2_SETTLE_FRAMES: u32 = 2;
/// Largest relative change of a relit cache patch, in millionths, that counts as
/// settled (0.2 %, well under a display code).
const GI2_SETTLE_CHANGE: u32 = 2000;

pub(crate) struct Gi2 {
    pub(crate) on: bool,
    ready: bool,
    desc_layout: Handle,
    layout: Handle,
    pool: Handle,
    sets: [Handle; 2],
    pipes: [Handle; 8],
    gbuffer_pipe: Handle,
    gbuf: Buffer,
    work: Buffer,
    out: Buffer,
    cache_keys: Buffer,
    cache: Buffer,
    /// The cache buffers are new and get zeroed before the next frame's passes.
    cache_clear: bool,
    /// The G-buffer is new and gets filled with empty records first.
    gbuf_clear: bool,
    frame: u32,
    /// Hash of the last frame's scene block, and frames it has stayed the same.
    scene_hash: u64,
    still: u32,
    /// Per frame slot: the cache pass's counters (gi2_cache.glsl `CacheStats`).
    stats: [Buffer; 2],
    /// Frames in a row whose relit patches all stayed within GI2_SETTLE_CHANGE, and
    /// the live patch count last read back.
    quiet: u32,
    live: u32,
    dims: [u32; 4],
}

impl Default for Gi2 {
    fn default() -> Self {
        Self {
            on: std::env::var("GENOS_GI").is_ok_and(|v| v == "v2"),
            ready: false,
            desc_layout: std::ptr::null_mut(),
            layout: std::ptr::null_mut(),
            pool: std::ptr::null_mut(),
            sets: [std::ptr::null_mut(); 2],
            pipes: [std::ptr::null_mut(); 8],
            gbuffer_pipe: std::ptr::null_mut(),
            gbuf: Buffer::empty(),
            work: Buffer::empty(),
            out: Buffer::empty(),
            cache_keys: Buffer::empty(),
            cache: Buffer::empty(),
            cache_clear: false,
            gbuf_clear: false,
            frame: 0,
            scene_hash: 0,
            still: 0,
            stats: [Buffer::empty(), Buffer::empty()],
            quiet: 0,
            live: 0,
            dims: [0; 4],
        }
    }
}

const GI2_PASS_NAMES: [&str; 8] = [
    "gi2 compact",
    "gi2 place",
    "gi2 trace",
    "gi2 light",
    "gi2 gather",
    "gi2 filter",
    "gi2 cache",
    "gi2 compose",
];

impl Gpu {
    /// Note how long a pipeline took to build (tests check it stays short).
    fn note_pipeline_time(&mut self, name: &str, start: Instant) {
        self.pipeline_times
            .push((name.to_string(), start.elapsed()));
    }

    /// Turn GI v2 on or off. The first time on builds its pipelines and buffers.
    /// While on, the current lighting's probe fields shrink to a placeholder (about
    /// 80 MB back); switching off rebuilds them empty, and the caller resets the tier
    /// so the current lighting relearns its light.
    pub(crate) fn set_gi2(&mut self, on: bool) -> Result<(), String> {
        if on == self.gi2.on && (!on || self.gi2.ready) {
            return Ok(());
        }
        unsafe {
            (self.fns.device_wait)(self.device);
        }
        self.gi2.on = on;
        if on && !self.gi2.ready {
            self.make_gi2()?;
            self.make_gi2_targets()?;
        }
        self.size_v1_light(!on)
    }

    /// Full-size (current lighting runs) or placeholder (GI v2 runs) probe fields.
    fn size_v1_light(&mut self, full: bool) -> Result<(), String> {
        let field = if full {
            crate::probe_tier::TIER_END as u64 * 16
        } else {
            256
        };
        let view = if full {
            (crate::probe_tier::TIER_END - crate::probe_tier::TIER_PROBES) as u64 * 16
        } else {
            256
        };
        if self.light_field[0].size == field && self.tier_view.size == view {
            return Ok(());
        }
        let share = self.light_families();
        for index in 0..2 {
            let mut old = std::mem::replace(&mut self.light_field[index], Buffer::empty());
            self.destroy_buffer(&mut old);
            self.light_field[index] =
                self.make_buffer_queues(field, 0x20 | 0x1 | 0x2, Memory::Upload, &share)?;
            self.write_buffer(&self.light_field[index], &vec![0u8; field as usize])?;
        }
        let mut old = std::mem::replace(&mut self.tier_view, Buffer::empty());
        self.destroy_buffer(&mut old);
        self.tier_view = self.make_buffer_queues(view, 0x20, Memory::Upload, &share)?;
        self.write_buffer(&self.tier_view, &vec![0u8; view as usize])?;
        // Nothing built into the old fields survives.
        self.pending_light = None;
        self.tier_batch = None;
        self.tier_started = None;
        self.light_busy = false;
        self.light_building = false;
        self.light_publish_flight = None;
        self.read_plan.clear();
        self.world_stale = true;
        self.write_light_set(0)?;
        self.write_light_set(1)
    }

    fn make_gi2(&mut self) -> Result<(), String> {
        #[repr(C)]
        struct Binding {
            binding: u32,
            kind: u32,
            count: u32,
            stages: u32,
            samplers: *const c_void,
        }
        #[repr(C)]
        struct SetInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            count: u32,
            bindings: *const Binding,
        }
        let bindings: Vec<Binding> = [0u32, 5, 6, 7, 8, 9, 10, 11]
            .iter()
            .map(|&b| Binding {
                binding: b,
                kind: 7,
                count: 1,
                stages: 0x20,
                samplers: std::ptr::null(),
            })
            .collect();
        let set_info = SetInfo {
            s_type: 32,
            next: std::ptr::null(),
            flags: 0,
            count: bindings.len() as u32,
            bindings: bindings.as_ptr(),
        };
        #[repr(C)]
        struct Range {
            stage: u32,
            offset: u32,
            size: u32,
        }
        #[repr(C)]
        struct LayoutInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            set_count: u32,
            sets: *const Handle,
            push_count: u32,
            push: *const Range,
        }
        #[repr(C)]
        struct Size {
            kind: u32,
            count: u32,
        }
        #[repr(C)]
        struct PoolInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            max_sets: u32,
            size_count: u32,
            sizes: *const Size,
        }
        #[repr(C)]
        struct Alloc {
            s_type: i32,
            next: *const c_void,
            pool: Handle,
            count: u32,
            layouts: *const Handle,
        }
        unsafe {
            check(
                (self.fns.create_desc_layout)(
                    self.device,
                    &set_info as *const SetInfo as *const u8,
                    std::ptr::null(),
                    &mut self.gi2.desc_layout,
                ),
                "gi2 descriptor layout",
            )?;
            let sets = [self.gi2.desc_layout];
            let push = Range {
                stage: 0x20,
                offset: 0,
                size: 32,
            };
            let layout = LayoutInfo {
                s_type: 30,
                next: std::ptr::null(),
                flags: 0,
                set_count: 1,
                sets: sets.as_ptr(),
                push_count: 1,
                push: &push,
            };
            check(
                (self.fns.create_layout)(
                    self.device,
                    &layout as *const LayoutInfo as *const u8,
                    std::ptr::null(),
                    &mut self.gi2.layout,
                ),
                "gi2 pipeline layout",
            )?;
            let size = Size { kind: 7, count: 16 };
            let pool = PoolInfo {
                s_type: 33,
                next: std::ptr::null(),
                flags: 0,
                max_sets: 2,
                size_count: 1,
                sizes: &size,
            };
            check(
                (self.fns.create_desc_pool)(
                    self.device,
                    &pool as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut self.gi2.pool,
                ),
                "gi2 descriptor pool",
            )?;
            let layouts = [self.gi2.desc_layout, self.gi2.desc_layout];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: self.gi2.pool,
                count: 2,
                layouts: layouts.as_ptr(),
            };
            check(
                (self.fns.alloc_desc)(
                    self.device,
                    &alloc as *const Alloc as *const u8,
                    self.gi2.sets.as_mut_ptr(),
                ),
                "gi2 descriptor sets",
            )?;
        }
        let codes: [&[u8]; 8] = [
            GI2_COMPACT_SPV,
            GI2_PLACE_SPV,
            GI2_TRACE_SPV,
            GI2_LIGHT_SPV,
            GI2_GATHER_SPV,
            GI2_FILTER_SPV,
            GI2_CACHE_SPV,
            GI2_COMPOSE_SPV,
        ];
        for (k, code) in codes.iter().enumerate() {
            let start = Instant::now();
            self.gi2.pipes[k] = self.make_compute_pipe(code, self.gi2.layout, GI2_PASS_NAMES[k])?;
            self.note_pipeline_time(GI2_PASS_NAMES[k], start);
        }
        let start = Instant::now();
        self.frag_override = Some(GBUFFER_FRAG_SPV);
        self.vert_override = Some(GBUFFER_VERT_SPV);
        let made = self.make_pipeline(true, false, false);
        self.frag_override = None;
        self.vert_override = None;
        self.gi2.gbuffer_pipe = made?;
        self.note_pipeline_time("gi2 gbuffer", start);
        self.gi2.ready = true;
        Ok(())
    }

    fn make_compute_pipe(&self, code: &[u8], layout: Handle, what: &str) -> Result<Handle, String> {
        let module = self.shader(code)?;
        #[repr(C)]
        struct Stage {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            stage: u32,
            module: Handle,
            name: *const i8,
            spec: *const c_void,
        }
        #[repr(C)]
        struct Pipe {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            stage: Stage,
            layout: Handle,
            base: Handle,
            base_index: i32,
        }
        let pipe = Pipe {
            s_type: 29,
            next: std::ptr::null(),
            flags: 0,
            stage: Stage {
                s_type: 18,
                next: std::ptr::null(),
                flags: 0,
                stage: 0x20,
                module,
                name: b"main\0".as_ptr() as *const i8,
                spec: std::ptr::null(),
            },
            layout,
            base: std::ptr::null_mut(),
            base_index: -1,
        };
        let mut pipeline = std::ptr::null_mut();
        unsafe {
            let result = (self.fns.create_compute)(
                self.device,
                std::ptr::null_mut(),
                1,
                &pipe as *const Pipe as *const u8,
                std::ptr::null(),
                &mut pipeline,
            );
            (self.fns.destroy_shader)(self.device, module, std::ptr::null());
            check(result, what)?;
        }
        Ok(pipeline)
    }

    /// Size the G-buffer, work and output buffers for the picture and point every
    /// descriptor at them.
    fn make_gi2_targets(&mut self) -> Result<(), String> {
        if !self.gi2.ready {
            return Ok(());
        }
        let (w, h) = (self.extent_w.max(1), self.extent_h.max(1));
        let cols = w.div_ceil(GI2_TILE);
        let rows = h.div_ceil(GI2_TILE);
        let probes = (cols * rows) as u64;
        let rays = probes * GI2_RAYS as u64 + (GI2_CACHE_BATCH * GI2_CACHE_RAYS) as u64;
        let pixels = w as u64 * h as u64;
        let work_vec4 = 2 * probes + 2 * rays + 14 * probes;
        let mut gbuf = std::mem::replace(&mut self.gi2.gbuf, Buffer::empty());
        let mut work = std::mem::replace(&mut self.gi2.work, Buffer::empty());
        let mut out = std::mem::replace(&mut self.gi2.out, Buffer::empty());
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        if self.gi2.cache_keys.buffer.is_null() {
            self.gi2.cache_keys =
                self.make_buffer((2 * GI2_CACHE_SLOTS + 1) * 4, 0x20 | 0x2, Memory::Device)?;
            self.gi2.cache = self.make_buffer(GI2_CACHE_SLOTS * 48, 0x20 | 0x2, Memory::Device)?;
            self.gi2.cache_clear = true;
            for k in 0..2 {
                self.gi2.stats[k] = self.make_buffer(16, 0x20 | 0x2, Memory::Readback)?;
            }
        }
        self.gi2.gbuf = self.make_buffer(16 + pixels * 16, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.gi2.work = self.make_buffer(work_vec4 * 16, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.gi2.out = self.make_buffer(pixels * 4, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.gi2.dims = [w, h, cols, rows];
        self.gi2.gbuf_clear = true;
        self.write_light_set(0)?;
        self.write_light_set(1)?;
        self.write_gi2_sets()
    }

    fn write_gi2_sets(&self) -> Result<(), String> {
        #[repr(C)]
        struct BufInfo {
            buffer: Handle,
            offset: u64,
            range: u64,
        }
        #[repr(C)]
        struct Write {
            s_type: i32,
            next: *const c_void,
            set: Handle,
            binding: u32,
            element: u32,
            count: u32,
            kind: u32,
            image: *const c_void,
            buffer: *const BufInfo,
            texel: *const c_void,
        }
        for slot in 0..2 {
            let buffers = [
                (0u32, self.frame_scene[slot].buffer),
                (5, self.mesh_field.buffer),
                (6, self.gi2.gbuf.buffer),
                (7, self.gi2.work.buffer),
                (8, self.gi2.out.buffer),
                (9, self.gi2.cache_keys.buffer),
                (10, self.gi2.cache.buffer),
                (11, self.gi2.stats[slot].buffer),
            ];
            let infos: Vec<BufInfo> = buffers
                .iter()
                .map(|(_, b)| BufInfo {
                    buffer: *b,
                    offset: 0,
                    range: u64::MAX,
                })
                .collect();
            let writes: Vec<Write> = buffers
                .iter()
                .zip(&infos)
                .map(|((binding, _), info)| Write {
                    s_type: 35,
                    next: std::ptr::null(),
                    set: self.gi2.sets[slot],
                    binding: *binding,
                    element: 0,
                    count: 1,
                    kind: 7,
                    image: std::ptr::null(),
                    buffer: info,
                    texel: std::ptr::null(),
                })
                .collect();
            unsafe {
                (self.fns.update_desc)(
                    self.device,
                    writes.len() as u32,
                    writes.as_ptr() as *const u8,
                    0,
                    std::ptr::null(),
                );
            }
        }
        Ok(())
    }

    /// Settle state for the debug tools, in the tier's terms: one "brick", settled
    /// once the scene (camera included) has not changed for GI2_SETTLE_FRAMES.
    fn gi2_stats(&self) -> crate::probe_tier::TierStats {
        let sweep = self.gi2.live.div_ceil(GI2_CACHE_BATCH).max(1) + 1;
        let settled = self.gi2.still >= GI2_SETTLE_FRAMES && self.gi2.quiet >= sweep;
        crate::probe_tier::TierStats {
            seen_bricks: 1,
            seen_settled: usize::from(settled),
            pending_bricks: usize::from(!settled),
            ..Default::default()
        }
    }

    /// GI v2 runs this frame: on, built, sized for the picture, no supersampling.
    fn gi2_active(&self) -> bool {
        self.gi2.on
            && self.gi2.ready
            && self.antialias != Antialias::Ssaa
            && self.gi2.dims[0] == self.extent_w
            && self.gi2.dims[1] == self.extent_h
    }

    /// Before the raster: the G-buffer size header, and the last frame's passes done
    /// with the buffers this frame's raster writes.
    fn gi2_begin(&mut self, slot: usize) {
        let header = [self.gi2.dims[0], self.gi2.dims[1], 0, 0];
        // This slot's last frame is done (its fence was waited): read its counters.
        if !self.gi2.cache_clear {
            if let Ok(words) = self.read_gi2_stats(slot) {
                if words[1] > 0 {
                    if words[0] <= GI2_SETTLE_CHANGE {
                        self.gi2.quiet = self.gi2.quiet.saturating_add(1);
                    } else {
                        self.gi2.quiet = 0;
                    }
                }
                self.gi2.live = words[2];
                if std::env::var_os("GENOS_GI_DEBUG").is_some() {
                    eprintln!(
                        "gi2 frame {} still {} change {} relit {} live {} missed {} quiet {}",
                        self.gi2.frame,
                        self.gi2.still,
                        words[0],
                        words[1],
                        words[2],
                        words[3],
                        self.gi2.quiet
                    );
                }
            }
        }
        unsafe {
            (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.stats[slot].buffer, 0, 16, 0);
            if self.gi2.cache_clear {
                self.gi2.cache_clear = false;
                (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.cache_keys.buffer, 0, u64::MAX, 0);
                (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.cache.buffer, 0, u64::MAX, 0);
            }
            if self.gi2.gbuf_clear {
                self.gi2.gbuf_clear = false;
                // Every record empty (distance all ones); the header goes in below.
                (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.gbuf.buffer, 0, u64::MAX, u32::MAX);
            }
            // The whole-buffer clears land before the count and header writes.
            self.memory_barrier(0x1000, 0x1000, 0x1000, 0x1000);
            // The live patch count, rebuilt by the compact pass.
            (self.fns.cmd_fill_buffer)(
                self.cmd,
                self.gi2.cache_keys.buffer,
                GI2_CACHE_SLOTS * 4,
                4,
                0,
            );
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.gi2.gbuf.buffer,
                0,
                16,
                header.as_ptr() as *const c_void,
            );
        }
        // Compute reads/writes and the header update before fragment writes.
        self.memory_barrier(
            0x800 | 0x1000,
            0x80 | 0x800,
            0x20 | 0x40 | 0x1000,
            0x20 | 0x40,
        );
    }

    fn read_gi2_stats(&self, slot: usize) -> Result<[u32; 4], String> {
        let mut words = [0u32; 4];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    self.gi2.stats[slot].memory,
                    0,
                    16,
                    0,
                    &mut mapped,
                ),
                "map gi2 stats",
            )?;
            std::ptr::copy_nonoverlapping(mapped as *const u32, words.as_mut_ptr(), 4);
            (self.fns.unmap_mem)(self.device, self.gi2.stats[slot].memory);
        }
        Ok(words)
    }

    /// After the raster: the passes, then the result into the colour image.
    fn gi2_passes(&mut self, slot: usize) {
        let [w, h, cols, rows] = self.gi2.dims;
        let probes = cols * rows;
        let rays = probes * GI2_RAYS + GI2_CACHE_BATCH * GI2_CACHE_RAYS;
        let bgra = if self.format == 44 { 1u32 } else { 0 };
        self.gi2.frame = self.gi2.frame.wrapping_add(1);
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.frame_bytes.hash(&mut hasher);
            hasher.finish()
        };
        if hash == self.gi2.scene_hash {
            self.gi2.still = self.gi2.still.saturating_add(1);
        } else {
            self.gi2.scene_hash = hash;
            self.gi2.still = 0;
        }
        let pc = [w, h, cols, rows, GI2_RAYS, GI2_TILE, bgra, self.gi2.frame];
        let image = self.color.image;
        // The raster colour as the base: pixels with no surface keep it.
        self.copy_image_buffer(image, 6, self.gi2.out.buffer, w, h, true);
        self.memory_barrier(0x80 | 0x1000, 0x800, 0x40 | 0x1000, 0x20 | 0x40);
        let set = self.gi2.sets[slot];
        let groups = [
            (0usize, (GI2_CACHE_SLOTS as u32).div_ceil(64), 1u32),
            (1, probes.div_ceil(64), 1),
            (2, rays.div_ceil(64), 1),
            (3, (w * h + rays).div_ceil(64), 1),
            (4, probes.div_ceil(64), 1),
            (5, probes.div_ceil(64), 1),
            (6, GI2_CACHE_BATCH.div_ceil(64), 1),
            (7, w.div_ceil(8), h.div_ceil(8)),
        ];
        unsafe {
            (self.fns.cmd_bind_set)(
                self.cmd,
                1,
                self.gi2.layout,
                0,
                1,
                &set,
                0,
                std::ptr::null(),
            );
            (self.fns.cmd_push)(
                self.cmd,
                self.gi2.layout,
                0x20,
                0,
                32,
                pc.as_ptr() as *const c_void,
            );
            for (pass, gx, gy) in groups {
                (self.fns.cmd_bind_pipe)(self.cmd, 1, self.gi2.pipes[pass]);
                (self.fns.cmd_dispatch)(self.cmd, gx.max(1), gy.max(1), 1);
                self.memory_barrier(0x800, 0x800 | 0x1000, 0x40, 0x20 | 0x40 | 0x800);
            }
        }
        self.image_barrier(image, 6, 7, 0x1000, 0x1000, 0x800, 0x1000);
        self.copy_image_buffer(image, 7, self.gi2.out.buffer, w, h, false);
        self.image_barrier(image, 7, 6, 0x1000, 0x1000 | 0x800, 0x1000, 0x800 | 0x20);
    }

    fn memory_barrier(&self, src_stage: u32, dst_stage: u32, src_access: u32, dst_access: u32) {
        #[repr(C)]
        struct Barrier {
            s_type: i32,
            next: *const c_void,
            src_access: u32,
            dst_access: u32,
        }
        let barrier = Barrier {
            s_type: 46,
            next: std::ptr::null(),
            src_access,
            dst_access,
        };
        unsafe {
            (self.fns.cmd_barrier)(
                self.cmd,
                src_stage,
                dst_stage,
                0,
                1,
                &barrier as *const Barrier as *const c_void,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            );
        }
    }

    fn destroy_gi2(&mut self) {
        let mut gbuf = std::mem::replace(&mut self.gi2.gbuf, Buffer::empty());
        let mut work = std::mem::replace(&mut self.gi2.work, Buffer::empty());
        let mut out = std::mem::replace(&mut self.gi2.out, Buffer::empty());
        let mut keys = std::mem::replace(&mut self.gi2.cache_keys, Buffer::empty());
        let mut cache = std::mem::replace(&mut self.gi2.cache, Buffer::empty());
        for k in 0..2 {
            let mut stats = std::mem::replace(&mut self.gi2.stats[k], Buffer::empty());
            self.destroy_buffer(&mut stats);
        }
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        self.destroy_buffer(&mut keys);
        self.destroy_buffer(&mut cache);
        unsafe {
            for pipe in self
                .gi2
                .pipes
                .iter_mut()
                .chain(std::iter::once(&mut self.gi2.gbuffer_pipe))
            {
                if !pipe.is_null() {
                    (self.fns.destroy_pipeline)(self.device, *pipe, std::ptr::null());
                    *pipe = std::ptr::null_mut();
                }
            }
            if !self.gi2.layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.gi2.layout, std::ptr::null());
                self.gi2.layout = std::ptr::null_mut();
            }
            if !self.gi2.pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.gi2.pool, std::ptr::null());
                self.gi2.pool = std::ptr::null_mut();
            }
            if !self.gi2.desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(self.device, self.gi2.desc_layout, std::ptr::null());
                self.gi2.desc_layout = std::ptr::null_mut();
            }
        }
        self.gi2.ready = false;
    }
}
