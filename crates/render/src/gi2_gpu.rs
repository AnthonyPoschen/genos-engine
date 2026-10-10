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
/// Screen probe tile at 720p and up, pixels (Lumen's size).
pub(crate) const GI2_TILE: u32 = 16;

/// Screen probe tile for a picture `height` pixels tall: GI2_TILE from 720p up;
/// below that the tile shrinks with the picture (down to 4 px) so the probes keep
/// the same spacing on screen, about 45 rows. A small picture would otherwise get
/// probes a ninth of its height apart and lose detail a 720p picture keeps.
/// GENOS_GI2_TILE (4..64) overrides it to experiment.
fn gi2_tile(height: u32) -> u32 {
    std::env::var("GENOS_GI2_TILE")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map_or((height / 45).clamp(4, GI2_TILE), |t| t.clamp(4, 64))
}

/// Rays per screen probe (Low in the design; a square for the stratified pattern).
pub(crate) const GI2_RAYS: u32 = 64;
/// Light cache sizes; gi2_cache_slots.glsl has the same numbers.
const GI2_CACHE_SLOTS: u64 = 262144;
const GI2_CACHE_BATCH: u32 = 16384;
const GI2_CACHE_RAYS: u32 = 16;
/// Strata of the screen probes' ray directions, one traced per frame, all kept
/// (gi2_common.glsl GI2_STRATA): the visibility history cap.
pub(crate) const GI2_STRATA: u32 = 8;
/// Bounds of moved objects one frame can list (gi2_common.glsl GI2_CHANGES); past
/// that every kept stratum is dropped.
const GI2_CHANGES: usize = 64;
/// Vec4s before the bounds in the changes block (gi2_common.glsl GI2_CHANGE_HEAD).
const GI2_CHANGE_HEAD: usize = 5;
/// GI v2 counts as settled once the scene block (camera included) has stayed the
/// same for this many frames (the screen probes average up to 32 frames' rays,
/// gi2_gather.comp) and a full sweep of the light cache changed no patch
/// by more than GI2_SETTLE_CHANGE.
const GI2_SETTLE_FRAMES: u32 = 32;
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
    /// The work buffer is new: zeroed first, so no probe keeps a stratum.
    work_clear: bool,
    /// The traced world as of the last frame's passes: the scene block (its
    /// occluder list is compared, gi2_changes) and the key of what else is traced
    /// (meshes, floor, roof); a change of the key drops every kept stratum.
    kept_bytes: Vec<u8>,
    kept_key: u64,
    /// Key of the traced world besides the occluders (set with the scene upload).
    pub(crate) trace_key: u64,
    /// Per frame slot, read back: rays the screen probes held and probes placed
    /// (gi2_gather.comp). The last frame read and its slot.
    probe_samples: [u32; 2],
    last_slot: usize,
    /// Frames the probes ran: which views block is last frame's (its parity).
    probe_runs: u32,
    frame: u32,
    /// Hash of what GI v2 sees (scene block, traced and drawn meshes), and frames
    /// it has stayed the same.
    scene_hash: u64,
    still: u32,
    /// Hash of this frame's draw instances (poses, colours, emission).
    pub(crate) draw_key: u64,
    /// Frames in a row drawn from held light (settled): the picture no longer changes.
    held: u32,
    /// Per frame slot: the cache pass's counters (gi2_cache.glsl `CacheStats`), and
    /// `still` as it was when that slot's frame ran (0: the scene had just changed).
    stats: [Buffer; 2],
    stats_still: [u32; 2],
    /// Frames in a row whose relit patches all stayed within GI2_SETTLE_CHANGE, and
    /// the live patch count last read back.
    quiet: u32,
    live: u32,
    dims: [u32; 4],
    /// The light cache's passes (compact, its rays' trace and light, relight) run on
    /// the async compute queue (GENOS_GI2_ASYNC=0 keeps them in the frame): at most
    /// one submission in flight, skipped while the last is still running.
    async_on: bool,
    world_cmd: Handle,
    world_fence: Handle,
    world_set: Handle,
    world_qp: Handle,
    /// Its own copy of the scene block and counters, both queues' sharing.
    world_scene: Buffer,
    world_stats: Buffer,
    world_busy: bool,
    /// What the submission in flight was built from: scene hash and still frames.
    world_hash: u64,
    world_still: u32,
    /// Light cache rounds run (gi2_round), and frames to hold off after the cache
    /// buffers were cleared on the graphics queue.
    world_round: u32,
    world_hold: u32,
    /// Hash of the scene's light without the camera (lamps, occluders, fire, smoke,
    /// floor, roof, sky; set with the scene upload), and with the traced and drawn
    /// meshes: what the light cache's light depends on.
    pub(crate) light_scene: u64,
    light_hash: u64,
    /// Rounds in a row, since that last changed, whose mature patches all stayed
    /// within GI2_SETTLE_CHANGE: past a full sweep, rounds relight young patches only.
    light_quiet: u32,
    young_ok: bool,
    /// What each round in flight was built from (frame slots, then the async one):
    /// the light hash and whether it took young patches only.
    stats_light: [u64; 2],
    stats_young: [bool; 2],
    world_light: u64,
    world_young: bool,
    /// Light cache rounds per second of wall time (GENOS_GI2_CACHE_HZ; 0: one every
    /// frame), when the last began, and whether each frame slot ran one.
    cache_hz: f64,
    last_round: Option<Instant>,
    stats_round: [bool; 2],
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
            work_clear: false,
            kept_bytes: Vec::new(),
            kept_key: 0,
            trace_key: 0,
            probe_samples: [0; 2],
            last_slot: 0,
            probe_runs: 0,
            frame: 0,
            scene_hash: 0,
            draw_key: 0,
            held: 0,
            still: 0,
            stats: [Buffer::empty(), Buffer::empty()],
            quiet: 0,
            stats_still: [0; 2],
            live: 0,
            dims: [0; 4],
            async_on: std::env::var("GENOS_GI2_ASYNC").is_ok_and(|v| v == "1"),
            world_cmd: std::ptr::null_mut(),
            world_fence: std::ptr::null_mut(),
            world_set: std::ptr::null_mut(),
            world_qp: std::ptr::null_mut(),
            world_scene: Buffer::empty(),
            world_stats: Buffer::empty(),
            world_busy: false,
            world_hash: 0,
            world_still: 0,
            world_round: 0,
            world_hold: 0,
            light_scene: 0,
            light_hash: 0,
            light_quiet: 0,
            young_ok: std::env::var("GENOS_GI2_YOUNG_ONLY").map_or(true, |v| v != "0"),
            stats_light: [0; 2],
            stats_young: [false; 2],
            world_light: 0,
            world_young: false,
            cache_hz: std::env::var("GENOS_GI2_CACHE_HZ")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            last_round: None,
            stats_round: [false; 2],
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
            let size = Size { kind: 7, count: 24 };
            let pool = PoolInfo {
                s_type: 33,
                next: std::ptr::null(),
                flags: 0,
                max_sets: 3,
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
            let layouts = [self.gi2.desc_layout; 3];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: self.gi2.pool,
                count: 3,
                layouts: layouts.as_ptr(),
            };
            let mut sets = [std::ptr::null_mut(); 3];
            check(
                (self.fns.alloc_desc)(
                    self.device,
                    &alloc as *const Alloc as *const u8,
                    sets.as_mut_ptr(),
                ),
                "gi2 descriptor sets",
            )?;
            self.gi2.sets = [sets[0], sets[1]];
            self.gi2.world_set = sets[2];
        }
        self.make_gi2_world()?;
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

    /// The async light cache submission's command buffer (compute family), its
    /// fence (signalled: nothing in flight) and timestamps.
    fn make_gi2_world(&mut self) -> Result<(), String> {
        #[repr(C)]
        struct AllocInfo {
            s_type: i32,
            next: *const c_void,
            pool: Handle,
            level: u32,
            count: u32,
        }
        #[repr(C)]
        struct FenceInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
        }
        if self.light_pool.is_null() {
            self.gi2.async_on = false;
            return Ok(());
        }
        let alloc = AllocInfo {
            s_type: 40,
            next: std::ptr::null(),
            pool: self.light_pool,
            level: 0,
            count: 1,
        };
        let fence = FenceInfo {
            s_type: 8,
            next: std::ptr::null(),
            flags: 0x1,
        };
        unsafe {
            check(
                (self.fns.alloc_cmd)(
                    self.device,
                    &alloc as *const AllocInfo as *const u8,
                    &mut self.gi2.world_cmd,
                ),
                "gi2 world command buffer",
            )?;
            check(
                (self.fns.create_fence)(
                    self.device,
                    &fence as *const FenceInfo as *const u8,
                    std::ptr::null(),
                    &mut self.gi2.world_fence,
                ),
                "gi2 world fence",
            )?;
        }
        self.gi2.world_qp = self.make_query_pool_n(2)?;
        Ok(())
    }

    /// Wait for the async light cache submission in flight, if any.
    fn wait_gi2_world(&mut self) {
        if !self.gi2.world_fence.is_null() {
            let fences = [self.gi2.world_fence];
            unsafe {
                (self.fns.wait_fences)(self.device, 1, fences.as_ptr(), 1, u64::MAX);
            }
        }
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
        self.wait_gi2_world();
        self.poll_gi2_world();
        let (w, h) = (self.extent_w.max(1), self.extent_h.max(1));
        let tile = gi2_tile(h);
        let cols = w.div_ceil(tile);
        let rows = h.div_ceil(tile);
        let probes = (cols * rows) as u64;
        let pixels = w as u64 * h as u64;
        // Probes (2), one slot per light cache ray (hit, then its light), filtered SH
        // (7), strata (2 per probe and stratum), kept hits and their light (a uint
        // each), the changes block, the views (2 blocks of 2 per probe) and the
        // cells (2 tables of R per probe), then the fills and their light (as the
        // kept hits) (gi2_common.glsl).
        let kept = probes * GI2_RAYS as u64 * GI2_STRATA as u64;
        let cache_rays = (GI2_CACHE_BATCH * GI2_CACHE_RAYS) as u64;
        let work_vec4 = gi2_change_at(probes, cache_rays, kept)
            + (GI2_CHANGE_HEAD + GI2_CHANGES) as u64
            + 4 * probes
            + 2 * GI2_RAYS as u64 * probes
            + kept
            + kept / 4;
        let mut gbuf = std::mem::replace(&mut self.gi2.gbuf, Buffer::empty());
        let mut work = std::mem::replace(&mut self.gi2.work, Buffer::empty());
        let mut out = std::mem::replace(&mut self.gi2.out, Buffer::empty());
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        // Both queues use the light cache and the work buffer's ray slots.
        let share = self.light_families();
        if self.gi2.cache_keys.buffer.is_null() {
            self.gi2.cache_keys = self.make_buffer_queues(
                (2 * GI2_CACHE_SLOTS + 2) * 4,
                0x20 | 0x2,
                Memory::Device,
                &share,
            )?;
            self.gi2.cache =
                self.make_buffer_queues(GI2_CACHE_SLOTS * 48, 0x20 | 0x2, Memory::Device, &share)?;
            self.gi2.cache_clear = true;
            for k in 0..2 {
                self.gi2.stats[k] = self.make_buffer(32, 0x20 | 0x2, Memory::Readback)?;
            }
            self.gi2.world_stats =
                self.make_buffer_queues(32, 0x20 | 0x2, Memory::Readback, &share)?;
            self.gi2.world_scene = self.make_buffer_queues(
                crate::pack::SCENE_CAPACITY as u64,
                0x20,
                Memory::Upload,
                &share,
            )?;
            self.write_buffer(&self.gi2.world_scene, &vec![0u8; crate::pack::SCENE_TAIL])?;
        }
        self.gi2.gbuf = self.make_buffer(16 + pixels * 16, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.gi2.work =
            self.make_buffer_queues(work_vec4 * 16, 0x20 | 0x1 | 0x2, Memory::Device, &share)?;
        self.gi2.out = self.make_buffer(pixels * 4, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.gi2.dims = [w, h, cols, rows];
        self.gi2.gbuf_clear = true;
        self.gi2.work_clear = true;
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
        for slot in 0..3 {
            let (scene, stats, set) = if slot < 2 {
                (
                    self.frame_scene[slot].buffer,
                    self.gi2.stats[slot].buffer,
                    self.gi2.sets[slot],
                )
            } else {
                (
                    self.gi2.world_scene.buffer,
                    self.gi2.world_stats.buffer,
                    self.gi2.world_set,
                )
            };
            if set.is_null() {
                continue;
            }
            let buffers = [
                (0u32, scene),
                (5, self.mesh_field.buffer),
                (6, self.gi2.gbuf.buffer),
                (7, self.gi2.work.buffer),
                (8, self.gi2.out.buffer),
                (9, self.gi2.cache_keys.buffer),
                (10, self.gi2.cache.buffer),
                (11, stats),
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
                    set,
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

    fn gi2_still_frames(&self) -> u32 {
        self.gi2.still
    }

    /// Rays the screen probes held in the last frame drawn, and probes placed
    /// (gi2_gather.comp): waits for that frame.
    pub(crate) fn gi2_probe_samples(&mut self) -> [u32; 2] {
        unsafe {
            (self.fns.device_wait)(self.device);
        }
        self.read_gi2_stats(self.gi2.last_slot)
            .map_or(self.gi2.probe_samples, |w| [w[6], w[7]])
    }

    /// The picture has stopped changing: a frame drawn from held light has been
    /// submitted after another, so one is on screen.
    fn gi2_picture_held(&self) -> bool {
        self.gi2.held >= 2
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
        let world = self.gi2.async_on;
        if world {
            self.poll_gi2_world();
        }
        // This slot's last frame is done (its fence was waited): read its counters
        // (the async queue's come back with its own submission).
        if let Ok(words) = self.read_gi2_stats(slot) {
            self.gi2.probe_samples = [words[6], words[7]];
            if words[7] > 0 && std::env::var_os("GENOS_GI_DEBUG").is_some() {
                eprintln!(
                    "gi2 probes rays {} probes {} per_probe {:.1}",
                    words[6],
                    words[7],
                    words[6] as f64 / words[7] as f64
                );
            }
        }
        if !self.gi2.cache_clear && !world {
            if !self.gi2.stats_round[slot] {
                // No light cache round ran in that frame.
            } else if let Ok(words) = self.read_gi2_stats(slot) {
                // A frame that ran just as the scene changed measured against the
                // old light; it does not count toward settling.
                let light = self.gi2.stats_light[slot] == self.gi2.light_hash;
                self.note_gi2_round(
                    words,
                    self.gi2.stats_still[slot] != 0,
                    light,
                    self.gi2.stats_young[slot],
                );
            }
        }
        unsafe {
            (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.stats[slot].buffer, 0, 32, 0);
            if self.gi2.cache_clear {
                self.gi2.cache_clear = false;
                // The async queue starts once this frame (its slot) is surely done.
                self.gi2.world_hold = 2;
                (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.cache_keys.buffer, 0, u64::MAX, 0);
                (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.cache.buffer, 0, u64::MAX, 0);
            }
            if self.gi2.work_clear {
                self.gi2.work_clear = false;
                (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.work.buffer, 0, u64::MAX, 0);
            }
            if self.gi2.gbuf_clear {
                self.gi2.gbuf_clear = false;
                // Every record empty (distance all ones); the header goes in below.
                (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.gbuf.buffer, 0, u64::MAX, u32::MAX);
            }
            // The whole-buffer clears land before the count and header writes.
            self.memory_barrier(0x1000, 0x1000, 0x1000, 0x1000);
            // The mature and young patch counts, rebuilt by the compact pass.
            if !world {
                for at in [GI2_CACHE_SLOTS, 2 * GI2_CACHE_SLOTS + 1] {
                    (self.fns.cmd_fill_buffer)(self.cmd, self.gi2.cache_keys.buffer, at * 4, 4, 0);
                }
            }
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

    /// Count one light cache round's counters toward settling (or not): a round
    /// built from an older scene than the current one measured the old light.
    /// `current`: built from the scene as it is now (camera included); `light`:
    /// from its light as it is now; `young_only`: the round took young patches only,
    /// so one that relit nothing found nothing left to light.
    fn note_gi2_round(&mut self, words: [u32; 8], current: bool, light: bool, young_only: bool) {
        if !current {
            self.gi2.quiet = 0;
        } else if words[1] > 0 {
            if words[0] <= GI2_SETTLE_CHANGE {
                self.gi2.quiet = self.gi2.quiet.saturating_add(1);
            } else {
                self.gi2.quiet = 0;
            }
        } else if young_only {
            self.gi2.quiet = self.gi2.quiet.saturating_add(1);
        }
        if !light {
            self.gi2.light_quiet = 0;
        } else if words[5] > 0 {
            if words[4] <= GI2_SETTLE_CHANGE {
                self.gi2.light_quiet = self.gi2.light_quiet.saturating_add(1);
            } else {
                self.gi2.light_quiet = 0;
            }
        }
        self.gi2.live = words[2];
        if std::env::var_os("GENOS_GI_DEBUG").is_some() {
            eprintln!(
                "gi2 frame {} still {} change {} relit {} live {} missed {} quiet {} mature_change {} mature_relit {} light_quiet {} young_only {}",
                self.gi2.frame,
                self.gi2.still,
                words[0],
                words[1],
                words[2],
                words[3],
                self.gi2.quiet,
                words[4],
                words[5],
                self.gi2.light_quiet,
                young_only
            );
        }
    }

    /// The async light cache round in flight has finished: read its counters and
    /// GPU span.
    fn poll_gi2_world(&mut self) {
        if !self.gi2.world_busy {
            return;
        }
        let status = unsafe { (self.fns.fence_status)(self.device, self.gi2.world_fence) };
        if status != 0 {
            return;
        }
        self.gi2.world_busy = false;
        if let Ok(words) = self.read_stats_memory(self.gi2.world_stats.memory) {
            let current = self.gi2.world_still > 0 && self.gi2.world_hash == self.gi2.scene_hash;
            let light = self.gi2.world_light == self.gi2.light_hash;
            self.note_gi2_round(words, current, light, self.gi2.world_young);
        }
        if self.gpu_times.is_some() && !self.gi2.world_qp.is_null() {
            let mut raw = [0u64; 2];
            let got = unsafe {
                (self.fns.get_query_results)(
                    self.device,
                    self.gi2.world_qp,
                    0,
                    2,
                    16,
                    raw.as_mut_ptr() as *mut c_void,
                    8,
                    0x1,
                )
            };
            let period = f64::from(self.timestamp_period);
            let bits = self.timestamp_bits;
            if got == 0 {
                if let Some(times) = self.gpu_times.as_mut() {
                    times.light_spans.push((raw[0], raw[1]));
                    times.world_ms += tick_delta(raw[0], raw[1], bits) as f64 * period / 1.0e6;
                    times.world_runs += 1;
                }
            }
        }
    }

    /// Record and submit one light cache round on the async compute queue: list
    /// the live patches, trace and light this round's batch of their rays, relight
    /// them. Skipped while the last round still runs, so at most one is in flight
    /// and a slow round never queues up work behind it.
    fn submit_gi2_world(&mut self, pc: [u32; 8]) -> Result<bool, String> {
        if self.gi2.world_busy || self.gi2.world_cmd.is_null() {
            return Ok(false);
        }
        if self.gi2.world_hold > 0 {
            self.gi2.world_hold -= 1;
            return Ok(false);
        }
        if !self.frame_bytes.is_empty() {
            self.write_buffer(&self.gi2.world_scene, &self.frame_bytes)?;
        }
        let round = self.gi2.world_round;
        self.gi2.world_round = round.wrapping_add(1);
        let mut pc = pc;
        // Light cache rays only (gi2_mode 2) and this round.
        pc[6] = (pc[6] & 0xcff) | 2 << 8 | (round & 0xf_ffff) << 12;
        let cmd = self.gi2.world_cmd;
        let fences = [self.gi2.world_fence];
        #[repr(C)]
        struct BeginInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            inherit: *const c_void,
        }
        #[repr(C)]
        struct Submit {
            s_type: i32,
            next: *const c_void,
            wait_count: u32,
            waits: *const Handle,
            stages: *const u32,
            cmd_count: u32,
            cmds: *const Handle,
            signal_count: u32,
            signals: *const Handle,
        }
        let rays = GI2_CACHE_BATCH * GI2_CACHE_RAYS;
        let groups = [
            (0usize, (GI2_CACHE_SLOTS as u32).div_ceil(64)),
            (2, rays.div_ceil(64)),
            (3, rays.div_ceil(64)),
            (6, GI2_CACHE_BATCH.div_ceil(64)),
        ];
        unsafe {
            check((self.fns.reset_fences)(self.device, 1, fences.as_ptr()), "reset gi2 world fence")?;
            check((self.fns.reset_cmd)(cmd, 0), "reset gi2 world cmd")?;
            let begin = BeginInfo {
                s_type: 42,
                next: std::ptr::null(),
                flags: 1,
                inherit: std::ptr::null(),
            };
            check(
                (self.fns.begin_cmd)(cmd, &begin as *const BeginInfo as *const u8),
                "begin gi2 world",
            )?;
            let qp = self.gi2.world_qp;
            if !qp.is_null() {
                (self.fns.cmd_reset_query)(cmd, qp, 0, 2);
                (self.fns.cmd_write_timestamp)(cmd, 0x1, qp, 0);
            }
            (self.fns.cmd_fill_buffer)(cmd, self.gi2.world_stats.buffer, 0, 32, 0);
            // The mature and young counts, rebuilt by the compact pass.
            (self.fns.cmd_fill_buffer)(cmd, self.gi2.cache_keys.buffer, GI2_CACHE_SLOTS * 4, 4, 0);
            (self.fns.cmd_fill_buffer)(
                cmd,
                self.gi2.cache_keys.buffer,
                (2 * GI2_CACHE_SLOTS + 1) * 4,
                4,
                0,
            );
            self.barrier_on(cmd, 0x1000, 0x800, 0x1000, 0x20 | 0x40);
            let set = self.gi2.world_set;
            (self.fns.cmd_bind_set)(cmd, 1, self.gi2.layout, 0, 1, &set, 0, std::ptr::null());
            (self.fns.cmd_push)(cmd, self.gi2.layout, 0x20, 0, 32, pc.as_ptr() as *const c_void);
            for (pass, gx) in groups {
                (self.fns.cmd_bind_pipe)(cmd, 1, self.gi2.pipes[pass]);
                (self.fns.cmd_dispatch)(cmd, gx.max(1), 1, 1);
                self.barrier_on(cmd, 0x800, 0x800, 0x40, 0x20 | 0x40);
            }
            // The counters reach the host.
            self.barrier_on(cmd, 0x800, 0x4000, 0x40, 0x2000);
            if !qp.is_null() {
                (self.fns.cmd_write_timestamp)(cmd, 0x2000, qp, 1);
            }
            check((self.fns.end_cmd)(cmd), "end gi2 world")?;
            let cmds = [cmd];
            let submit = Submit {
                s_type: 4,
                next: std::ptr::null(),
                wait_count: 0,
                waits: std::ptr::null(),
                stages: std::ptr::null(),
                cmd_count: 1,
                cmds: cmds.as_ptr(),
                signal_count: 0,
                signals: std::ptr::null(),
            };
            check(
                (self.fns.queue_submit)(
                    self.compute_queue,
                    1,
                    &submit as *const Submit as *const u8,
                    self.gi2.world_fence,
                ),
                "gi2 world submit",
            )?;
        }
        self.gi2.world_busy = true;
        self.gi2.world_hash = self.gi2.scene_hash;
        self.gi2.world_still = self.gi2.still;
        self.gi2.world_light = self.gi2.light_hash;
        self.gi2.world_young = pc[6] & 1 << 10 != 0;
        Ok(true)
    }

    fn read_gi2_stats(&self, slot: usize) -> Result<[u32; 8], String> {
        self.read_stats_memory(self.gi2.stats[slot].memory)
    }

    fn read_stats_memory(&self, memory: Handle) -> Result<[u32; 8], String> {
        let mut words = [0u32; 8];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    memory,
                    0,
                    32,
                    0,
                    &mut mapped,
                ),
                "map gi2 stats",
            )?;
            std::ptr::copy_nonoverlapping(mapped as *const u32, words.as_mut_ptr(), 8);
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(words)
    }

    /// After the raster: the passes, then the result into the colour image.
    fn gi2_passes(&mut self, slot: usize) {
        let [w, h, cols, rows] = self.gi2.dims;
        let probes = cols * rows;
        let probe_rays = probes * GI2_RAYS;
        let rays = probe_rays + GI2_CACHE_BATCH * GI2_CACHE_RAYS;
        let bgra = if self.format == 44 { 1u32 } else { 0 };
        self.gi2.frame = self.gi2.frame.wrapping_add(1);
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            // Lamps, occluders and camera; the traced meshes; the drawn ones.
            self.frame_bytes.hash(&mut hasher);
            self.mesh_field_key.hash(&mut hasher);
            self.gi2.draw_key.hash(&mut hasher);
            hasher.finish()
        };
        if hash == self.gi2.scene_hash {
            self.gi2.still = self.gi2.still.saturating_add(1);
        } else {
            self.gi2.scene_hash = hash;
            self.gi2.still = 0;
            self.gi2.quiet = 0;
        }
        self.gi2.stats_still[slot] = self.gi2.still;
        self.gi2.last_slot = slot;
        // What moved in the traced world since the last frame: the probes drop the
        // kept rays that cross it (gi2_gather.comp). Light needs nothing here: every
        // kept hit is shaded again each frame.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.gi2.trace_key.hash(&mut hasher);
            self.mesh_field_key.hash(&mut hasher);
            hasher.finish()
        };
        let frozen = self.gi2_stats().seen_settled == 1;
        let had = !self.gi2.kept_bytes.is_empty() && key == self.gi2.kept_key;
        let changes = if had {
            gi2_changes(&self.gi2.kept_bytes, &self.frame_bytes)
        } else {
            None
        };
        if std::env::var_os("GENOS_GI_DEBUG").is_some() {
            if frozen {
                eprintln!("gi2 frozen");
            } else {
                eprintln!(
                    "gi2 kept had {had} changes {}",
                    changes.as_ref().map_or(-1, |c| c.len() as i64)
                );
            }
        }
        let mut block = vec![0f32; 4 * (GI2_CHANGE_HEAD + GI2_CHANGES)];
        block[2] = (self.gi2.probe_runs & 1) as f32;
        match &changes {
            None => block[1] = 1.0,
            Some(list) => {
                block[0] = list.len() as f32;
                for (j, b) in list.iter().enumerate() {
                    let at = 4 * (GI2_CHANGE_HEAD + j);
                    block[at..at + 4].copy_from_slice(b);
                }
            }
        }
        if had {
            // Last frame's camera (scene_data.glsl): eye at byte 48, the basis at 528.
            let f = |at: usize| {
                let b = &self.gi2.kept_bytes[at..at + 4];
                f32::from_le_bytes([b[0], b[1], b[2], b[3]])
            };
            for (v, at) in [(1usize, 48usize), (2, 528), (3, 544), (4, 560)] {
                for c in 0..4 {
                    block[4 * v + c] = f(at + 4 * c);
                }
            }
            block[4 + 3] = 1.0;
        }
        // A frame drawn from held light runs no probes: what it saw is still what
        // the probes last ran on.
        if !frozen {
            self.gi2.kept_key = key;
            self.gi2.kept_bytes.clone_from(&self.frame_bytes);
            self.gi2.probe_runs = self.gi2.probe_runs.wrapping_add(1);
        }
        let block_at = 16
            * gi2_change_at(
                probes as u64,
                (GI2_CACHE_BATCH * GI2_CACHE_RAYS) as u64,
                probe_rays as u64 * GI2_STRATA as u64,
            );
        let used = 4 * (GI2_CHANGE_HEAD + changes.as_ref().map_or(0, |l| l.len()));
        unsafe {
            // The last frame's passes are done with the block before it is written.
            self.memory_barrier(0x800, 0x1000, 0x20 | 0x40, 0x1000);
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.gi2.work.buffer,
                block_at,
                used as u64 * 4,
                block.as_ptr() as *const c_void,
            );
            self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
        }
        // The light alone (no camera): while it holds and a full sweep has changed
        // no mature patch, rounds relight young patches only. Any change goes back
        // to full sweeps at once.
        let light = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.gi2.light_scene.hash(&mut hasher);
            self.mesh_field_key.hash(&mut hasher);
            self.gi2.draw_key.hash(&mut hasher);
            hasher.finish()
        };
        if light != self.gi2.light_hash {
            self.gi2.light_hash = light;
            self.gi2.light_quiet = 0;
        }
        let sweep = self.gi2.live.div_ceil(GI2_CACHE_BATCH).max(1) + 1;
        let young_only = self.gi2.young_ok && self.gi2.light_quiet >= sweep;
        self.gi2.stats_light[slot] = light;
        self.gi2.stats_young[slot] = young_only;
        // z: bgra, fills (bit 6), young-only rounds and lamp picks (gi2_common.glsl);
        // the gather and filter passes add their round (bits 1-2) below.
        let picks = std::env::var("GENOS_GI2_CACHE_PICKS").is_ok_and(|v| v == "1");
        let fill = std::env::var("GENOS_GI2_FILL").map_or(true, |v| v != "0");
        let z = bgra
            | u32::from(fill) << 6
            | u32::from(young_only) << 10
            | u32::from(picks) << 11;
        let pc = [w, h, cols, rows, GI2_RAYS, gi2_tile(h), z, self.gi2.frame];
        let image = self.color.image;
        // The raster colour as the base: pixels with no surface keep it.
        self.copy_image_buffer(image, 6, self.gi2.out.buffer, w, h, true);
        self.memory_barrier(0x80 | 0x1000, 0x800, 0x40 | 0x1000, 0x20 | 0x40);
        let set = self.gi2.sets[slot];
        // Settled (frozen above): the probes and the light cache hold their light,
        // and only the pixels' direct light and the final pass run, so a still
        // scene draws the same picture every frame (no residual cache drift) and
        // idles cheaply.
        self.gi2.held = if frozen { self.gi2.held.saturating_add(1) } else { 0 };
        // A light cache round is due (GENOS_GI2_CACHE_HZ caps them per second). On
        // the async queue (GENOS_GI2_ASYNC=1) it goes there, else into this frame;
        // a frame without one runs the picture's passes only (gi2_mode 1).
        let due = !frozen
            && (self.gi2.cache_hz <= 0.0
                || self
                    .gi2
                    .last_round
                    .is_none_or(|t| t.elapsed().as_secs_f64() * self.gi2.cache_hz >= 1.0));
        let mut round = false;
        if due && self.gi2.async_on {
            match self.submit_gi2_world(pc) {
                Ok(true) => self.gi2.last_round = Some(Instant::now()),
                Ok(false) => {}
                Err(e) => {
                    eprintln!("gi2: async light cache off: {e}");
                    self.gi2.async_on = false;
                }
            }
        } else if due {
            round = true;
            self.gi2.last_round = Some(Instant::now());
        }
        self.gi2.stats_round[slot] = round;
        let mut pc = pc;
        if round {
            let n = self.gi2.world_round;
            self.gi2.world_round = n.wrapping_add(1);
            pc[6] |= (n & 0xf_ffff) << 12;
        } else {
            pc[6] |= 1 << 8;
        }
        let kept = probe_rays * GI2_STRATA;
        let groups: Vec<(usize, u32, u32)> = if frozen {
            vec![(3, (w * h).div_ceil(64), 1), (7, w.div_ceil(8), h.div_ceil(8))]
        } else if !round {
            vec![
                (1usize, probes.div_ceil(64), 1u32),
                // The picks (gather's first round), this frame's rays, then one thread
                // per kept hit (re-traced where it crossed a change) and per fill slot
                // (gi2_trace.comp).
                (4, probes.div_ceil(64), 1),
                (2, (probe_rays + 2 * kept).div_ceil(64), 1),
                (3, (w * h + 2 * kept).div_ceil(64), 1),
                (4, probes.div_ceil(64), 1),
                (5, probes.div_ceil(64), 1),
                (5, probes.div_ceil(64), 1),
                (5, probes.div_ceil(64), 1),
                (7, w.div_ceil(8), h.div_ceil(8)),
            ]
        } else {
            vec![
                (0usize, (GI2_CACHE_SLOTS as u32).div_ceil(64), 1u32),
                (1, probes.div_ceil(64), 1),
                (4, probes.div_ceil(64), 1),
                (2, (rays + 2 * kept).div_ceil(64), 1),
                (3, (w * h + 2 * kept + rays - probe_rays).div_ceil(64), 1),
                (4, probes.div_ceil(64), 1),
                (5, probes.div_ceil(64), 1),
                (5, probes.div_ceil(64), 1),
                (5, probes.div_ceil(64), 1),
                (6, GI2_CACHE_BATCH.div_ceil(64), 1),
                (7, w.div_ceil(8), h.div_ceil(8)),
            ]
        };
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
            // GENOS_GPU_TIMES: a stamp before the passes and after each (full frames).
            let stamps = self
                .gpu_times
                .as_mut()
                .filter(|t| !t.gi2_pool.is_null())
                .map(|t| {
                    t.gi2_ids[slot] = groups.iter().map(|g| g.0).collect();
                    t.gi2_pool
                });
            let first = slot as u32 * GI2_STAMPS;
            if let Some(pool) = stamps {
                (self.fns.cmd_reset_query)(self.cmd, pool, first, GI2_STAMPS);
                (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, first);
            }
            let mut filter_round = 0u32;
            let mut picked = false;
            for (k, (pass, gx, gy)) in groups.iter().copied().enumerate() {
                if pass == 4 {
                    // The gather's two rounds: the picks (bit 1), then the binning.
                    let mut round_pc = pc;
                    round_pc[6] |= u32::from(!picked) << 1;
                    picked = true;
                    (self.fns.cmd_push)(
                        self.cmd,
                        self.gi2.layout,
                        0x20,
                        0,
                        32,
                        round_pc.as_ptr() as *const c_void,
                    );
                }
                if pass == 5 {
                    // The spatial filter's three rounds (gi2_filter_round).
                    let mut round_pc = pc;
                    round_pc[6] |= filter_round << 1;
                    filter_round += 1;
                    (self.fns.cmd_push)(
                        self.cmd,
                        self.gi2.layout,
                        0x20,
                        0,
                        32,
                        round_pc.as_ptr() as *const c_void,
                    );
                }
                (self.fns.cmd_bind_pipe)(self.cmd, 1, self.gi2.pipes[pass]);
                (self.fns.cmd_dispatch)(self.cmd, gx.max(1), gy.max(1), 1);
                self.memory_barrier(0x800, 0x800 | 0x1000, 0x40, 0x20 | 0x40 | 0x800);
                if let Some(pool) = stamps {
                    (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, first + 1 + k as u32);
                }
            }
        }
        self.image_barrier(image, 6, 7, 0x1000, 0x1000, 0x800, 0x1000);
        self.copy_image_buffer(image, 7, self.gi2.out.buffer, w, h, false);
        self.image_barrier(image, 7, 6, 0x1000, 0x1000 | 0x800, 0x1000, 0x800 | 0x20);
    }

    fn memory_barrier(&self, src_stage: u32, dst_stage: u32, src_access: u32, dst_access: u32) {
        self.barrier_on(self.cmd, src_stage, dst_stage, src_access, dst_access);
    }

    fn barrier_on(
        &self,
        cmd: Handle,
        src_stage: u32,
        dst_stage: u32,
        src_access: u32,
        dst_access: u32,
    ) {
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
                cmd,
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
        self.wait_gi2_world();
        self.gi2.world_busy = false;
        let mut world_stats = std::mem::replace(&mut self.gi2.world_stats, Buffer::empty());
        let mut world_scene = std::mem::replace(&mut self.gi2.world_scene, Buffer::empty());
        self.destroy_buffer(&mut world_stats);
        self.destroy_buffer(&mut world_scene);
        unsafe {
            if !self.gi2.world_fence.is_null() {
                (self.fns.destroy_fence)(self.device, self.gi2.world_fence, std::ptr::null());
                self.gi2.world_fence = std::ptr::null_mut();
            }
        }
        // The command buffer goes with the light command pool; the query pool too
        // is left to the device (as the frame's are).
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

/// Vec4 index of the changes block in the work buffer (gi2_common.glsl
/// gi2_change_at), for `probes` probes, `cache_rays` light cache rays and `kept`
/// kept hits.
fn gi2_change_at(probes: u64, cache_rays: u64, kept: u64) -> u64 {
    2 * probes + cache_rays + 7 * probes + 2 * GI2_STRATA as u64 * probes + kept + kept / 4
}

/// Spheres (centre, radius) around every occluder that moved, came or went between
/// two scene blocks (`pack::scene_bytes`): the bounds before and after, so a kept
/// ray that crosses either sees something else now. None when there are more than
/// GI2_CHANGES (drop every kept ray instead). Lamps are not compared: kept hits are
/// shaded again every frame, so light needs no change list.
pub(crate) fn gi2_changes(old: &[u8], new: &[u8]) -> Option<Vec<[f32; 4]>> {
    fn occs(bytes: &[u8]) -> Vec<&[u8]> {
        let word = |at: usize| {
            bytes
                .get(at * 4..at * 4 + 4)
                .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) as usize
        };
        let start = crate::pack::SCENE_TAIL + word(0) * 32;
        (0..word(1))
            .filter_map(|i| bytes.get(start + i * 64..start + i * 64 + 64))
            .collect()
    }
    fn bounds(rec: &[u8]) -> [f32; 4] {
        let f = |i: usize| f32::from_le_bytes([rec[i * 4], rec[i * 4 + 1], rec[i * 4 + 2], rec[i * 4 + 3]]);
        // center_shape xyz; extent: x and z half extents, y full height, w radius.
        let (hx, hy, hz, r) = (f(4).abs(), f(5).abs() * 0.5, f(6).abs(), f(7).abs());
        let (hx, hz) = (hx.max(r), hz.max(r));
        [f(0), f(1), f(2), (hx * hx + hy * hy + hz * hz).sqrt() + 0.05]
    }
    let (a, b) = (occs(old), occs(new));
    if a == b {
        return Some(Vec::new());
    }
    let left: std::collections::HashSet<&[u8]> = a.iter().copied().collect();
    let right: std::collections::HashSet<&[u8]> = b.iter().copied().collect();
    let mut out = Vec::new();
    for rec in a.iter().filter(|r| !right.contains(*r)).chain(b.iter().filter(|r| !left.contains(*r))) {
        if out.len() == GI2_CHANGES {
            return None;
        }
        out.push(bounds(rec));
    }
    Some(out)
}

