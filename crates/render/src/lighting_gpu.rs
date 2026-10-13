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
//   cache    relight a batch of light cache patches (lighting_cache.glsl): every bounce
//            after the first, read where probe and cache rays hit
//   compose  direct + probe irradiance per pixel, tone curve, dither
//
// The result goes back into the colour image, so anti-aliasing, overlays, readback
// and present work as before. This is the only lighting: the radiance cascades and
// the world probe tier it replaced are gone (docs/systems/lighting/README.md).

/// Screen probe tile edge in pixels (Medium and above in the design).
/// Screen probe tile at 720p and up, pixels (Lumen's size).
pub(crate) const LIGHTING_TILE: u32 = 16;

/// Screen probe tile for a picture `height` pixels tall: LIGHTING_TILE from 720p up;
/// below that the tile shrinks with the picture (down to 4 px) so the probes keep
/// the same spacing on screen, about 45 rows. A small picture would otherwise get
/// probes a ninth of its height apart and lose detail a 720p picture keeps.
/// GENOS_LIGHTING_TILE (4..64) overrides it to experiment.
fn lighting_tile(height: u32) -> u32 {
    std::env::var("GENOS_LIGHTING_TILE")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map_or((height / 45).clamp(4, LIGHTING_TILE), |t| t.clamp(4, 64))
}

/// Rays per screen probe (Low in the design; a square for the stratified pattern).
pub(crate) const LIGHTING_RAYS: u32 = 64;
/// Light cache sizes; lighting_cache_slots.glsl has the same numbers.
const LIGHTING_CACHE_SLOTS: u64 = 1048576;
const LIGHTING_CACHE_BATCH: u32 = 98304;
const LIGHTING_CACHE_RAYS: u32 = 16;
/// ckeys word of the WRC refresh lists' two counts (lighting_cache.glsl LIGHTING_WRC_CHG_COUNT_AT).
const LIGHTING_WRC_COUNTS_AT: u64 = 2 * LIGHTING_CACHE_SLOTS + 2 + 66;
/// Strata of the screen probes' ray directions, one traced per frame, all kept
/// (lighting_common.glsl LIGHTING_STRATA): the visibility history cap.
pub(crate) const LIGHTING_STRATA: u32 = 8;
/// Bounds of moved objects one frame can list (lighting_common.glsl LIGHTING_CHANGES); past
/// that every kept stratum is dropped.
const LIGHTING_CHANGES: usize = 64;
/// Vec4s before the bounds in the changes block (lighting_common.glsl LIGHTING_CHANGE_HEAD).
const LIGHTING_CHANGE_HEAD: usize = 5;
/// GI v2 counts as settled once the scene block (camera included) has stayed the
/// same for this many frames (the screen probes average up to 32 frames' rays,
/// lighting_gather.comp) and a full sweep of the light cache changed no patch
/// by more than LIGHTING_SETTLE_CHANGE.
const LIGHTING_SETTLE_FRAMES: u32 = 32;
/// Largest relative change of a relit cache patch, in millionths, that counts as
/// settled (0.2 %, well under a display code).
const LIGHTING_SETTLE_CHANGE: u32 = 2000;
/// 8×8 compose blocks the quiet list can hold (lighting_cache.glsl LIGHTING_COMPOSE_CAP).
const LIGHTING_COMPOSE_CAP: u64 = 65535;

/// First key-buffer word of the quiet-compose list. Matches LIGHTING_COMPOSE_AT.
fn lighting_compose_at() -> u64 {
    2 * LIGHTING_CACHE_SLOTS
        + 2
        + 17600
        + 16
        + 3 * 65536
        + 1024
        + 32768
        + 16
        + 5 * LIGHTING_CACHE_SLOTS
        + 5 * 65537
        + 6 * 65536
        + 16
}

fn lighting_ckeys_words() -> u64 {
    // count, y, z, full flag, stamps, list, field packs, four rect words.
    lighting_compose_at() + 4 + 2 * LIGHTING_COMPOSE_CAP + 5 * LIGHTING_CACHE_SLOTS
}

fn lighting_compose_minx() -> u64 {
    lighting_compose_at() + 4 + 2 * LIGHTING_COMPOSE_CAP + LIGHTING_CACHE_SLOTS
}

pub(crate) struct Lighting {
    ready: bool,
    desc_layout: Handle,
    layout: Handle,
    pool: Handle,
    sets: [Handle; 2],
    pipes: [Handle; 9],
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
    /// The picture in the output buffer was drawn for this size. A new buffer
    /// has no picture yet, so the next compose covers every pixel.
    compose_valid: bool,
    /// The traced world as of the last frame's passes: the scene block (its
    /// occluder list is compared, lighting_changes) and the key of what else is traced
    /// (meshes, floor, roof); a change of the key drops every kept stratum.
    kept_bytes: Vec<u8>,
    kept_key: u64,
    /// Key of the traced world besides the occluders (set with the scene upload).
    pub(crate) trace_key: u64,
    /// Per frame slot, read back: rays the screen probes held and probes placed
    /// (lighting_gather.comp). The last frame read and its slot.
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
    /// Per frame slot: the cache pass's counters (lighting_cache.glsl `CacheStats`), and
    /// `still` as it was when that slot's frame ran (0: the scene had just changed).
    stats: [Buffer; 2],
    // S78W: the shaders' stats live in device memory (atomics on host-visible
    // memory went over PCIe: the gather's two per-probe atomics cost 0.85 ms);
    // copied to `stats` at the end of the frame's passes.
    stats_dev: [Buffer; 2],
    stats_still: [u32; 2],
    /// Frames in a row whose relit patches all stayed within LIGHTING_SETTLE_CHANGE, and
    /// the live patch count last read back.
    quiet: u32,
    live: u32,
    dims: [u32; 4],
    /// The light cache's passes (compact, its rays' trace and light, relight) run on
    /// the async compute queue (GENOS_LIGHTING_ASYNC=0 keeps them in the frame): at most
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
    /// Light cache rounds run (lighting_round), and frames to hold off after the cache
    /// buffers were cleared on the graphics queue.
    world_round: u32,
    world_hold: u32,
    /// Hash of the scene's light without the camera (lamps, occluders, fire, smoke,
    /// floor, roof, sky; set with the scene upload), and with the traced and drawn
    /// meshes: what the light cache's light depends on.
    pub(crate) light_scene: u64,
    light_hash: u64,
    /// Rounds in a row, since that last changed, whose mature patches all stayed
    /// within LIGHTING_SETTLE_CHANGE: past a full sweep, rounds relight young patches only.
    light_quiet: u32,
    /// Occluders (index in the scene's list) that moved in the last
    /// LIGHTING_MOVER_KEEP frames, with the frames left: movers. The surface cache leaves
    /// their pixels to the probes; once still that long they are scenery again.
    movers: std::collections::BTreeMap<usize, u32>,
    /// Traced mesh instances (mesh_field order): last poses, the ones that moved in
    /// the last LIGHTING_MOVER_KEEP frames, and every instance's bounding sphere now.
    mesh_poses: Vec<[f32; 16]>,
    mesh_movers: std::collections::BTreeMap<usize, u32>,
    mesh_spheres: Vec<[f32; 4]>,
    young_ok: bool,
    /// What each round in flight was built from (frame slots, then the async one):
    /// the light hash and whether it took young patches only.
    stats_light: [u64; 2],
    stats_young: [bool; 2],
    /// S78S debug dump (GENOS_LIGHTING_DUMP=N, GENOS_LIGHTING_DUMP_DIR): the raw
    /// cache, cache keys, work, G-buffer and output buffers copied at frames N and
    /// N + 1 (one readback set each), written to disk a few frames later. Never
    /// on in the normal loop; perf/dump.py splits them into per-component files.
    dump: Vec<(u64, Vec<(&'static str, Buffer)>)>,
    world_light: u64,
    world_young: bool,
    /// Light cache rounds per second of wall time (GENOS_LIGHTING_CACHE_HZ; 0: one every
    /// frame), when the last began, and whether each frame slot ran one.
    cache_hz: f64,
    last_round: Option<Instant>,
    stats_round: [bool; 2],
    /// Last frame this slot counted unfilled pixels. Read at the next begin.
    unfilled_slot: [UnfilledSlot; 2],
    /// Overrides `GENOS_LIGHTING_ATROUS_DIRTY` for the a-trous list fallback.
    /// Fraction of filtered texels; `None` reads the settings each frame.
    atrous_dirty: Option<f32>,
    /// Settings in force. A draw copies the scene, or the override, then the env.
    settings: genos_scene::LightingSettings,
    /// Wins over the scene until cleared.
    override_settings: Option<genos_scene::LightingSettings>,
    sun_height: f32,
    /// The million-slot cache is allocated. Off, a small dummy stands in.
    cache_full: bool,
    /// Last buffer class. A live edit rebuilds only when this changes.
    alloc_key: (u32, u32, u32, u32, u32),
    /// Per flight slot: bucket tag of each lighting dispatch, in order.
    frame_tags: [Vec<u8>; 2],
    frame_counts: [[u32; BUCKETS]; 2],
    /// Shadows rode on the direct and hit dispatches of that slot.
    frame_shadow: [bool; 2],
    /// The prefill scout was due on that slot. It shares the cache-ray dispatch.
    frame_prefill: [bool; 2],
    /// Last collected frame (one frame behind the one just submitted).
    last_ms: [f64; BUCKETS],
    last_dispatches: [u32; BUCKETS],
    /// Control block uploaded with the frame, reused by the async queue.
    ctrl_bytes: [u8; 64],
    /// Lamp and geometry changes kept for a few frames of ring tests.
    wake: WakeQueue,
    /// How many times this flight slot has scheduled rings. Picks the persist slot.
    wake_phase: [u32; 2],
    /// The stats tail still holds the previous ring. The first frame clears it.
    wake_ready: [bool; 2],
    /// Host pointer for `stats`, mapped once. A per-frame map of this cached
    /// readback was the rings-only read on a settled corner.
    stats_map: [*mut u8; 2],
    /// The work buffer's wake-event block is already zero. A settled frame
    /// does not rewrite it.
    wake_block_clear: bool,
}

/// A-trous levels 1-5 in the last frame that ran them (S78X dense fallback).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightingAtrousStats {
    /// Dirty slots over filtered texels, the fullest of levels 1-5 (0..1).
    pub dirty_fraction: f32,
    /// `0` dense full-frame sweep, `1` dirty work list. Meaningful when `ran`.
    pub mode: u32,
    /// Threshold the GPU used (`GENOS_LIGHTING_ATROUS_DIRTY`, 0..1).
    pub threshold: f32,
    /// Filtered texels `dirty_fraction` divided by.
    pub filtered: u32,
    /// Levels 1-5 ran this frame.
    pub ran: bool,
}

/// Probes and rays that reached a trace in the last frame, plus the ring-wake
/// counters (zero while the scheduler is the legacy check ray).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightingWakeStats {
    pub probes_traced: u32,
    pub rays_traced: u32,
    pub rings_tested: u32,
    pub groups_split: u32,
    pub probes_woken: u32,
    pub verify_hits: u32,
    pub verify_misses: u32,
}

/// A probe the ring scheduler traced in full last frame, and where it stood.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightingWakeProbe {
    pub index: u32,
    pub position: [f32; 3],
}

impl LightingWakeStats {
    fn from_words(w: &[u32]) -> Self {
        Self {
            probes_traced: *w.get(WAKE_TRACED).unwrap_or(&0),
            rays_traced: *w.get(WAKE_RAYS).unwrap_or(&0),
            rings_tested: *w.get(WAKE_RINGS).unwrap_or(&0),
            groups_split: *w.get(WAKE_SPLITS).unwrap_or(&0),
            probes_woken: *w.get(WAKE_WOKEN).unwrap_or(&0),
            verify_hits: *w.get(WAKE_HITS).unwrap_or(&0),
            verify_misses: *w.get(WAKE_MISSES).unwrap_or(&0),
        }
    }
}

fn lighting_atrous_from(w: &[u32; LIGHTING_STATS_WORDS]) -> LightingAtrousStats {
    let mode_word = w[LIGHTING_ATROUS_STAT + 1];
    LightingAtrousStats {
        dirty_fraction: w[LIGHTING_ATROUS_STAT] as f32 / 1.0e6,
        mode: u32::from(mode_word == 1),
        threshold: w[LIGHTING_ATROUS_STAT + 2] as f32 / 1.0e6,
        filtered: w[LIGHTING_ATROUS_STAT + 3],
        ran: mode_word != 0,
    }
}

