// GI frame path (docs/systems/gi-design.md), included into gpu.rs.
//
// The frame rasters a G-buffer (gbuffer.frag, same vertices and depth prepass),
// then runs small compute passes, each with at most one trace site:
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
// and present work as before. This is the only lighting path. The GI v1 path it
// replaced is gone (docs/systems/lighting/README.md).

/// Screen probe tile edge in pixels (Medium and above in the design).
/// Screen probe tile at 720p and up, pixels (Lumen's size).
pub(crate) const LIGHTING_TILE: u32 = 16;

/// Screen probe tile for a picture `height` pixels tall: LIGHTING_TILE from 720p up;
/// below that the tile shrinks with the picture (down to 4 px) so the probes keep
/// the same spacing on screen, about 45 rows. A small picture would otherwise get
/// probes a ninth of its height apart and lose detail a 720p picture keeps.
/// GENOS_LIGHTING_TILE (4..64) overrides it to experiment.
#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
fn lighting_tile(height: u32) -> u32 {
    genos_scene::lighting_env("GENOS_LIGHTING_TILE")
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
/// `pc.dims` + `pc.params` + `pc.lamp`. A shader that ignores `pc.lamp` still
/// accepts a 48-byte push.
const LIGHTING_PC_BYTES: u32 = 48;
/// ckeys word of the WRC refresh lists' two counts (lighting_cache.glsl LIGHTING_WRC_CHG_COUNT_AT).
const LIGHTING_WRC_COUNTS_AT: u64 = 2 * LIGHTING_CACHE_SLOTS + 2 + 66;
/// Strata of the screen probes' ray directions, one traced per frame, all kept
/// (lighting_common.glsl LIGHTING_STRATA): the visibility history cap.
pub(crate) const LIGHTING_STRATA: u32 = 8;
    /// Bounds of moved objects one frame can list (lighting_common.glsl LIGHTING_CHANGES).
    /// Spheres past this wait for a later frame. They are not a full-frame drop.
const LIGHTING_CHANGES: usize = 64;
/// Vec4s before the bounds in the changes block (lighting_common.glsl LIGHTING_CHANGE_HEAD).
const LIGHTING_CHANGE_HEAD: usize = 5;
/// GI counts as settled once this many frames have passed since the last change
/// record (the screen probes average up to 32 frames' rays, lighting_gather.comp)
/// and a full sweep of the light cache changed no patch by more than
/// LIGHTING_SETTLE_CHANGE. A camera move is not a change record.
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
    /// High: full resolution, adaptive off. The adaptive branch is not in this SPIR-V.
    atrous_plain: Handle,
    screen_pipe: Handle,
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
    /// The traced world as of the last frame the probes ran. Change records compare
    /// it with this frame. It is not rewritten while GI is frozen.
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
    /// Static light key of the frame the async cache round was built from.
    /// Movers, point lamps, and the camera are not in it.
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
    /// World-cache cells a frame's change records dirty.
    regions: crate::lighting_regions::WorldCacheRegions,
    /// Change spheres that did not fit the GPU list. Retried next frame.
    /// Never promoted to a full-frame drop.
    pending_spheres: Vec<[f32; 4]>,
    /// Mesh bounds from the previous sample.
    mesh_bounds: Vec<crate::lighting_regions::Aabb>,
    /// Mesh mover records since `note_field_movers`, consumed by the passes.
    mesh_records: Vec<crate::lighting_regions::ChangeRecord>,
    /// Floor, roof, sky, fire, and the sun. Point lamps and occluders are absent,
    /// so a mover does not clear the quiet schedule of every cell.
    static_light: u64,
    /// Eye and camera basis from the previous frame. Compared even while GI is
    /// frozen, so a camera that stops returns to the dirty-list compose.
    /// `kept_bytes` stays the scene the probes last shaded.
    camera_stamp: [u8; 60],
    camera_stamp_set: bool,
    /// Last frame's debug view word (scene header, byte 252). A view change
    /// redraws the picture. It is not a change record.
    debug_view_word: u32,
    debug_view_set: bool,
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
    /// Camera, occluders, meshes, and point lamps. The directional sun and the
    /// sky are left out, so a small sun step can stay off the full compose.
    view_content_key: u64,
    view_key: u64,
    view_key_valid: bool,
    sun_live_dir: [f32; 3],
    sun_live_light: f32,
    sun_live_n: u32,
    sun_commit_dir: [f32; 3],
    sun_commit_light: f32,
    sun_commit_n: u32,
    sun_commit_valid: bool,
    /// Frames whose shader reported the small-sun bit. The test reads this.
    sun_frames: u32,
    /// Last GPU mark (bit 0 sun, bit 1 catch-up) and the band it shaded.
    sun_gpu_mark: u32,
    sun_gpu_slice: u32,
    /// Sun compose. Built the first time a small sun step needs it.
    sun_pipe: Handle,
    /// The linear-bounce slice was cleared for this sun run. The first frame
    /// walks every pixel; later frames walk the band.
    sun_bounce_live: bool,
    /// Last compose: 1 full picture, 2 dirty list, 3 sun band. 0 before any.
    compose_kind: u8,
    /// Compose dispatches recorded for that frame. The sun path is one.
    compose_dispatches: u32,
    /// Point lamps only. `None` until the first frame, so a still open does not
    /// look like a lamp that just stopped.
    lamp_key: Option<u64>,
    lamp_moving: bool,
    catch_left: u32,
    /// The million-slot cache is allocated. Off, a small dummy stands in.
    cache_full: bool,
    /// Last buffer class. A live edit rebuilds only when this changes.
    alloc_key: (u32, u32, u32, u32, u32, u32, u32),
    /// Ping-pong reservoirs. 64 bytes unless the mode is reuse.
    lamp_res: Buffer,
    /// Fill the reservoir with empty lamps once, not every frame.
    lamp_res_clear: bool,
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
    /// Filtered texels `dirty_fraction` divided by. On the screen wavelet this
    /// is the fine-texel pixel count.
    pub filtered: u32,
    /// Levels 1-5 ran this frame. The screen wavelet sets this too.
    pub ran: bool,
    /// Pixels whose fine texel entered the screen wavelet.
    pub screen_pixels: u32,
    /// Pixels that read a 25 cm cell and were left on its running mean.
    pub coarse_pixels: u32,
    /// Fine pixels whose input moved more than 2% from the stored field.
    pub changed_pixels: u32,
}

/// Shadow rays traced last frame, and spatial reservoir merges.
///
/// `pixels` is primary pixels, `kept` probe hits, `fills` probe fills, `cache`
/// light-cache rays. Counts stay zero unless the diag push bit was set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LampPickCounts {
    pub pixels: u32,
    pub kept: u32,
    pub fills: u32,
    pub cache: u32,
    pub spatial: u32,
}

fn lighting_atrous_from(w: &[u32; LIGHTING_STATS_WORDS]) -> LightingAtrousStats {
    let mode_word = w[LIGHTING_ATROUS_STAT + 1];
    LightingAtrousStats {
        dirty_fraction: w[LIGHTING_ATROUS_STAT] as f32 / 1.0e6,
        mode: u32::from(mode_word == 1),
        threshold: w[LIGHTING_ATROUS_STAT + 2] as f32 / 1.0e6,
        filtered: w[LIGHTING_ATROUS_STAT + 3],
        ran: mode_word != 0,
        // diag[61..63] sit at stats words 85..87 (diag[0] is word 24).
        changed_pixels: w[85],
        screen_pixels: w[86],
        coarse_pixels: w[87],
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
    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
    )]
    fn default() -> Self {
        Self {
            ready: false,
            desc_layout: std::ptr::null_mut(),
            layout: std::ptr::null_mut(),
            pool: std::ptr::null_mut(),
            sets: [std::ptr::null_mut(); 2],
            pipes: [std::ptr::null_mut(); 9],
            atrous_plain: std::ptr::null_mut(),
            screen_pipe: std::ptr::null_mut(),
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
            async_on: genos_scene::lighting_env("GENOS_LIGHTING_ASYNC").is_ok_and(|v| v == "1"),
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
            regions: crate::lighting_regions::WorldCacheRegions::new(0.25),
            pending_spheres: Vec::new(),
            mesh_bounds: Vec::new(),
            mesh_records: Vec::new(),
            static_light: 0,
            camera_stamp: [0; 60],
            camera_stamp_set: false,
            debug_view_word: 0,
            debug_view_set: false,
            young_ok: genos_scene::lighting_env("GENOS_LIGHTING_YOUNG_ONLY").map_or(true, |v| v != "0"),
            stats_light: [0; 2],
            stats_young: [false; 2],
            dump: Vec::new(),
            world_light: 0,
            world_young: false,
            cache_hz: genos_scene::lighting_env("GENOS_LIGHTING_CACHE_HZ")
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
            view_content_key: 0,
            view_key: 0,
            view_key_valid: false,
            sun_live_dir: [0.0; 3],
            sun_live_light: 0.0,
            sun_live_n: 0,
            sun_commit_dir: [0.0; 3],
            sun_commit_light: 0.0,
            sun_commit_n: 0,
            sun_commit_valid: false,
            sun_frames: 0,
            sun_gpu_mark: 0,
            sun_gpu_slice: 0,
            sun_pipe: std::ptr::null_mut(),
            sun_bounce_live: false,
            compose_kind: 0,
            compose_dispatches: 0,
            lamp_key: None,
            lamp_moving: false,
            catch_left: 0,
            cache_full: false,
            alloc_key: (0, 0, 0, 0, 0, 0, 0),
            lamp_res: Buffer::empty(),
            lamp_res_clear: false,
            frame_tags: [Vec::new(), Vec::new()],
            frame_counts: [[0; BUCKETS]; 2],
            frame_shadow: [false; 2],
            frame_prefill: [false; 2],
            last_ms: [0.0; BUCKETS],
            last_dispatches: [0; BUCKETS],
            ctrl_bytes: [0; 64],
        }
    }
}