/// Key of what the probes trace besides the occluders and meshes: the floor and the
/// roof (their place and colour, which kept hits hold).
pub(crate) fn gi2_trace_key(pack: &crate::pack::Pack) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut floats: Vec<f32> = Vec::new();
    floats.extend(pack.floor_center);
    floats.push(pack.floor_half_x);
    floats.push(pack.floor_half_z);
    floats.extend(pack.floor_color);
    floats.extend(pack.ceiling);
    for f in floats {
        f.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Hash of what lights the scene, without the camera: the lamp and occluder lists
/// as uploaded (the tail of `bytes`, `pack::scene_bytes`), fire, smoke, floor, roof
/// and sky.
pub(crate) fn gi2_light_scene(pack: &crate::pack::Pack, bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let word = |at: usize| {
        bytes
            .get(at * 4..at * 4 + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) as usize
    };
    let tail = crate::pack::SCENE_TAIL;
    let lists = (word(0) * 2 + word(1) * 4) * 16;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes[tail.min(bytes.len())..(tail + lists).min(bytes.len())].hash(&mut hasher);
    let mut floats: Vec<f32> = Vec::new();
    floats.extend(pack.fire.position);
    floats.extend(pack.fire.color);
    floats.push(pack.fire.strength);
    for puff in &pack.puffs {
        floats.extend(puff.center);
        floats.push(puff.radius);
        floats.push(puff.density);
    }
    floats.extend(pack.floor_center);
    floats.push(pack.floor_half_x);
    floats.push(pack.floor_half_z);
    floats.extend(pack.floor_color);
    floats.extend(pack.ceiling);
    floats.extend(pack.sky);
    for f in floats {
        f.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}