/// Whether the lighting has settled: the scene (camera included) has held still
/// long enough and the light cache has stopped changing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettleStats {
    pub settled: bool,
    /// Frames in a row the scene has not changed.
    pub still_frames: u32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            ready: false,
            desc_layout: std::ptr::null_mut(),
            layout: std::ptr::null_mut(),
            pool: std::ptr::null_mut(),
            sets: [std::ptr::null_mut(); 2],
            pipes: [std::ptr::null_mut(); 9],
            gbuffer_pipe: std::ptr::null_mut(),
            gbuf: Buffer::empty(),
            work: Buffer::empty(),
            out: Buffer::empty(),
            cache_keys: Buffer::empty(),
            cache: Buffer::empty(),
            cache_clear: false,
            gbuf_clear: false,
            work_clear: false,
            compose_valid: false,
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
            stats_dev: [Buffer::empty(), Buffer::empty()],
            quiet: 0,
            stats_still: [0; 2],
            live: 0,
            dims: [0; 4],
            async_on: std::env::var("GENOS_LIGHTING_ASYNC").is_ok_and(|v| v == "1"),
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
            movers: std::collections::BTreeMap::new(),
            mesh_poses: Vec::new(),
            mesh_movers: std::collections::BTreeMap::new(),
            mesh_spheres: Vec::new(),
            young_ok: std::env::var("GENOS_LIGHTING_YOUNG_ONLY").map_or(true, |v| v != "0"),
            stats_light: [0; 2],
            stats_young: [false; 2],
            dump: Vec::new(),
            world_light: 0,
            world_young: false,
            cache_hz: std::env::var("GENOS_LIGHTING_CACHE_HZ")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            last_round: None,
            stats_round: [false; 2],
            unfilled_slot: [UnfilledSlot::default(); 2],
            atrous_dirty: None,
            settings: genos_scene::LightingSettings::default(),
            override_settings: None,
            sun_height: 0.0,
            cache_full: false,
            alloc_key: (0, 0, 0, 0, 0),
            frame_tags: [Vec::new(), Vec::new()],
            frame_counts: [[0; BUCKETS]; 2],
            frame_shadow: [false; 2],
            frame_prefill: [false; 2],
            last_ms: [0.0; BUCKETS],
            last_dispatches: [0; BUCKETS],
            ctrl_bytes: [0; 64],
            wake: WakeQueue::new(),
            wake_phase: [0; 2],
            wake_ready: [false; 2],
            stats_map: [std::ptr::null_mut(); 2],
            wake_block_clear: false,
        }
    }
}

/// S70A dispatch kinds in a round's group list (the third field; real heights stay below).
const LIGHTING_ARGS: u32 = 1 << 30;
const LIGHTING_MODE1: u32 = LIGHTING_ARGS + 1;
const LIGHTING_DENSE_RAYS: u32 = LIGHTING_ARGS + 2;
const LIGHTING_DENSE_ENTRIES: u32 = LIGHTING_ARGS + 3;
/// Picture trace/direct of the verification stride only (`lighting_mode` 3).
const LIGHTING_WAKE_VERIFY: u32 = LIGHTING_ARGS + 4;
/// Word of the dispatch args in the cache keys (lighting_cache.glsl LIGHTING_ARGS_AT).
const LIGHTING_ARGS_AT: u64 = 2 * LIGHTING_CACHE_SLOTS + 2 + 17600;
/// A-trous list metadata (lighting_cache.glsl LIGHTING_ATROUS_*). The threshold
/// word is `lighting_atrous_ppm_at()`; the meta words follow it.
const LIGHTING_CHECK_P_AT: u64 = LIGHTING_ARGS_AT + 16 + 3 * 65536 + 1024 + 32768;
const LIGHTING_ATROUS_LIST_CAP: u64 = 65536;
const LIGHTING_ATROUS_MARK_AT: u64 = LIGHTING_CHECK_P_AT + 16;
const LIGHTING_ATROUS_LIST_AT: u64 = LIGHTING_ATROUS_MARK_AT + 5 * LIGHTING_CACHE_SLOTS;
/// Sits after the compose words (`lighting_ckeys_words`); it used to start where
/// `LIGHTING_COMPOSE_AT` starts, so the two regions aliased.
fn lighting_atrous_ppm_at() -> u64 {
    lighting_ckeys_words()
}
/// ppm, previous fraction, 64 filtered-texel bins, 5 level modes, 5 * 64 dense recount bins.
const LIGHTING_ATROUS_META_WORDS: u64 = 1 + 1 + 64 + 5 + 5 * 64;
const LIGHTING_PASS_NAMES: [&str; 9] = [
    "lighting compact",
    "lighting place",
    "lighting trace",
    "lighting direct",
    "lighting gather",
    "lighting filter",
    "lighting cache",
    "lighting compose",
    "lighting atrous",
];

impl Gpu {
    /// Track which traced mesh instances move (surface cache movers).
    pub(crate) fn note_field_movers(&mut self, live: &[&crate::mesh_field::FieldInstance]) {
        let g = &mut self.lighting;
        lighting_mesh_movers(live, &mut g.mesh_poses, &mut g.mesh_movers, &mut g.mesh_spheres);
    }

    /// Note how long a pipeline took to build (tests check it stays short).
    fn note_pipeline_time(&mut self, name: &str, start: Instant) {
        self.pipeline_times
            .push((name.to_string(), start.elapsed()));
    }