/// S70A dispatch kinds in a round's group list (the third field; real heights stay below).
const LIGHTING_ARGS: u32 = 1 << 30;
const LIGHTING_MODE1: u32 = LIGHTING_ARGS + 1;
const LIGHTING_DENSE_RAYS: u32 = LIGHTING_ARGS + 2;
const LIGHTING_DENSE_ENTRIES: u32 = LIGHTING_ARGS + 3;
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
/// Sun-step words after the a-trous meta. Matches `LIGHTING_SUN_AT` in
/// lighting_cache.glsl (8 words: flags, committed direction, both lights, catch-up).
/// The picture compose does not read them. They are not the regime tube.
const LIGHTING_SUN_WORDS: u64 = 8;
const LIGHTING_SUN_SMALL_BIT: u32 = 1;
/// Picture compose over the whole grid.
const COMPOSE_FULL: u8 = 1;
/// Dirty 8×8 list.
const COMPOSE_LIST: u8 = 2;
/// One full-grid sun compose. Probes are walked on the band.
const COMPOSE_SUN: u8 = 3;

fn compose_label(kind: u8) -> &'static str {
    match kind {
        COMPOSE_FULL => "full",
        COMPOSE_LIST => "list",
        COMPOSE_SUN => "sun",
        _ => "none",
    }
}
/// A step inside this relative sun/sky irradiance of the last full refresh stays amortised.
const LIGHTING_SUN_IRR: f32 = 0.02;
/// Stats words 142 and 143 (the unfilled pads). Bit 0 of the mark is the sun path.
const LIGHTING_SUN_MARK: usize = 142;
const LIGHTING_SUN_SLICE_WORD: usize = 143;