    fn make_lighting(&mut self) -> Result<(), String> {
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
                    &mut self.lighting.desc_layout,
                ),
                "lighting descriptor layout",
            )?;
            let sets = [self.lighting.desc_layout];
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
                    &mut self.lighting.layout,
                ),
                "lighting pipeline layout",
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
                    &mut self.lighting.pool,
                ),
                "lighting descriptor pool",
            )?;
            let layouts = [self.lighting.desc_layout; 3];
            let alloc = Alloc {
                s_type: 34,
                next: std::ptr::null(),
                pool: self.lighting.pool,
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
                "lighting descriptor sets",
            )?;
            self.lighting.sets = [sets[0], sets[1]];
            self.lighting.world_set = sets[2];
        }
        self.make_lighting_world()?;
        let codes: [&[u8]; 9] = [
            LIGHTING_COMPACT_SPV,
            LIGHTING_PLACE_SPV,
            LIGHTING_TRACE_SPV,
            LIGHTING_DIRECT_SPV,
            LIGHTING_GATHER_SPV,
            LIGHTING_FILTER_SPV,
            LIGHTING_CACHE_SPV,
            LIGHTING_COMPOSE_SPV,
            LIGHTING_ATROUS_SPV,
        ];
        for (k, code) in codes.iter().enumerate() {
            let start = Instant::now();
            self.lighting.pipes[k] = self.make_compute_pipe(code, self.lighting.layout, LIGHTING_PASS_NAMES[k])?;
            self.note_pipeline_time(LIGHTING_PASS_NAMES[k], start);
        }
        let start = Instant::now();
        self.frag_override = Some(GBUFFER_FRAG_SPV);
        self.vert_override = Some(GBUFFER_VERT_SPV);
        let made = self.make_pipeline(true, false, false);
        self.frag_override = None;
        self.vert_override = None;
        self.lighting.gbuffer_pipe = made?;
        self.note_pipeline_time("lighting gbuffer", start);
        self.lighting.ready = true;
        Ok(())
    }

    /// The async light cache submission's command buffer (compute family), its
    /// fence (signalled: nothing in flight) and timestamps.
    fn make_lighting_world(&mut self) -> Result<(), String> {
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
        if self.compute_pool.is_null() {
            self.lighting.async_on = false;
            return Ok(());
        }
        let alloc = AllocInfo {
            s_type: 40,
            next: std::ptr::null(),
            pool: self.compute_pool,
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
                    &mut self.lighting.world_cmd,
                ),
                "lighting world command buffer",
            )?;
            check(
                (self.fns.create_fence)(
                    self.device,
                    &fence as *const FenceInfo as *const u8,
                    std::ptr::null(),
                    &mut self.lighting.world_fence,
                ),
                "lighting world fence",
            )?;
        }
        // Stamps 0 and 1: the round's start and end; 2 to 5: after each pass.
        self.lighting.world_qp = self.make_query_pool_n(6)?;
        Ok(())
    }

    /// Wait for the async light cache submission in flight, if any.
    fn wait_lighting_world(&mut self) {
        if !self.lighting.world_fence.is_null() {
            let fences = [self.lighting.world_fence];
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
    fn make_lighting_targets(&mut self) -> Result<(), String> {
        if !self.lighting.ready {
            return Ok(());
        }
        self.wait_all_inflight()?;
        self.wait_lighting_world();
        self.poll_lighting_world();
        let (w, h) = (self.extent_w.max(1), self.extent_h.max(1));
        let settings = self.lighting.settings.clone();
        let tile = if settings.probes.tile == 0 {
            lighting_tile(h)
        } else {
            settings.probes.tile
        };
        let probes_on = settings.allocates_probe_work();
        let (cols, rows) = if probes_on {
            (w.div_ceil(tile), h.div_ceil(tile))
        } else {
            (0, 0)
        };
        let probes = (cols as u64) * (rows as u64);
        let pixels = w as u64 * h as u64;
        let rays = settings.probe_rays() as u64;
        // Probes (2), one slot per light cache ray (hit, then its light), filtered SH
        // (7), strata (2 per probe and stratum), kept hits, their light (a uint
        // each) and a direct-luminance baseline (one float each), the changes
        // block, the views (2 blocks of 2 per probe) and the cells (2 tables of R
        // per probe), then the fills and their light (as the kept hits)
        // (lighting_common.glsl).
        // Bounce 0 keeps the GI flag but skips every GI buffer. The alloc key
        // matches GI off, so the work buffer stays the small one and the change
        // block must not be written at the GI offset.
        let gi = settings.gi.enabled && settings.bounce.count > 0;
        let kept = if probes_on {
            probes * rays * LIGHTING_STRATA as u64
        } else {
            0
        };
        let cache_rays = if gi {
            (LIGHTING_CACHE_BATCH * LIGHTING_CACHE_RAYS) as u64
        } else {
            0
        };
        let work_vec4 = if gi {
            lighting_change_at(probes, cache_rays, kept)
        } else {
            256
        };
        let work_tail = if gi {
            (LIGHTING_CHANGE_HEAD + LIGHTING_CHANGES) as u64
            + 4 * probes
            + 2 * rays * probes
            + kept
            + kept / 4
            + pixels.div_ceil(2) // S78J: per-pixel direct light reuse (signature, light)
            + 2 * probes + rays * probes // S78Q: held-probe gather (stamps, raw cells)
            + probes // wake record per probe (luminance, frame, flags)
            + 16 // eight wake events, two vec4 each
        } else {
            0
        };
        let work_vec4 = work_vec4 + work_tail;
        let mut gbuf = std::mem::replace(&mut self.lighting.gbuf, Buffer::empty());
        let mut work = std::mem::replace(&mut self.lighting.work, Buffer::empty());
        let mut out = std::mem::replace(&mut self.lighting.out, Buffer::empty());
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        // Both queues use the light cache and the work buffer's ray slots.
        let share = self.light_families();
        if self.lighting.stats[0].buffer.is_null() {
            for k in 0..2 {
                // Header plus the unfilled mask. The mask is copied only while the
                // detector is on; the room stays so a hotkey can turn it on.
                let stats_bytes = lighting_stats_storage_bytes();
                self.lighting.stats[k] = self.make_buffer(stats_bytes, 0x20 | 0x2, Memory::Readback)?;
                self.pin_stats(k);
                self.lighting.stats_dev[k] = self.make_buffer(stats_bytes, 0x20 | 0x2 | 0x1, Memory::Device)?;
            }
            self.lighting.world_stats =
                self.make_buffer_queues(LIGHTING_STATS_BYTES, 0x20 | 0x2, Memory::Readback, &share)?;
            self.lighting.world_scene = self.make_buffer_queues(
                crate::pack::SCENE_CAPACITY as u64,
                0x20,
                Memory::Upload,
                &share,
            )?;
            self.write_buffer(&self.lighting.world_scene, &vec![0u8; crate::pack::SCENE_TAIL])?;
        }
        let want_cache = settings.allocates_cache();
        if want_cache != self.lighting.cache_full || self.lighting.cache.buffer.is_null() {
            let mut keys = std::mem::replace(&mut self.lighting.cache_keys, Buffer::empty());
            let mut cache = std::mem::replace(&mut self.lighting.cache, Buffer::empty());
            self.destroy_buffer(&mut keys);
            self.destroy_buffer(&mut cache);
            if want_cache {
                // The compose words and the a-trous meta must not alias the a-trous lists.
                assert!(lighting_compose_at() >= LIGHTING_ATROUS_LIST_AT + 5 * (LIGHTING_ATROUS_LIST_CAP + 1) + 6 * 65536 + 16);
                assert!(lighting_atrous_ppm_at() >= lighting_compose_minx() + 4 * LIGHTING_CACHE_SLOTS);
                self.lighting.cache_keys = self.make_buffer_queues(
                    (lighting_atrous_ppm_at() + LIGHTING_ATROUS_META_WORDS) * 4,
                    0x20 | 0x2 | 0x100 | 0x1,
                    Memory::Device,
                    &share,
                )?;
                self.lighting.cache = self.make_buffer_queues(
                    LIGHTING_CACHE_SLOTS * 368,
                    0x20 | 0x2 | 0x1,
                    Memory::Device,
                    &share,
                )?;
                self.lighting.cache_clear = true;
            } else {
                // Descriptors stay valid. Shaders do not index these while GI is off.
                self.lighting.cache_keys =
                    self.make_buffer_queues(4096, 0x20 | 0x2 | 0x100 | 0x1, Memory::Device, &share)?;
                self.lighting.cache = self.make_buffer_queues(4096, 0x20 | 0x2 | 0x1, Memory::Device, &share)?;
                self.lighting.cache_clear = false;
            }
            self.lighting.cache_full = want_cache;
        }
        self.lighting.gbuf = self.make_buffer(16 + pixels * 16, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.lighting.work =
            self.make_buffer_queues(work_vec4.max(1) * 16, 0x20 | 0x1 | 0x2, Memory::Device, &share)?;
        // Picture, emitted light, and (when direct refresh is slower than every
        // frame) one stored direct value per pixel. High stays at pixels * 8.
        let hist = u64::from(settings.direct.refresh_frames > 1);
        self.lighting.out = self.make_buffer(pixels * (8 + 4 * hist), 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.lighting.dims = [w, h, cols, rows];
        self.lighting.alloc_key = lighting_alloc_key(&settings);
        self.lighting.compose_valid = false;
        self.lighting.gbuf_clear = true;
        self.lighting.work_clear = gi;
        self.lighting.wake_block_clear = false;
        self.write_raster_sets()?;
        self.write_lighting_sets()
    }

    fn write_lighting_sets(&self) -> Result<(), String> {
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
                    self.lighting.stats_dev[slot].buffer,
                    self.lighting.sets[slot],
                )
            } else {
                (
                    self.lighting.world_scene.buffer,
                    self.lighting.world_stats.buffer,
                    self.lighting.world_set,
                )
            };
            if set.is_null() {
                continue;
            }
            let buffers = [
                (0u32, scene),
                (5, self.mesh_field.buffer),
                (6, self.lighting.gbuf.buffer),
                (7, self.lighting.work.buffer),
                (8, self.lighting.out.buffer),
                (9, self.lighting.cache_keys.buffer),
                (10, self.lighting.cache.buffer),
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
    /// once the scene (camera included) has not changed for LIGHTING_SETTLE_FRAMES.
    pub(crate) fn lighting_stats(&self) -> SettleStats {
        let sweep = self.lighting.live.div_ceil(LIGHTING_CACHE_BATCH).max(1) + 1;
        // World cells on (GENOS_LIGHTING_WORLD=2): their running means (cap frames) have
        // to forget the poses before this one, so settling waits four caps.
        let world_mode = self.lighting.settings.world_mode();
        let world_wait = match world_mode {
            2 => {
                4 * std::env::var("GENOS_LIGHTING_WORLD_CAP").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(32)
            }
            // Surface cache: GENOS_LIGHTING_SC_SETTLE frames still (experiment: how long
            // its texels take to take in the bounced light).
            3 => std::env::var("GENOS_LIGHTING_SC_SETTLE").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0),
            _ => 0,
        };
        // Surface cache with flag 8192: texels at the cap keep relighting (their
        // change never drops under LIGHTING_SETTLE_CHANGE), so it settles on frames still
        // (GENOS_LIGHTING_SC_SETTLE, default 300) and the light having held.
        let sc_relit = world_wait != 0
            || (world_mode == 3 && lighting_sc_flags_env() & 8192 != 0);
        let settled = if world_mode == 3 && sc_relit {
            self.lighting.still >= LIGHTING_SETTLE_FRAMES.max(if world_wait != 0 { world_wait } else { 300 }) && self.lighting.light_quiet >= 2
        } else if probe_only(&self.lighting.settings) {
            // Probe-only keeps relighting its batch, so the change counter never
            // drops. Settle on frames still and the light having held, the same
            // idle the surface cache uses once a still scene has gone quiet.
            self.lighting.still >= LIGHTING_SETTLE_FRAMES && self.lighting.light_quiet >= 2
        } else {
            self.lighting.still >= LIGHTING_SETTLE_FRAMES.max(world_wait) && self.lighting.quiet >= sweep
        };
        SettleStats {
            settled,
            still_frames: self.lighting.still,
        }
    }

    fn lighting_still_frames(&self) -> u32 {
        self.lighting.still
    }

    /// Rays the screen probes held in the last frame drawn, and probes placed
    /// (lighting_gather.comp): waits for that frame.
    pub(crate) fn lighting_probe_samples(&mut self) -> [u32; 2] {
        unsafe {
            (self.fns.device_wait)(self.device);
        }
        self.read_lighting_stats(self.lighting.last_slot)
            .map_or(self.lighting.probe_samples, |w| [w[6], w[7]])
    }

    /// Wake counters for the last frame drawn. Waits for that frame.
    pub(crate) fn lighting_wake_stats(&mut self) -> LightingWakeStats {
        unsafe {
            (self.fns.device_wait)(self.device);
        }
        self.read_stats16(self.lighting.stats[self.lighting.last_slot].memory)
            .map(|w| LightingWakeStats::from_words(&w))
            .unwrap_or_default()
    }

    /// Probes the ring scheduler fully traced last frame, up to 32, with the
    /// places they stood. Verification samples are not in this list.
    pub(crate) fn lighting_wake_list(&mut self) -> Vec<LightingWakeProbe> {
        unsafe {
            (self.fns.device_wait)(self.device);
        }
        let Some(words) = self.read_stats16(self.lighting.stats[self.lighting.last_slot].memory).ok() else {
            return Vec::new();
        };
        let n = (words.get(WAKE_WOKEN).copied().unwrap_or(0) as usize).min(WAKE_LIST_CAP);
        (0..n)
            .map(|i| {
                let at = WAKE_LIST + i * 4;
                LightingWakeProbe {
                    index: words.get(at).copied().unwrap_or(0),
                    position: [
                        f32::from_bits(words.get(at + 1).copied().unwrap_or(0)),
                        f32::from_bits(words.get(at + 2).copied().unwrap_or(0)),
                        f32::from_bits(words.get(at + 3).copied().unwrap_or(0)),
                    ],
                }
            })
            .collect()
    }

/// A-trous dirty fraction, mode and threshold for the last frame drawn.
    /// Waits for that frame. The words are the `lightingatrous` line.
    pub(crate) fn lighting_atrous_stats(&mut self) -> LightingAtrousStats {
        unsafe {
            (self.fns.device_wait)(self.device);
        }
        let slot = self.lighting.last_slot;
        self.read_stats16(self.lighting.stats[slot].memory)
            .map(|w| lighting_atrous_from(&w))
            .unwrap_or_default()
    }

    /// The picture has stopped changing: a frame drawn from held light has been
    /// submitted after another, so one is on screen.
    fn lighting_picture_held(&self) -> bool {
        self.lighting.held >= 2
    }

    /// The lighting runs this frame: built and sized for the picture.
    fn lighting_active(&self) -> bool {
        self.lighting.ready
            && self.lighting.dims[0] == self.extent_w
            && self.lighting.dims[1] == self.extent_h
    }

    /// Before the raster: the G-buffer size header, and the last frame's passes done
    /// with the buffers this frame's raster writes.
    fn lighting_begin(&mut self, slot: usize) {
        let header = [self.lighting.dims[0], self.lighting.dims[1], 0, 0];
        let world = self.lighting.async_on;
        if world {
            self.poll_lighting_world();
        }
        // This slot's last frame is done (its fence was waited): read its counters
        // once. Debug, settle, and the ring vote all want the same words. A second
        // map of the cached readback was rings-only and showed up as CPU time on
        // the lit corner while the GPU total stayed equal.
        let trace_on = std::env::var_os("GENOS_LIGHTING_SC_TRACE").is_some();
        let debug = lighting_debug_enabled();
        let read = lighting_stats_wanted(self.lighting.settings.world_mode());
        let adapt = wake_config(&self.lighting.settings).mode == 1 && self.lighting.wake_ready[slot];
        let words = if trace_on || debug || read || adapt {
            self.read_stats16(self.lighting.stats[slot].memory).ok()
        } else {
            None
        };
        if trace_on || debug {
            if let Some(words) = words.as_ref() {
                self.emit_trace_sc(words, "frame", trace_on);
            }
        }
        self.lighting_dump_write();
        // S70B (no GPU->CPU reads in the frame loop): the surface cache's settling
        // and young-only rounds come from what the CPU already knows (frames still,
        // rounds since the light changed), so its counters are read only for the
        // debug output or the older world modes that settle on them.
        if read {
            if let Some(words) = words.as_ref() {
                self.lighting.probe_samples = [words[6], words[7]];
                if words[7] > 0 && debug {
                    eprintln!(
                        "lighting probes rays {} probes {} per_probe {:.1}",
                        words[6],
                        words[7],
                        words[6] as f64 / words[7] as f64
                    );
                }
            }
        }
        if !self.lighting.cache_clear && !world && self.lighting.stats_round[slot] && read {
            if let Some(words) = words.as_ref() {
                // A frame that ran just as the scene changed measured against the
                // old light; it does not count toward settling.
                let mut head = [0u32; 8];
                head.copy_from_slice(&words[..8]);
                let light = self.lighting.stats_light[slot] == self.lighting.light_hash;
                self.note_lighting_round(
                    head,
                    self.lighting.stats_still[slot] != 0,
                    light,
                    self.lighting.stats_young[slot],
                );
            }
        }
        if adapt {
            if let Some(words) = words.as_ref() {
                self.lighting.wake.adapt(words[WAKE_HITS], words[WAKE_MISSES]);
            }
        }
        let clear_bytes = if self.lighting.wake_ready[slot] {
            WAKE_CLEAR_WORDS as u64 * 4
        } else {
            self.lighting.wake_ready[slot] = true;
            LIGHTING_STATS_BYTES
        };
        unsafe {
            (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.stats_dev[slot].buffer, 0, clear_bytes, 0);
            // The ring frontier lives in the tail of the header. A later frame
            // clears only the counters, so the unfilled words after that tail
            // are cleared on their own.
            if clear_bytes < LIGHTING_STATS_BYTES {
                (self.fns.cmd_fill_buffer)(
                    self.cmd,
                    self.lighting.stats_dev[slot].buffer,
                    (LIGHTING_UNFILLED_WORD as u64) * 4,
                    16,
                    0,
                );
            }
            if self.unfilled.count {
                let mask_bytes = unfilled_mask_bytes(self.lighting.dims[0], self.lighting.dims[1]);
                if mask_bytes > 0 {
                    (self.fns.cmd_fill_buffer)(
                        self.cmd,
                        self.lighting.stats_dev[slot].buffer,
                        LIGHTING_STATS_BYTES,
                        mask_bytes,
                        0,
                    );
                }
            }
            // S78S change counters on (GENOS_LIGHTING_SC_TRACE): the enable word
            // (chg[31]) is set on the GPU; the counts are read with the stats (async).
            if lighting_chg_on() {
                (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.stats_dev[slot].buffer, LIGHTING_CHG_ENABLE_BYTE, 4, 1);
            }
            if self.lighting.cache_clear {
                self.lighting.cache_clear = false;
                // The async queue starts once this frame (its slot) is surely done.
                self.lighting.world_hold = 2;
                (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.cache_keys.buffer, 0, u64::MAX, 0);
                (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.cache.buffer, 0, u64::MAX, 0);
            }
            if self.lighting.work_clear {
                self.lighting.work_clear = false;
                (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.work.buffer, 0, u64::MAX, 0);
            }
            self.lighting.gbuf_clear = false;
            // Empty records (distance all ones). Compose no longer visits every
            // pixel, so the next raster cannot rely on it to clear what it skipped.
            // The header is written just below.
            (self.fns.cmd_fill_buffer)(
                self.cmd,
                self.lighting.gbuf.buffer,
                0,
                self.lighting.gbuf.size,
                u32::MAX,
            );
            // Quiet-compose indirect args: group count x starts at 0, y and z stay 1.
            // The key table is absent while GI is off; those offsets would be past the dummy.
            if self.lighting.cache_full {
                let yz = [1u32, 1u32];
                (self.fns.cmd_fill_buffer)(
                    self.cmd,
                    self.lighting.cache_keys.buffer,
                    lighting_compose_at() * 4,
                    4,
                    0,
                );
                (self.fns.cmd_update_buffer)(
                    self.cmd,
                    self.lighting.cache_keys.buffer,
                    (lighting_compose_at() + 1) * 4,
                    8,
                    yz.as_ptr() as *const c_void,
                );
            }
            // The whole-buffer clears land before the count and header writes.
            self.memory_barrier(0x1000, 0x1000, 0x1000, 0x1000);
            // The mature and young patch counts, rebuilt by the compact pass.
            if !world && self.lighting.cache_full {
                for at in [LIGHTING_CACHE_SLOTS, 2 * LIGHTING_CACHE_SLOTS + 1, LIGHTING_WRC_COUNTS_AT, LIGHTING_WRC_COUNTS_AT + 1, LIGHTING_WRC_COUNTS_AT + 4, LIGHTING_WRC_COUNTS_AT + 5, LIGHTING_WRC_COUNTS_AT + 6, LIGHTING_WRC_COUNTS_AT + 7] {
                    (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.cache_keys.buffer, at * 4, 4, 0);
                }
            }
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.lighting.gbuf.buffer,
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
    fn note_lighting_round(&mut self, words: [u32; 8], current: bool, light: bool, young_only: bool) {
        if !current {
            self.lighting.quiet = 0;
        } else if words[1] > 0 {
            if words[0] <= LIGHTING_SETTLE_CHANGE {
                self.lighting.quiet = self.lighting.quiet.saturating_add(1);
            } else {
                self.lighting.quiet = 0;
            }
        } else if young_only {
            self.lighting.quiet = self.lighting.quiet.saturating_add(1);
        }
        if !light {
            self.lighting.light_quiet = 0;
        } else if self.lighting.settings.world_mode() == 3 {
            // Surface cache: texels keep adding sets, so their change says nothing
            // of the light; rounds since the light last changed.
            self.lighting.light_quiet = self.lighting.light_quiet.saturating_add(1);
        } else if !probe_only(&self.lighting.settings) && words[5] > 0 {
            // Probe-only is counted on the CPU. Its batch keeps this counter high,
            // so the counter cannot veto a still scene.
            if words[4] <= LIGHTING_SETTLE_CHANGE {
                self.lighting.light_quiet = self.lighting.light_quiet.saturating_add(1);
            } else {
                self.lighting.light_quiet = 0;
            }
        }
        self.lighting.live = words[2];
        if lighting_debug_enabled() {
            eprintln!(
                "lighting frame {} still {} change {} relit {} live {} missed {} quiet {} mature_change {} mature_relit {} light_quiet {} young_only {}",
                self.lighting.frame,
                self.lighting.still,
                words[0],
                words[1],
                words[2],
                words[3],
                self.lighting.quiet,
                words[4],
                words[5],
                self.lighting.light_quiet,
                young_only
            );
        }
    }

    /// The async light cache round in flight has finished: read its counters and
    /// GPU span.
    fn poll_lighting_world(&mut self) {
        if !self.lighting.world_busy {
            return;
        }
        let status = unsafe { (self.fns.fence_status)(self.device, self.lighting.world_fence) };
        if status != 0 {
            return;
        }
        self.lighting.world_busy = false;
        self.trace_sc(self.lighting.world_stats.memory, "round");
        if let Ok(words) = self.read_stats_memory(self.lighting.world_stats.memory) {
            let current = self.lighting.world_still > 0 && self.lighting.world_hash == self.lighting.scene_hash;
            let light = self.lighting.world_light == self.lighting.light_hash;
            self.note_lighting_round(words, current, light, self.lighting.world_young);
        }
        if self.gpu_times.is_some() && !self.lighting.world_qp.is_null() {
            let mut raw = [0u64; 6];
            let got = unsafe {
                (self.fns.get_query_results)(
                    self.device,
                    self.lighting.world_qp,
                    0,
                    6,
                    48,
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
                    // Per pass: compact, trace, light, cache.
                    let mut prev = raw[0];
                    for k in 0..4 {
                        times.world_pass[k] += tick_delta(prev, raw[2 + k], bits) as f64 * period / 1.0e6;
                        prev = raw[2 + k];
                    }
                }
            }
        }
    }

    /// Record and submit one light cache round on the async compute queue: list
    /// the live patches, trace and light this round's batch of their rays, relight
    /// them. Skipped while the last round still runs, so at most one is in flight
    /// and a slow round never queues up work behind it.
    fn submit_lighting_world(&mut self, pc: [u32; 8]) -> Result<bool, String> {
        if self.lighting.world_busy || self.lighting.world_cmd.is_null() {
            return Ok(false);
        }
        if self.lighting.world_hold > 0 {
            self.lighting.world_hold -= 1;
            return Ok(false);
        }
        if !self.frame_bytes.is_empty() {
            self.write_buffer(&self.lighting.world_scene, &self.frame_bytes)?;
        }
        let round = self.lighting.world_round;
        self.lighting.world_round = round.wrapping_add(1);
        let mut pc = pc;
        // Light cache rays only (lighting_mode 2) and this round.
        pc[6] = (pc[6] & 0xcff) | 2 << 8 | (round & 0xf_ffff) << 12;
        let cmd = self.lighting.world_cmd;
        let fences = [self.lighting.world_fence];
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
        let rays = LIGHTING_CACHE_BATCH * LIGHTING_CACHE_RAYS;
        let groups = [
            (0usize, (LIGHTING_CACHE_SLOTS as u32).div_ceil(64)),
            (2, rays.div_ceil(64)),
            (3, rays.div_ceil(64)),
            (6, LIGHTING_CACHE_BATCH.div_ceil(64)),
        ];
        unsafe {
            check((self.fns.reset_fences)(self.device, 1, fences.as_ptr()), "reset lighting world fence")?;
            check((self.fns.reset_cmd)(cmd, 0), "reset lighting world cmd")?;
            let begin = BeginInfo {
                s_type: 42,
                next: std::ptr::null(),
                flags: 1,
                inherit: std::ptr::null(),
            };
            check(
                (self.fns.begin_cmd)(cmd, &begin as *const BeginInfo as *const u8),
                "begin lighting world",
            )?;
            let qp = self.lighting.world_qp;
            if !qp.is_null() {
                (self.fns.cmd_reset_query)(cmd, qp, 0, 6);
                (self.fns.cmd_write_timestamp)(cmd, 0x1, qp, 0);
            }
            (self.fns.cmd_fill_buffer)(cmd, self.lighting.world_stats.buffer, 0, LIGHTING_STATS_BYTES, 0);
            self.barrier_on(cmd, 0x1000, 0x1000, 0x1000, 0x1000);
            (self.fns.cmd_update_buffer)(
                cmd,
                self.lighting.world_stats.buffer,
                LIGHTING_CTRL_WORD * 4,
                self.lighting.ctrl_bytes.len() as u64,
                self.lighting.ctrl_bytes.as_ptr() as *const c_void,
            );
            // The mature and young counts, rebuilt by the compact pass.
            (self.fns.cmd_fill_buffer)(cmd, self.lighting.cache_keys.buffer, LIGHTING_CACHE_SLOTS * 4, 4, 0);
            // The WRC refresh lists' counts (change, background; lighting_cache.glsl).
            (self.fns.cmd_fill_buffer)(cmd, self.lighting.cache_keys.buffer, LIGHTING_WRC_COUNTS_AT * 4, 8, 0);
            (self.fns.cmd_fill_buffer)(cmd, self.lighting.cache_keys.buffer, (LIGHTING_WRC_COUNTS_AT + 4) * 4, 8, 0);
            (self.fns.cmd_fill_buffer)(
                cmd,
                self.lighting.cache_keys.buffer,
                (2 * LIGHTING_CACHE_SLOTS + 1) * 4,
                4,
                0,
            );
            self.barrier_on(cmd, 0x1000, 0x800, 0x1000, 0x20 | 0x40);
            let set = self.lighting.world_set;
            (self.fns.cmd_bind_set)(cmd, 1, self.lighting.layout, 0, 1, &set, 0, std::ptr::null());
            (self.fns.cmd_push)(cmd, self.lighting.layout, 0x20, 0, 32, pc.as_ptr() as *const c_void);
            for (k, (pass, gx)) in groups.into_iter().enumerate() {
                (self.fns.cmd_bind_pipe)(cmd, 1, self.lighting.pipes[pass]);
                (self.fns.cmd_dispatch)(cmd, gx.max(1), 1, 1);
                self.barrier_on(cmd, 0x800, 0x800, 0x40, 0x20 | 0x40);
                if !qp.is_null() {
                    (self.fns.cmd_write_timestamp)(cmd, 0x800, qp, 2 + k as u32);
                }
            }
            // The counters reach the host.
            self.barrier_on(cmd, 0x800, 0x4000, 0x40, 0x2000);
            if !qp.is_null() {
                (self.fns.cmd_write_timestamp)(cmd, 0x2000, qp, 1);
            }
            check((self.fns.end_cmd)(cmd), "end lighting world")?;
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
                    self.lighting.world_fence,
                ),
                "lighting world submit",
            )?;
        }
        self.lighting.world_busy = true;
        self.lighting.world_hash = self.lighting.scene_hash;
        self.lighting.world_still = self.lighting.still;
        self.lighting.world_light = self.lighting.light_hash;
        self.lighting.world_young = pc[6] & 1 << 10 != 0;
        Ok(true)
    }

    fn read_lighting_stats(&self, slot: usize) -> Result<[u32; 8], String> {
        self.read_stats_memory(self.lighting.stats[slot].memory)
    }

    fn read_stats_memory(&self, memory: Handle) -> Result<[u32; 8], String> {
        let all = self.read_stats16(memory)?;
        let mut words = [0u32; 8];
        words.copy_from_slice(&all[..8]);
        Ok(words)
    }

    /// Surface cache trace (GENOS_LIGHTING_SC_TRACE=1): one line per readback with the
    /// frame, frames still and the claimed / restarted / freed / at-cap counters.
    /// `GENOS_LIGHTING_DEBUG` prints the a-trous line on its own.
    fn trace_sc(&self, memory: Handle, queue: &str) {
        let trace = std::env::var_os("GENOS_LIGHTING_SC_TRACE").is_some();
        let debug = lighting_debug_enabled();
        if !trace && !debug {
            return;
        }
        if let Ok(w) = self.read_stats16(memory) {
            self.emit_trace_sc(&w, queue, trace);
        }
    }

    fn emit_trace_sc(&self, w: &[u32; LIGHTING_STATS_WORDS], queue: &str, trace: bool) {
        if trace {
                eprintln!(
                    "lightingsc {queue} frame {} still {} relit {} claimed {} restarted {} freed {} atcap {} live {} thr {} extra {} partial {} sets {} rays {} due {} waiting {} scouts {} trickle {} boost {} checks {} changed {}",
                    self.lighting.frame, self.lighting.still, w[1], w[8], w[9], w[10], w[11], w[2], w[12], w[13], w[14], w[15], w[16], w[17], w[18], w[19], w[20], w[21], w[22], w[23]
                );
                eprintln!("lightingdg {queue} frame {} d {:?}", self.lighting.frame, &w[24..88]);
                eprintln!("lightingchg {queue} frame {} c {:?}", self.lighting.frame, &w[88..119]);
            }
            let mode = match w[LIGHTING_ATROUS_STAT + 1] {
                1 => "list",
                2 => "dense",
                _ => "off",
            };
            eprintln!(
                "lightingatrous {queue} frame {} dirty {:.4} mode {mode} threshold {:.4} filtered {}",
                self.lighting.frame,
                w[LIGHTING_ATROUS_STAT] as f64 / 1.0e6,
                w[LIGHTING_ATROUS_STAT + 2] as f64 / 1.0e6,
                w[LIGHTING_ATROUS_STAT + 3]
            );
            let wake = LightingWakeStats::from_words(w);
            eprintln!(
                "lightingwake {queue} frame {} traced {} rays {} rings {} splits {} woken {} hits {} misses {}",
                self.lighting.frame,
                wake.probes_traced,
                wake.rays_traced,
                wake.rings_tested,
                wake.groups_split,
                wake.probes_woken,
                wake.verify_hits,
                wake.verify_misses
            );
    }

    /// Host pointer kept for the life of the stats readback. `None` for the
    /// async queue's own buffer, which is mapped on the read.
    fn stats_host(&self, memory: Handle) -> Option<*const u32> {
        for k in 0..2 {
            let ptr = self.lighting.stats_map[k];
            if !ptr.is_null() && self.lighting.stats[k].memory == memory {
                return Some(ptr.cast());
            }
        }
        None
    }

    fn pin_stats(&mut self, k: usize) {
        let buf = &self.lighting.stats[k];
        if buf.memory.is_null() || buf.size == 0 {
            return;
        }
        let mut mapped = std::ptr::null_mut();
        let rc = unsafe { (self.fns.map_mem)(self.device, buf.memory, 0, buf.size, 0, &mut mapped) };
        if rc == 0 {
            self.lighting.stats_map[k] = mapped.cast();
        }
    }

    fn unpin_stats(&mut self, k: usize) {
        if self.lighting.stats_map[k].is_null() {
            return;
        }
        self.lighting.stats_map[k] = std::ptr::null_mut();
        let memory = self.lighting.stats[k].memory;
        if !memory.is_null() {
            unsafe { (self.fns.unmap_mem)(self.device, memory) };
        }
    }

    fn read_stats16(&self, memory: Handle) -> Result<[u32; LIGHTING_STATS_WORDS], String> {
        let mut words = [0u32; LIGHTING_STATS_WORDS];
        if let Some(src) = self.stats_host(memory) {
            unsafe { copy_host_words(src, words.as_mut_ptr(), LIGHTING_STATS_WORDS) };
            return Ok(words);
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    memory,
                    0,
                    LIGHTING_STATS_BYTES,
                    0,
                    &mut mapped,
                ),
                "map lighting stats",
            )?;
            std::ptr::copy_nonoverlapping(mapped as *const u32, words.as_mut_ptr(), LIGHTING_STATS_WORDS);
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(words)
    }

    fn read_words(&self, memory: Handle, count: usize) -> Result<Vec<u32>, String> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut words = vec![0u32; count];
        if let Some(src) = self.stats_host(memory) {
            let bytes = self
                .lighting
                .stats
                .iter()
                .find(|b| b.memory == memory)
                .map(|b| b.size)
                .unwrap_or(0);
            if (count as u64) * 4 <= bytes {
                unsafe { copy_host_words(src, words.as_mut_ptr(), count) };
                return Ok(words);
            }
        }
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(
                    self.device,
                    memory,
                    0,
                    (count * 4) as u64,
                    0,
                    &mut mapped,
                ),
                "map lighting stats",
            )?;
            std::ptr::copy_nonoverlapping(mapped as *const u32, words.as_mut_ptr(), count);
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(words)
    }

    /// The slot fence has been waited. Print the unfilled line and, past the dump
    /// threshold, write one event. Clears the slot so the next begin does not
    /// save the same frame again.
    fn take_unfilled(&mut self, slot: usize) {
        let rec = self.lighting.unfilled_slot[slot];
        if !rec.recorded {
            self.last_unfilled = None;
            return;
        }
        self.lighting.unfilled_slot[slot].recorded = false;
        let words = unfilled_mask_words(rec.width, rec.height) as usize;
        let Ok(buf) = self.read_words(self.lighting.stats[slot].memory, LIGHTING_STATS_WORDS + words) else {
            self.last_unfilled = None;
            return;
        };
        let pixels = *buf.get(LIGHTING_UNFILLED_WORD).unwrap_or(&0);
        let shaded = *buf.get(LIGHTING_SHADED_WORD).unwrap_or(&0);
        let max_run = if pixels == 0 {
            0
        } else {
            crate::unfilled::max_unfilled_run(rec.width, rec.height, &buf[LIGHTING_STATS_WORDS..])
        };
        let stats = UnfilledStats::from_counts(rec.frame, pixels, shaded, max_run);
        self.last_unfilled = Some(stats);
        eprintln!("{}", stats.line());
        let Some(threshold) = self.unfilled.dump_pct else {
            return;
        };
        if !rec.shot || stats.pct <= threshold {
            return;
        }
        let Some(shot) = self.read_picture_slot(slot, rec.width, rec.height) else {
            eprintln!("unfilled save: picture was not read back");
            return;
        };
        let dump = crate::unfilled::UnfilledDump {
            stats,
            width: rec.width,
            height: rec.height,
            eye: rec.eye,
            yaw: rec.yaw,
            pitch: rec.pitch,
            shot,
            mask: buf[LIGHTING_STATS_WORDS..].to_vec(),
        };
        match crate::unfilled::save_unfilled_event(
            std::path::Path::new(&self.debug_out),
            self.unfilled_saved,
            &dump,
        ) {
            Ok(Some(dir)) => {
                self.unfilled_saved = self.unfilled_saved.saturating_add(1);
                eprintln!(
                    "unfilled saved {} frame {} pct {:.2}",
                    dir.display(),
                    stats.frame,
                    stats.pct
                );
            }
            Ok(None) => {}
            Err(err) => eprintln!("unfilled save: {err}"),
        }
    }

    fn read_picture_slot(&self, slot: usize, width: u32, height: u32) -> Option<Vec<u8>> {
        let bytes = width as usize * height as usize * 4;
        if bytes == 0 || self.hosts[slot].size < bytes as u64 {
            return None;
        }
        let mut out = vec![0u8; bytes];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            if (self.fns.map_mem)(
                self.device,
                self.hosts[slot].memory,
                0,
                bytes as u64,
                0,
                &mut mapped,
            ) != 0
            {
                return None;
            }
            std::ptr::copy_nonoverlapping(mapped as *const u8, out.as_mut_ptr(), bytes);
            (self.fns.unmap_mem)(self.device, self.hosts[slot].memory);
        }
        Some(out)
    }

    /// After the raster: the passes, then the result into the colour image.
    /// Copy the scene (or the override), apply env locks, and rebuild buffers
    /// when a size knob changed.
    fn sync_lighting_settings(&mut self, scene: &genos_scene::Scene) -> Result<(), String> {
        let mut desired = self
            .lighting
            .override_settings
            .clone()
            .unwrap_or_else(|| scene.lighting.clone());
        desired.apply_env();
        desired.sanitize();
        let debug = scene.debug.effective(self.out_cli.as_deref());
        self.debug_out = debug.out;
        let key = lighting_alloc_key(&desired);
        let resized = self.lighting.dims[0] != self.extent_w || self.lighting.dims[1] != self.extent_h;
        let rebuild = key != self.lighting.alloc_key || resized;
        self.lighting.settings = desired;
        self.lighting.sun_height = sun_height(scene);
        if rebuild && self.lighting.ready {
            self.make_lighting_targets()?;
        }
        Ok(())
    }

    fn lighting_passes(&mut self, slot: usize) {
        let [w, h, cols, rows] = self.lighting.dims;
        let probes = cols * rows;
        let probe_rays = probes * self.lighting.settings.probe_rays();
        let bgra = if self.format == 44 { 1u32 } else { 0 };
        self.lighting.frame = self.lighting.frame.wrapping_add(1);
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            // Lamps, occluders and camera; the traced meshes; the drawn ones.
            self.frame_bytes.hash(&mut hasher);
            self.mesh_field_key.hash(&mut hasher);
            self.lighting.draw_key.hash(&mut hasher);
            hasher.finish()
        };
        if hash == self.lighting.scene_hash {
            self.lighting.still = self.lighting.still.saturating_add(1);
        } else {
            self.lighting.scene_hash = hash;
            self.lighting.still = 0;
            self.lighting.quiet = 0;
        }
        self.lighting.stats_still[slot] = self.lighting.still;
        self.lighting.last_slot = slot;
        // What moved in the traced world since the last frame: the probes drop the
        // kept rays that cross it (lighting_gather.comp). Light needs nothing here: every
        // kept hit is shaded again each frame.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.lighting.trace_key.hash(&mut hasher);
            self.mesh_field_key.hash(&mut hasher);
            hasher.finish()
        };
        let frozen = self.lighting_stats().settled;
        let wake_cfg = wake_config(&self.lighting.settings);
        if wake_cfg.mode == 1 {
            self.lighting.wake.begin_frame();
        }
        let had = !self.lighting.kept_bytes.is_empty() && key == self.lighting.kept_key;
        let changes = if had {
            lighting_changes(&self.lighting.kept_bytes, &self.frame_bytes)
        } else {
            None
        };
        if lighting_debug_enabled() {
            if frozen {
                eprintln!("lighting frozen");
            } else {
                eprintln!(
                    "lighting kept had {had} changes {}",
                    changes.as_ref().map_or(-1, |c| c.len() as i64)
                );
            }
        }
        if wake_cfg.mode == 1
            && !self.lighting.kept_bytes.is_empty()
            && wake_note_due(self.lighting.still)
        {
            // Diff the occluders even when the trace key changed. `changes` is
            // None in that case because the shader list was not built, and the
            // queue would read that as an overflow and wake every probe.
            // A still frame already hashed the lamps and the occluders.
            let extra = if had {
                None
            } else {
                lighting_changes(&self.lighting.kept_bytes, &self.frame_bytes)
            };
            let wake_changes = if had { changes.as_deref() } else { extra.as_deref() };
            self.lighting.wake.note(
                &self.lighting.kept_bytes,
                &self.frame_bytes,
                wake_changes,
            );
            for (&i, &left) in &self.lighting.mesh_movers {
                if left == LIGHTING_MOVER_KEEP {
                    if let Some(sph) = self.lighting.mesh_spheres.get(i) {
                        self.lighting.wake.note_sphere([sph[0], sph[1], sph[2]], sph[3]);
                    }
                }
            }
        }
        let mut block = vec![0f32; 4 * (LIGHTING_CHANGE_HEAD + LIGHTING_CHANGES)];
        block[2] = (self.lighting.probe_runs & 1) as f32;
        // World irradiance cells (lighting_cache.glsl, experiment): GENOS_LIGHTING_WORLD mode
        // (1: pixels read the light cache, 2: probes average into world cells, 3:
        // the surface cache) and GENOS_LIGHTING_WORLD_CAP frames (mode 3: ray sets a
        // texel averages, default 16).
        let world = self.lighting.settings.world_mode();
        let cap = std::env::var("GENOS_LIGHTING_WORLD_CAP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(if world == 3 { LIGHTING_DEFAULT_SC_CAP } else { 32 })
            .clamp(1, 999);
        // Surface cache switches (lighting_cache.glsl lighting_sc_flags).
        // Flags ride in head z above the probe-run bit (exact to 2^22); w keeps mode
        // * 1000 + cap (flags * 100000 there lost precision past 2^24 and was capped).
        // S72K: bits 23 and up go in head 3.w (was: capped at 2^23 - 1, so flag
        // 16777216 and up turned every lower flag on instead).
        // S78S debug isolate toggles (lighting_toggles): direct only rides on
        // flag 2097152 (no cache light at hits), the rest in head 3.w bits 12-15.
        let toggles = lighting_toggles();
        let flags_all = lighting_sc_flags_env() | if toggles & LIGHTING_TOGGLE_DIRECT_ONLY != 0 { 2097152 } else { 0 };
        let flags = flags_all & ((1 << 23) - 1);
        block[2] = ((self.lighting.probe_runs & 1) + 2 * flags) as f32;
        block[3] = (world * 1000 + cap) as f32;
        let mut used = 0;
        // Vec4s of the block the GPU gets: head, changes and movers.
        let mut sent = LIGHTING_CHANGE_HEAD;
        match &changes {
            None => block[1] = 1.0,
            Some(list) => {
                block[0] = list.len() as f32;
                for (j, b) in list.iter().enumerate() {
                    let at = 4 * (LIGHTING_CHANGE_HEAD + j);
                    block[at..at + 4].copy_from_slice(b);
                }
                used = list.len();
            }
        }
        sent += used;
        // Movers (surface cache): bounds of every occluder that moved in the
        // last LIGHTING_MOVER_KEEP frames, after the changes; head w carries their count above mode and cap.
        // Compared whenever there is a last scene (not only when the trace key held:
        // a spinning box can change the key every frame and must stay a mover).
        if world == 3 && !self.lighting.kept_bytes.is_empty() {
            let movers = lighting_movers(&self.lighting.kept_bytes, &self.frame_bytes, &mut self.lighting.movers);
            let mut movers = movers;
            // Traced mesh instances that moved: id 10001 + instance index (the
            // shader reads their frame from the mesh field).
            for &i in self.lighting.mesh_movers.keys() {
                if let Some(sph) = self.lighting.mesh_spheres.get(i) {
                    movers.push([*sph, [0.0, 0.0, 0.0, (10001 + i) as f32], [0.0; 4], [0.0; 4]]);
                }
            }
            let n = movers.len().min((LIGHTING_CHANGES - used) / 4);
            for (j, m) in movers.iter().take(n).enumerate() {
                for (k, b) in m.iter().enumerate() {
                    let at = 4 * (LIGHTING_CHANGE_HEAD + used + 4 * j + k);
                    block[at..at + 4].copy_from_slice(b);
                }
            }
            block[3] += (n * 200000) as f32;
            sent += 4 * n;
            if lighting_debug_enabled() {
                eprintln!("lighting world {world} cap {cap} flags {flags} local {} movers {n} of {} occs {} meshes {} of {} {:?}", u8::from(lighting_sc_local_env()), self.lighting.movers.len(), lighting_occs(&self.frame_bytes).len(), self.lighting.mesh_movers.len(), self.lighting.mesh_poses.len(), movers.iter().map(|m| (m[0], m[2], m[3])).collect::<Vec<_>>());
            }
            // Object-local texels for movers (GIBS): GENOS_LIGHTING_SC_LOCAL=1.
            if lighting_sc_local_env() {
                block[3] += 100000.0;
            }
        }
        if had {
            // Last frame's camera (scene_data.glsl): eye at byte 48, the basis at 528.
            let f = |at: usize| {
                let b = &self.lighting.kept_bytes[at..at + 4];
                f32::from_le_bytes([b[0], b[1], b[2], b[3]])
            };
            // S74A: eye at byte 16, the basis at 96/112/128 (scene_data.glsl); 48 and
            // 528-560 were an older layout, so the last camera read as garbage: the
            // scouts saw a moving camera every frame and the probes never reprojected.
            for (v, at) in [(1usize, 16usize), (2, 96), (3, 112), (4, 128)] {
                for c in 0..4 {
                    block[4 * v + c] = f(at + 4 * c);
                }
            }
            block[4 + 3] = 1.0;
        }
        // The camera up's w (head 3.w) is unread: the flags' high bits.
        // S72S: the budget shift (GENOS_LIGHTING_SC_BUDGET_SHIFT, 0-7: every
        // per-round ray budget / 2^shift) rides in bits 9-11 above the high flags.
        let budget_shift = self.lighting.settings.gi.budget_shift.min(7);
        block[4 * 3 + 3] = (((flags_all >> 23) & 511) | (budget_shift << 9) | ((toggles & 15) << 12)) as f32;
        // A frame drawn from held light runs no probes: what it saw is still what
        // the probes last ran on.
        if !frozen {
            self.lighting.kept_key = key;
            self.lighting.kept_bytes.clone_from(&self.frame_bytes);
            self.lighting.probe_runs = self.lighting.probe_runs.wrapping_add(1);
        }
        let block_at = 16
            * lighting_change_at(
                probes as u64,
                (LIGHTING_CACHE_BATCH * LIGHTING_CACHE_RAYS) as u64,
                probe_rays as u64 * LIGHTING_STRATA as u64,
            );
        // Movers ride after the changes: upload them too (S49/S50L sent only the
        // head and changes, so every mover read zeros on the GPU).
        let used = 4 * sent;
        let atrous_ppm = lighting_atrous_ppm(Some(
            self.lighting
                .atrous_dirty
                .unwrap_or(self.lighting.settings.denoiser.dirty_fraction),
        ));
        if self.lighting.settings.gi.enabled && self.lighting.settings.bounce.count > 0 {
            unsafe {
                // The last frame's passes are done with the block before it is written.
                self.memory_barrier(0x800, 0x1000, 0x20 | 0x40, 0x1000);
                (self.fns.cmd_update_buffer)(
                    self.cmd,
                    self.lighting.work.buffer,
                    block_at,
                    used as u64 * 4,
                    block.as_ptr() as *const c_void,
                );
                if self.lighting.cache_full {
                    // S78X: the a-trous dirty-fraction threshold (millionths). A cache
                    // clear earlier this frame zeroed the word; the levels read it.
                    (self.fns.cmd_update_buffer)(
                        self.cmd,
                        self.lighting.cache_keys.buffer,
                        lighting_atrous_ppm_at() * 4,
                        4,
                        &atrous_ppm as *const u32 as *const c_void,
                    );
                }
                self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
            }
        }
        // The light alone (no camera): while it holds and a full sweep has changed
        // no mature patch, rounds relight young patches only. Any change goes back
        // to full sweeps at once.
        let light = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.lighting.light_scene.hash(&mut hasher);
            self.mesh_field_key.hash(&mut hasher);
            // The drawn list follows the camera (culling), so the surface cache
            // (mode 3) leaves it out: a turn is not a light change there.
            if self.lighting.settings.world_mode() != 3 {
                self.lighting.draw_key.hash(&mut hasher);
            }
            hasher.finish()
        };
        if light != self.lighting.light_hash {
            self.lighting.light_hash = light;
            self.lighting.light_quiet = 0;
        } else if probe_only(&self.lighting.settings) && self.lighting.still > 0 {
            self.lighting.light_quiet = self.lighting.light_quiet.saturating_add(1);
        }
        let sweep = self.lighting.live.div_ceil(LIGHTING_CACHE_BATCH).max(1) + 1;
        // Surface cache (mode 3): bit 10 says the light has held two rounds, and a
        // round that relights nothing has nothing left to converge.
        let sc = self.lighting.settings.world_mode() == 3;
        let young_only = if sc { self.lighting.light_quiet >= 2 } else { self.lighting.young_ok && self.lighting.light_quiet >= sweep };
        self.lighting.stats_light[slot] = light;
        self.lighting.stats_young[slot] = young_only || sc;
        // z: bgra, fills (bit 6), young-only rounds and lamp picks (lighting_common.glsl);
        // the gather and filter passes add their round (bits 1-2) below.
        let picks = std::env::var("GENOS_LIGHTING_CACHE_PICKS").is_ok_and(|v| v == "1");
        let fill = std::env::var("GENOS_LIGHTING_FILL").map_or(true, |v| v != "0");
        // Moving objects (debugging.md, spinning boxes): a probe stratum traced from
        // inside a change is filled, not kept (bit 3); patches near a change are
        // relit first (bit 5); a patch not lit yet reads its lit neighbours (bit 7).
        // Each on unless its variable is 0.
        let on = |name: &str| std::env::var(name).map_or(true, |v| v != "0");
        let moved_drop = on("GENOS_LIGHTING_MOVED_DROP");
        let urgent = on("GENOS_LIGHTING_URGENT");
        let borrow = on("GENOS_LIGHTING_BORROW");
        let z = bgra
            | u32::from(moved_drop) << 3
            | u32::from(urgent) << 5
            | u32::from(borrow) << 7
            | u32::from(fill) << 6
            | u32::from(young_only) << 10
            | u32::from(picks) << 11;
        let tile = if self.lighting.settings.probes.tile == 0 {
            lighting_tile(h)
        } else {
            self.lighting.settings.probes.tile
        };
        let pc = [w, h, cols, rows, self.lighting.settings.probe_rays(), tile, z, self.lighting.frame];
        let image = self.color.image;
        let blocks = (w.div_ceil(8) as u64) * (h.div_ceil(8) as u64);
        // Full compose when the picture has no history yet, the scene moved, the
        // surface cache is off, or the block list cannot cover the image.
        // GENOS_LIGHTING_COMPOSE_ALL forces it (an A/B against the quiet path).
        let compose_full = !self.lighting.compose_valid
            || self.lighting.still == 0
            || self.lighting.settings.world_mode() != 3
            || !self.lighting.settings.compose_reuse
            || !self.lighting.settings.gi.enabled
            || std::env::var_os("GENOS_LIGHTING_COMPOSE_ALL").is_some()
            || blocks > LIGHTING_COMPOSE_CAP;
        // The raster colour as the base: pixels with no surface keep it. A held
        // frame keeps last frame's picture in the buffer instead.
        if compose_full {
            self.copy_image_buffer(image, 6, self.lighting.out.buffer, w, h, true);
        }
        self.memory_barrier(0x80 | 0x1000, 0x800, 0x40 | 0x1000, 0x20 | 0x40);
        let set = self.lighting.sets[slot];
        // Settled (frozen above): the probes and the light cache hold their light,
        // and only the pixels' direct light and the final pass run, so a still
        // scene draws the same picture every frame (no residual cache drift) and
        // idles cheaply. Rings adds the verification stride on that same idle.
        self.lighting.held = if frozen { self.lighting.held.saturating_add(1) } else { 0 };
        // A light cache round is due (GENOS_LIGHTING_CACHE_HZ caps them per second). On
        // the async queue (GENOS_LIGHTING_ASYNC=1) it goes there, else into this frame;
        // a frame without one runs the picture's passes only (lighting_mode 1).
        let due = !frozen
            && (self.lighting.cache_hz <= 0.0
                || self
                    .lighting
                    .last_round
                    .is_none_or(|t| t.elapsed().as_secs_f64() * self.lighting.cache_hz >= 1.0));
        let try_async = due
            && self.lighting.async_on
            && self.lighting.cache_full
            && self.lighting.settings.cache_rays_this_frame(self.lighting.frame);
        let round = due && !try_async;
        self.lighting.stats_round[slot] = round;
        let mut pc = pc;
        if round {
            let n = self.lighting.world_round;
            self.lighting.world_round = n.wrapping_add(1);
            pc[6] |= (n & 0xf_ffff) << 12;
            self.lighting.last_round = Some(Instant::now());
        } else {
            pc[6] |= 1 << 8;
        }
        let kept = probe_rays * LIGHTING_STRATA;
        let wake_rings = wake_cfg.mode == 1;
        // The kept-hit tail of the trace launch only re-traces geometry crossings
        // and fills. A lamp move is not in that list. Rings with nothing to cross
        // drops the tail. The probe rays stay until the frame is frozen: replacing
        // them with the verification stride froze a field that did not match legacy.
        let span_kept = match &changes {
            None => true,
            Some(list) => !list.is_empty(),
        };
        let planned = light_groups(
            &self.lighting.settings,
            self.lighting.frame,
            frozen,
            round,
            w,
            h,
            probes,
            probe_rays,
            kept,
            wake_rings,
            span_kept,
        );
        self.lighting.ctrl_bytes = pack_lighting_ctrl(
            &self.lighting.settings,
            self.lighting.frame,
            self.lighting.sun_height,
            planned.bias,
            planned.pixel_only,
        );
        if try_async {
            match self.submit_lighting_world(pc) {
                Ok(true) => self.lighting.last_round = Some(Instant::now()),
                Ok(false) => {}
                Err(e) => {
                    eprintln!("lighting: async light cache off: {e}");
                    self.lighting.async_on = false;
                }
            }
        }
        self.lighting.frame_tags[slot] = planned.tags.clone();
        self.lighting.frame_counts[slot] = planned.counts;
        self.lighting.frame_shadow[slot] = self.lighting.settings.shadows.enabled
            && (planned.counts[B_DIRECT as usize] > 0 || planned.counts[B_HIT as usize] > 0);
        self.lighting.frame_prefill[slot] = self.lighting.settings.prefill_this_frame();
        let groups = planned.groups;
        let atrous_stages = planned.atrous_stages;
        self.memory_barrier(0x1000, 0x1000, 0x1000, 0x1000);
        unsafe {
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.lighting.stats_dev[slot].buffer,
                LIGHTING_CTRL_WORD * 4,
                self.lighting.ctrl_bytes.len() as u64,
                self.lighting.ctrl_bytes.as_ptr() as *const c_void,
            );
            let phase = self.lighting.wake_phase[slot];
            self.lighting.wake_phase[slot] = phase.wrapping_add(1);
            let mut wake_hdr = [0u32; 13];
            wake_hdr[WAKE_MODE - 140] = wake_cfg.mode;
            wake_hdr[WAKE_TOL - 140] = self.lighting.wake.tolerance(wake_cfg.tolerance).to_bits();
            wake_hdr[WAKE_BUDGET - 140] = wake_cfg.budget;
            wake_hdr[WAKE_RADIUS - 140] = wake_cfg.max_radius.to_bits();
            wake_hdr[WAKE_EVENTS - 140] = if wake_cfg.mode == 1 {
                self.lighting.wake.len() as u32
            } else {
                0
            };
            wake_hdr[WAKE_PHASE - 140] = phase;
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.lighting.stats_dev[slot].buffer,
                (WAKE_MODE as u64) * 4,
                wake_hdr.len() as u64 * 4,
                wake_hdr.as_ptr() as *const c_void,
            );
            if wake_cfg.mode == 1 && self.lighting.settings.gi.enabled && probes > 0 {
                let queued = self.lighting.wake.len();
                if wake_block_due(queued, self.lighting.wake_block_clear) {
                    let packed = self.lighting.wake.pack();
                    let at = lighting_wake_events_at(
                        probes as u64,
                        self.lighting.settings.probe_rays() as u64,
                        w as u64,
                        h as u64,
                    ) * 16;
                    debug_assert!(
                        at + 256 <= self.lighting.work.size,
                        "wake events sit past the work buffer ({at} of {})",
                        self.lighting.work.size
                    );
                    if at + 256 <= self.lighting.work.size {
                        (self.fns.cmd_update_buffer)(
                            self.cmd,
                            self.lighting.work.buffer,
                            at,
                            256,
                            packed.as_ptr() as *const c_void,
                        );
                        self.lighting.wake_block_clear = queued == 0;
                    }
                }
            }
            if self.lighting.cache_full {
            (self.fns.cmd_fill_buffer)(
                self.cmd,
                self.lighting.cache_keys.buffer,
                (lighting_compose_at() + 3) * 4,
                4,
                u32::from(compose_full),
            );
            if compose_full {
                // Empty rects (min > max) so this frame's compose records who reads
                // each texel. The next held frame marks only those blocks.
                let words = LIGHTING_CACHE_SLOTS * 2;
                let minx = lighting_compose_minx();
                (self.fns.cmd_fill_buffer)(
                    self.cmd,
                    self.lighting.cache_keys.buffer,
                    minx * 4,
                    words * 4,
                    u32::MAX,
                );
                (self.fns.cmd_fill_buffer)(
                    self.cmd,
                    self.lighting.cache_keys.buffer,
                    (minx + words) * 4,
                    words * 4,
                    0,
                );
            }
            }
            self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x1);
            (self.fns.cmd_bind_set)(
                self.cmd,
                1,
                self.lighting.layout,
                0,
                1,
                &set,
                0,
                std::ptr::null(),
            );
            (self.fns.cmd_push)(
                self.cmd,
                self.lighting.layout,
                0x20,
                0,
                32,
                pc.as_ptr() as *const c_void,
            );
            // GENOS_GPU_TIMES: a stamp before the passes and after each (full frames).
            let stamps = self
                .gpu_times
                .as_mut()
                .filter(|t| !t.lighting_pool.is_null())
                .map(|t| {
                    t.lighting_ids[slot] = groups.iter().map(|g| g.0).collect();
                    t.lighting_pool
                });
            let first = slot as u32 * LIGHTING_STAMPS;
            if let Some(pool) = stamps {
                (self.fns.cmd_reset_query)(self.cmd, pool, first, LIGHTING_STAMPS);
                (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, first);
            }
            let mut filter_round = 0u32;
            let mut atrous_i = 0usize;
            let mut atrous_ran = false;
            let mut picked = false;
            // The push constants the next pass sees (the gather and filter rounds add bits).
            let mut cur_pc = pc;
            for (k, (pass, gx, gy)) in groups.iter().copied().enumerate() {
                if pass == 4 {
                    // The gather's two rounds: the picks (bit 1), then the binning.
                    let mut round_pc = pc;
                    round_pc[6] |= u32::from(!picked) << 1;
                    picked = true;
                    (self.fns.cmd_push)(
                        self.cmd,
                        self.lighting.layout,
                        0x20,
                        0,
                        32,
                        round_pc.as_ptr() as *const c_void,
                    );
                    cur_pc = round_pc;
                }
                if pass == 8 {
                    // S72V: the a-trous stage (lighting_atrous_stage, bits 1-3).
                    // S72V6: bits 1-3 cleared first; bit 3 is the gather's "changes
                    // this frame", so on frames with a change every stage read 4-7
                    // and the variance pass and first levels never ran.
                    // Five iterations are stages 0..6. Fewer iterations jump to the push.
                    let stage = atrous_stages.get(atrous_i).copied().unwrap_or(0);
                    atrous_i += 1;
                    atrous_ran = true;
                    let mut round_pc = pc;
                    round_pc[6] = (round_pc[6] & !0xe) | stage << 1;
                    (self.fns.cmd_push)(self.cmd, self.lighting.layout, 0x20, 0, 32, round_pc.as_ptr() as *const c_void);
                    cur_pc = round_pc;
                }
                if pass == 7 && atrous_ran {
                    (self.fns.cmd_push)(self.cmd, self.lighting.layout, 0x20, 0, 32, pc.as_ptr() as *const c_void);
                    cur_pc = pc;
                }
                if pass == 5 {
                    // The spatial filter's three rounds (lighting_filter_round).
                    let mut round_pc = pc;
                    round_pc[6] |= filter_round << 1;
                    filter_round += 1;
                    (self.fns.cmd_push)(
                        self.cmd,
                        self.lighting.layout,
                        0x20,
                        0,
                        32,
                        round_pc.as_ptr() as *const c_void,
                    );
                    cur_pc = round_pc;
                }
                // S70A kinds (the third field past 1): the args pass, the dense
                // indirect dispatches, mode-1 picture parts of a round.
                let kind = if gy >= LIGHTING_ARGS { gy } else { 0 };
                if kind != 0 {
                    let mut k_pc = cur_pc;
                    k_pc[6] |= 1 << 4;
                    if kind == LIGHTING_MODE1 {
                        k_pc[6] = (cur_pc[6] & !0x310) | 1 << 8;
                    } else if kind == LIGHTING_WAKE_VERIFY {
                        // Mode 3: this dispatch is the verification stride, not the grid.
                        k_pc[6] = (cur_pc[6] & !0x310) | 3 << 8;
                    } else if kind == LIGHTING_DENSE_RAYS {
                        k_pc[6] = (k_pc[6] & !0x300) | 2 << 8;
                    }
                    (self.fns.cmd_push)(
                        self.cmd,
                        self.lighting.layout,
                        0x20,
                        0,
                        32,
                        k_pc.as_ptr() as *const c_void,
                    );
                }
                (self.fns.cmd_bind_pipe)(self.cmd, 1, self.lighting.pipes[pass]);
                if pass == 7 && !compose_full {
                    (self.fns.cmd_dispatch_indirect)(
                        self.cmd,
                        self.lighting.cache_keys.buffer,
                        lighting_compose_at() * 4,
                    );
                } else {
                    match kind {
                        LIGHTING_DENSE_RAYS | LIGHTING_DENSE_ENTRIES => {
                            let at = (LIGHTING_ARGS_AT + if kind == LIGHTING_DENSE_RAYS { 0 } else { 3 }) * 4;
                            (self.fns.cmd_dispatch_indirect)(self.cmd, self.lighting.cache_keys.buffer, at);
                        }
                        _ => (self.fns.cmd_dispatch)(self.cmd, gx.max(1), if kind != 0 { 1 } else { gy.max(1) }, 1),
                    }
                }
                if kind != 0 {
                    (self.fns.cmd_push)(self.cmd, self.lighting.layout, 0x20, 0, 32, cur_pc.as_ptr() as *const c_void);
                }
                // The args pass's sizes are read as indirect commands (stage 0x2, access 0x1).
                self.memory_barrier(0x800, 0x800 | 0x1000 | 0x2, 0x40, 0x20 | 0x40 | 0x800 | 0x1);
                if let Some(pool) = stamps {
                    (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, first + 1 + k as u32);
                }
            }
        }
        if compose_full {
            self.lighting.compose_valid = true;
        }
        self.image_barrier(image, 6, 7, 0x1000, 0x1000, 0x800, 0x1000);
        self.copy_image_buffer(image, 7, self.lighting.out.buffer, w, h, false);
        self.image_barrier(image, 7, 6, 0x1000, 0x1000 | 0x800, 0x1000, 0x800 | 0x20);
        // S78W: the frame's stats to the host-visible copy (read a frame later).
        self.memory_barrier(0x800 | 0x1000, 0x1000, 0x40 | 0x800, 0x800);
        unsafe {
            #[repr(C)]
            struct Region {
                src: u64,
                dst: u64,
                size: u64,
            }
            let copy_bytes = if self.unfilled.count {
                LIGHTING_STATS_BYTES + unfilled_mask_bytes(w, h)
            } else {
                LIGHTING_STATS_BYTES
            };
            let region = Region { src: 0, dst: 0, size: copy_bytes };
            self.lighting.unfilled_slot[slot] = if self.unfilled.count {
                UnfilledSlot {
                    recorded: true,
                    frame: self.lighting.frame,
                    width: w,
                    height: h,
                    eye: self.unfilled_eye,
                    yaw: self.unfilled_yaw,
                    pitch: self.unfilled_pitch,
                    shot: self.copy_picture,
                }
            } else {
                UnfilledSlot::default()
            };
            (self.fns.cmd_copy_buffer)(
                self.cmd,
                self.lighting.stats_dev[slot].buffer,
                self.lighting.stats[slot].buffer,
                1,
                &region as *const Region as *const u8,
            );
        }
        self.memory_barrier(0x1000, 0x800 | 0x1000 | 0x4000, 0x1000, 0x20 | 0x40 | 0x800 | 0x2000);
        self.lighting_dump_copy();
    }

    /// S78S: frames N and N + 1 (GENOS_LIGHTING_DUMP): copy the lighting buffers
    /// into readback buffers at the end of the frame's passes.
    fn lighting_dump_copy(&mut self) {
        let Some(n) = std::env::var("GENOS_LIGHTING_DUMP").ok().and_then(|v| v.parse::<u64>().ok()) else {
            return;
        };
        let f = self.lighting.frame as u64;
        if f != n && f != n + 1 {
            return;
        }
        let srcs: [(&'static str, Handle, u64); 5] = [
            ("cache", self.lighting.cache.buffer, self.lighting.cache.size),
            ("keys", self.lighting.cache_keys.buffer, self.lighting.cache_keys.size),
            ("work", self.lighting.work.buffer, self.lighting.work.size),
            ("gbuf", self.lighting.gbuf.buffer, self.lighting.gbuf.size),
            ("out", self.lighting.out.buffer, self.lighting.out.size),
        ];
        self.memory_barrier(0x800 | 0x1000, 0x1000, 0x40 | 0x800, 0x800);
        let mut set = Vec::new();
        for (name, src, size) in srcs {
            let Ok(dst) = self.make_buffer(size, 0x2, Memory::Readback) else {
                eprintln!("lighting dump: no readback buffer for {name} ({size} bytes)");
                continue;
            };
            #[repr(C)]
            struct Region {
                src: u64,
                dst: u64,
                size: u64,
            }
            let region = Region { src: 0, dst: 0, size };
            unsafe {
                (self.fns.cmd_copy_buffer)(self.cmd, src, dst.buffer, 1, &region as *const Region as *const u8);
            }
            set.push((name, dst));
        }
        self.memory_barrier(0x1000, 0x800 | 0x1000, 0x1000, 0x20 | 0x40 | 0x800);
        self.lighting.dump.push((f, set));
    }

    /// S78S: the dumped frames, once surely done (a few frames on): write each
    /// buffer as dir/f{frame}_{name}.bin and the layout (dims, probes) as JSON.
    fn lighting_dump_write(&mut self) {
        if self.lighting.dump.is_empty() {
            return;
        }
        let last = self.lighting.dump.iter().map(|d| d.0).max().unwrap_or(0);
        if (self.lighting.frame as u64) < last + 3 {
            return;
        }
        unsafe {
            (self.fns.device_wait)(self.device);
        }
        let dir = std::env::var("GENOS_LIGHTING_DUMP_DIR").unwrap_or_else(|_| "lighting-dump".into());
        let _ = std::fs::create_dir_all(&dir);
        let dumps = std::mem::take(&mut self.lighting.dump);
        for (f, set) in dumps {
            for (name, mut buf) in set {
                let mut raw = vec![0u8; buf.size as usize];
                unsafe {
                    let mut mapped = std::ptr::null_mut();
                    if (self.fns.map_mem)(self.device, buf.memory, 0, buf.size, 0, &mut mapped) == 0 {
                        std::ptr::copy_nonoverlapping(mapped as *const u8, raw.as_mut_ptr(), raw.len());
                        (self.fns.unmap_mem)(self.device, buf.memory);
                    }
                }
                let path = format!("{dir}/f{f}_{name}.bin");
                if let Err(e) = std::fs::write(&path, &raw) {
                    eprintln!("lighting dump: {path}: {e}");
                }
                self.destroy_buffer(&mut buf);
            }
            let d = self.lighting.dims;
            let json = format!(
                "{{\"frame\": {f}, \"width\": {}, \"height\": {}, \"probes_x\": {}, \"probes_y\": {}, \"rays\": {}, \"strata\": {}, \"cache_slots\": {}}}\n",
                d[0], d[1], d[2], d[3], LIGHTING_RAYS, LIGHTING_STRATA, LIGHTING_CACHE_SLOTS
            );
            let _ = std::fs::write(format!("{dir}/f{f}_layout.json"), json);
            eprintln!("lighting dump: frame {f} written to {dir}");
        }
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

    fn destroy_lighting(&mut self) {
        self.wait_lighting_world();
        self.lighting.world_busy = false;
        let mut world_stats = std::mem::replace(&mut self.lighting.world_stats, Buffer::empty());
        let mut world_scene = std::mem::replace(&mut self.lighting.world_scene, Buffer::empty());
        self.destroy_buffer(&mut world_stats);
        self.destroy_buffer(&mut world_scene);
        unsafe {
            if !self.lighting.world_fence.is_null() {
                (self.fns.destroy_fence)(self.device, self.lighting.world_fence, std::ptr::null());
                self.lighting.world_fence = std::ptr::null_mut();
            }
        }
        // The command buffer goes with the light command pool; the query pool too
        // is left to the device (as the frame's are).
        let mut gbuf = std::mem::replace(&mut self.lighting.gbuf, Buffer::empty());
        let mut work = std::mem::replace(&mut self.lighting.work, Buffer::empty());
        let mut out = std::mem::replace(&mut self.lighting.out, Buffer::empty());
        let mut keys = std::mem::replace(&mut self.lighting.cache_keys, Buffer::empty());
        let mut cache = std::mem::replace(&mut self.lighting.cache, Buffer::empty());
        for k in 0..2 {
            self.unpin_stats(k);
            let mut stats_dev = std::mem::replace(&mut self.lighting.stats_dev[k], Buffer::empty());
            self.destroy_buffer(&mut stats_dev);
            let mut stats = std::mem::replace(&mut self.lighting.stats[k], Buffer::empty());
            self.destroy_buffer(&mut stats);
        }
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        self.destroy_buffer(&mut keys);
        self.destroy_buffer(&mut cache);
        unsafe {
            for pipe in self
                .lighting
                .pipes
                .iter_mut()
                .chain(std::iter::once(&mut self.lighting.gbuffer_pipe))
            {
                if !pipe.is_null() {
                    (self.fns.destroy_pipeline)(self.device, *pipe, std::ptr::null());
                    *pipe = std::ptr::null_mut();
                }
            }
            if !self.lighting.layout.is_null() {
                (self.fns.destroy_layout)(self.device, self.lighting.layout, std::ptr::null());
                self.lighting.layout = std::ptr::null_mut();
            }
            if !self.lighting.pool.is_null() {
                (self.fns.destroy_desc_pool)(self.device, self.lighting.pool, std::ptr::null());
                self.lighting.pool = std::ptr::null_mut();
            }
            if !self.lighting.desc_layout.is_null() {
                (self.fns.destroy_desc_layout)(self.device, self.lighting.desc_layout, std::ptr::null());
                self.lighting.desc_layout = std::ptr::null_mut();
            }
        }
        self.lighting.ready = false;
    }
}

/// GPU writes this memory. A plain copy can be reused from the last frame.
unsafe fn copy_host_words(src: *const u32, dst: *mut u32, n: usize) {
    for i in 0..n {
        dst.add(i).write(src.add(i).read_volatile());
    }
}

/// S70B: the frame's counters are read back only where something uses them.
fn lighting_stats_wanted(world: u32) -> bool {
    lighting_debug_enabled() || !(world == 3 && lighting_sc_flags_env() & 8192 != 0)
}

/// `Some(false)` when `arg` is `--lighting-debug` (the value, if any, is inline).
/// `None` when `arg` is some other flag.
pub fn lighting_debug_flag_needs_value(arg: &str) -> Option<bool> {
    let (name, _) = lighting_debug_dash(arg)?;
    (name == "lighting-debug").then_some(false)
}

/// Reject a bad `--lighting-debug` argument. The value is inline
/// (`--lighting-debug=0`); a bare flag turns the counters on.
pub fn check_lighting_debug_arg(arg: &str, _next: Option<&str>) -> Result<(), String> {
    parse_lighting_debug_flag(arg).map(|_| ())
}

/// `--lighting-debug` wins. `GENOS_LIGHTING_DEBUG` applies only when that
/// argument was omitted, and any set value turns the counters on.
fn lighting_debug_enabled() -> bool {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    lighting_debug_from(&refs, std::env::var_os("GENOS_LIGHTING_DEBUG").is_some()).unwrap_or(false)
}

fn lighting_debug_from(args: &[&str], env_set: bool) -> Result<bool, String> {
    let mut chosen = None;
    for arg in args {
        if let Some(on) = parse_lighting_debug_flag(arg)? {
            chosen = Some(on);
        }
    }
    Ok(chosen.unwrap_or(env_set))
}

fn lighting_debug_dash(arg: &str) -> Option<(&str, Option<&str>)> {
    let rest = arg.strip_prefix("--")?;
    if rest.is_empty() {
        return None;
    }
    match rest.split_once('=') {
        Some((name, value)) => Some((name, Some(value))),
        None => Some((rest, None)),
    }
}

fn parse_lighting_debug_flag(arg: &str) -> Result<Option<bool>, String> {
    let Some((name, inline)) = lighting_debug_dash(arg) else {
        return Ok(None);
    };
    if name != "lighting-debug" {
        return Ok(None);
    }
    let Some(value) = inline else {
        return Ok(Some(true));
    };
    if value.is_empty() {
        return Err("--lighting-debug needs a value".into());
    }
    match value.to_ascii_lowercase().as_str() {
        "1" | "on" | "true" => Ok(Some(true)),
        "0" | "off" | "false" => Ok(Some(false)),
        other => Err(format!(
            "--lighting-debug {other} is not 0, 1, on, off, true, or false"
        )),
    }
}

// Defaults with no env set (S54K, gi-v2): GI v2, the surface cache, cap 64, the
// S49 flag set and object-local texels. Each env var still overrides.
const LIGHTING_DEFAULT_SC_CAP: u32 = 64;
const LIGHTING_DEFAULT_SC_FLAGS: u32 = 6007050;
/// Lighting stats buffer: 24 words, diag[64], chg[32], the a-trous fallback
/// (fraction, mode, threshold, filtered texels) at `LIGHTING_ATROUS_STAT`, the
/// control block at word 124 (16 words), the probe-wake block, then the
/// unfilled-pixel count and the shaded-pixel count. Two pad words keep the
/// mask on the header boundary. The mask follows, one bit per composed pixel,
/// and is not part of this header.
/// The change-counter enable word stays chg[31] (word 119).
/// Words 140..151 are the probe-wake counters (lighting_cache.glsl). Word 152
/// is the persist phase. Words 160..295 (ring bits and the woken list) are
/// cleared every frame. Words 296..391 are the ring frontier and are not.
/// Words 392 and 393 are the unfilled and shaded counts. The mask starts at 396.
const LIGHTING_STATS_WORDS: usize = 396;
const LIGHTING_STATS_BYTES: u64 = LIGHTING_STATS_WORDS as u64 * 4;
const WAKE_MODE: usize = 140;
const WAKE_TOL: usize = 141;
const WAKE_BUDGET: usize = 142;
const WAKE_RADIUS: usize = 143;
const WAKE_EVENTS: usize = 144;
const WAKE_RINGS: usize = 145;
const WAKE_SPLITS: usize = 146;
const WAKE_WOKEN: usize = 147;
const WAKE_HITS: usize = 148;
const WAKE_MISSES: usize = 149;
const WAKE_TRACED: usize = 150;
const WAKE_RAYS: usize = 151;
const WAKE_PHASE: usize = 152;
const WAKE_LIST: usize = 168;
const WAKE_LIST_CAP: usize = 32;
const WAKE_CLEAR_WORDS: usize = 296;
/// After the wake tail. The control block occupies 124..140, then the wake block.
const LIGHTING_UNFILLED_WORD: usize = 392;
const LIGHTING_SHADED_WORD: usize = 393;
/// Word 0 of the mask. Matches lighting_cache.glsl.
const LIGHTING_UNFILLED_MASK_AT: usize = 396;
/// (4096/32) * 2160. A larger picture clamps the mask.
const LIGHTING_UNFILLED_MASK_CAP: u32 = 276480;
const _: () = assert!(LIGHTING_UNFILLED_MASK_AT == LIGHTING_STATS_WORDS);
const _: () = assert!(LIGHTING_CTRL_WORD as usize + 16 == WAKE_MODE);
const _: () = assert!(WAKE_CLEAR_WORDS + 96 == LIGHTING_UNFILLED_WORD);
const _: () = assert!(LIGHTING_UNFILLED_WORD + 4 == LIGHTING_UNFILLED_MASK_AT);
const LIGHTING_CHG_ENABLE_WORD: u64 = 24 + 64 + 31;

/// One slot's unfilled readback: written at the end of the passes, read after the fence.
#[derive(Clone, Copy, Default)]
struct UnfilledSlot {
    recorded: bool,
    frame: u32,
    width: u32,
    height: u32,
    eye: [f32; 3],
    yaw: f32,
    pitch: f32,
    shot: bool,
}

fn unfilled_mask_words(width: u32, height: u32) -> u32 {
    if width == 0 || height == 0 {
        return 0;
    }
    let stride = width.div_ceil(32);
    stride.saturating_mul(height).min(LIGHTING_UNFILLED_MASK_CAP)
}

fn unfilled_mask_bytes(width: u32, height: u32) -> u64 {
    u64::from(unfilled_mask_words(width, height)) * 4
}

fn lighting_stats_storage_bytes() -> u64 {
    LIGHTING_STATS_BYTES + u64::from(LIGHTING_UNFILLED_MASK_CAP) * 4
}
const LIGHTING_CHG_ENABLE_BYTE: u64 = LIGHTING_CHG_ENABLE_WORD * 4;
const LIGHTING_ATROUS_STAT: usize = 120;
/// S78S: the change counters (lighting_chg) run with GENOS_LIGHTING_SC_TRACE.
fn lighting_chg_on() -> bool {
    std::env::var_os("GENOS_LIGHTING_SC_TRACE").is_some()
}

/// `GENOS_LIGHTING_ATROUS_DIRTY` in millionths. Default 0.25: levels 1-5 keep
/// the work lists while the fullest level is at or under a quarter of the
/// filtered texels, and run the dense sweep above that. `0` is always dense,
/// `1` keeps the lists up to the storage cap. `override_frac` wins (tests).
fn lighting_atrous_ppm(override_frac: Option<f32>) -> u32 {
    let frac = override_frac.unwrap_or_else(|| {
        std::env::var("GENOS_LIGHTING_ATROUS_DIRTY")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.25)
    });
    (frac.clamp(0.0, 1.0) * 1_000_000.0).round() as u32
}

/// GENOS_LIGHTING_SC_FLAGS, default LIGHTING_DEFAULT_SC_FLAGS.
fn lighting_sc_flags_env() -> u32 {
    static V: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("GENOS_LIGHTING_SC_FLAGS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(LIGHTING_DEFAULT_SC_FLAGS)
    })
}

/// GENOS_LIGHTING_SC_LOCAL: object-local texels, on unless 0.
fn lighting_sc_local_env() -> bool {
    std::env::var("GENOS_LIGHTING_SC_LOCAL").map_or(true, |v| v != "0")
}

/// Vec4 index of the changes block in the work buffer (lighting_common.glsl
/// lighting_change_at), for `probes` probes, `cache_rays` light cache rays and `kept`
/// kept hits. The two `kept / 4` terms are the rgb9e5 light words and the
/// direct-luminance baseline, four hits per vec4.
fn lighting_change_at(probes: u64, cache_rays: u64, kept: u64) -> u64 {
    2 * probes
        + cache_rays
        + 7 * probes
        + 2 * LIGHTING_STRATA as u64 * probes
        + kept
        + kept / 4
        + kept / 4
}

/// Vec4 index of the first wake event. Matches `lighting_wake_event_at`:
/// after the gather's raw cells (`lighting_graw_at(probes, 0)`), one vec4 per probe.
fn lighting_wake_events_at(probes: u64, rays: u64, width: u64, height: u64) -> u64 {
    let cache_rays = (LIGHTING_CACHE_BATCH as u64) * (LIGHTING_CACHE_RAYS as u64);
    let kept = probes * rays * LIGHTING_STRATA as u64;
    let change = lighting_change_at(probes, cache_rays, kept);
    let view2 = change + (LIGHTING_CHANGE_HEAD + LIGHTING_CHANGES) as u64 + 4 * probes;
    let fill0 = view2 + 2 * probes * rays;
    let fill_light = fill0 + kept;
    let pix = fill_light + kept / 4;
    let gdirty = pix + (width * height + 1) / 2;
    let probe0 = gdirty + 2 * probes + probes * rays;
    probe0 + probes
}