fn lighting_sun_at() -> u64 {
    lighting_atrous_ppm_at() + LIGHTING_ATROUS_META_WORDS
}
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
        let now: Vec<crate::lighting_regions::Aabb> = live
            .iter()
            .map(|inst| {
                let (min, max) = inst.world_bounds();
                crate::lighting_regions::Aabb { min, max }
            })
            .collect();
        self.lighting.mesh_records = crate::lighting_regions::bounds_change_records(
            &mut self.lighting.mesh_bounds,
            &now,
        );
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
        let bindings: Vec<Binding> = [0u32, 5, 6, 7, 8, 9, 10, 11, 12]
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
                size: LIGHTING_PC_BYTES,
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
            let size = Size { kind: 7, count: 27 };
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
        self.lighting.atrous_plain = self.make_compute_pipe(
            LIGHTING_ATROUS_PLAIN_SPV,
            self.lighting.layout,
            "atrous-plain",
        )?;
        self.note_pipeline_time("atrous-plain", start);
        let start = Instant::now();
        self.lighting.screen_pipe = self.make_compute_pipe(
            LIGHTING_SCREEN_SPV,
            self.lighting.layout,
            "screen-denoise",
        )?;
        self.note_pipeline_time("screen-denoise", start);
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
    ///
    /// The fence is created signalled. `submit_lighting_world` resets it only
    /// once the command buffer is ready to submit. Waiting here when nothing
    /// was submitted blocks exit forever: a failed record leaves the fence
    /// unsignalled, and `u64::MAX` never returns. A hung compose is a different
    /// wait (`vkDeviceWaitIdle` in `Drop`); that one ends when the picture
    /// shader does not walk the occluder grid.
    fn wait_lighting_world(&mut self) {
        if !self.lighting.world_busy || self.lighting.world_fence.is_null() {
            return;
        }
        let _ = self.wait_fence(self.lighting.world_fence, "lighting world");
        self.lighting.world_busy = false;
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
        // (7), strata (2 per probe and stratum), kept hits and their light (a uint
        // each), the changes block, the views (2 blocks of 2 per probe) and the
        // cells (2 tables of R per probe), then the fills and their light (as the
        // kept hits) (lighting_common.glsl).
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
                // +SLOTS: screen-owner election, past the sun block. See
                // LIGHTING_SCREEN_OWNER_AT. Another SLOTS plus a 3-word header:
                // the screen launch's tap-list dedup (LIGHTING_SCREEN_LIST_AT).
                self.lighting.cache_keys = self.make_buffer_queues(
                    (lighting_atrous_ppm_at()
                        + LIGHTING_ATROUS_META_WORDS
                        + LIGHTING_SUN_WORDS
                        + LIGHTING_CACHE_SLOTS
                        + 3
                        + LIGHTING_CACHE_SLOTS)
                        * 4,
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
                // Descriptors stay valid. The shader GI bit stays off with these
                // stand-ins, so the passes do not index them.
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
        // One extra slice holds linear bounce for a sun direct-patch. The
        // default compose does not write it. High (refresh every frame) is
        // picture + glow + that slice. The screen wavelet appends two vec4
        // slices (32 bytes a pixel) after that, so it does not alias the bounce.
        // Hash keeps the GI buffer. Preset high uses the screen slices.
        let screen = u64::from(settings.denoiser.enabled && settings.denoiser.screen);
        self.lighting.out = self.make_buffer(
            pixels * (12 + 4 * hist + 32 * screen),
            0x20 | 0x1 | 0x2,
            Memory::Device,
        )?;
        let mut lamp_res = std::mem::replace(&mut self.lighting.lamp_res, Buffer::empty());
        self.destroy_buffer(&mut lamp_res);
        let reuse = settings.lamp_pick.reuse();
        // Two frames of 16-byte records. Off keeps a dummy so binding 12 is valid.
        let lamp_bytes = if reuse { pixels.saturating_mul(32).max(64) } else { 64 };
        self.lighting.lamp_res = self.make_buffer_queues(lamp_bytes, 0x20 | 0x2, Memory::Device, &share)?;
        self.lighting.lamp_res_clear = reuse;
        self.lighting.dims = [w, h, cols, rows];
        self.lighting.alloc_key = lighting_alloc_key(&settings);
        self.lighting.compose_valid = false;
        self.lighting.sun_bounce_live = false;
        self.lighting.gbuf_clear = true;
        self.lighting.work_clear = gi;
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
                (12, self.lighting.lamp_res.buffer),
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

    /// Settle state for the debug tools. `still_frames` counts frames since the
    /// last change record. A camera move does not reset it.
    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
    )]
    pub(crate) fn lighting_stats(&self) -> SettleStats {
        let sweep = self.lighting.live.div_ceil(LIGHTING_CACHE_BATCH).max(1) + 1;
        // World cells on (GENOS_LIGHTING_WORLD=2): their running means (cap frames) have
        // to forget the poses before this one, so settling waits four caps.
        let world_mode = self.lighting.settings.world_mode();
        let world_wait = match world_mode {
            2 => {
                4 * genos_scene::lighting_env("GENOS_LIGHTING_WORLD_CAP").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(32)
            }
            // Surface cache: GENOS_LIGHTING_SC_SETTLE frames still (experiment: how long
            // its texels take to take in the bounced light).
            3 => genos_scene::lighting_env("GENOS_LIGHTING_SC_SETTLE").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0),
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

    pub(crate) fn gi_region_stats(&self) -> crate::lighting_regions::GiRegionStats {
        crate::lighting_regions::GiRegionStats {
            change_records: self.lighting.regions.change_records(),
            still_frames: self.lighting.still,
            deferred_spheres: u32::try_from(self.lighting.pending_spheres.len()).unwrap_or(u32::MAX),
        }
    }

    /// Rays the screen probes held in the last frame drawn, and probes placed
    /// (lighting_gather.comp): waits for that frame.
    pub(crate) fn lighting_probe_samples(&mut self) -> [u32; 2] {
        self.idle_device();
        self.read_lighting_stats(self.lighting.last_slot)
            .map_or(self.lighting.probe_samples, |w| [w[6], w[7]])
    }

    /// Shadow rays the last frame actually traced, by consumer, plus spatial
    /// merges. Zero unless `GENOS_LIGHTING_LAMP_PICK_DIAG` was set for that frame.
    /// Waits for that frame.
    pub(crate) fn lighting_lamp_pick_counts(&mut self) -> LampPickCounts {
        self.idle_device();
        let slot = self.lighting.last_slot;
        self.read_stats16(self.lighting.stats[slot].memory)
            .map(|w| LampPickCounts {
                pixels: w[LAMP_PICK_DIAG_WORD],
                kept: w[LAMP_PICK_DIAG_WORD + 1],
                fills: w[LAMP_PICK_DIAG_WORD + 2],
                cache: w[LAMP_PICK_DIAG_WORD + 3],
                spatial: w[LAMP_PICK_DIAG_WORD + 4],
            })
            .unwrap_or_default()
    }

    /// A-trous dirty fraction, mode and threshold for the last frame drawn.
    /// Waits for that frame. The words are the `lightingatrous` line.
    pub(crate) fn lighting_atrous_stats(&mut self) -> LightingAtrousStats {
        self.idle_device();
        let slot = self.lighting.last_slot;
        self.read_stats16(self.lighting.stats[slot].memory)
            .map(|w| lighting_atrous_from(&w))
            .unwrap_or_default()
    }

    /// Build the sun compose pipeline the first time a small step needs it.
    /// Startup still compiles the nine lighting pipelines only.
    fn ensure_sun_pipe(&mut self) -> Result<(), String> {
        if !self.lighting.sun_pipe.is_null() {
            return Ok(());
        }
        let start = Instant::now();
        self.lighting.sun_pipe =
            self.make_compute_pipe(SUN_COMPOSE_SPV, self.lighting.layout, "lighting sun compose")?;
        self.note_pipeline_time("lighting sun compose", start);
        Ok(())
    }

    /// Record the sun and the view, minus the sun and the sky. Point-lamp motion
    /// arms the corner catch-up for the frame the lamps stop.
    pub(crate) fn note_lighting_sun(&mut self, pack: &crate::pack::Pack) {
        self.lighting.view_content_key = lighting_view_content_key(pack);
        let (dir, light, n) = lighting_sun_live(pack);
        self.lighting.sun_live_dir = dir;
        self.lighting.sun_live_light = light;
        self.lighting.sun_live_n = n;
        let key = lighting_point_lamp_key(pack);
        match self.lighting.lamp_key {
            None => self.lighting.lamp_key = Some(key),
            Some(prev) if prev != key => {
                self.lighting.lamp_key = Some(key);
                self.lighting.lamp_moving = true;
                self.lighting.catch_left = 0;
            }
            Some(_) if self.lighting.lamp_moving => {
                self.lighting.lamp_moving = false;
                self.lighting.catch_left = sun_controls().catch_frames;
            }
            _ => {}
        }
    }

    /// True when this frame's sun step is inside the staleness budget and the
    /// camera, meshes, and point lamps held. The committed sun stays put while
    /// amortising, so the next tick is measured from the last full refresh.
    fn note_sun_amortise(&mut self) -> bool {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.lighting.view_content_key.hash(&mut hasher);
        self.mesh_field_key.hash(&mut hasher);
        self.lighting.draw_key.hash(&mut hasher);
        let view_key = hasher.finish();
        let view_held = self.lighting.view_key_valid && view_key == self.lighting.view_key;
        self.lighting.view_key = view_key;
        self.lighting.view_key_valid = true;
        let controls = sun_controls();
        let amortise = controls.enabled
            && !controls.full_effort
            && sun_step_small(&self.lighting)
            && view_held
            && self.lighting.sun_live_n > 0
            && self.lighting.settings.gi.enabled
            && self.lighting.settings.world_mode() == 3;
        if !amortise {
            self.lighting.sun_commit_dir = self.lighting.sun_live_dir;
            self.lighting.sun_commit_light = self.lighting.sun_live_light;
            self.lighting.sun_commit_n = self.lighting.sun_live_n;
            self.lighting.sun_commit_valid = true;
        }
        amortise
    }

    /// GPU mark from the last submitted frame. Waits for that frame.
    pub(crate) fn lighting_sun_stats(&mut self) -> LightingSunStats {
        self.idle_device();
        let slot = self.lighting.last_slot;
        if let Ok(words) = self.read_stats16(self.lighting.stats[slot].memory) {
            self.lighting.sun_gpu_mark = words[LIGHTING_SUN_MARK];
            self.lighting.sun_gpu_slice = words[LIGHTING_SUN_SLICE_WORD];
        }
        self.publish_sun_counter(false);
        LightingSunStats {
            gpu_sun: self.lighting.sun_gpu_mark & 1 != 0,
            gpu_catch: self.lighting.sun_gpu_mark & 2 != 0,
            slice: self.lighting.sun_gpu_slice,
            host_frames: self.lighting.sun_frames,
            catch_left: self.lighting.catch_left,
            full_effort: sun_controls().full_effort,
            compose: self.lighting.compose_kind,
            dispatches: self.lighting.compose_dispatches,
        }
    }

    /// The counter is the proof that full effort changed the work. A still or
    /// moving bench never takes the sun path, so it does not write this file.
    fn sun_counter_due(&self) -> bool {
        self.lighting.sun_frames > 0
            || sun_controls().full_effort
            || lighting_debug_on()
    }

    fn publish_sun_counter(&self, catch_log: bool) {
        let dir = sun_out_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let controls = sun_controls();
        let catch = u32::from(self.lighting.sun_gpu_mark & 2 != 0);
        let text = format!(
            "frame {}\nsun {}\nslice {}\ncatch {}\nhost_frames {}\ncatch_left {}\nbands {}\nfull_effort {}\ncompose {}\ndispatches {}\n",
            self.lighting.frame,
            u32::from(self.lighting.sun_gpu_mark & 1 != 0),
            self.lighting.sun_gpu_slice,
            catch,
            self.lighting.sun_frames,
            self.lighting.catch_left,
            controls.bands,
            u32::from(controls.full_effort),
            compose_label(self.lighting.compose_kind),
            self.lighting.compose_dispatches,
        );
        let _ = std::fs::write(dir.join("counter.txt"), text);
        if catch_log && controls.debug {
            append_catch_log(
                &dir,
                self.lighting.frame,
                catch,
                self.lighting.catch_left,
            );
        }
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
        // (the async queue's come back with its own submission).
        self.trace_sc(self.lighting.stats[slot].memory, "frame");
        self.lighting_dump_write();
        // The sun mark is a later read of a copy the frame already made. It does
        // not update light_quiet. That was the debug print that froze a still scene.
        if self.sun_counter_due() {
            if let Ok(words) = self.read_stats16(self.lighting.stats[slot].memory) {
                self.lighting.sun_gpu_mark = words[LIGHTING_SUN_MARK];
                self.lighting.sun_gpu_slice = words[LIGHTING_SUN_SLICE_WORD];
                if lighting_debug_on() {
                    eprintln!(
                        "lightingsun frame {} sun {} slice {} catch {} host_frames {}",
                        self.lighting.frame,
                        u32::from(self.lighting.sun_gpu_mark & 1 != 0),
                        self.lighting.sun_gpu_slice,
                        u32::from(self.lighting.sun_gpu_mark & 2 != 0),
                        self.lighting.sun_frames,
                    );
                }
                self.publish_sun_counter(true);
            }
        }
        // S70B (no GPU->CPU reads in the frame loop): the surface cache's settling
        // and young-only rounds come from what the CPU already knows (frames still,
        // rounds since the light changed), so its counters are read only for the
        // debug output or the older world modes that settle on them.
        let read = lighting_stats_wanted(self.lighting.settings.world_mode());
        if !read {
        } else if let Ok(words) = self.read_lighting_stats(slot) {
            self.lighting.probe_samples = [words[6], words[7]];
            if words[7] > 0 && lighting_debug_on() {
                eprintln!(
                    "lighting probes rays {} probes {} per_probe {:.1}",
                    words[6],
                    words[7],
                    words[6] as f64 / words[7] as f64
                );
            }
        }
        if !self.lighting.cache_clear && !world {
            if !self.lighting.stats_round[slot] {
                // No light cache round ran in that frame.
            } else if !read {
            } else if let Ok(words) = self.read_lighting_stats(slot) {
                // A frame that ran just as the scene changed measured against the
                // old light; it does not count toward settling.
                let light = self.lighting.stats_light[slot] == self.lighting.light_hash;
                self.note_lighting_round(
                    words,
                    self.lighting.stats_still[slot] != 0,
                    light,
                    self.lighting.stats_young[slot],
                );
            }
        }
        unsafe {
            (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.stats_dev[slot].buffer, 0, LIGHTING_STATS_BYTES, 0);
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
            if self.lighting.lamp_res_clear && !self.lighting.lamp_res.buffer.is_null() {
                self.lighting.lamp_res_clear = false;
                (self.fns.cmd_fill_buffer)(
                    self.cmd,
                    self.lighting.lamp_res.buffer,
                    0,
                    self.lighting.lamp_res.size,
                    u32::MAX,
                );
            }
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
        if lighting_debug_on() {
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
    fn submit_lighting_world(&mut self, pc: [u32; 12]) -> Result<bool, String> {
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
            (self.fns.cmd_push)(cmd, self.lighting.layout, 0x20, 0, LIGHTING_PC_BYTES, pc.as_ptr() as *const c_void);
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
            check((self.fns.reset_fences)(self.device, 1, fences.as_ptr()), "reset lighting world fence")?;
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
    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
    )]
    fn trace_sc(&self, memory: Handle, queue: &str) {
        let trace = genos_scene::lighting_env_os("GENOS_LIGHTING_SC_TRACE").is_some();
        let debug = lighting_debug_on();
        if !trace && !debug {
            return;
        }
        if let Ok(w) = self.read_stats16(memory) {
            if trace {
                eprintln!(
                    "lightingsc {queue} frame {} still {} relit {} claimed {} restarted {} freed {} atcap {} live {} thr {} extra {} partial {} sets {} rays {} due {} waiting {} scouts {} trickle {} boost {} checks {} changed {}",
                    self.lighting.frame, self.lighting.still, w[1], w[8], w[9], w[10], w[11], w[2], w[12], w[13], w[14], w[15], w[16], w[17], w[18], w[19], w[20], w[21], w[22], w[23]
                );
                eprintln!("lightingdg {queue} frame {} d {:?}", self.lighting.frame, &w[24..88]);
                eprintln!("lightingchg {queue} frame {} c {:?}", self.lighting.frame, &w[88..119]);
            }
            let mode_word = w[LIGHTING_ATROUS_STAT + 1];
            let mode = match mode_word {
                1 => "list",
                2 => "dense",
                3 => "screen",
                _ => "off",
            };
            eprintln!(
                "lightingatrous {queue} frame {} dirty {:.4} mode {mode} threshold {:.4} filtered {}",
                self.lighting.frame,
                w[LIGHTING_ATROUS_STAT] as f64 / 1.0e6,
                w[LIGHTING_ATROUS_STAT + 2] as f64 / 1.0e6,
                w[LIGHTING_ATROUS_STAT + 3]
            );
            if mode_word == 3 {
                let fine = w[86];
                let coarse = w[87];
                let changed = w[85];
                eprintln!(
                    "lightingdenoise {queue} frame {} pixels {fine} coarse {coarse} changed {changed} filtered {fine} mode screen ran 1",
                    self.lighting.frame
                );
                self.write_denoise_log(fine, coarse, changed);
            }
        }
    }

    /// `./genos-debug/denoise/` (or `<out>/denoise` when `--out` is set). Written
    /// only from the stats read the debug line already performed.
    fn write_denoise_log(&self, fine: u32, coarse: u32, changed: u32) {
        let dir = std::path::Path::new(&self.debug_out).join("denoise");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let text = format!(
            "frame {}\npixels {fine}\ncoarse {coarse}\nchanged {changed}\nfiltered {fine}\nmode screen\nran 1\n",
            self.lighting.frame
        );
        let _ = std::fs::write(dir.join("latest.txt"), &text);
        let mut line = text.replace('\n', " ");
        line.push('\n');
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("frames.log"))
        {
            use std::io::Write;
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn read_stats16(&self, memory: Handle) -> Result<[u32; LIGHTING_STATS_WORDS], String> {
        let mut words = [0u32; LIGHTING_STATS_WORDS];
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

    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
    )]
    fn lighting_passes(&mut self, slot: usize) {
        let [w, h, cols, rows] = self.lighting.dims;
        let probes = cols * rows;
        let probe_rays = probes * self.lighting.settings.probe_rays();
        let bgra = if self.format == 44 { 1u32 } else { 0 };
        self.lighting.frame = self.lighting.frame.wrapping_add(1);
        // Change records dirty the cells a mover or a point lamp touches. The
        // camera, the draw list, and a whole-scene hash do not. There is no
        // full-frame drop.
        let have_prev = !self.lighting.kept_bytes.is_empty();
        let mut records = if have_prev {
            crate::lighting_regions::scene_change_records(&self.lighting.kept_bytes, &self.frame_bytes)
        } else {
            Vec::new()
        };
        records.extend(std::mem::take(&mut self.lighting.mesh_records));
        self.lighting
            .regions
            .set_cell(self.lighting.settings.world_cache.cell_metres);
        self.lighting.regions.apply(&records);
        let deferred = !self.lighting.pending_spheres.is_empty();
        // The first frame has no previous scene: stillness stays 0, as a hash
        // mismatch did. After that, only a change record or a deferred sphere
        // resets it. The camera does not.
        if !have_prev || !records.is_empty() || deferred {
            self.lighting.still = 0;
        } else {
            self.lighting.still = self.lighting.still.saturating_add(1);
        }
        self.lighting.scene_hash = self.lighting.static_light;
        let stamp = camera_stamp(&self.frame_bytes);
        let camera_moved = match stamp {
            Some(now) => !self.lighting.camera_stamp_set || now != self.lighting.camera_stamp,
            None => true,
        };
        if let Some(now) = stamp {
            self.lighting.camera_stamp = now;
            self.lighting.camera_stamp_set = true;
        }
        let debug_view = header_word(&self.frame_bytes, 252);
        let view_changed = match debug_view {
            Some(word) => !self.lighting.debug_view_set || word != self.lighting.debug_view_word,
            None => false,
        };
        if let Some(word) = debug_view {
            self.lighting.debug_view_word = word;
            self.lighting.debug_view_set = true;
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
        // The change list drops kept probe rays that cross a sphere. A lamp's
        // reach would cover the room, and those hits are shaded again anyway.
        // Lamp records still dirty world-cache cells. The surface cache marks
        // texels inside the reach through lighting_sc_lamp_touched.
        let mut geometry = Vec::new();
        for record in &records {
            if let crate::lighting_regions::ChangeRecord::Mover { .. } = record {
                geometry.push(*record);
            }
        }
        let mut spheres = std::mem::take(&mut self.lighting.pending_spheres);
        spheres.extend(crate::lighting_regions::change_spheres(&geometry));
        let (spheres, rest) = crate::lighting_regions::split_spheres(spheres, LIGHTING_CHANGES);
        self.lighting.pending_spheres = rest;
        if lighting_debug_on() {
            if frozen {
                eprintln!("lighting frozen");
            } else {
                eprintln!(
                    "lighting records {} spheres {} deferred {}",
                    records.len(),
                    spheres.len(),
                    self.lighting.pending_spheres.len()
                );
            }
        }
        let mut block = vec![0f32; 4 * (LIGHTING_CHANGE_HEAD + LIGHTING_CHANGES)];
        block[2] = (self.lighting.probe_runs & 1) as f32;
        // World irradiance cells (lighting_cache.glsl, experiment): GENOS_LIGHTING_WORLD mode
        // (1: pixels read the light cache, 2: probes average into world cells, 3:
        // the surface cache) and GENOS_LIGHTING_WORLD_CAP frames (mode 3: ray sets a
        // texel averages, default 16).
        let world = self.lighting.settings.world_mode();
        let cap = genos_scene::lighting_env("GENOS_LIGHTING_WORLD_CAP")
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
        // Bit 20 is the estimator. Bit 30 selects the older hold path.
        // `surface_cache.blend` forces both, so GENOS_LIGHTING_SC_FLAGS cannot
        // override them.
        const GI_ESTIMATOR: u32 = 1 << 20;
        const SURFACE_CACHE_HOLD: u32 = 1 << 30;
        let mut flags_all = lighting_sc_flags_env()
            | if toggles & LIGHTING_TOGGLE_DIRECT_ONLY != 0 {
                2097152
            } else {
                0
            };
        flags_all |= GI_ESTIMATOR;
        match self.lighting.settings.surface_cache.blend {
            genos_scene::SurfaceCacheBlend::Msme => flags_all &= !SURFACE_CACHE_HOLD,
            genos_scene::SurfaceCacheBlend::Hold => flags_all |= SURFACE_CACHE_HOLD,
        }
        let flags = flags_all & ((1 << 23) - 1);
        block[2] = ((self.lighting.probe_runs & 1) + 2 * flags) as f32;
        block[3] = (world * 1000 + cap) as f32;
        // Head y drops kept rays only on the first frame, when there is no
        // previous scene. A mover never sets it.
        block[0] = spheres.len() as f32;
        block[1] = if have_prev { 0.0 } else { 1.0 };
        for (j, b) in spheres.iter().enumerate() {
            let at = 4 * (LIGHTING_CHANGE_HEAD + j);
            block[at..at + 4].copy_from_slice(b);
        }
        let used = spheres.len();
        // Vec4s of the block the GPU gets: head, changes and movers.
        let mut sent = LIGHTING_CHANGE_HEAD + used;
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
            if lighting_debug_on() {
                eprintln!("lighting world {world} cap {cap} flags {flags} local {} movers {n} of {} occs {} meshes {} of {} {:?}", u8::from(self.lighting.settings.surface_cache.attach_movers), self.lighting.movers.len(), lighting_occs(&self.frame_bytes).len(), self.lighting.mesh_movers.len(), self.lighting.mesh_poses.len(), movers.iter().map(|m| (m[0], m[2], m[3])).collect::<Vec<_>>());
            }
            // Object-local cache entries for movers (GIBS §1.5). The public
            // knob is surface_cache.attach_movers. Static geometry stays on
            // the world lattice.
            if self.lighting.settings.surface_cache.attach_movers {
                block[3] += 100000.0;
            }
        }
        if have_prev {
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
        // GI can be on while this buffer is still the 4096-byte stand-in. Probes
        // and the world cache stay off, so the alloc key does not rebuild, and the
        // changes block would start past the end (the cache-ray offset). Bounce 0
        // uploads nothing.
        let bytes = used as u64 * 4;
        if self.lighting.settings.gi.enabled
            && self.lighting.settings.bounce.count > 0
            && block_at + bytes <= self.lighting.work.size
        {
            unsafe {
                // The last frame's passes are done with the block before it is written.
                self.memory_barrier(0x800, 0x1000, 0x20 | 0x40, 0x1000);
                (self.fns.cmd_update_buffer)(
                    self.cmd,
                    self.lighting.work.buffer,
                    block_at,
                    bytes,
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
                    // MSME firefly sigma (MultiscaleMeanEstimator.hlsl, default 8).
                    // Word 13 of the dispatch args. A zero word is sigma 0, which
                    // tightens the clamp, so this is written every full-cache frame.
                    let firefly_sigma = self.lighting.settings.surface_cache.firefly_sigma.to_bits();
                    let ray_budget = self.lighting.settings.surface_cache.ray_budget;
                    let dormant_rays = self.lighting.settings.surface_cache.dormant_rays;
                    (self.fns.cmd_update_buffer)(
                        self.cmd,
                        self.lighting.cache_keys.buffer,
                        (LIGHTING_ARGS_AT + 11) * 4,
                        4,
                        &ray_budget as *const u32 as *const c_void,
                    );
                    (self.fns.cmd_update_buffer)(
                        self.cmd,
                        self.lighting.cache_keys.buffer,
                        (LIGHTING_ARGS_AT + 12) * 4,
                        4,
                        &dormant_rays as *const u32 as *const c_void,
                    );
                    (self.fns.cmd_update_buffer)(
                        self.cmd,
                        self.lighting.cache_keys.buffer,
                        (LIGHTING_ARGS_AT + 13) * 4,
                        4,
                        &firefly_sigma as *const u32 as *const c_void,
                    );
                }
                self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
            }
        }
        // Floor, roof, sky, fire, and a large sun step reset the quiet schedule.
        // A small sun step stays on the hash from the last full refresh: taking
        // the new direction would relight the room as a whole-scene light change.
        // A mover or a point lamp is a change record and does not reset this.
        let sun_amortise = self.note_sun_amortise();
        let light = self.lighting.static_light;
        let tracked = if sun_amortise { self.lighting.light_hash } else { light };
        let counters_read = self.lighting.async_on
            || lighting_stats_wanted(self.lighting.settings.world_mode());
        if tracked != self.lighting.light_hash {
            self.lighting.light_hash = tracked;
            self.lighting.light_quiet = 0;
        } else if have_prev
            && lighting_counts_light_on_cpu(
                self.lighting.settings.world_mode(),
                probe_only(&self.lighting.settings),
                counters_read,
            )
        {
            // S70B skips the cache-counter read on the default surface cache, and
            // note_lighting_round is what used to raise light_quiet. Settled needs
            // that count. Probe-only has always counted here.
            self.lighting.light_quiet = self.lighting.light_quiet.saturating_add(1);
        }
        let sweep = self.lighting.live.div_ceil(LIGHTING_CACHE_BATCH).max(1) + 1;
        // Surface cache (mode 3): bit 10 says the light has held two rounds, and a
        // round that relights nothing has nothing left to converge.
        let sc = self.lighting.settings.world_mode() == 3;
        let young_only = if sc { self.lighting.light_quiet >= 2 } else { self.lighting.young_ok && self.lighting.light_quiet >= sweep };
        self.lighting.stats_light[slot] = tracked;
        self.lighting.stats_young[slot] = young_only || sc;
        // z: bgra, fills (bit 6), young-only rounds and lamp picks (lighting_common.glsl);
        // the gather and filter passes add their round (bits 1-2) below.
        let picks = genos_scene::lighting_env("GENOS_LIGHTING_CACHE_PICKS").is_ok_and(|v| v == "1");
        let fill = genos_scene::lighting_env("GENOS_LIGHTING_FILL").map_or(true, |v| v != "0");
        // Moving objects (debugging.md, spinning boxes): a probe stratum traced from
        // inside a change is filled, not kept (bit 3); patches near a change are
        // relit first (bit 5); a patch not lit yet reads its lit neighbours (bit 7).
        // Each on unless its variable is 0.
        let on = |name: &str| genos_scene::lighting_env(name).map_or(true, |v| v != "0");
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
        let lamp = lamp_pick_push(&self.lighting.settings);
        let pc = [
            w,
            h,
            cols,
            rows,
            self.lighting.settings.probe_rays(),
            tile,
            z,
            self.lighting.frame,
            lamp[0],
            lamp[1],
            lamp[2],
            lamp[3],
        ];
        let image = self.color.image;
        let blocks = (w.div_ceil(8) as u64) * (h.div_ceil(8) as u64);
        // Full compose when the picture has no history yet, the camera or the
        // geometry moved, the surface cache is off, or the block list cannot
        // cover the image. A small sun step, with the camera held, takes one
        // full-grid sun compose instead of that list. GENOS_LIGHTING_COMPOSE_ALL
        // forces the full picture.
        let compose_full = !self.lighting.compose_valid
            || camera_moved
            || view_changed
            || self.lighting.still == 0 && !sun_amortise
            || self.lighting.settings.world_mode() != 3
            || !self.lighting.settings.compose_reuse
            || !self.lighting.settings.gi.enabled
            || genos_scene::lighting_env_os("GENOS_LIGHTING_COMPOSE_ALL").is_some()
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
        // idles cheaply.
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
        let controls = sun_controls();
        let sun_flags = if sun_amortise { LIGHTING_SUN_SMALL_BIT } else { 0 }
            | (controls.bands << 8)
            | ((self.lighting.frame % controls.bands) << 16);
        let mut sun_compose = sun_amortise && !compose_full;
        if sun_compose {
            if let Err(err) = self.ensure_sun_pipe() {
                eprintln!("lighting: sun compose off: {err}");
                sun_compose = false;
            }
        }
        if !sun_compose {
            self.lighting.sun_bounce_live = false;
        }
        self.lighting.compose_kind = if sun_compose {
            COMPOSE_SUN
        } else if compose_full {
            COMPOSE_FULL
        } else {
            COMPOSE_LIST
        };
        self.lighting.compose_dispatches = 0;
        let catch_word = u32::from(self.lighting.catch_left > 0);
        let sun_words = [
            0u32,
            sun_flags,
            self.lighting.sun_commit_dir[0].to_bits(),
            self.lighting.sun_commit_dir[1].to_bits(),
            self.lighting.sun_commit_dir[2].to_bits(),
            self.lighting.sun_commit_light.to_bits(),
            self.lighting.sun_live_light.to_bits(),
            catch_word,
        ];
        if catch_word != 0 {
            self.lighting.catch_left -= 1;
        }
        if sun_amortise {
            self.lighting.sun_frames = self.lighting.sun_frames.saturating_add(1);
        }
        self.memory_barrier(0x1000, 0x1000, 0x1000, 0x1000);
        unsafe {
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.lighting.stats_dev[slot].buffer,
                LIGHTING_CTRL_WORD * 4,
                self.lighting.ctrl_bytes.len() as u64,
                self.lighting.ctrl_bytes.as_ptr() as *const c_void,
            );
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
                (self.fns.cmd_update_buffer)(
                    self.cmd,
                    self.lighting.cache_keys.buffer,
                    lighting_sun_at() * 4,
                    32,
                    sun_words.as_ptr() as *const c_void,
                );
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
                LIGHTING_PC_BYTES,
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
                        LIGHTING_PC_BYTES,
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
                    (self.fns.cmd_push)(self.cmd, self.lighting.layout, 0x20, 0, LIGHTING_PC_BYTES, round_pc.as_ptr() as *const c_void);
                    cur_pc = round_pc;
                }
                if pass == 7 && atrous_ran {
                    (self.fns.cmd_push)(self.cmd, self.lighting.layout, 0x20, 0, LIGHTING_PC_BYTES, pc.as_ptr() as *const c_void);
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
                        LIGHTING_PC_BYTES,
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
                    } else if kind == LIGHTING_DENSE_RAYS {
                        k_pc[6] = (k_pc[6] & !0x300) | 2 << 8;
                    }
                    (self.fns.cmd_push)(
                        self.cmd,
                        self.lighting.layout,
                        0x20,
                        0,
                        LIGHTING_PC_BYTES,
                        k_pc.as_ptr() as *const c_void,
                    );
                }
                // Pass 7 is compose: the sun path binds its own pipeline.
                // Pass 8 is a-trous: the plain kernel has no adaptive branch.
                // Adaptive uses the scalar shader that still reads the bit.
                let pipe = if pass == 7 && sun_compose {
                    self.lighting.sun_pipe
                } else if pass == 8 && self.lighting.settings.denoiser.screen {
                    self.lighting.screen_pipe
                } else if pass == 8 && self.lighting.settings.denoiser.plain() {
                    self.lighting.atrous_plain
                } else {
                    self.lighting.pipes[pass]
                };
                (self.fns.cmd_bind_pipe)(self.cmd, 1, pipe);
                if pass == 7 {
                    self.lighting.compose_dispatches = 1;
                }
                if pass == 7 && sun_compose {
                    // One dispatch. The old path indirect-dispatched the dirty
                    // list (a near-full probe walk under a moving sun) and then
                    // dispatched the whole grid again to paint direct. Both sat
                    // in this timestamp. The slice is cleared once per run so
                    // the first frame stores bounce everywhere; later frames
                    // walk only the band.
                    if !self.lighting.sun_bounce_live {
                        let hist = u64::from(self.lighting.settings.direct.refresh_frames > 1);
                        let pixels = u64::from(w) * u64::from(h);
                        let at = (2 + hist) * pixels * 4;
                        (self.fns.cmd_fill_buffer)(
                            self.cmd,
                            self.lighting.out.buffer,
                            at,
                            pixels * 4,
                            0,
                        );
                        self.lighting.sun_bounce_live = true;
                        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
                    }
                    (self.fns.cmd_dispatch)(self.cmd, w.div_ceil(8).max(1), h.div_ceil(8).max(1), 1);
                } else if pass == 7 && !compose_full {
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
                    (self.fns.cmd_push)(self.cmd, self.lighting.layout, 0x20, 0, LIGHTING_PC_BYTES, cur_pc.as_ptr() as *const c_void);
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
    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
    )]
    fn lighting_dump_copy(&mut self) {
        let Some(n) = genos_scene::lighting_env("GENOS_LIGHTING_DUMP").ok().and_then(|v| v.parse::<u64>().ok()) else {
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
    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
    )]
    fn lighting_dump_write(&mut self) {
        if self.lighting.dump.is_empty() {
            return;
        }
        let last = self.lighting.dump.iter().map(|d| d.0).max().unwrap_or(0);
        if (self.lighting.frame as u64) < last + 3 {
            return;
        }
        self.idle_device();
        let dir = genos_scene::lighting_env("GENOS_LIGHTING_DUMP_DIR").unwrap_or_else(|_| "lighting-dump".into());
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
        let mut lamp_res = std::mem::replace(&mut self.lighting.lamp_res, Buffer::empty());
        let mut keys = std::mem::replace(&mut self.lighting.cache_keys, Buffer::empty());
        let mut cache = std::mem::replace(&mut self.lighting.cache, Buffer::empty());
        for k in 0..2 {
            let mut stats_dev = std::mem::replace(&mut self.lighting.stats_dev[k], Buffer::empty());
            self.destroy_buffer(&mut stats_dev);
            let mut stats = std::mem::replace(&mut self.lighting.stats[k], Buffer::empty());
            self.destroy_buffer(&mut stats);
        }
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        self.destroy_buffer(&mut lamp_res);
        self.destroy_buffer(&mut keys);
        self.destroy_buffer(&mut cache);
        unsafe {
            for pipe in self
                .lighting
                .pipes
                .iter_mut()
                .chain(std::iter::once(&mut self.lighting.atrous_plain))
                .chain(std::iter::once(&mut self.lighting.screen_pipe))
                .chain(std::iter::once(&mut self.lighting.gbuffer_pipe))
                .chain(std::iter::once(&mut self.lighting.sun_pipe))
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

/// S70B: the frame's counters are read back only where something uses them.
fn lighting_stats_wanted(world: u32) -> bool {
    lighting_debug_on() || !(world == 3 && lighting_sc_flags_env() & 8192 != 0)
}

/// Frames since the light last changed, counted on the CPU. The default surface
/// cache (world mode 3, flag 8192) does not read the cache counters, so
/// `note_lighting_round` never runs and `light_quiet` would stay 0. `settled`
/// and the young-only bit both need it. Probe-only does not take the count
/// from the counters either.
fn lighting_counts_light_on_cpu(world_mode: u32, probe_only: bool, counters_read: bool) -> bool {
    probe_only || (world_mode == 3 && !counters_read)
}

// Defaults with no env set (S54K): GI, the surface cache, cap 64, the
// S49 flag set and object-local texels. Each env var still overrides.
const LIGHTING_DEFAULT_SC_CAP: u32 = 64;
const LIGHTING_DEFAULT_SC_FLAGS: u32 = 6007050;
/// Lighting stats buffer: 24 words, diag[64], chg[32], the a-trous fallback
/// (fraction, mode, threshold, filtered texels) at `LIGHTING_ATROUS_STAT`, the
/// control block at word 124 (16 words), then the unfilled-pixel count and the
/// shaded-pixel count. Two pad words keep the mask on a 144-word boundary. The
/// mask follows, one bit per composed pixel, and is not part of this header.
/// The change-counter enable word stays chg[31] (word 119).
const LIGHTING_STATS_WORDS: usize = 144;
/// `gstats.diag[48]` (pixel shadow rays). The header is 24 words.
const LAMP_PICK_DIAG_WORD: usize = 24 + 48;
const _: () = assert!(LAMP_PICK_DIAG_WORD + 4 < LIGHTING_STATS_WORDS);
const LIGHTING_STATS_BYTES: u64 = LIGHTING_STATS_WORDS as u64 * 4;
/// Words 140 and 141. The control block occupies 124..140.
const LIGHTING_UNFILLED_WORD: usize = 140;
const LIGHTING_SHADED_WORD: usize = 141;
/// Word 0 of the mask. Matches lighting_cache.glsl.
const LIGHTING_UNFILLED_MASK_AT: usize = 144;
/// (4096/32) * 2160. A larger picture clamps the mask.
const LIGHTING_UNFILLED_MASK_CAP: u32 = 276480;
const _: () = assert!(LIGHTING_UNFILLED_MASK_AT == LIGHTING_STATS_WORDS);
const _: () = assert!(LIGHTING_CTRL_WORD as usize + 16 == LIGHTING_UNFILLED_WORD);
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
#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
fn lighting_chg_on() -> bool {
    genos_scene::lighting_env_os("GENOS_LIGHTING_SC_TRACE").is_some()
}

/// `GENOS_LIGHTING_ATROUS_DIRTY` in millionths. Default 0.25: levels 1-5 keep
/// the work lists while the fullest level is at or under a quarter of the
/// filtered texels, and run the dense sweep above that. `0` is always dense,
/// `1` keeps the lists up to the storage cap. `override_frac` wins (tests).
#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
fn lighting_atrous_ppm(override_frac: Option<f32>) -> u32 {
    let frac = override_frac.unwrap_or_else(|| {
        genos_scene::lighting_env("GENOS_LIGHTING_ATROUS_DIRTY")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.25)
    });
    (frac.clamp(0.0, 1.0) * 1_000_000.0).round() as u32
}

/// GENOS_LIGHTING_SC_FLAGS, default LIGHTING_DEFAULT_SC_FLAGS.
#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
fn lighting_sc_flags_env() -> u32 {
    static V: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        genos_scene::lighting_env("GENOS_LIGHTING_SC_FLAGS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(LIGHTING_DEFAULT_SC_FLAGS)
    })
}

/// Vec4 index of the changes block in the work buffer (lighting_common.glsl
/// lighting_change_at), for `probes` probes, `cache_rays` light cache rays and `kept`
/// kept hits.
fn lighting_change_at(probes: u64, cache_rays: u64, kept: u64) -> u64 {
    2 * probes + cache_rays + 7 * probes + 2 * LIGHTING_STRATA as u64 * probes + kept + kept / 4
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

/// Floor, roof, sky, fire, smoke, and directional lamps (direction and colour).
/// Point lamps, occluders, and the camera are change records or ignored, so they
/// do not reset the quiet schedule of every cell.
pub(crate) fn lighting_static_key(pack: &crate::pack::Pack, bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let word = |at: usize| {
        bytes
            .get(at * 4..at * 4 + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) as usize
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let tail = crate::pack::SCENE_TAIL;
    for index in 0..word(0) {
        let at = tail + index * 32;
        let Some(rec) = bytes.get(at..at + 32) else {
            break;
        };
        let directional = f32::from_le_bytes([rec[12], rec[13], rec[14], rec[15]]);
        if directional <= 0.5 {
            continue;
        }
        rec[..12].hash(&mut hasher);
        rec[16..28].hash(&mut hasher);
    }
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
    for value in floats {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Eye at byte 16 (12 bytes) and the basis at bytes 96..144.
fn camera_stamp(bytes: &[u8]) -> Option<[u8; 60]> {
    let eye = bytes.get(16..28)?;
    let basis = bytes.get(96..144)?;
    let mut out = [0u8; 60];
    out[..12].copy_from_slice(eye);
    out[12..].copy_from_slice(basis);
    Some(out)
}

/// One little-endian word of the scene block. Byte 252 is `pack.debug_view`.
fn header_word(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    let Ok(arr) = <[u8; 4]>::try_from(slice) else {
        return None;
    };
    Some(u32::from_le_bytes(arr))
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
#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
fn lighting_toggles() -> u32 {
    let mut text = genos_scene::lighting_env("GENOS_LIGHTING_TOGGLES").unwrap_or_default();
    if let Ok(path) = genos_scene::lighting_env("GENOS_LIGHTING_TOGGLES_FILE") {
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

/// Last submitted frame's sun-path proof. `gpu_sun` is the shader's own bit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightingSunStats {
    pub gpu_sun: bool,
    pub gpu_catch: bool,
    pub slice: u32,
    pub host_frames: u32,
    pub catch_left: u32,
    /// `--lighting-full-effort` or `GENOS_LIGHTING_FULL_EFFORT=1` is on.
    pub full_effort: bool,
    /// 1 full picture compose, 2 dirty list, 3 sun band. 0 before any compose.
    pub compose: u8,
    /// Compose dispatches in that frame. The sun path is one.
    pub dispatches: u32,
}

#[derive(Clone, Copy, Default)]
struct SunCli {
    enabled: Option<bool>,
    full_effort: Option<bool>,
    bands: Option<u32>,
    catch_frames: Option<u32>,
    debug: Option<bool>,
}

#[derive(Clone, Copy)]
struct SunControls {
    enabled: bool,
    full_effort: bool,
    bands: u32,
    catch_frames: u32,
    debug: bool,
}

fn lighting_debug_on() -> bool {
    sun_controls().debug
}

#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
fn sun_controls() -> SunControls {
    static CLI: std::sync::OnceLock<SunCli> = std::sync::OnceLock::new();
    let cli = CLI.get_or_init(sun_cli_from_args);
    let enabled = cli.enabled.unwrap_or_else(|| sun_env_bool("GENOS_LIGHTING_SUN", true));
    let full_effort = cli
        .full_effort
        .unwrap_or_else(|| genos_scene::lighting_env("GENOS_LIGHTING_FULL_EFFORT").ok().as_deref() == Some("1"));
    let bands = cli
        .bands
        .unwrap_or_else(|| sun_env_u32("GENOS_LIGHTING_SUN_BANDS", 8))
        .clamp(1, 64);
    let catch_frames = cli
        .catch_frames
        .unwrap_or_else(|| sun_env_u32("GENOS_LIGHTING_CATCH_UP", 4))
        .min(16);
    let debug = cli
        .debug
        .unwrap_or_else(|| genos_scene::lighting_env_os("GENOS_LIGHTING_DEBUG").is_some());
    SunControls {
        enabled,
        full_effort,
        bands,
        catch_frames,
        debug,
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
fn sun_env_bool(name: &str, default: bool) -> bool {
    match genos_scene::lighting_env(name) {
        Ok(text) => !matches!(text.trim(), "0" | "off" | "false"),
        Err(_) => default,
    }
}

fn sun_env_u32(name: &str, default: u32) -> u32 {
    genos_scene::lighting_env(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// One line per debug frame, so a lamp-stop run can be counted. The first
/// write of the process truncates the file.
fn append_catch_log(dir: &std::path::Path, frame: u32, catch: u32, catch_left: u32) {
    use std::io::Write;
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    let path = dir.join("catch.log");
    if STARTED.set(()).is_ok() {
        let _ = std::fs::write(&path, "");
    }
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "frame {frame} catch {catch} catch_left {catch_left}");
}

fn sun_out_dir() -> std::path::PathBuf {
    if let Some(dir) = genos_scene::DebugSettings::cli_out() {
        return std::path::PathBuf::from(dir).join("sun");
    }
    std::path::PathBuf::from("genos-debug/sun")
}

fn sun_cli_from_args() -> SunCli {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cli = SunCli::default();
    let mut i = 0;
    while i < args.len() {
        let next = args.get(i + 1).map(String::as_str);
        if let Some(Ok(flag)) = parse_sun_flag(&args[i], next) {
            match flag {
                SunFlag::Enabled(v) => cli.enabled = Some(v),
                SunFlag::FullEffort => cli.full_effort = Some(true),
                SunFlag::Bands(v) => cli.bands = Some(v),
                SunFlag::Catch(v) => cli.catch_frames = Some(v),
                SunFlag::Debug => cli.debug = Some(true),
            }
            if lighting_sun_flag_needs_value(&args[i]) == Some(true) {
                i += 1;
            }
        }
        i += 1;
    }
    cli
}

enum SunFlag {
    Enabled(bool),
    FullEffort,
    Bands(u32),
    Catch(u32),
    Debug,
}

fn sun_dash(arg: &str) -> Option<(&str, Option<&str>)> {
    let rest = arg.strip_prefix("--")?;
    match rest.split_once('=') {
        Some((name, value)) => Some((name, Some(value))),
        None => Some((rest, None)),
    }
}

/// `Some(true)` when `arg` is a sun-path flag that reads the next argument.
/// `Some(false)` when the flag already includes its value. `None` otherwise.
pub fn lighting_sun_flag_needs_value(arg: &str) -> Option<bool> {
    let (name, inline) = sun_dash(arg)?;
    match name {
        "lighting-full-effort" | "lighting-debug" => Some(false),
        "lighting-sun" | "lighting-sun-bands" | "lighting-catch-up" => Some(inline.is_none()),
        _ => None,
    }
}

/// Reject a bad sun-path argument. `next` is the following argument when
/// [`lighting_sun_flag_needs_value`] is `Some(true)`.
pub fn check_lighting_sun_arg(arg: &str, next: Option<&str>) -> Result<(), String> {
    match parse_sun_flag(arg, next) {
        Some(result) => result.map(|_| ()),
        None => Ok(()),
    }
}

fn parse_sun_flag(arg: &str, next: Option<&str>) -> Option<Result<SunFlag, String>> {
    let (name, inline) = sun_dash(arg)?;
    let value = inline.or(next);
    let flag = match name {
        "lighting-full-effort" => {
            if inline.is_some() {
                return Some(Err("--lighting-full-effort takes no value".into()));
            }
            SunFlag::FullEffort
        }
        "lighting-debug" => {
            if inline.is_some() {
                return Some(Err("--lighting-debug takes no value".into()));
            }
            SunFlag::Debug
        }
        "lighting-sun" => {
            let Some(text) = value else {
                return Some(Err("--lighting-sun needs on or off".into()));
            };
            let on = match text {
                "on" | "1" => true,
                "off" | "0" => false,
                _ => return Some(Err(format!("--lighting-sun is on or off, got {text}"))),
            };
            SunFlag::Enabled(on)
        }
        "lighting-sun-bands" => {
            let Some(text) = value else {
                return Some(Err("--lighting-sun-bands needs 1..64".into()));
            };
            let n: u32 = match text.parse() {
                Ok(n) => n,
                Err(_) => {
                    return Some(Err(format!("--lighting-sun-bands needs 1..64, got {text}")));
                }
            };
            if !(1..=64).contains(&n) {
                return Some(Err(format!("--lighting-sun-bands needs 1..64, got {text}")));
            }
            SunFlag::Bands(n)
        }
        "lighting-catch-up" => {
            let Some(text) = value else {
                return Some(Err("--lighting-catch-up needs 0..16".into()));
            };
            let n: u32 = match text.parse() {
                Ok(n) => n,
                Err(_) => return Some(Err(format!("--lighting-catch-up needs 0..16, got {text}"))),
            };
            if n > 16 {
                return Some(Err(format!("--lighting-catch-up needs 0..16, got {text}")));
            }
            SunFlag::Catch(n)
        }
        _ => return None,
    };
    Some(Ok(flag))
}

fn lighting_sun_live(pack: &crate::pack::Pack) -> ([f32; 3], f32, u32) {
    let mut dir = [0.0_f32; 3];
    let mut n = 0u32;
    let mut light = pack.sky.into_iter().fold(0.0_f32, f32::max);
    for lamp in &pack.lamps {
        if !lamp.directional {
            continue;
        }
        if n == 0 {
            dir = lamp.pos;
        }
        n += 1;
        let up = (-lamp.pos[1]).max(0.0);
        let peak = lamp.color.into_iter().fold(0.0_f32, f32::max);
        light += up * peak;
    }
    (dir, light, n)
}

fn sun_step_small(lighting: &Lighting) -> bool {
    if !lighting.sun_commit_valid || lighting.sun_live_n == 0 || lighting.sun_live_n != lighting.sun_commit_n {
        return false;
    }
    let dot = lighting.sun_live_dir[0] * lighting.sun_commit_dir[0]
        + lighting.sun_live_dir[1] * lighting.sun_commit_dir[1]
        + lighting.sun_live_dir[2] * lighting.sun_commit_dir[2];
    if dot < (2.0_f32.to_radians()).cos() {
        return false;
    }
    let scale = lighting.sun_live_light.max(lighting.sun_commit_light).max(1.0e-3);
    let irr = (lighting.sun_live_light - lighting.sun_commit_light).abs() / scale;
    if irr > LIGHTING_SUN_IRR {
        return false;
    }
    // A day-clock tick is about 0.05 degrees. Treating that as "unchanged" and
    // then moving the commit forward meant the step never accumulated, so the
    // path never engaged. Any change of the stored direction, still inside the
    // budget, is a small step. A frozen sun compares equal and stays off.
    lighting.sun_live_dir != lighting.sun_commit_dir || irr > 1.0e-6
}

fn hash_f32(hasher: &mut std::collections::hash_map::DefaultHasher, v: f32) {
    use std::hash::Hash;
    v.to_bits().hash(hasher);
}

fn lighting_point_lamp_key(pack: &crate::pack::Pack) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for lamp in &pack.lamps {
        if lamp.directional {
            continue;
        }
        for v in lamp.pos {
            hash_f32(&mut hasher, v);
        }
        for v in lamp.color {
            hash_f32(&mut hasher, v);
        }
    }
    hasher.finish()
}

/// What a held camera sees, excluding the directional sun and the sky.
fn lighting_view_content_key(pack: &crate::pack::Pack) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for lamp in &pack.lamps {
        if lamp.directional {
            continue;
        }
        for v in lamp.pos {
            hash_f32(&mut hasher, v);
        }
        for v in lamp.color {
            hash_f32(&mut hasher, v);
        }
        hash_f32(&mut hasher, lamp.range);
    }
    for v in pack.eye {
        hash_f32(&mut hasher, v);
    }
    for v in pack.view_right {
        hash_f32(&mut hasher, v);
    }
    for v in pack.view_up {
        hash_f32(&mut hasher, v);
    }
    for v in pack.view_forward {
        hash_f32(&mut hasher, v);
    }
    hash_f32(&mut hasher, pack.view_tan);
    hash_f32(&mut hasher, pack.view_aspect);
    for v in pack.floor_center {
        hash_f32(&mut hasher, v);
    }
    hash_f32(&mut hasher, pack.floor_half_x);
    hash_f32(&mut hasher, pack.floor_half_z);
    for v in pack.floor_color {
        hash_f32(&mut hasher, v);
    }
    for v in pack.ceiling {
        hash_f32(&mut hasher, v);
    }
    for v in pack.fire.position {
        hash_f32(&mut hasher, v);
    }
    for v in pack.fire.color {
        hash_f32(&mut hasher, v);
    }
    hash_f32(&mut hasher, pack.fire.strength);
    pack.occs.len().hash(&mut hasher);
    for occ in &pack.occs {
        for v in occ.center {
            hash_f32(&mut hasher, v);
        }
        hash_f32(&mut hasher, occ.shape);
        hash_f32(&mut hasher, occ.half_x);
        hash_f32(&mut hasher, occ.height);
        hash_f32(&mut hasher, occ.half_z);
        hash_f32(&mut hasher, occ.radius);
        hash_f32(&mut hasher, occ.yaw);
        for v in occ.albedo {
            hash_f32(&mut hasher, v);
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod sun_step {
    use super::{
        check_lighting_sun_arg, lighting_sun_flag_needs_value, sun_step_small, Lighting,
        LIGHTING_COMPOSE_SPV, SUN_COMPOSE_SPV,
    };

    fn lighting_at(dir: [f32; 3], light: f32) -> Lighting {
        let mut lighting = Lighting::default();
        lighting.sun_commit_valid = true;
        lighting.sun_commit_n = 1;
        lighting.sun_live_n = 1;
        lighting.sun_commit_dir = [0.0, -1.0, 0.0];
        lighting.sun_commit_light = 1.0;
        lighting.sun_live_dir = dir;
        lighting.sun_live_light = light;
        lighting
    }

    #[test]
    fn the_surface_cache_counts_quiet_light_without_the_counter_read() {
        assert!(super::lighting_counts_light_on_cpu(3, false, false));
        assert!(!super::lighting_counts_light_on_cpu(3, false, true));
        assert!(super::lighting_counts_light_on_cpu(0, true, true));
        assert!(!super::lighting_counts_light_on_cpu(0, false, true));
    }

    #[test]
    fn a_frozen_sun_is_not_a_small_step() {
        assert!(!sun_step_small(&lighting_at([0.0, -1.0, 0.0], 1.0)));
    }

    #[test]
    fn a_day_tick_inside_two_degrees_is_a_small_step() {
        let rad = 0.2_f32.to_radians();
        let dir = [rad.sin(), -rad.cos(), 0.0];
        assert!(sun_step_small(&lighting_at(dir, 1.0)));
    }

    #[test]
    fn a_step_past_two_degrees_or_two_percent_is_a_full_refresh() {
        let rad = 3.0_f32.to_radians();
        assert!(!sun_step_small(&lighting_at([rad.sin(), -rad.cos(), 0.0], 1.0)));
        assert!(!sun_step_small(&lighting_at([0.0, -1.0, 0.0], 1.03)));
    }

    #[test]
    fn sun_arguments_reject_a_bad_value() {
        assert!(lighting_sun_flag_needs_value("--lighting-full-effort").is_some_and(|v| !v));
        assert!(lighting_sun_flag_needs_value("--lighting-sun").unwrap());
        assert!(lighting_sun_flag_needs_value("--lighting-sun=off").is_some_and(|v| !v));
        assert!(lighting_sun_flag_needs_value("--preset").is_none());
        assert!(check_lighting_sun_arg("--lighting-sun", Some("on")).is_ok());
        assert!(check_lighting_sun_arg("--lighting-sun-bands", Some("8")).is_ok());
        assert!(check_lighting_sun_arg("--lighting-catch-up", Some("0")).is_ok());
        assert!(check_lighting_sun_arg("--lighting-sun-bands", Some("0")).is_err());
        assert!(check_lighting_sun_arg("--lighting-sun-bands", Some("nope")).is_err());
        assert!(check_lighting_sun_arg("--lighting-sun-bands", None).is_err());
        assert!(check_lighting_sun_arg("--lighting-catch-up", Some("17")).is_err());
        assert!(check_lighting_sun_arg("--lighting-full-effort=1", None).is_err());
        assert!(lighting_sun_flag_needs_value("--lighting-debug").is_some_and(|v| !v));
        assert!(check_lighting_sun_arg("--lighting-debug", None).is_ok());
        assert!(check_lighting_sun_arg("--lighting-debug=1", None).is_err());
        assert!(check_lighting_sun_arg("--scale", None).is_ok());
    }

    /// The picture compose stays the GI shader. The sun compose is that pass
    /// plus the sun words, not an occluder walk copied into both.
    #[test]
    fn compose_spirv_stays_near_the_picture_pass() {
        assert!(
            LIGHTING_COMPOSE_SPV.len() < 220_000,
            "compose SPIR-V is {} bytes",
            LIGHTING_COMPOSE_SPV.len()
        );
        let extra = SUN_COMPOSE_SPV.len().saturating_sub(LIGHTING_COMPOSE_SPV.len());
        assert!(
            extra < 32_000,
            "sun compose grew by {extra} bytes (compose {}, sun {})",
            LIGHTING_COMPOSE_SPV.len(),
            SUN_COMPOSE_SPV.len()
        );
    }
}

include!("lighting_dispatch.rs");