/// Occluder records of a scene block (`pack::scene_bytes`), 64 bytes each.
fn lighting_occs(bytes: &[u8]) -> Vec<&[u8]> {
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

/// Bounding sphere of an occluder record.
fn lighting_occ_bounds(rec: &[u8]) -> [f32; 4] {
    let f = |i: usize| f32::from_le_bytes([rec[i * 4], rec[i * 4 + 1], rec[i * 4 + 2], rec[i * 4 + 3]]);
    let (hx, hy, hz, r) = (f(4).abs(), f(5).abs() * 0.5, f(6).abs(), f(7).abs());
    let (hx, hz) = (hx.max(r), hz.max(r));
    [f(0), f(1), f(2), (hx * hx + hy * hy + hz * hz).sqrt() + 0.05]
}

/// Movers: occluders (by index) whose record differs between two scene blocks of
/// the same length join `set` for good (a list of another length starts it again);
/// returns their bounds now.
pub(crate) fn lighting_movers(old: &[u8], new: &[u8], set: &mut std::collections::BTreeMap<usize, u32>) -> Vec<[[f32; 4]; 4]> {
    let (a, b) = (lighting_occs(old), lighting_occs(new));
    set.retain(|_, left| {
        *left = left.saturating_sub(1);
        *left > 0
    });
    if a.len() != b.len() {
        set.clear();
    } else {
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            if x != y {
                set.insert(i, LIGHTING_MOVER_KEEP);
            }
        }
    }
    set.keys()
        .filter_map(|&i| b.get(i).map(|r| (i, r)))
        .map(|(i, r)| {
            let f = |k: usize| f32::from_le_bytes([r[k * 4], r[k * 4 + 1], r[k * 4 + 2], r[k * 4 + 3]]);
            let rad = f(7).abs();
            // Shape 0 is a box (half extents x, z; height); else a round solid of
            // radius r. Cosine and sine of the yaw sit in the bounce vec4 (z, w).
            let (hx, hz) = if f(3) < 0.5 { (f(4).abs(), f(6).abs()) } else { (rad, rad) };
            let (c, s) = if f(3) < 0.5 { (f(14), f(15)) } else { (1.0, 0.0) };
            [
                lighting_occ_bounds(r),
                [f(0), f(1), f(2), (i + 1) as f32],
                [hx, f(5).abs() * 0.5, hz, 0.0],
                [c, s, 0.0, 0.0],
            ]
        })
        .collect()
}

/// Mesh movers: traced instances whose pose changed join `set` for
/// LIGHTING_MOVER_KEEP frames (another count starts it again); `poses` and `spheres`
/// become this frame's.
pub(crate) fn lighting_mesh_movers(
    live: &[&crate::mesh_field::FieldInstance],
    poses: &mut Vec<[f32; 16]>,
    set: &mut std::collections::BTreeMap<usize, u32>,
    spheres: &mut Vec<[f32; 4]>,
) {
    set.retain(|_, left| {
        *left = left.saturating_sub(1);
        *left > 0
    });
    if poses.len() != live.len() {
        set.clear();
    } else {
        for (i, (old, inst)) in poses.iter().zip(live).enumerate() {
            if *old != inst.pose {
                set.insert(i, LIGHTING_MOVER_KEEP);
            }
        }
    }
    poses.clear();
    poses.extend(live.iter().map(|f| f.pose));
    spheres.clear();
    spheres.extend(live.iter().map(|f| {
        let (lo, hi) = f.world_bounds();
        let c: [f32; 3] = std::array::from_fn(|a| 0.5 * (lo[a] + hi[a]));
        let r = (0..3).map(|a| (0.5 * (hi[a] - lo[a])).powi(2)).sum::<f32>().sqrt();
        [c[0], c[1], c[2], r + 0.05]
    }));
}

/// Frames an occluder stays a mover after it last moved (Lumen treats a mesh as
/// dynamic while it moves; once it rests its surroundings are ordinary cache).
const LIGHTING_MOVER_KEEP: u32 = 240;

/// Spheres (centre, radius) around every occluder that moved, came or went between
/// two scene blocks (`pack::scene_bytes`): the bounds before and after, so a kept
/// ray that crosses either sees something else now. None when there are more than
/// LIGHTING_CHANGES (drop every kept ray instead). Lamps are not compared: kept hits are
/// shaded again every frame, so light needs no change list.
pub(crate) fn lighting_changes(old: &[u8], new: &[u8]) -> Option<Vec<[f32; 4]>> {
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
        if out.len() == LIGHTING_CHANGES {
            return None;
        }
        out.push(bounds(rec));
    }
    Some(out)
}

/// Key of what the probes trace besides the occluders and meshes: the floor and the
/// roof (their place and colour, which kept hits hold).
pub(crate) fn lighting_trace_key(pack: &crate::pack::Pack) -> u64 {
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
pub(crate) fn lighting_scene_key(pack: &crate::pack::Pack, bytes: &[u8]) -> u64 {
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

/// S78S debug isolate toggles: attribute a change to exactly one component.
/// GENOS_LIGHTING_TOGGLES (start) or the file named by GENOS_LIGHTING_TOGGLES_FILE
/// (re-read every frame, so `echo direct_only > file` switches live), words:
/// freeze_cache (no surface-cache / world-cache value writes), direct_only (no
/// cache light at ray hits), bounce_only (no direct light at pixels or hits),
/// freeze_probes (kept-hit lights not rewritten).
const LIGHTING_TOGGLE_FREEZE_CACHE: u32 = 1;
const LIGHTING_TOGGLE_DIRECT_ONLY: u32 = 2;
const LIGHTING_TOGGLE_BOUNCE_ONLY: u32 = 4;
const LIGHTING_TOGGLE_FREEZE_PROBES: u32 = 8;
fn lighting_toggles() -> u32 {
    let mut text = std::env::var("GENOS_LIGHTING_TOGGLES").unwrap_or_default();
    if let Ok(path) = std::env::var("GENOS_LIGHTING_TOGGLES_FILE") {
        if let Ok(t) = std::fs::read_to_string(path) {
            text = t;
        }
    }
    let mut bits = 0;
    for w in text.split(|c: char| c == ',' || c.is_whitespace()).filter(|w| !w.is_empty()) {
        bits |= match w {
            "freeze_cache" => LIGHTING_TOGGLE_FREEZE_CACHE,
            "direct_only" => LIGHTING_TOGGLE_DIRECT_ONLY,
            "bounce_only" => LIGHTING_TOGGLE_BOUNCE_ONLY,
            "freeze_probes" => LIGHTING_TOGGLE_FREEZE_PROBES,
            _ => 0,
        };
    }
    bits
}

include!("lighting_dispatch.rs");
include!("probe_wake.rs");
