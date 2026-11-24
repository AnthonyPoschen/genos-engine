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
/// A non-zero `probe_tile` setting (4..64, mirrored by GENOS_LIGHTING_TILE)
/// replaces it.
fn lighting_tile(height: u32) -> u32 {
    (height / 45).clamp(4, LIGHTING_TILE)
}

/// Rays per screen probe (Low in the design; a square for the stratified pattern).
pub(crate) const LIGHTING_RAYS: u32 = 64;
/// Light cache sizes; lighting_cache_slots.glsl has the same numbers.
const LIGHTING_CACHE_SLOTS: u64 = 1048576;
const LIGHTING_CACHE_BATCH: u32 = 98304;
const LIGHTING_CACHE_RAYS: u32 = 16;
/// ADR 0039 stage 2 SH-L1 cell storage, a low-spec knob: fp16 (24 B a slot)
/// or fp32 (48 B). lighting_cache_slots.glsl `LIGHTING_SH_HALF` has the same value.
const LIGHTING_SH_HALF: bool = true;
/// Cache bytes a slot: 23 vec4 regions, the SH-L1 (1.5 vec4 at fp16, 3 at
/// fp32), padding so the shown-age word is region 26 in both layouts, then
/// that word, then regions 27..30: the 16 rgb9e5 direction bins of an
/// upper cascade cell (`CASCADE_BINS_AT` in lighting_cascade.glsl).
/// Compose does not read the age. It must not share region 23 with the SH.
const LIGHTING_CACHE_SLOT_BYTES: u64 = {
    let sh = if LIGHTING_SH_HALF { 24 } else { 48 };
    let pad = if LIGHTING_SH_HALF { 24 } else { 0 };
    23 * 16 + sh + pad + 16 + CASCADE_BIN_BYTES
};
/// Upper cascade cell direction bins: 16 rgb9e5 words, four vec4 regions.
const CASCADE_BIN_BYTES: u64 = 4 * 16;
/// `pc.dims` + `pc.params` + `pc.lamp`. A shader that ignores `pc.lamp` still
/// accepts a 48-byte push.
const LIGHTING_PC_BYTES: u32 = 48;
/// ckeys word of the WRC refresh lists' two counts (lighting_cache.glsl LIGHTING_WRC_CHG_COUNT_AT).
const LIGHTING_WRC_COUNTS_AT: u64 = 2 * LIGHTING_CACHE_SLOTS + 2 + 66;
/// Off-screen fine-texel admission count (`LIGHTING_FINE_COUNT_AT`). Zeroed each round.
const LIGHTING_FINE_COUNT_AT: u64 = LIGHTING_WRC_COUNTS_AT + 20;
/// Create, drop, lookahead. Not zeroed.
const LIGHTING_FINE_WORD_AT: u64 = LIGHTING_WRC_COUNTS_AT + 24;
/// ckeys word of the drain flag (lighting_cache.glsl LIGHTING_DRAIN_FLAG_AT).
const LIGHTING_DRAIN_FLAG_AT: u64 = 2 * LIGHTING_CACHE_SLOTS + 2 + 82;
/// ckeys word of the last host-hash change frame (lighting_cache.glsl LIGHTING_LAMP_CATCH_AT).
const LIGHTING_LAMP_CATCH_AT: u64 = 2 * LIGHTING_CACHE_SLOTS + 2 + 86;
/// Strata of the screen probes' ray directions, one traced per frame, all kept
/// (lighting_common.glsl LIGHTING_STRATA): the visibility history cap.
pub(crate) const LIGHTING_STRATA: u32 = 8;
/// Bounds of moved objects one frame can list (lighting_common.glsl LIGHTING_CHANGES); past
/// that every kept stratum is dropped.
const LIGHTING_CHANGES: usize = 64;
/// Vec4s before the bounds in the changes block (lighting_common.glsl LIGHTING_CHANGE_HEAD).
const LIGHTING_CHANGE_HEAD: usize = 5;
/// GI counts as settled once the scene block (camera included) has stayed the
/// same for this many frames (the screen probes average up to 32 frames' rays,
/// lighting_gather.comp) and a full sweep of the light cache changed no patch
/// by more than LIGHTING_SETTLE_CHANGE.
const LIGHTING_SETTLE_FRAMES: u32 = 32;
/// Largest relative change of a relit cache patch, in millionths, that counts as
/// settled (0.2 %, well under a display code).
const LIGHTING_SETTLE_CHANGE: u32 = 2000;
/// 8×8 compose blocks the quiet list can hold (lighting_cache.glsl LIGHTING_COMPOSE_CAP).
const LIGHTING_COMPOSE_CAP: u64 = 65535;
/// Frames after a cache clear that compose the whole picture and walk bounce.
/// The clear frame's cache is empty, and that walk is stored as a finished
/// black bounce (`pack + 1`). The next held frames reuse it and leave the
/// previous picture in the buffer. Three frames covers the clear and the two
/// frames whose rays make the bounce visible. The presented bounce gate reads
/// the third draw.
const BOUNCE_FILL_FRAMES: u32 = 3;
/// Shader `LIGHTING_STILL_REFRESH`. The drift pass.
const LIGHTING_STILL_REFRESH: u32 = 32;
/// Shader `LIGHTING_FILL_WALK`. The fill walks this many frames, then the hold.
const LIGHTING_FILL_WALK: u32 = 8;
/// Shader `LIGHTING_SCENE_CHANGED_MARK`. Added to the change count's fraction
/// when the host scene hash changed this frame.
const LIGHTING_SCENE_CHANGED_MARK: f32 = 0.25;
/// Shader `LIGHTING_LIT_CHANGED_MARK`. Added on top of the scene mark when
/// what lights the scene changed (`lighting_scene_key`: lamps, occluders,
/// fire, smoke, floor, roof and sky), not only the camera. The scene hash
/// has the camera in it, so the scene mark is set on every frame of a pan.
/// The two marks together stay under 0.5, the count's own threshold.
const LIGHTING_LIT_CHANGED_MARK: f32 = 0.125;

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

/// Words through the lattice table, then one lit-from offer word and one
/// probe-seed election word per slot. Matches `LIGHTING_SH_LATTICE_AT` and
/// `LIGHTING_SC_SEED_AT` in `lighting_cache.glsl`. The per-pixel seed payload
/// is not included: its size is the picture.
pub(super) fn lighting_sh_bin_count_at() -> u64 {
    lighting_atrous_ppm_at()
        + LIGHTING_ATROUS_META_WORDS
        + LIGHTING_SUN_WORDS
        + LIGHTING_CACHE_SLOTS
        + 3
        + LIGHTING_CACHE_SLOTS
        + LIGHTING_CACHE_SLOTS
}

pub(super) fn lighting_sh_keys_words() -> u64 {
    const STORE_BINS: u64 = 16;
    const STORE_CAP: u64 = 8192;
    const LATTICE_CAP: u64 = 16384;
    const STRIDE: u64 = 14;
    let lists = lighting_sh_bin_count_at() + STORE_BINS;
    let lattice = lists + STORE_BINS * STORE_CAP;
    // Lit-from offer and probe-seed election, one word per slot, past the lattice.
    lattice + LATTICE_CAP * STRIDE + 2 * LIGHTING_CACHE_SLOTS
}

/// Words after the fill lists. Matches `lighting_reach.comp`:
/// 2048 mask words (one bit per 32 px tile), 65536 change words, one bit per slot.
fn lighting_reach_words() -> u64 {
    const MASK: u64 = 2048;
    const CHANGE: u64 = 65536;
    MASK + CHANGE + LIGHTING_CACHE_SLOTS / 32
}

pub(super) fn lighting_fill_keys_words(pixels: u64) -> u64 {
    // The fill lists are gone. Reach starts at the seed-pay end.
    lighting_sh_keys_words() + pixels * 8
}

const CASCADE_CLAIMED_WORDS: u64 = 32768;
const CASCADE_UPPER_CAP: u64 = 16384;
const CASCADE_UPPER_RAYS_MAX: u64 = 64;
/// Flags, three due counts, three indirect commands, three spare words.
const CASCADE_UPPER_HEAD_WORDS: u64 = 16;
const CASCADE_UPPER_BANDS: u64 = 3;
const CASCADE_TIME_QUERIES: u32 = 6;

fn lighting_upper_hits_words() -> u64 {
    CASCADE_UPPER_CAP * CASCADE_UPPER_RAYS_MAX * CASCADE_L0_HIT_WORDS
}

fn lighting_upper_words() -> u64 {
    CASCADE_CLAIMED_WORDS
        + CASCADE_UPPER_HEAD_WORDS
        + CASCADE_UPPER_BANDS * CASCADE_UPPER_CAP
        + lighting_upper_hits_words()
}

fn lighting_upper_claimed_at(pixels: u64) -> u64 {
    lighting_level0_counts_at(pixels) + CASCADE_L0_COUNT_WORDS + lighting_level0_hits_words()
}

fn lighting_keys_words(pixels: u64) -> u64 {
    lighting_fill_keys_words(pixels)
        + lighting_reach_words()
        + lighting_level0_words(pixels)
        + lighting_upper_words()
}

/// Level-0 surface samples, after the reach words. One count word, then two
/// tables of `CASCADE_LEVEL0_CAP` records of `CASCADE_LEVEL0_STRIDE` words
/// (12 header words plus four packed slot radiances), then four words per
/// pixel: published radiance and stamp, then the insert slot and the blend
/// slot, then inserted/traced counts and one four-word hit record per voxel
/// slot and ray. Matches `lighting_cascade.glsl`.
const CASCADE_LEVEL0_GROUPS: u32 = 16384;
const CASCADE_LEVEL0_CAP: u64 = 16384;
const CASCADE_LEVEL0_STRIDE: u64 = 16;
const CASCADE_LEVEL0_TABLES: u64 = 2;
const CASCADE_LEVEL0_PIXEL_WORDS: u64 = 4;
const CASCADE_L0_RAYS: u64 = 32;
/// Uniform rays plus lamp-cone rays a voxel traces. Matches `CASCADE_L0_TRACED`.
const CASCADE_L0_TRACED: u64 = CASCADE_L0_RAYS + 4;
const CASCADE_L0_HIT_WORDS: u64 = 4;
/// Inserted and traced counts, the indirect command, the live-slot list.
const CASCADE_L0_COUNT_WORDS: u64 = 2 + 4 + CASCADE_LEVEL0_CAP;

fn lighting_level0_table_words() -> u64 {
    CASCADE_LEVEL0_TABLES * CASCADE_LEVEL0_CAP * CASCADE_LEVEL0_STRIDE
}

fn lighting_level0_hits_words() -> u64 {
    CASCADE_LEVEL0_CAP * CASCADE_L0_TRACED * CASCADE_L0_HIT_WORDS
}

fn lighting_level0_words(pixels: u64) -> u64 {
    1 + lighting_level0_table_words()
        + pixels * CASCADE_LEVEL0_PIXEL_WORDS
        + CASCADE_L0_COUNT_WORDS
        + lighting_level0_hits_words()
}

fn lighting_level0_counts_at(pixels: u64) -> u64 {
    lighting_fill_keys_words(pixels)
        + lighting_reach_words()
        + 1
        + lighting_level0_table_words()
        + pixels * CASCADE_LEVEL0_PIXEL_WORDS
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
    /// Tile mask for a mover. A lamp does not use it.
    reach_pipe: Handle,
    gbuffer_pipe: Handle,
    gbuf: Buffer,
    work: Buffer,
    out: Buffer,
    /// Scene-referred radiance, RGBA16F. w is the eye distance in metres.
    scene_lin: Buffer,
    /// Host copy of `scene_lin`, allocated for a picture readback and freed after it.
    scene_host: Buffer,
    /// Exposure, display mode, bloom intensity, output mode, then white, peak, and format.
    /// Host-visible. The compose shader reads the first `vec4`.
    hdr_params: Buffer,
    /// HDR10 or scRGB: RGBA16F codes, then one BGRA word per pixel. Empty on SDR.
    hdr_encode: Buffer,
    /// Bloom mip pyramid. Finest level is RGBA16F. Later levels are RGB16F pairs. No full-resolution image.
    bloom: Buffer,
    /// Last still-check share from the GPU meter. Zero until a comparison exists.
    still_pop: f64,
    /// GPU histogram and the luma halves, one buffer per frame in flight.
    /// The header is the bin counts. Both frames used to fill and copy one
    /// buffer: on mailbox that is a WRITE_AFTER_WRITE. Each slot compares its
    /// own previous half. The byte count comes from the post chain.
    meter_dev: [Buffer; 2],
    /// Host copy of the meter header, one buffer per frame in flight.
    /// Both frames used to copy into one buffer: the second `vkCmdCopyBuffer`
    /// was a WRITE_AFTER_WRITE. Read the slot whose fence just waited.
    meter_host: [Buffer; 2],
    /// A meter dispatch is in the frame the fence just waited for, per slot.
    meter_pending: [bool; 2],
    /// The meter has stored one luma half in this slot, so the next pass can compare.
    meter_has_prev: [bool; 2],
    /// Which luma half holds this slot's previous sample. The dispatch flips it.
    meter_ping: [u32; 2],
    /// Bin count of the meter dispatch `take_meter` reads. Zero when that pass did not run.
    meter_bins: u32,
    /// [`crate::hdr_output::post_signature`] of the buffers `make_lighting_targets` built.
    post_sig: u64,
    /// `layout_post` is measuring the chain. Fallback's GI size must not force SDR.
    budget_hdr: bool,
    /// Preset the budget measurement classifies. High keeps a fallback-shaped grade off Fallback.
    budget_preset: genos_scene::LightingPreset,
    /// Panel the budget measurement classifies. Desktop is the 1440p gate.
    budget_panel: crate::hdr_output::DeckPanel,
    /// Whether the budget measurement saw an HDR10 pair. The 1440p gate does not.
    budget_hdr10_listed: bool,
    /// The display pass writes the bloom tent and lattice samples into the encode buffer.
    bloom_probe: bool,
    /// EV100 from the last GPU histogram. Empty until a frame had samples.
    meter_ev: Option<f64>,
    /// The camera position auto exposure last saw, to find a cut.
    pub(crate) exposure_eye: Option<[f32; 3]>,
    /// Frames auto exposure still waits after a camera cut, and the scale
    /// when it cut (`hdr_output::cut_exposure`).
    pub(crate) exposure_cut: u8,
    exposure_cut_from: f64,
    hdr: genos_scene::HdrSettings,
    /// The blended look the last frame drew with. `None` before the first frame.
    hdr_look: Option<crate::hdr_output::HdrLook>,
    /// Last scale written. Legacy bytes stay at 1.
    exposure_scale: f32,
    /// EV100 the last scale was built from.
    last_ev100: f64,
    /// Last eight exposure scales, oldest first. `get_hdr` returns these.
    exposure_log: Vec<f64>,
    /// Last histogram bin counts. The dump under `hdr-output/` is the same vector.
    histogram_counts: Vec<u32>,
    cache_keys: Buffer,
    cache: Buffer,
    /// Host copy of this frame's dirty-compose list length (one u32).
    compose_host: Buffer,
    /// The cache buffers are new and get zeroed before the next frame's passes.
    cache_clear: bool,
    /// The G-buffer is new and gets filled with empty records first.
    gbuf_clear: bool,
    /// The work buffer is new: zeroed first, so no probe keeps a stratum.
    work_clear: bool,
    /// The picture in the output buffer was drawn for this size. A new buffer
    /// has no picture yet, so the next compose covers every pixel.
    compose_valid: bool,
    /// Frames since the cache was cleared, capped at [`BOUNCE_FILL_FRAMES`].
    /// Below that, compose covers the picture and the bounce slice is zero,
    /// so the shader walks instead of reusing the empty store.
    bounce_fill: u32,
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
    /// Hash of what GI sees (scene block, traced and drawn meshes), and frames
    /// it has stayed the same.
    scene_hash: u64,
    still: u32,
    /// Frame the host hash last changed. The hold reads it for LIGHTING_HOLD_CATCH
    /// frames (shader LIGHTING_LAMP_CATCH_AT). 0 until the first change.
    scene_catch: u32,
    /// This frame's hash change is occluder motion with the camera, lamps, fog,
    /// and meshes held. The drawn solid moves with that occluder. The
    /// picture-wide mark stays off.
    mover_local: bool,
    /// Tiles still moving, from the previous stats readback (`diag[8]`).
    reach_hot: u32,
    /// Marked tiles from the previous stats readback. A mask that covers half
    /// the picture is unproven, so the next frame unholds the picture.
    reach_tiles: u32,
    /// This frame's mover footprint could not be proved local, or the caller
    /// asked for the picture-wide control. The scene mark is set.
    reach_wide: bool,
    /// Test probe. Forces the picture-wide unhold so a local run can be
    /// compared with one that never held a tile.
    picture_unhold: bool,
    /// Frames to keep the lattice walk on after a local unhold. The readback
    /// is a frame or two behind the mask.
    reach_arm: u32,
    kept_froxel: bool,
    kept_mesh: u64,
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
    /// the async compute queue (`experiments.cache_async`; off keeps them in the frame):
    /// at most one submission in flight, skipped while the last is still running.
    /// Copied from the settings each frame unless `async_failed`.
    async_on: bool,
    /// The async queue failed once: it stays off for the rest of the run.
    async_failed: bool,
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
    /// `light_scene` on the last frame that marked the change block.
    light_scene_seen: u64,
    light_hash: u64,
    /// Rounds in a row, since that last changed, whose mature patches all stayed
    /// within LIGHTING_SETTLE_CHANGE: past a full sweep, rounds relight young patches only.
    light_quiet: u32,
    /// The last cache round's WRC samples still differ from the held light by
    /// more than LIGHTING_DRAIN_GAP: the bounce is draining or filling, not settled.
    draining: bool,
    /// The last read cache round's batch entries, 16 rays each (stats word 147).
    round_entries: u32,
    /// Cache rays that round actually traced (stats word 16). Scouts are not in this.
    cache_rays: u32,
    /// Occluders (index in the scene's list) that moved in the last
    /// LIGHTING_MOVER_KEEP frames, with the frames left: movers. The surface cache leaves
    /// their pixels to the probes; once still that long they are scenery again.
    movers: std::collections::BTreeMap<usize, u32>,
    /// Traced mesh instances (mesh_field order): last poses, the ones that moved in
    /// the last LIGHTING_MOVER_KEEP frames, and every instance's bounding sphere now.
    mesh_poses: Vec<[f32; 16]>,
    mesh_movers: std::collections::BTreeMap<usize, u32>,
    mesh_spheres: Vec<[f32; 4]>,
    /// Parallel to live field order: `FieldInstance::object` for draw→mesh oid.
    mesh_field_object: Vec<Option<usize>>,
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
    /// When the last light cache round began (`experiments.cache_hz` caps them), and
    /// whether each frame slot ran one.
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
    /// Display pass (AgX or the shoulder). Built the first HDR frame.
    agx_pipe: Handle,
    /// Same-frame bloom pyramid. Built the first frame that asks for bloom.
    bloom_pipe: Handle,
    /// Histogram and still check. Built the first frame that stores scene radiance.
    meter_pipe: Handle,
    /// Compose with the scene image and bloom. The plain pipe stays free of them.
    compose_hdr_pipe: Handle,
    /// #142: compose's surface-cache writes (claims, stamps, fills, seeds,
    /// offers), dispatched over compose's blocks just before it. Null when the
    /// kill switch keeps them in compose.
    compose_maint_pipe: Handle,
    /// Same-frame gather of the bins and lattices maintenance just wrote.
    /// Null when the kill switch keeps the writes inside compose.
    sh_drain_pipe: Handle,
    /// Level-0 resolve. Writes the per-pixel upward footprint blend after the
    /// level-0 shade pass. Picture compose loads those words and does not
    /// walk the hash.
    cascade_pub_pipe: Handle,
    /// Trace 32 rays for every live level-0 voxel. Null if compile failed.
    cascade_l0_trace_pipe: Handle,
    /// Shade those rays, merge with the parent SH, and store the EMA.
    cascade_l0_shade_pipe: Handle,
    cascade_upper_claim_pipe: Handle,
    cascade_upper_pick_pipe: Handle,
    cascade_upper_trace_pipe: Handle,
    cascade_upper_shade_pipe: Handle,
    /// Timestamps around claim/pick, each upper band, and level 0.
    cascade_qp: Handle,
    cascade_times: crate::CascadeTimes,
    /// #142 kill switch (GENOS_COMPOSE_SC_WRITES=1): compose's specialization
    /// constant 0, writes compiled into compose and no maintenance pass.
    compose_writes: bool,
    /// Sun compose with the scene image and bloom.
    sun_hdr_pipe: Handle,
    /// The linear-bounce slice was cleared for this sun run. The first frame
    /// walks every pixel; later frames walk the band.
    sun_bounce_live: bool,
    /// Last compose: 1 full picture, 2 dirty list, 3 sun band. 0 before any.
    compose_kind: u8,
    /// Compose dispatches recorded for that frame. The sun path is one.
    compose_dispatches: u32,
    /// Maintenance and the lattice drain ran this frame. A held camera skips
    /// both except on frame 32. The picture compile still runs.
    maint_ran: bool,
    /// Point lamps only. `None` until the first frame, so a still open does not
    /// look like a lamp that just stopped.
    lamp_key: Option<u64>,
    lamp_moving: bool,
    catch_left: u32,
    /// The million-slot cache is allocated. Off, a small dummy stands in.
    cache_full: bool,
    /// Pixel count the key buffer was sized for. A larger frame rebuilds it
    /// so the fill tail is not written past the allocation.
    cache_key_pixels: u64,
    /// Last buffer class. A live edit rebuilds only when this changes.
    alloc_key: (u32, u32, u32, u32, u32, u32, u32),
    /// Ping-pong reservoirs. 64 bytes unless the mode is reuse.
    lamp_res: Buffer,
    /// Fill the reservoir with empty lamps once, not every frame.
    lamp_res_clear: bool,
    /// Two vec4s per pixel: the disc-visibility mean and the surface it belongs to.
    area_mean: Buffer,
    /// Fill that mean with zeros once, before the first shade.
    area_clear: bool,
    /// Noise-gate shade. Production stays [`LampShadeGate::Product`].
    shade_gate: LampShadeGate,
    /// ReSTIR DI reservoirs. A 256-byte stand-in while the flag is off.
    restir_buf: Buffer,
    restir_pipe: Handle,
    /// Product spatial pass. Four neighbours, scalar slots, no light tree.
    restir_spatial_pipe: Handle,
    /// Spatial pass when the neighbour knob is above 4. The 17-wide lists stay here.
    restir_spatial_wide: Handle,
    /// Last spatial bind: 1 is `restir-spatial`, 2 is `restir-spatial-wide`. 0 before either.
    restir_spatial_bound: u8,
    restir_direct: Handle,
    /// This frame shades screen pixels from the reservoir.
    restir_frame: bool,
    restir_slot: u32,
    restir_prev: crate::restir_live::RestirCam,
    restir_have_prev: bool,
    /// Timestamp query of the three ReSTIR passes. `u32::MAX` if this slot skipped them.
    restir_query: [u32; 2],
    restir_lamps: Vec<crate::pack::GpuLamp>,
    restir_prev_lamps: Vec<crate::pack::GpuLamp>,
    restir_view: crate::restir_live::RestirCam,
    restir_w: u32,
    restir_h: u32,
    restir_clear: bool,
    /// Product frames whose reservoir upload failed and shaded the labelled sum.
    restir_prepare_failures: u32,
    /// Test hook. The next product prepare returns the labelled exact path.
    restir_force_fail: bool,
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
    /// Sum of ReSTIR pass milliseconds, and how many frames that sum holds.
    /// Order: initial, temporal, spatial, prepare, shade. Shade is the direct
    /// dispatch, so its visibility ray is inside that number.
    restir_ms_sum: [f64; 5],
    restir_ms_frames: u32,
    /// Last collected frame, same order as `restir_ms_sum`. Zero until one lands.
    restir_ms_last: [f64; 5],
    /// How many frames have written `last_ms`. The CSV writes one row per step.
    pass_samples: u32,
    /// Last collected frame by dispatch pass id. Index 6 is the cache relight,
    /// index 9 is the SH prep. The cache bucket sums both. The `LIGHTING_MS`
    /// line is nine pass ids, so pass 9 is not in that total.
    pass_id_ms: [f64; 10],
    /// Bottom-of-pipe lighting span of the last collected frame, the `lighting`
    /// column of `GPU_MS`. Pass 9 and the prepare stamp sit in this span and
    /// outside the nine-wide `LIGHTING_MS` total.
    span_ms: f64,
    /// Flight slot whose host stats were copied on the last submitted frame.
    stats_slot: usize,
    /// Bloom plus the display pass. Zero when timestamps are off.
    hdr_ms: f64,
    /// Query index of that pair. `u32::MAX` means this slot did not stamp it.
    hdr_query: [u32; 2],
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
    /// Variance kernels that did not take the dirty early-out. A held camera
    /// leaves a texel with 16 sets out of this count until the 32-frame
    /// backstop. The count is the texels still being filtered.
    pub filter_heavy: u32,
    /// Screen-list cells written by a full rebuild this frame. Zero when the
    /// held list was reused.
    pub list_rebuilt: u32,
    /// Cells appended onto a reused screen list this frame.
    pub list_appended: u32,
    /// Pixels whose compose walked the world cache (`diag[0]`). A held frame
    /// reuses the stored bounce and this stays near zero.
    pub cache_walks: u32,
    /// Screen tiles a mover reached this frame (`diag[8]`, low 16 bits).
    pub reach_tiles: u32,
    /// Tiles kept because the bounce still moved more than one percent.
    pub reach_hot: u32,
    /// Corner tiles of the picture. Bit 0 is the origin, then +x, +y, +x+y.
    /// A set bit means that corner was unheld.
    pub reach_corners: u32,
    /// The host could not prove the mover's footprint, so this frame unheld
    /// the picture. A full mask is that case: there is no outside to compare.
    pub reach_wide: bool,
}

/// Shadow rays traced last frame, and how pixels shaded.
///
/// `pixels` is primary pixels, `kept` probe hits, `fills` probe fills, `cache`
/// light-cache rays. `exact` shaded the labelled sum, `restir` shaded one
/// ReSTIR DI sample. Counts stay zero unless the diag push bit was set.
/// `exact` / `restir` are `diag[58]` and `diag[59]`. `diag[52]` is the
/// gradient-budget overflow in compact. `diag[57]`
/// (`LIGHTING_DIAG_UNDER_TRUSTED`, stats word 81) counts displaying picture
/// pixels whose fine read was under a full set count. The pairwise spatial
/// merge and the trusted-colour copy are gone, and the later words are not
/// shifted.
/// `history_accepts` / `history_rejects` are restir header words 43 and 44:
/// the live temporal reprojection kept or rejected a surfaced pixel. A history
/// reset counts neither. `restir_point_adds` is header word 45: near point
/// lamps the direct loop added on a ReSTIR pixel. It stays 0 while that loop
/// skips them. The host reads a copy at the tail of the stats readback. Compact
/// keeps `gstats.diag` 5, 6, and 13 when `LIGHTING_DIAG` is on.
/// The kept/fill gate counts are `diag[53..56]` and `diag[60]`: hit threads
/// launched with a live hit (`kept_live`, `fill_live`), kept hits in this
/// frame's strata (`kept_due`), and hits that reach the lamp list (`kept_shade`,
/// `fill_shade`). `near_tests` is non-due kept hits that ran the near-change
/// walk, `px_lamps` / `hit_lamps` the lamp-list entries at shaded pixels and
/// shaded hits (`diag[34]`, `diag[38]`, `diag[39]`). A `LIGHTING_DIAG` shader
/// build also writes these words from the surface-cache trace, so read them
/// from a default build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LampPickCounts {
    pub pixels: u32,
    pub kept: u32,
    pub fills: u32,
    pub cache: u32,
    pub exact: u32,
    pub restir: u32,
    pub kept_live: u32,
    pub kept_due: u32,
    pub kept_shade: u32,
    pub fill_live: u32,
    pub fill_shade: u32,
    pub near_tests: u32,
    pub px_lamps: u32,
    pub hit_lamps: u32,
    /// Surfaced pixels whose live temporal reprojection kept history.
    pub history_accepts: u32,
    /// Surfaced pixels whose live temporal reprojection rejected history.
    pub history_rejects: u32,
    /// Near point lamps added by the direct loop on a ReSTIR pixel.
    pub restir_point_adds: u32,
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
        // diag[60] is word 84 (diag[0] is word 24).
        filter_heavy: w[84],
        // diag[40] and diag[41]: the screen list rebuilt this frame, and the cells appended.
        list_rebuilt: w[64],
        list_appended: w[65],
        // diag[0] is word 24.
        cache_walks: w[24],
        // diag[8] is word 32. Low 16 tiles, next 12 hot, top 4 corners.
        reach_tiles: w[32] & 0xffff,
        reach_hot: (w[32] >> 16) & 0xfff,
        reach_corners: w[32] >> 28,
        reach_wide: false,
    }
}

/// Whether the lighting has settled: the scene (camera included) has held still
/// long enough and the light cache has stopped changing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettleStats {
    pub settled: bool,
    /// Frames in a row the scene has not changed.
    pub still_frames: u32,
    /// The surface cache's drain flag as the host last read it (a few frames
    /// old): its samples still differ from the held light, so it keeps relighting.
    pub draining: bool,
    /// The last read cache round's batch entries (16 rays each), a few frames old.
    pub round_entries: u32,
    /// Cache rays traced that round (texels and world cells, not scouts), a few
    /// frames old. Zero means the round dispatched no cache rays.
    pub cache_rays: u32,
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
            atrous_plain: std::ptr::null_mut(),
            screen_pipe: std::ptr::null_mut(),
            reach_pipe: std::ptr::null_mut(),
            gbuffer_pipe: std::ptr::null_mut(),
            gbuf: Buffer::empty(),
            work: Buffer::empty(),
            out: Buffer::empty(),
            scene_lin: Buffer::empty(),
            scene_host: Buffer::empty(),
            hdr_params: Buffer::empty(),
            hdr_encode: Buffer::empty(),
            bloom: Buffer::empty(),
            still_pop: 0.0,
            meter_dev: [Buffer::empty(), Buffer::empty()],
            meter_host: [Buffer::empty(), Buffer::empty()],
            meter_pending: [false, false],
            meter_has_prev: [false, false],
            meter_ping: [0, 0],
            meter_bins: 0,
            post_sig: 0,
            budget_hdr: false,
            budget_preset: genos_scene::LightingPreset::High,
            budget_panel: crate::hdr_output::DeckPanel::Desktop,
            budget_hdr10_listed: false,
            bloom_probe: false,
            meter_ev: None,
            exposure_eye: None,
            exposure_cut: 0,
            exposure_cut_from: 0.0,
            hdr: genos_scene::HdrSettings::legacy_bytes(),
            hdr_look: None,
            exposure_scale: 1.0,
            last_ev100: 0.0,
            exposure_log: Vec::new(),
            histogram_counts: Vec::new(),
            cache_keys: Buffer::empty(),
            cache: Buffer::empty(),
            compose_host: Buffer::empty(),
            cache_clear: false,
            gbuf_clear: false,
            work_clear: false,
            compose_valid: false,
            bounce_fill: 0,
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
            scene_catch: 0,
            mover_local: false,
            reach_hot: 0,
            reach_tiles: 0,
            reach_wide: false,
            picture_unhold: false,
            reach_arm: 0,
            kept_froxel: false,
            kept_mesh: 0,
            stats: [Buffer::empty(), Buffer::empty()],
            stats_dev: [Buffer::empty(), Buffer::empty()],
            quiet: 0,
            stats_still: [0; 2],
            live: 0,
            dims: [0; 4],
            async_on: false,
            async_failed: false,
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
            light_scene_seen: 0,
            light_hash: 0,
            light_quiet: 0,
            draining: false,
            round_entries: 0,
            cache_rays: 0,
            movers: std::collections::BTreeMap::new(),
            mesh_poses: Vec::new(),
            mesh_movers: std::collections::BTreeMap::new(),
            mesh_spheres: Vec::new(),
            mesh_field_object: Vec::new(),
            stats_light: [0; 2],
            stats_young: [false; 2],
            dump: Vec::new(),
            world_light: 0,
            world_young: false,
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
            agx_pipe: std::ptr::null_mut(),
            bloom_pipe: std::ptr::null_mut(),
            meter_pipe: std::ptr::null_mut(),
            compose_hdr_pipe: std::ptr::null_mut(),
            compose_maint_pipe: std::ptr::null_mut(),
            sh_drain_pipe: std::ptr::null_mut(),
            cascade_pub_pipe: std::ptr::null_mut(),
            cascade_l0_trace_pipe: std::ptr::null_mut(),
            cascade_l0_shade_pipe: std::ptr::null_mut(),
            cascade_upper_claim_pipe: std::ptr::null_mut(),
            cascade_upper_pick_pipe: std::ptr::null_mut(),
            cascade_upper_trace_pipe: std::ptr::null_mut(),
            cascade_upper_shade_pipe: std::ptr::null_mut(),
            cascade_qp: std::ptr::null_mut(),
            cascade_times: crate::CascadeTimes::default(),
            compose_writes: genos_scene::lighting_env("GENOS_COMPOSE_SC_WRITES")
                .is_ok_and(|v| v == "1"),
            sun_hdr_pipe: std::ptr::null_mut(),
            sun_bounce_live: false,
            compose_kind: 0,
            compose_dispatches: 0,
            maint_ran: false,
            lamp_key: None,
            lamp_moving: false,
            catch_left: 0,
            cache_full: false,
            cache_key_pixels: 0,
            alloc_key: (0, 0, 0, 0, 0, 0, 0),
            lamp_res: Buffer::empty(),
            lamp_res_clear: false,
            area_mean: Buffer::empty(),
            area_clear: false,
            shade_gate: LampShadeGate::Product,
            restir_buf: Buffer::empty(),
            restir_pipe: std::ptr::null_mut(),
            restir_spatial_pipe: std::ptr::null_mut(),
            restir_spatial_wide: std::ptr::null_mut(),
            restir_spatial_bound: 0,
            restir_direct: std::ptr::null_mut(),
            restir_frame: false,
            restir_slot: 0,
            restir_prev: crate::restir_live::RestirCam::default(),
            restir_have_prev: false,
            restir_query: [u32::MAX; 2],
            restir_lamps: Vec::new(),
            restir_prev_lamps: Vec::new(),
            restir_view: crate::restir_live::RestirCam::default(),
            restir_w: 0,
            restir_h: 0,
            restir_clear: false,
            restir_prepare_failures: 0,
            restir_force_fail: false,
            frame_tags: [Vec::new(), Vec::new()],
            frame_counts: [[0; BUCKETS]; 2],
            frame_shadow: [false; 2],
            frame_prefill: [false; 2],
            last_ms: [0.0; BUCKETS],
            last_dispatches: [0; BUCKETS],
            restir_ms_sum: [0.0; 5],
            restir_ms_frames: 0,
            restir_ms_last: [0.0; 5],
            pass_samples: 0,
            pass_id_ms: [0.0; 10],
            span_ms: 0.0,
            stats_slot: 0,
            hdr_ms: 0.0,
            hdr_query: [u32::MAX; 2],
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
/// `gstats.diag[0]` is stats word 24. `diag[3]` counts sun-compose world-cache walks.
const LIGHTING_SUN_WALK: usize = 27;

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

fn copy_gpu_lamp(lamp: &crate::pack::GpuLamp) -> crate::pack::GpuLamp {
    crate::pack::GpuLamp {
        pos: lamp.pos,
        color: lamp.color,
        directional: lamp.directional,
        range: lamp.range,
    }
}

impl Gpu {
    /// Track which traced mesh instances move (surface cache movers).
    pub(crate) fn note_field_movers(&mut self, live: &[&crate::mesh_field::FieldInstance]) {
        let g = &mut self.lighting;
        lighting_mesh_movers(live, &mut g.mesh_poses, &mut g.mesh_movers, &mut g.mesh_spheres);
        g.mesh_field_object.clear();
        g.mesh_field_object
            .extend(live.iter().map(|f| f.object));
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
        let bindings: Vec<Binding> = [0u32, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
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
            // 3 sets × 21 storage bindings (1-4 and 15: the cone-traced cascades).
            let size = Size { kind: 7, count: 63 };
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
            LIGHTING_PLACE_SPV,
        ];
        for (k, code) in codes.iter().enumerate() {
            let start = Instant::now();
            // Pass 7 is compose. Specialization constant 0 is the surface-cache
            // writes. The picture compile stays read-only unless
            // GENOS_COMPOSE_SC_WRITES=1. Maintenance requests the lattice and
            // enqueues texels; the drain fills them before this compile loads.
            let spec = (k == 7).then_some(u32::from(self.lighting.compose_writes));
            // Pass 3 is direct. Constant 3 is the lamp disc. Knockout aims the
            // pixel ray at the centre. Constants 0 and 2 stay the ReSTIR defaults.
            if k == 3 && !crate::gate::guard(crate::gate::LAMP_DISC) {
                self.lighting.pipes[k] = self.make_compute_pipe_ids(
                    code,
                    self.lighting.layout,
                    LIGHTING_PASS_NAMES[k],
                    &[(3, 0)],
                )?;
            } else {
                self.lighting.pipes[k] = self.make_compute_pipe_spec(code, self.lighting.layout, LIGHTING_PASS_NAMES[k], spec)?;
            }
            self.note_pipeline_time(LIGHTING_PASS_NAMES[k], start);
        }
        if !self.lighting.compose_writes {
            let start = Instant::now();
            self.lighting.compose_maint_pipe = self.make_compute_pipe(
                LIGHTING_COMPOSE_MAINT_SPV,
                self.lighting.layout,
                "compose-maint",
            )?;
            self.note_pipeline_time("compose-maint", start);
            let start = Instant::now();
            self.lighting.sh_drain_pipe = self.make_compute_pipe(
                LIGHTING_SH_DRAIN_SPV,
                self.lighting.layout,
                "sh-drain",
            )?;
            self.note_pipeline_time("sh-drain", start);
        }
        let start = Instant::now();
        self.lighting.cascade_pub_pipe = self.make_compute_pipe(
            LIGHTING_CASCADE_PUB_SPV,
            self.lighting.layout,
            "cascade-pub",
        )?;
        self.note_pipeline_time("cascade-pub", start);
        let start = Instant::now();
        self.lighting.cascade_l0_trace_pipe = self.make_compute_pipe(
            LIGHTING_CASCADE_L0_TRACE_SPV,
            self.lighting.layout,
            "cascade-l0-trace",
        )?;
        self.note_pipeline_time("cascade-l0-trace", start);
        let start = Instant::now();
        self.lighting.cascade_l0_shade_pipe = self.make_compute_pipe(
            LIGHTING_CASCADE_L0_SHADE_SPV,
            self.lighting.layout,
            "cascade-l0-shade",
        )?;
        self.note_pipeline_time("cascade-l0-shade", start);
        let start = Instant::now();
        self.lighting.cascade_upper_claim_pipe = self.make_compute_pipe(
            LIGHTING_CASCADE_UPPER_CLAIM_SPV,
            self.lighting.layout,
            "cascade-upper-claim",
        )?;
        self.note_pipeline_time("cascade-upper-claim", start);
        let start = Instant::now();
        self.lighting.cascade_upper_pick_pipe = self.make_compute_pipe(
            LIGHTING_CASCADE_UPPER_PICK_SPV,
            self.lighting.layout,
            "cascade-upper-pick",
        )?;
        self.note_pipeline_time("cascade-upper-pick", start);
        let start = Instant::now();
        self.lighting.cascade_upper_trace_pipe = self.make_compute_pipe(
            LIGHTING_CASCADE_UPPER_TRACE_SPV,
            self.lighting.layout,
            "cascade-upper-trace",
        )?;
        self.note_pipeline_time("cascade-upper-trace", start);
        let start = Instant::now();
        self.lighting.cascade_upper_shade_pipe = self.make_compute_pipe(
            LIGHTING_CASCADE_UPPER_SHADE_SPV,
            self.lighting.layout,
            "cascade-upper-shade",
        )?;
        self.note_pipeline_time("cascade-upper-shade", start);
        let start = Instant::now();
        self.lighting.reach_pipe = self.make_compute_pipe(
            LIGHTING_REACH_SPV,
            self.lighting.layout,
            "reach",
        )?;
        self.note_pipeline_time("reach", start);
        let start = Instant::now();
        self.frag_override = Some(GBUFFER_FRAG_SPV);
        self.vert_override = Some(GBUFFER_VERT_SPV);
        self.depth_equal_only = true;
        let made = self.make_pipeline(true, false, false);
        self.depth_equal_only = false;
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
            self.lighting.async_failed = true;
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
        self.lighting.cascade_qp = self.make_query_pool_n(CASCADE_TIME_QUERIES)?;
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
        // Shutdown polls this fence from the event loop. Waiting here would
        // block that loop inside device destruction.
        if self.lifecycle_release {
            self.lighting.world_busy = false;
            return;
        }
        if !self.lighting.world_busy || self.lighting.world_fence.is_null() {
            return;
        }
        let _ = self.wait_fence(self.lighting.world_fence, "lighting world");
        self.lighting.world_busy = false;
    }

    /// Specialization constants for `restir-direct`.
    ///
    /// `(constant id, value)`. Constant 0 is the few-lamp cap. Constant 2
    /// keeps history. `[0, 0]` is `moving_lamp_wall` knocked out.
    fn moving_lamp_wall_spec() -> [(u32, u32); 2] {
        if crate::gate::guard(crate::gate::MOVING_LAMP_WALL) {
            [(0, 4), (2, 1)]
        } else {
            [(0, 0), (2, 0)]
        }
    }

    fn make_compute_pipe(&self, code: &[u8], layout: Handle, what: &str) -> Result<Handle, String> {
        self.make_compute_pipe_ids(code, layout, what, &[])
    }

    /// `spec` is specialization constant 0 (a GLSL bool is a VkBool32), or none.
    fn make_compute_pipe_spec(
        &self,
        code: &[u8],
        layout: Handle,
        what: &str,
        spec: Option<u32>,
    ) -> Result<Handle, String> {
        match spec {
            Some(value) => self.make_compute_pipe_ids(code, layout, what, &[(0, value)]),
            None => self.make_compute_pipe_ids(code, layout, what, &[]),
        }
    }

    /// `(id, value)` pairs. At most two. Empty keeps the shader defaults.
    /// The moving-lamp wall sets constant 0 (exact-lamp cap) and constant 2
    /// (keep history). Constant 1 is the cache slot count and stays default.
    fn make_compute_pipe_ids(
        &self,
        code: &[u8],
        layout: Handle,
        what: &str,
        ids: &[(u32, u32)],
    ) -> Result<Handle, String> {
        let module = self.shader(code)?;
        #[repr(C)]
        #[derive(Copy, Clone)]
        struct MapEntry {
            id: u32,
            offset: u32,
            size: usize,
        }
        #[repr(C)]
        struct SpecInfo {
            count: u32,
            entries: *const MapEntry,
            size: usize,
            data: *const c_void,
        }
        let mut entries = [MapEntry {
            id: 0,
            offset: 0,
            size: 4,
        }; 2];
        let mut data = [0u32; 2];
        let count = ids.len().min(2);
        for i in 0..count {
            entries[i] = MapEntry {
                id: ids[i].0,
                offset: u32::try_from(i.saturating_mul(4)).unwrap_or(0),
                size: 4,
            };
            data[i] = ids[i].1;
        }
        let spec_info = SpecInfo {
            count: u32::try_from(count).unwrap_or(0),
            entries: entries.as_ptr(),
            size: count.saturating_mul(4),
            data: data.as_ptr().cast::<c_void>(),
        };
        let spec_ptr = if count == 0 {
            std::ptr::null()
        } else {
            &spec_info as *const SpecInfo as *const c_void
        };
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
                spec: spec_ptr,
            },
            layout,
            base: std::ptr::null_mut(),
            base_index: -1,
        };
        let mut pipeline = std::ptr::null_mut();
        unsafe {
            let result = (self.fns.create_compute)(
                self.device,
                self.pipeline_cache,
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

    /// Size the first buffers from the process launch, not from the High default.
    ///
    /// Device creation runs before a scene exists. The High default would
    /// allocate the million-slot cache and free it once the launch preset is
    /// potato. lavapipe keeps those pages until `vkDestroyDevice`, and that
    /// release misses the quit budget. `--preset` wins over
    /// `GENOS_LIGHTING_PRESET`.
    pub(crate) fn apply_launch_preset(&mut self) -> Result<(), String> {
        let mut launch = genos_scene::LightingSettings::default();
        launch.apply_env()?;
        self.lighting.settings = launch;
        Ok(())
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
            + pixels * 6 // probe-seed scratch, two vec4 per pixel, stride 6
        } else {
            0
        };
        let work_vec4 = work_vec4 + work_tail;
        let mut gbuf = std::mem::replace(&mut self.lighting.gbuf, Buffer::empty());
        let mut work = std::mem::replace(&mut self.lighting.work, Buffer::empty());
        let mut out = std::mem::replace(&mut self.lighting.out, Buffer::empty());
        let mut scene_lin = std::mem::replace(&mut self.lighting.scene_lin, Buffer::empty());
        let mut scene_host = std::mem::replace(&mut self.lighting.scene_host, Buffer::empty());
        let mut hdr_params = std::mem::replace(&mut self.lighting.hdr_params, Buffer::empty());
        let mut hdr_encode = std::mem::replace(&mut self.lighting.hdr_encode, Buffer::empty());
        let mut bloom = std::mem::replace(&mut self.lighting.bloom, Buffer::empty());
        let mut meter_dev = std::mem::replace(
            &mut self.lighting.meter_dev,
            [Buffer::empty(), Buffer::empty()],
        );
        let mut meter_host = std::mem::replace(
            &mut self.lighting.meter_host,
            [Buffer::empty(), Buffer::empty()],
        );
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        self.destroy_buffer(&mut scene_lin);
        self.destroy_buffer(&mut scene_host);
        self.destroy_buffer(&mut hdr_params);
        self.destroy_buffer(&mut hdr_encode);
        self.destroy_buffer(&mut bloom);
        for dev in &mut meter_dev {
            self.destroy_buffer(dev);
        }
        for host in &mut meter_host {
            self.destroy_buffer(host);
        }
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
        let keys_small = want_cache && self.lighting.cache_full && pixels > self.lighting.cache_key_pixels;
        if want_cache != self.lighting.cache_full || self.lighting.cache.buffer.is_null() || keys_small {
            let mut keys = std::mem::replace(&mut self.lighting.cache_keys, Buffer::empty());
            let mut cache = std::mem::replace(&mut self.lighting.cache, Buffer::empty());
            self.destroy_buffer(&mut keys);
            self.destroy_buffer(&mut cache);
            if want_cache {
                // The compose words and the a-trous meta must not alias the a-trous lists.
                assert!(lighting_compose_at() >= LIGHTING_ATROUS_LIST_AT + 5 * (LIGHTING_ATROUS_LIST_CAP + 1) + 6 * 65536 + 16);
                assert!(lighting_atrous_ppm_at() >= lighting_compose_minx() + 4 * LIGHTING_CACHE_SLOTS);
                // Through the lattice, then the lit-from offer and the seed
                // election (lighting_sh_keys_words). Eight words per pixel
                // after that: the seed winner's payload, four corners.
                self.lighting.cache_keys = self.make_buffer_queues(
                    lighting_keys_words(pixels) * 4,
                    0x20 | 0x2 | 0x100 | 0x1,
                    Memory::Device,
                    &share,
                )?;
                // 23 vec4 regions a slot, plus ADR 0039 stage 2's SH-L1
                // (LIGHTING_SH_AT.. in lighting_cache.glsl, LIGHTING_SH_HALF).
                self.lighting.cache = self.make_buffer_queues(
                    LIGHTING_CACHE_SLOTS * LIGHTING_CACHE_SLOT_BYTES,
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
            self.lighting.cache_key_pixels = if want_cache { pixels } else { 0 };
        }
        if want_cache && self.lighting.compose_host.buffer.is_null() {
            // One word: the dirty-compose list length, copied after the passes.
            self.lighting.compose_host = self.make_buffer(256, 0x2, Memory::Readback)?;
        }
        // GBUF_STRIDE = 6 uints/pixel (object id in word 5).
        self.lighting.gbuf = self.make_buffer(16 + pixels * 24, 0x20 | 0x1 | 0x2, Memory::Device)?;
        self.lighting.work =
            self.make_buffer_queues(work_vec4.max(1) * 16, 0x20 | 0x1 | 0x2, Memory::Device, &share)?;
        // Picture, emitted light, and (when direct refresh is slower than every
        // frame) one stored direct value per pixel. High stays at pixels * 8.
        let hist = u64::from(settings.direct.refresh_frames > 1);
        // One extra slice holds linear bounce. Picture compose stores the last
        // full walk there and a held frame reads it. A cache clear zeros it
        // for BOUNCE_FILL_FRAMES. High (refresh every frame) is picture + glow
        // + that slice. The screen wavelet appends two vec4 slices (32 bytes
        // a pixel) after that, so it does not alias the bounce.
        // Hash keeps the GI buffer. Preset high uses the screen slices.
        let screen = u64::from(settings.denoiser.enabled && settings.denoiser.screen);
        self.lighting.out = self.make_buffer(
            pixels * (12 + 4 * hist + 32 * screen),
            0x20 | 0x1 | 0x2,
            Memory::Device,
        )?;
        let encode_full = self.post_encode_full();
        let probe = self.lighting.bloom_probe && !encode_full;
        let chain = crate::hdr_output::post_chain(w, h, &self.lighting.hdr, encode_full);
        self.lighting.scene_lin =
            self.make_buffer(chain.scene_bytes, 0x20 | 0x1 | 0x2, Memory::Device)?;
        // Transfer-dst so the pyramid can write the bloom intensity word.
        // Eight floats: scale, mode, bloom, output, white, peak, format, bloom divisor.
        self.lighting.hdr_params = self.make_buffer(chain.params_bytes, 0x20 | 0x2, Memory::Upload)?;
        let encode_bytes = if probe {
            pixels.saturating_mul(12)
        } else {
            chain.encode_bytes
        };
        self.lighting.hdr_encode = if encode_bytes == 0 {
            Buffer::empty()
        } else {
            // Storage for the encode pass, transfer-src when the fp16 image is copied.
            self.make_buffer(encode_bytes, 0x20 | 0x1, Memory::Upload)?
        };
        self.lighting.bloom = if chain.bloom_bytes == 0 {
            Buffer::empty()
        } else {
            self.make_buffer(chain.bloom_bytes, 0x20, Memory::Upload)?
        };
        // Storage for the histogram, transfer so the header can be cleared and copied.
        // One device buffer per flight. Mailbox used to fill one buffer from
        // both frames: that copy is a WRITE_AFTER_WRITE. Each slot compares
        // its own previous half. The byte count comes from the post chain.
        for slot in 0..2 {
            self.lighting.meter_dev[slot] = if chain.meter_bytes == 0 {
                Buffer::empty()
            } else {
                self.make_buffer(chain.meter_bytes, 0x20 | 0x1 | 0x2, Memory::Device)?
            };
            self.lighting.meter_host[slot] = if chain.meter_host_bytes == 0 {
                Buffer::empty()
            } else {
                self.make_buffer(chain.meter_host_bytes, 0x2, Memory::Readback)?
            };
        }
        self.lighting.post_sig =
            crate::hdr_output::post_signature(&self.lighting.hdr, encode_full, probe);
        self.lighting.meter_pending = [false, false];
        self.lighting.meter_bins = 0;
        self.lighting.meter_has_prev = [false, false];
        self.lighting.meter_ping = [0, 0];
        self.lighting.meter_ev = None;
        self.lighting.still_pop = 0.0;
        self.lighting.histogram_counts.clear();
        let mut lamp_res = std::mem::replace(&mut self.lighting.lamp_res, Buffer::empty());
        self.destroy_buffer(&mut lamp_res);
        let reuse = settings.lamp_pick.reuse();
        // 32 bytes a pixel: the reservoir and the geo record (depth, normal,
        // exact/trusted kind, shaded direct). Spatial reuse keeps the previous
        // frame. Temporal only (radius 0) keeps one slot. Off binds a dummy so
        // binding 12 is valid.
        let slots = if settings.lamp_pick.radius > 0 { 2 } else { 1 };
        let lamp_bytes = if reuse {
            pixels.saturating_mul(32 * slots).max(64)
        } else {
            64
        };
        self.lighting.lamp_res = self.make_buffer_queues(lamp_bytes, 0x20 | 0x2, Memory::Device, &share)?;
        self.lighting.lamp_res_clear = reuse;
        let mut area_old = std::mem::replace(&mut self.lighting.area_mean, Buffer::empty());
        self.destroy_buffer(&mut area_old);
        // Two frames of two vec4s a pixel. The shade writes the current slot
        // and reads the previous frame at the reprojected pixel. A resize
        // zeros both slots before the next trace.
        let area_bytes = pixels.saturating_mul(32 * 2).max(64);
        self.lighting.area_mean =
            self.make_buffer_queues(area_bytes, 0x20 | 0x2, Memory::Device, &share)?;
        self.lighting.area_clear = true;
        // ReSTIR reservoirs are sized here, after the frames in flight have
        // finished. A resize inside a recording frame is not this path.
        let mut restir_old = std::mem::replace(&mut self.lighting.restir_buf, Buffer::empty());
        self.destroy_buffer(&mut restir_old);
        let restir_bytes = if settings.restir_di {
            crate::restir_live::buffer_bytes(pixels)
        } else {
            256
        };
        self.lighting.restir_buf = self.make_buffer(restir_bytes, 0x20 | 0x2 | 0x1, Memory::Device)?;
        self.lighting.restir_w = if settings.restir_di { w } else { 0 };
        self.lighting.restir_h = if settings.restir_di { h } else { 0 };
        self.lighting.restir_clear = true;
        self.lighting.restir_have_prev = false;
        self.lighting.restir_slot = 0;
        self.lighting.restir_frame = false;
        self.lighting.dims = [w, h, cols, rows];
        self.lighting.alloc_key = lighting_alloc_key(&settings);
        self.lighting.compose_valid = false;
        self.lighting.bounce_fill = 0;
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
            let (scene, stats, set, meter) = if slot < 2 {
                (
                    self.frame_scene[slot].buffer,
                    self.lighting.stats_dev[slot].buffer,
                    self.lighting.sets[slot],
                    self.lighting.meter_dev[slot].buffer,
                )
            } else {
                (
                    self.lighting.world_scene.buffer,
                    self.lighting.world_stats.buffer,
                    self.lighting.world_set,
                    self.lighting.meter_dev[0].buffer,
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
                (13, self.lighting.scene_lin.buffer),
                (14, self.lighting.hdr_params.buffer),
                (16, self.bound_or_standin(&self.lighting.bloom)),
                (17, self.bound_or_standin(&self.lighting.hdr_encode)),
                (18, if meter.is_null() { self.lighting.hdr_params.buffer } else { meter }),
                (9, self.lighting.cache_keys.buffer),
                (10, self.lighting.cache.buffer),
                (11, stats),
                (12, self.lighting.lamp_res.buffer),
                (19, self.lighting.restir_buf.buffer),
                (20, self.bound_or_standin(&self.lighting.area_mean)),
                (1, self.bound_or_standin(&self.cone.table)),
                (2, self.bound_or_standin(&self.cone.fine)),
                (3, self.bound_or_standin(&self.cone.records)),
                (4, self.bound_or_standin(&self.cone.coarse)),
                (15, self.bound_or_standin(&self.cone.work)),
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
                4 * match self.lighting.settings.experiments.world_cap {
                    0 => 32,
                    cap => cap,
                }
            }
            // Surface cache: `experiments.sc_settle` frames still (experiment: how long
            // its texels take to take in the bounced light).
            3 => self.lighting.settings.experiments.sc_settle,
            _ => 0,
        };
        // Surface cache with flag 8192: texels at the cap keep relighting (their
        // change never drops under LIGHTING_SETTLE_CHANGE), so it settles on frames still
        // (`experiments.sc_settle`, default 300) and the light having held.
        let sc_relit = world_wait != 0
            || (world_mode == 3 && self.lighting.settings.experiments.sc_flags & 8192 != 0);
        let settled = if world_mode == 3 && sc_relit {
            self.lighting.still >= LIGHTING_SETTLE_FRAMES.max(if world_wait != 0 { world_wait } else { 300 })
                && self.lighting.light_quiet >= 2
                && !self.lighting.draining
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
            draining: self.lighting.draining,
            round_entries: self.lighting.round_entries,
            cache_rays: self.lighting.cache_rays,
        }
    }

    fn lighting_still_frames(&self) -> u32 {
        self.lighting.still
    }

    /// Rays the screen probes held in the last frame drawn, and probes placed
    /// (lighting_gather.comp): waits for that frame.
    pub(crate) fn lighting_probe_samples(&mut self) -> [u32; 2] {
        self.idle_device();
        self.read_lighting_stats(self.lighting.last_slot)
            .map_or(self.lighting.probe_samples, |w| [w[6], w[7]])
    }

    /// Shadow rays the last frame actually traced, by consumer. Zero unless
    /// `GENOS_LIGHTING_LAMP_PICK_DIAG` was set for that frame. Waits for that frame.
    pub(crate) fn lighting_lamp_pick_counts(&mut self) -> LampPickCounts {
        self.idle_device();
        let slot = self.lighting.last_slot;
        let restir_diag = self.read_restir_diag(self.lighting.stats[slot].memory);
        self.read_stats16(self.lighting.stats[slot].memory)
            .map_or_else(|_| LampPickCounts::default(), |w| LampPickCounts {
                pixels: w[LAMP_PICK_DIAG_WORD],
                kept: w[LAMP_PICK_DIAG_WORD + 1],
                fills: w[LAMP_PICK_DIAG_WORD + 2],
                cache: w[LAMP_PICK_DIAG_WORD + 3],
                exact: w[LAMP_PICK_DIAG_WORD + 10],
                restir: w[LAMP_PICK_DIAG_WORD + 11],
                kept_live: w[LAMP_PICK_DIAG_WORD + 5],
                kept_due: w[LAMP_PICK_DIAG_WORD + 6],
                kept_shade: w[LAMP_PICK_DIAG_WORD + 7],
                fill_live: w[LAMP_PICK_DIAG_WORD + 8],
                fill_shade: w[LAMP_PICK_DIAG_WORD + 12],
                near_tests: w[LAMP_PICK_DIAG_WORD - 14],
                px_lamps: w[LAMP_PICK_DIAG_WORD - 10],
                hit_lamps: w[LAMP_PICK_DIAG_WORD - 9],
                history_accepts: restir_diag[0],
                history_rejects: restir_diag[1],
                restir_point_adds: restir_diag[2],
            })
    }

    /// Displaying picture pixels whose fine world-cache read was under a full
    /// set count last frame. Stats word 81 (`gstats.diag[57]`). Waits.
    pub(crate) fn lighting_under_trusted_pixels(&mut self) -> u32 {
        self.idle_device();
        const WORD: usize = 24 + 57;
        self.read_stats16(self.lighting.stats[self.lighting.last_slot].memory)
            .map_or(0, |w| w[WORD])
    }

    /// Visible surfaces last frame whose published sample was empty. A real
    /// zero has weight and is not counted. Stats word 35 (`gstats.diag[11]`,
    /// `LIGHTING_DIAG_CASCADE_GAP`). Waits.
    pub(crate) fn lighting_cascade_gaps(&mut self) -> u32 {
        self.idle_device();
        const WORD: usize = 24 + 11;
        self.read_stats16(self.lighting.stats[self.lighting.last_slot].memory)
            .map_or(0, |w| w[WORD])
    }

    /// Level-0 voxels inserted and traced last frame. Waits.
    pub(crate) fn lighting_level0_counts(&mut self) -> Result<(u32, u32), String> {
        self.idle_device();
        if self.lighting.cache_keys.buffer.is_null() {
            return Ok((0, 0));
        }
        let [width, height, _, _] = self.lighting.dims;
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        let offset = lighting_level0_counts_at(pixels).saturating_mul(4);
        let raw = self.copy_device_range(self.lighting.cache_keys.buffer, offset, 8)?;
        let words = u32_words(&raw);
        let inserted = words.first().copied().unwrap_or(0);
        let traced = words.get(1).copied().unwrap_or(0);
        Ok((inserted, traced))
    }

    /// Live level-0 voxels from the table written by the last finished frame.
    /// Waits for that frame.
    pub(crate) fn lighting_level0_voxels(&mut self) -> Result<Vec<crate::Level0Voxel>, String> {
        self.idle_device();
        if self.lighting.cache_keys.buffer.is_null() {
            return Ok(Vec::new());
        }
        let [width, height, _, _] = self.lighting.dims;
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        let start = lighting_fill_keys_words(pixels) + lighting_reach_words();
        let table = CASCADE_LEVEL0_TABLES
            .saturating_mul(CASCADE_LEVEL0_CAP)
            .saturating_mul(CASCADE_LEVEL0_STRIDE);
        let words = 1 + table;
        let offset = start.saturating_mul(4);
        let bytes = words.saturating_mul(4);
        if offset.saturating_add(bytes) > self.lighting.cache_keys.size {
            return Err(format!(
                "level-0 table at {offset}+{bytes} is past keys {}",
                self.lighting.cache_keys.size
            ));
        }
        let raw = self.copy_device_range(self.lighting.cache_keys.buffer, offset, bytes)?;
        let keys = u32_words(&raw);
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut latest = 0u32;
        let mut voxels = Vec::new();
        let stride = usize::try_from(CASCADE_LEVEL0_STRIDE).map_err(|err| err.to_string())?;
        let mut at = 1usize;
        while at + stride <= keys.len() {
            let rec = &keys[at..at + stride];
            at += stride;
            if rec[0] == 0 || rec[1] == 0 {
                continue;
            }
            latest = latest.max(rec[1]);
        }
        if latest == 0 {
            return Ok(Vec::new());
        }
        at = 1;
        while at + stride <= keys.len() {
            let rec = &keys[at..at + stride];
            at += stride;
            if rec[0] == 0 || rec[1] != latest {
                continue;
            }
            let cell = [
                i32::from_le_bytes(rec[2].to_le_bytes()),
                i32::from_le_bytes(rec[3].to_le_bytes()),
                i32::from_le_bytes(rec[4].to_le_bytes()),
            ];
            let packed = rec[5];
            let voxel = crate::Level0Voxel {
                cell,
                level: packed & 255,
                axis: packed >> 8,
                pos: [
                    f32::from_bits(rec[6]),
                    f32::from_bits(rec[7]),
                    f32::from_bits(rec[8]),
                ],
                radiance: crate::cache_probe::unpack_rgb9e5(rec[10]),
                weight: f32::from_bits(rec[11]),
            };
            if voxels.iter().any(|other: &crate::Level0Voxel| other.key() == voxel.key()) {
                continue;
            }
            voxels.push(voxel);
        }
        Ok(voxels)
    }

    /// Serial plan iterations (`gstats.pad2`, stats word 20) and the histogram
    /// dispatch's group count (`gstats.pad3`, word 21) from the last frame.
    /// A slot-table scan is hundreds of thousands of iterations and a dispatch
    /// over every cache slot. Waits for that frame.
    pub(crate) fn lighting_fill_compact_work(&mut self) -> [u32; 2] {
        self.idle_device();
        const SERIAL: usize = 20;
        const GROUPS: usize = 21;
        self.read_stats16(self.lighting.stats[self.lighting.last_slot].memory)
            .map_or([0, 0], |w| [w[SERIAL], w[GROUPS]])
    }

    /// Last frame: rays, on-screen, remainder, checks, cells committed, live.
    pub(crate) fn lighting_fill_ray_split(&mut self) -> [u32; 6] {
        self.idle_device();
        self.read_stats16(self.lighting.stats[self.lighting.last_slot].memory)
            .map_or([0; 6], |w| [w[16], w[17], w[19], w[22], w[1], w[2]])
    }



    /// Product frames that missed the reservoir and shaded the labelled exact sum.
    pub(crate) fn lighting_restir_prepare_failures(&self) -> u32 {
        self.lighting.restir_prepare_failures
    }

    /// The next product prepare shades the labelled exact sum and counts a failure.
    pub(crate) fn force_restir_prepare_failure(&mut self, on: bool) {
        self.lighting.restir_force_fail = on;
    }

    /// A-trous dirty fraction, mode and threshold for the last frame drawn.
    /// Waits for that frame. The words are the `lightingatrous` line.
    pub(crate) fn lighting_atrous_stats(&mut self) -> LightingAtrousStats {
        self.idle_device();
        let slot = self.lighting.last_slot;
        let mut stats = self
            .read_stats16(self.lighting.stats[slot].memory)
            .map_or_else(|_| LightingAtrousStats::default(), |w| lighting_atrous_from(&w));
        stats.reach_wide = self.lighting.reach_wide;
        stats
    }

    /// Tile bits from the last frame. Word 0 covers tiles 0..31.
    pub(crate) fn lighting_reach_mask(&mut self) -> Vec<u32> {
        self.idle_device();
        let [w, h, _, _] = self.lighting.dims;
        if !self.lighting.ready || w == 0 || h == 0 {
            return Vec::new();
        }
        let pixels = u64::from(w) * u64::from(h);
        let at = lighting_fill_keys_words(pixels) * 4;
        let bytes = 2048u64 * 4;
        self.copy_device_range(self.lighting.cache_keys.buffer, at, bytes).map_or_else(
            |_| Vec::new(),
            |raw| {
                raw.chunks_exact(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect()
            },
        )
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

    /// The G-buffer pipeline embeds the color render pass, so a format change rebuilds it.
    fn remake_gbuffer_pipe(&mut self) -> Result<(), String> {
        if !self.lighting.ready {
            return Ok(());
        }
        self.frag_override = Some(GBUFFER_FRAG_SPV);
        self.vert_override = Some(GBUFFER_VERT_SPV);
        self.depth_equal_only = true;
        let made = block(|| self.make_pipeline(true, false, false));
        self.depth_equal_only = false;
        self.frag_override = None;
        self.vert_override = None;
        self.lighting.gbuffer_pipe = made?;
        Ok(())
    }

    /// AgX display pipeline. The lattice sample is not in the GI compose shader.
    fn ensure_agx_pipe(&mut self) -> Result<(), String> {
        if !self.lighting.agx_pipe.is_null() {
            return Ok(());
        }
        let start = Instant::now();
        self.lighting.agx_pipe =
            self.make_compute_pipe(HDR_AGX_SPV, self.lighting.layout, "hdr agx")?;
        self.note_pipeline_time("hdr agx", start);
        Ok(())
    }

    fn ensure_meter_pipe(&mut self) -> Result<(), String> {
        if !self.lighting.meter_pipe.is_null() {
            return Ok(());
        }
        let start = Instant::now();
        self.lighting.meter_pipe =
            self.make_compute_pipe(HDR_METER_SPV, self.lighting.layout, "hdr meter")?;
        self.note_pipeline_time("hdr meter", start);
        Ok(())
    }

    fn ensure_bloom_pipe(&mut self) -> Result<(), String> {
        if !self.lighting.bloom_pipe.is_null() {
            return Ok(());
        }
        let start = Instant::now();
        self.lighting.bloom_pipe =
            self.make_compute_pipe(HDR_BLOOM_SPV, self.lighting.layout, "hdr bloom")?;
        self.note_pipeline_time("hdr bloom", start);
        Ok(())
    }

    fn ensure_hdr_compose(&mut self) -> Result<(), String> {
        if !self.lighting.compose_hdr_pipe.is_null() {
            return Ok(());
        }
        let start = Instant::now();
        self.lighting.compose_hdr_pipe = self.make_compute_pipe_spec(
            LIGHTING_COMPOSE_HDR_SPV,
            self.lighting.layout,
            "lighting compose hdr",
            Some(1u32),
        )?;
        self.note_pipeline_time("lighting compose hdr", start);
        Ok(())
    }

    fn ensure_sun_hdr_pipe(&mut self) -> Result<(), String> {
        if !self.lighting.sun_hdr_pipe.is_null() {
            return Ok(());
        }
        let start = Instant::now();
        self.lighting.sun_hdr_pipe = self.make_compute_pipe(
            SUN_COMPOSE_HDR_SPV,
            self.lighting.layout,
            "lighting sun compose hdr",
        )?;
        self.note_pipeline_time("lighting sun compose hdr", start);
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
        self.lighting.restir_lamps.clear();
        self.lighting.restir_lamps.extend(pack.lamps.iter().map(copy_gpu_lamp));
        self.lighting.restir_view = crate::restir_live::RestirCam {
            eye: pack.eye,
            right: pack.view_right,
            up: pack.view_up,
            forward: pack.view_forward,
            tan: pack.view_tan,
            aspect: pack.view_aspect,
            valid: true,
        };
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
        let mut walks = 0u32;
        if let Ok(words) = self.read_stats16(self.lighting.stats[slot].memory) {
            self.lighting.sun_gpu_mark = words[LIGHTING_SUN_MARK];
            self.lighting.sun_gpu_slice = words[LIGHTING_SUN_SLICE_WORD];
            walks = words[LIGHTING_SUN_WALK];
        }
        self.publish_sun_counter(false);
        LightingSunStats {
            gpu_sun: self.lighting.sun_gpu_mark & 1 != 0,
            gpu_catch: self.lighting.sun_gpu_mark & 2 != 0,
            slice: self.lighting.sun_gpu_slice,
            walks,
            host_frames: self.lighting.sun_frames,
            catch_left: self.lighting.catch_left,
            full_effort: sun_controls().full_effort,
            compose: self.lighting.compose_kind,
            dispatches: self.lighting.compose_dispatches,
            blocks: self.read_compose_blocks(),
            maint_ran: self.lighting.maint_ran,
        }
    }

    /// Dirty-list length copied at the end of the last submitted frame.
    /// Zero when that copy was not recorded. Not a frame-loop sync: the caller
    /// has already waited.
    fn read_compose_blocks(&self) -> u32 {
        if self.lighting.compose_host.memory.is_null() {
            return 0;
        }
        self.read_stats_word(self.lighting.compose_host.memory, 0).unwrap_or(0)
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
    fn lighting_begin(&mut self, slot: usize) -> Result<(), String> {
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
        // The drain flag is the one exception, and it is not a sync read: the
        // GPU decides draining itself (args pass, LIGHTING_DRAIN_FLAG_AT keeps
        // the settle rounds going); the host only needs it so it does not idle
        // the cache meanwhile. It is one word of this slot's stats ring buffer,
        // whose fence the frame loop already waited (frames in flight), so the
        // value is a few frames old and nothing waits on the GPU for it.
        if self.lighting.settings.world_mode() == 3 {
            if self.lighting.stats_round[slot] {
                if let Ok(flag) = self.read_stats_word(self.lighting.stats[slot].memory, LIGHTING_DRAIN_WORD + 2) {
                    self.lighting.draining = flag != 0;
                }
                if let Ok(e) = self.read_stats_word(self.lighting.stats[slot].memory, LIGHTING_DRAIN_WORD + 3) {
                    self.lighting.round_entries = e;
                }
                if let Ok(rays) = self.read_stats_word(self.lighting.stats[slot].memory, LIGHTING_CACHE_RAYS_WORD) {
                    self.lighting.cache_rays = rays;
                }
                // The live-cell count sizes the cache sweep. Mode 3 does not
                // take the full stats read (S70B). The fence for this slot
                // has already been waited.
                if let Ok(live) = self.read_stats_word(self.lighting.stats[slot].memory, 2) {
                    self.lighting.live = live;
                }
                // diag[8], a frame or two behind the dispatch that wrote it.
                if let Ok(reach) = self.read_stats_word(self.lighting.stats[slot].memory, 32) {
                    self.lighting.reach_hot = (reach >> 16) & 0xfff;
                    self.lighting.reach_tiles = reach & 0xffff;
                }
            } else if !self.lighting.async_on {
                // That slot did not dispatch a cache round. The stats buffer was
                // cleared and never counted, so the probe is zero, not the
                // previous round's leftover. The async queue reports its own
                // round from poll_lighting_world.
                self.lighting.cache_rays = 0;
                self.lighting.round_entries = 0;
            }
        }
        let read = lighting_stats_wanted(
            self.lighting.settings.world_mode(),
            self.lighting.settings.experiments.sc_flags,
        );
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
                // The picture still holds the previous view, and the bounce
                // slice may hold an empty store from that view. The fill
                // window composes the whole picture and zeros the slice.
                self.lighting.bounce_fill = 0;
                self.lighting.compose_valid = false;
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
            // The whole-buffer clears land before the count and header writes.
            // A subrange fill of the key table is the same buffer as the clear above.
            self.memory_barrier(0x1000, 0x1000, 0x1000, 0x1000);
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
                let cmd = self.cmd;
                let dst = self.lighting.cache_keys.buffer;
                self.write_device(cmd, dst, (lighting_compose_at() + 1) * 4, pod_bytes(&yz))?;
            }
            // The mature and young patch counts, rebuilt by the compact pass.
            if !world && self.lighting.cache_full {
                for at in [LIGHTING_CACHE_SLOTS, 2 * LIGHTING_CACHE_SLOTS + 1, LIGHTING_WRC_COUNTS_AT, LIGHTING_WRC_COUNTS_AT + 1, LIGHTING_WRC_COUNTS_AT + 4, LIGHTING_WRC_COUNTS_AT + 5, LIGHTING_WRC_COUNTS_AT + 6, LIGHTING_WRC_COUNTS_AT + 7, LIGHTING_FINE_COUNT_AT] {
                    (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.cache_keys.buffer, at * 4, 4, 0);
                }
                self.write_fine_lod(self.cmd)?;
            }
            // Settled: the cache holds its light and runs no rounds, so the drain
            // flag would keep the last value its args pass wrote. The host settled
            // on a flag a few frames old, and the GPU may have re-armed it since;
            // left set, every pixel's direct pass keeps gathering strictly at rest.
            if self.lighting.cache_full && self.lighting.settings.world_mode() == 3 && self.lighting_stats().settled {
                (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.cache_keys.buffer, LIGHTING_DRAIN_FLAG_AT * 4, 4, 0);
            }
            let cmd = self.cmd;
            let dst = self.lighting.gbuf.buffer;
            self.write_device(cmd, dst, 0, pod_bytes(&header))?;
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
        Ok(())
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
        let round_stats = self.read_stats16(self.lighting.world_stats.memory).ok();
        if let Some(words) = round_stats {
            let current = self.lighting.world_still > 0 && self.lighting.world_hash == self.lighting.scene_hash;
            let light = self.lighting.world_light == self.lighting.light_hash;
            let mut head = [0u32; 8];
            head.copy_from_slice(&words[..8]);
            self.lighting.draining = words[LIGHTING_DRAIN_WORD + 2] != 0;
            self.lighting.round_entries = words[LIGHTING_DRAIN_WORD + 3];
            self.lighting.cache_rays = words[LIGHTING_CACHE_RAYS_WORD];
            self.note_lighting_round(head, current, light, self.lighting.world_young);
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
                    if times.print {
                        times.light_spans.push((raw[0], raw[1]));
                        times.world_ms += tick_delta(raw[0], raw[1], bits) as f64 * period / 1.0e6;
                        times.world_runs += 1;
                        // Per pass: compact, trace, light, cache.
                        let mut prev = raw[0];
                        for k in 0..4 {
                            times.world_pass[k] +=
                                tick_delta(prev, raw[2 + k], bits) as f64 * period / 1.0e6;
                            prev = raw[2 + k];
                        }
                    }
                    if times.record {
                        if let Some(words) = round_stats {
                            times.pending_world_rays =
                                times.pending_world_rays.saturating_add(words[16]);
                            times.pending_world_relit =
                                times.pending_world_relit.saturating_add(words[1]);
                        }
                    }
                }
            }
        }
    }

    fn clear_cascade_l0_frame(&mut self, w: u32, h: u32) {
        let pixels = u64::from(w).saturating_mul(u64::from(h));
        let offset = lighting_level0_counts_at(pixels).saturating_mul(4);
        let bytes = CASCADE_L0_COUNT_WORDS.saturating_mul(4);
        let table_words = CASCADE_LEVEL0_CAP * CASCADE_LEVEL0_STRIDE;
        let table_at = lighting_fill_keys_words(pixels)
            + lighting_reach_words()
            + 1
            + (u64::from(self.lighting.frame) & 1) * table_words;
        let table_offset = table_at.saturating_mul(4);
        let table_bytes = table_words.saturating_mul(4);
        if self.lighting.cache_keys.buffer.is_null()
            || offset.saturating_add(bytes) > self.lighting.cache_keys.size
            || table_offset.saturating_add(table_bytes) > self.lighting.cache_keys.size
        {
            return;
        }
        // SAFETY: the command buffer and the key buffer are the frame's live
        // handles. The fill is the same call used for other per-frame counters.
        unsafe {
            (self.fns.cmd_fill_buffer)(
                self.cmd,
                self.lighting.cache_keys.buffer,
                offset,
                bytes,
                0,
            );
            (self.fns.cmd_fill_buffer)(
                self.cmd,
                self.lighting.cache_keys.buffer,
                table_offset,
                table_bytes,
                0,
            );
            // Zero live voxels, one group deep and one wide. The seat pass
            // counts the groups.
            self.memory_barrier(0x1000, 0x1000, 0x1000, 0x1000);
            let args = [0u32, 1, 1, 0];
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.lighting.cache_keys.buffer,
                offset + 8,
                16,
                args.as_ptr().cast::<std::ffi::c_void>(),
            );
            self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
        }
    }

    /// Trace and shade every live level-0 voxel, then leave their radiance
    /// in the table for the publish pass.
    fn dispatch_cascade_l0(&mut self) {
        let trace = self.lighting.cascade_l0_trace_pipe;
        let shade = self.lighting.cascade_l0_shade_pipe;
        if trace.is_null() || shade.is_null() {
            return;
        }
        let [w, h, _, _] = self.lighting.dims;
        let pixels = u64::from(w).saturating_mul(u64::from(h));
        let args = (lighting_level0_counts_at(pixels) + 2).saturating_mul(4);
        // SAFETY: the command buffer and both pipelines are the lighting
        // frame's live handles. Insert has already claimed this frame's
        // voxels and the seat pass listed them. Publish has not run. The
        // command is three words inside the key buffer.
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, trace);
            (self.fns.cmd_dispatch_indirect)(self.cmd, self.lighting.cache_keys.buffer, args);
            self.memory_barrier(0x800, 0x800 | 0x2, 0x40, 0x20 | 0x40 | 0x1);
            (self.fns.cmd_bind_pipe)(self.cmd, 1, shade);
            (self.fns.cmd_dispatch_indirect)(self.cmd, self.lighting.cache_keys.buffer, args);
            self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
        }
    }

    fn stamp_cascade(&mut self, query: u32) {
        if self.lighting.cascade_qp.is_null() || self.timestamp_period <= 0.0 {
            return;
        }
        unsafe {
            (self.fns.cmd_write_timestamp)(self.cmd, 0x800, self.lighting.cascade_qp, query);
        }
    }

    fn write_cascade_upper_header(&mut self, w: u32, h: u32, period: u32) {
        let pixels = u64::from(w).saturating_mul(u64::from(h));
        let claimed = lighting_upper_claimed_at(pixels);
        let head = claimed + CASCADE_CLAIMED_WORDS;
        if self.lighting.cache_keys.buffer.is_null() {
            return;
        }
        // The claim bits are not cleared here: pick consumes them, so a claim
        // a ray hit made after last frame's pick still keeps its cell.
        let list_bytes = (CASCADE_UPPER_HEAD_WORDS + CASCADE_UPPER_BANDS * CASCADE_UPPER_CAP)
            .saturating_mul(4);
        unsafe {
            (self.fns.cmd_fill_buffer)(
                self.cmd,
                self.lighting.cache_keys.buffer,
                head.saturating_mul(4),
                list_bytes,
                0,
            );
        }
        self.memory_barrier(0x1000, 0x1000, 0x1000, 0x1000);
        let flags = period & 255;
        // Flags, three counts, then (groups, 1, 1) per band. Pick raises groups.
        let words = [flags, 0, 0, 0, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 0, 0];
        unsafe {
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.lighting.cache_keys.buffer,
                head.saturating_mul(4),
                64,
                words.as_ptr().cast::<std::ffi::c_void>(),
            );
        }
        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20);
    }

    fn set_cascade_upper_band(&mut self, w: u32, h: u32, band: u32, period: u32) {
        let pixels = u64::from(w).saturating_mul(u64::from(h));
        let head = lighting_upper_claimed_at(pixels) + CASCADE_CLAIMED_WORDS;
        let flags = (period & 255) | ((band & 3) << 16);
        let words = [flags];
        unsafe {
            (self.fns.cmd_update_buffer)(
                self.cmd,
                self.lighting.cache_keys.buffer,
                head.saturating_mul(4),
                4,
                words.as_ptr().cast::<std::ffi::c_void>(),
            );
        }
        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20);
    }

    /// Claim ancestors, pick this frame's due upper cells, then refresh
    /// coarse to fine so level 0 can read this frame's parent SH.
    fn dispatch_cascade_upper(&mut self) {
        let claim = self.lighting.cascade_upper_claim_pipe;
        let pick = self.lighting.cascade_upper_pick_pipe;
        let trace = self.lighting.cascade_upper_trace_pipe;
        let shade = self.lighting.cascade_upper_shade_pipe;
        if claim.is_null() || pick.is_null() || trace.is_null() || shade.is_null() {
            return;
        }
        let [w, h, _, _] = self.lighting.dims;
        let period = crate::cascade::refresh_period(
            self.lighting.settings.world_cache.cascade_refresh_period,
        );
        self.lighting.cascade_times.period = period;
        if !self.lighting.cascade_qp.is_null() && self.timestamp_period > 0.0 {
            unsafe {
                (self.fns.cmd_reset_query)(
                    self.cmd,
                    self.lighting.cascade_qp,
                    0,
                    CASCADE_TIME_QUERIES,
                );
            }
        }
        self.write_cascade_upper_header(w, h, period);
        self.stamp_cascade(0);
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, claim);
            (self.fns.cmd_dispatch)(self.cmd, CASCADE_LEVEL0_GROUPS.div_ceil(64), 1, 1);
            self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
            (self.fns.cmd_bind_pipe)(self.cmd, 1, pick);
            let slots = u32::try_from(LIGHTING_CACHE_SLOTS).unwrap_or(1);
            (self.fns.cmd_dispatch)(self.cmd, slots.div_ceil(64), 1, 1);
            self.memory_barrier(0x800, 0x800 | 0x2, 0x40, 0x20 | 0x40 | 0x1);
        }
        self.stamp_cascade(1);
        let head = lighting_upper_claimed_at(u64::from(w).saturating_mul(u64::from(h)))
            + CASCADE_CLAIMED_WORDS;
        // Bands coarse (2) to fine (0) so a parent is refreshed before its child.
        for (i, band) in [2u32, 1, 0].into_iter().enumerate() {
            self.set_cascade_upper_band(w, h, band, period);
            let args = (head + 4 + u64::from(band) * 3).saturating_mul(4);
            // SAFETY: the command is three words of the header this
            // function wrote, raised by the pick dispatch above.
            unsafe {
                (self.fns.cmd_bind_pipe)(self.cmd, 1, trace);
                (self.fns.cmd_dispatch_indirect)(self.cmd, self.lighting.cache_keys.buffer, args);
                self.memory_barrier(0x800, 0x800 | 0x2, 0x40, 0x20 | 0x40 | 0x1);
                (self.fns.cmd_bind_pipe)(self.cmd, 1, shade);
                (self.fns.cmd_dispatch_indirect)(self.cmd, self.lighting.cache_keys.buffer, args);
                self.memory_barrier(0x800, 0x800 | 0x2, 0x40, 0x20 | 0x40 | 0x1);
            }
            self.stamp_cascade(2 + u32::try_from(i).unwrap_or(0));
        }
    }

    pub(crate) fn lighting_cascade_times(&mut self) -> crate::CascadeTimes {
        self.idle_device();
        self.read_cascade_times();
        self.lighting.cascade_times
    }

    fn read_cascade_times(&mut self) {
        if self.lighting.cascade_qp.is_null() || self.timestamp_period <= 0.0 {
            return;
        }
        let mut raw = [0u64; 6];
        let got = unsafe {
            (self.fns.get_query_results)(
                self.device,
                self.lighting.cascade_qp,
                0,
                CASCADE_TIME_QUERIES,
                48,
                raw.as_mut_ptr().cast::<std::ffi::c_void>(),
                8,
                0x1,
            )
        };
        if got != 0 {
            return;
        }
        let period = f64::from(self.timestamp_period);
        let bits = self.timestamp_bits;
        let ms = |a: u64, b: u64| tick_delta(a, b, bits) as f64 * period / 1.0e6;
        let times = &mut self.lighting.cascade_times;
        times.claim_ms = ms(raw[0], raw[1]);
        times.upper_coarse_ms = ms(raw[1], raw[2]);
        times.upper_mid_ms = ms(raw[2], raw[3]);
        times.upper_fine_ms = ms(raw[3], raw[4]);
        times.l0_ms = ms(raw[4], raw[5]);
        if genos_scene::lighting_env_flag("GENOS_LIGHTING_CASCADE_TIMES") {
            eprintln!(
                "cascade times period {} claim {:.3} ms coarse {:.3} mid {:.3} fine {:.3} l0 {:.3} upper {:.3}",
                times.period,
                times.claim_ms,
                times.upper_coarse_ms,
                times.upper_mid_ms,
                times.upper_fine_ms,
                times.l0_ms,
                times.upper_ms(),
            );
        }
    }

    /// Clear the distance bins, run maintenance, then gather what it enqueued.
    /// The sun path and the compose-writes kill switch leave this empty.
    /// The gather stays on the cache timestamp, not in compose.
    fn dispatch_sh_prep(&mut self, sun_compose: bool, compose_full: bool, gx: u32, gy: u32) {
        self.lighting.maint_ran = false;
        let maint = self.lighting.compose_maint_pipe;
        let drain = self.lighting.sh_drain_pipe;
        if sun_compose || maint.is_null() || drain.is_null() {
            return;
        }
        // Publish reads the lattice centre, or the cell on a miss. The colour
        // this walk produces is thrown away. The drain fills the centre the
        // publish pass reads. A camera move does not rebuild the lattice.
        // The opening, the 32-frame refresh, and a mover's reach still request
        // it. A held camera with nothing local still returns, which is the
        // still-idle probe.
        let refresh = self.lighting.frame != 0
            && self.lighting.frame.is_multiple_of(LIGHTING_STILL_REFRESH);
        let opening = self.lighting.frame != 0 && self.lighting.frame < LIGHTING_FILL_WALK;
        let bounce_filling = self.lighting.bounce_fill < BOUNCE_FILL_FRAMES;
        let blocks = u64::from(gx) * u64::from(gy);
        let lattice = bounce_filling
            || opening
            || refresh
            || !self.lighting.compose_valid
            || self.lighting.settings.world_mode() != 3
            || !self.lighting.settings.compose_reuse
            || !self.lighting.settings.gi.enabled
            || blocks > LIGHTING_COMPOSE_CAP
            || (self.particle_stamped && !self.lighting.hdr.legacy_bytes)
            || self.lighting.reach_arm != 0;
        if !lattice {
            return;
        }
        self.lighting.maint_ran = true;
        const BINS: u64 = 16;
        const LATTICE_CAP: u32 = 16384;
        // SAFETY: the command buffer, the key buffer, and both pipelines are the
        // lighting frame's live handles. The fill, the dispatches, and the
        // barriers are the same calls the compose loop already records.
        unsafe {
            (self.fns.cmd_fill_buffer)(
                self.cmd,
                self.lighting.cache_keys.buffer,
                lighting_sh_bin_count_at() * 4,
                BINS * 4,
                0,
            );
            self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
            (self.fns.cmd_bind_pipe)(self.cmd, 1, maint);
            if compose_full {
                (self.fns.cmd_dispatch)(self.cmd, gx.max(1), gy.max(1), 1);
            } else {
                (self.fns.cmd_dispatch_indirect)(
                    self.cmd,
                    self.lighting.cache_keys.buffer,
                    lighting_compose_at() * 4,
                );
            }
            self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
            (self.fns.cmd_bind_pipe)(self.cmd, 1, drain);
            (self.fns.cmd_dispatch)(self.cmd, LATTICE_CAP / 64, 1, 1);
        }
    }

    /// Reservoir shade, the ReSTIR module, or the pre-reservoir sum.
    fn direct_pipe(&self) -> Handle {
        if self.lighting.restir_frame && !self.lighting.restir_direct.is_null() {
            return self.lighting.restir_direct;
        }
        self.lighting.pipes[3]
    }

    /// The fine-band distances, for the cache shaders.
    fn write_fine_lod(&mut self, cmd: Handle) -> Result<(), String> {
        let cache = self.lighting.settings.surface_cache;
        let words = [
            cache.fine_lod_create.to_bits(),
            cache.fine_lod_drop.to_bits(),
            cache.fine_lod_lookahead.to_bits(),
        ];
        self.write_device(
            cmd,
            self.lighting.cache_keys.buffer,
            LIGHTING_FINE_WORD_AT * 4,
            pod_bytes(&words),
        )
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
            self.begin_reused(cmd)?;
            let qp = self.lighting.world_qp;
            if !qp.is_null() {
                (self.fns.cmd_reset_query)(cmd, qp, 0, 6);
                (self.fns.cmd_write_timestamp)(cmd, 0x1, qp, 0);
            }
            (self.fns.cmd_fill_buffer)(cmd, self.lighting.world_stats.buffer, 0, LIGHTING_STATS_BYTES, 0);
            self.barrier_on(cmd, 0x1000, 0x1000, 0x1000, 0x1000);
            let ctrl = self.lighting.ctrl_bytes;
            let dst = self.lighting.world_stats.buffer;
            self.write_device(cmd, dst, LIGHTING_CTRL_WORD * 4, &ctrl)?;
            // The mature and young counts, rebuilt by the compact pass.
            (self.fns.cmd_fill_buffer)(cmd, self.lighting.cache_keys.buffer, LIGHTING_CACHE_SLOTS * 4, 4, 0);
            // The WRC refresh lists' counts (change, background; lighting_cache.glsl).
            (self.fns.cmd_fill_buffer)(cmd, self.lighting.cache_keys.buffer, LIGHTING_WRC_COUNTS_AT * 4, 8, 0);
            (self.fns.cmd_fill_buffer)(cmd, self.lighting.cache_keys.buffer, (LIGHTING_WRC_COUNTS_AT + 4) * 4, 8, 0);
            (self.fns.cmd_fill_buffer)(cmd, self.lighting.cache_keys.buffer, LIGHTING_FINE_COUNT_AT * 4, 4, 0);
            self.write_fine_lod(cmd)?;
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
                let pipe = if pass == 3 {
                    self.direct_pipe()
                } else {
                    self.lighting.pipes[pass]
                };
                (self.fns.cmd_bind_pipe)(cmd, 1, pipe);
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
                let hist = crate::shown_age::AgeHist::from_diag(&w[24..88]);
                eprintln!("{}", crate::shown_age::line(queue, self.lighting.frame, &hist));
                eprintln!("lightingdg {queue} frame {} d {:?}", self.lighting.frame, &w[24..88]);
                // diag index is stats word 24 + index. See LIGHTING_RESET_* in lighting_cache.glsl.
                eprintln!(
                    "lightingreset {queue} frame {} live {} floor {} burst {} mover_rays {} traced {} leash {} coarsen {} age {} evict {} adopt {} decay {} reproject {} direct_miss {} mean {} fade {} restir {} mover {} pool {}",
                    self.lighting.frame,
                    w[2],
                    w[17],
                    w[19],
                    w[24 + 51],
                    w[16],
                    w[24 + 42],
                    w[24 + 43],
                    w[24 + 44],
                    w[24 + 45],
                    w[24 + 46],
                    w[24 + 47],
                    w[24 + 4],
                    w[24 + 10],
                    w[24 + 49],
                    w[24 + 50],
                    w[24 + 7],
                    w[24 + 9],
                    0
                );
                eprintln!("lightingdrain {queue} frame {} sample {} held {} draining {}", self.lighting.frame, w[LIGHTING_DRAIN_WORD], w[LIGHTING_DRAIN_WORD + 1], w[LIGHTING_DRAIN_WORD + 2] != 0);
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
                let heavy = w[84];
                let walks = w[24];
                let reach = w[32];
                let tiles = reach & 0xffff;
                let hot = (reach >> 16) & 0xfff;
                let corners = reach >> 28;
                eprintln!(
                    "lightingdenoise {queue} frame {} pixels {fine} coarse {coarse} changed {changed} filtered {fine} heavy {heavy} walks {walks} reach {tiles} hot {hot} corners {corners} mode screen ran 1",
                    self.lighting.frame
                );
                self.write_denoise_log(fine, coarse, changed, heavy, walks, tiles, hot, corners);
            }
        }
    }

    /// `./genos-debug/denoise/` (or `<out>/denoise` when `--out` is set). Written
    /// only from the stats read the debug line already performed.
    fn write_denoise_log(
        &self,
        fine: u32,
        coarse: u32,
        changed: u32,
        heavy: u32,
        walks: u32,
        tiles: u32,
        hot: u32,
        corners: u32,
    ) {
        let dir = std::path::Path::new(&self.debug_out).join("denoise");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let text = format!(
            "frame {}\npixels {fine}\ncoarse {coarse}\nchanged {changed}\nfiltered {fine}\nheavy {heavy}\nwalks {walks}\nreach {tiles}\nhot {hot}\ncorners {corners}\nmode screen\nran 1\n",
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

    /// Header words 43, 44, and 45, copied to the tail of this slot's stats
    /// readback. Three zeros when the copy did not run.
    fn read_restir_diag(&self, memory: Handle) -> [u32; 3] {
        if memory.is_null() {
            return [0; 3];
        }
        let mut words = [0u32; 3];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            if check(
                (self.fns.map_mem)(
                    self.device,
                    memory,
                    restir_diag_tail_offset(),
                    RESTIR_DIAG_TAIL_BYTES,
                    0,
                    &mut mapped,
                ),
                "map restir diag",
            )
            .is_err()
            {
                return [0; 3];
            }
            std::ptr::copy_nonoverlapping(mapped.cast::<u32>(), words.as_mut_ptr(), 3);
            (self.fns.unmap_mem)(self.device, memory);
        }
        words
    }

    /// One word of a stats ring buffer whose fence was already waited.
    fn read_stats_word(&self, memory: Handle, word: usize) -> Result<u32, String> {
        let value;
        unsafe {
            let mut mapped = std::ptr::null_mut();
            check(
                (self.fns.map_mem)(self.device, memory, (word * 4) as u64, 4, 0, &mut mapped),
                "map lighting stats word",
            )?;
            value = std::ptr::read_unaligned(mapped as *const u32);
            (self.fns.unmap_mem)(self.device, memory);
        }
        Ok(value)
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
        desired.apply_env()?;
        desired.sanitize();
        cone_effective(&mut desired);
        let debug = scene.debug.effective(self.out_cli.as_deref());
        self.debug_out = debug.out;
        let async_on = desired.experiments.cache_async && !self.lighting.async_failed;
        if self.lighting.async_on && !async_on {
            // Let the round in flight finish before its results stop being read.
            self.wait_lighting_world();
        }
        self.lighting.async_on = async_on;
        self.lighting.settings = desired;
        // A look transition blends ev100, compensation and the bloom intensity
        // toward the scene's values. With transition 0 this is the scene's value.
        let (hdr, look) =
            crate::hdr_output::step_look(self.lighting.hdr_look, &scene.hdr.effective()?, f64::from(self.frame_dt));
        self.lighting.hdr = hdr;
        self.lighting.hdr_look = Some(look);
        self.lighting.sun_height = sun_height(scene);
        // The first swapchain is built before a scene exists. A listed HDR pair
        // replaces it once the settings ask for that pair.
        self.refresh_hdr_surface()?;
        let key = lighting_alloc_key(&self.lighting.settings);
        let resized = self.lighting.dims[0] != self.extent_w || self.lighting.dims[1] != self.extent_h;
        let encode_full = self.post_encode_full();
        let probe = self.lighting.bloom_probe && !encode_full;
        let post_sig = crate::hdr_output::post_signature(&self.lighting.hdr, encode_full, probe);
        if (key != self.lighting.alloc_key || resized || post_sig != self.lighting.post_sig)
            && self.lighting.ready
        {
            self.make_lighting_targets()?;
        }
        Ok(())
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
    )]
    /// Exposure scale for this frame. Legacy bytes stay at 1. Auto, fixed, and
    /// camera call the `hdr_output` references.
    fn hdr_exposure_scale(&mut self) -> f32 {
        let settings = self.lighting.hdr.clone();
        if settings.legacy_bytes {
            self.lighting.last_ev100 = 0.0;
            return 1.0;
        }
        let ev = match settings.exposure_mode {
            genos_scene::ExposureMode::Fixed => settings.ev100,
            genos_scene::ExposureMode::Camera => {
                crate::hdr_output::camera_ev100(settings.aperture, settings.shutter, settings.iso)
            }
            genos_scene::ExposureMode::Auto => self.lighting.meter_ev.unwrap_or(self.lighting.last_ev100),
        };
        self.lighting.last_ev100 = ev;
        let target = crate::hdr_output::exposure_with_compensation(ev, settings.compensation);
        if settings.exposure_mode == genos_scene::ExposureMode::Auto && self.lighting.exposure_cut > 0 {
            let left = self.lighting.exposure_cut;
            if left == crate::hdr_output::EXPOSURE_CUT_FRAMES {
                self.lighting.exposure_cut_from = f64::from(self.lighting.exposure_scale);
            }
            self.lighting.exposure_cut = left.saturating_sub(1);
            return crate::hdr_output::pack_f32(crate::hdr_output::cut_exposure(
                left,
                self.lighting.exposure_cut_from,
                target,
            ));
        }
        let next = crate::hdr_output::step_exposure(
            f64::from(self.lighting.exposure_scale),
            target,
            f64::from(self.frame_dt),
            settings.adapt_down,
            settings.adapt_up,
        );
        crate::hdr_output::pack_f32(next)
    }

    /// Header from this flight's previous meter. That slot's fence has waited.
    /// The other flight may still be copying its own host buffer.
    fn take_meter(&mut self) {
        let slot = self.flight;
        if !self.lighting.meter_pending[slot] || self.lighting.meter_host[slot].memory.is_null() {
            return;
        }
        self.lighting.meter_pending[slot] = false;
        let bins = usize::try_from(self.lighting.meter_bins).unwrap_or(0);
        if bins == 0 {
            return;
        }
        let words = bins.saturating_add(4);
        let bytes = u64::try_from(words.saturating_mul(4)).unwrap_or(0);
        if bytes == 0 || self.lighting.meter_host[slot].size < bytes {
            return;
        }
        let mut raw = vec![0_u32; words];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            if (self.fns.map_mem)(
                self.device,
                self.lighting.meter_host[slot].memory,
                0,
                bytes,
                0,
                &mut mapped,
            ) != 0
            {
                return;
            }
            std::ptr::copy_nonoverlapping(mapped.cast::<u32>(), raw.as_mut_ptr(), words);
            (self.fns.unmap_mem)(self.device, self.lighting.meter_host[slot].memory);
        }
        let had_prev = self.lighting.meter_has_prev[slot];
        let moved = raw.get(bins).copied().unwrap_or(0);
        let total = raw.get(bins.saturating_add(1)).copied().unwrap_or(0);
        if had_prev {
            self.lighting.still_pop = if total == 0 {
                1.0
            } else {
                f64::from(moved) / f64::from(total)
            };
        } else {
            self.lighting.still_pop = 0.0;
        }
        self.lighting.meter_has_prev[slot] = true;
        if self.lighting.hdr.exposure_mode != genos_scene::ExposureMode::Auto {
            return;
        }
        let counts = raw[..bins].to_vec();
        let occupied = counts
            .iter()
            .fold(0_u32, |acc, count| acc.saturating_add(*count));
        if occupied == 0 {
            return;
        }
        let hdr = &self.lighting.hdr;
        self.lighting.meter_ev = Some(crate::hdr_output::auto_ev100(
            &counts,
            hdr.adapt_strength,
            hdr.highlight_percentile,
            hdr.highlight_stops,
        ));
        self.write_histogram_dump(&counts);
        self.lighting.histogram_counts = counts;
    }

    fn write_histogram_dump(&self, counts: &[u32]) {
        if self.debug_out.is_empty() || counts.is_empty() {
            return;
        }
        let dir = std::path::Path::new(&self.debug_out).join("hdr-output");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let mut body = String::new();
        for count in counts {
            body.push_str(&count.to_string());
            body.push('\n');
        }
        let _ = std::fs::write(dir.join("histogram.txt"), body);
    }

    /// Start (`begin`) or end of bloom plus the display pass.
    /// The query index stays on the start so the next frame can read the pair.
    fn stamp_hdr_time(&mut self, slot: usize, begin: bool) {
        if self.hdr_time_pool.is_null() {
            self.lighting.hdr_query[slot] = u32::MAX;
            return;
        }
        let base = u32::try_from(slot).unwrap_or(0).saturating_mul(2);
        let query = if begin { base } else { base.saturating_add(1) };
        unsafe {
            if begin {
                (self.fns.cmd_reset_query)(self.cmd, self.hdr_time_pool, base, 2);
            }
            (self.fns.cmd_write_timestamp)(self.cmd, 0x800, self.hdr_time_pool, query);
        }
        if begin {
            self.lighting.hdr_query[slot] = base;
        }
    }

    fn bloom_active(&self) -> bool {
        let hdr = &self.lighting.hdr;
        !hdr.legacy_bytes && hdr.bloom && hdr.bloom_intensity > 0.0 && hdr.bloom_levels > 0
    }

    /// Same-frame pyramid into the bloom buffer. Compose has stored `scene_lin`.
    /// The intensity word is updated after the chain so the display pass adds it.
    /// Upsamples write the finer mip in place. The display pass tents that mip to full resolution.
    fn record_gpu_bloom(&mut self, width: u32, height: u32) -> Result<(), String> {
        if self.lighting.bloom_pipe.is_null() || width == 0 || height == 0 {
            return Ok(());
        }
        let levels = crate::hdr_output::bloom_level_count(&self.lighting.hdr);
        if levels == 0 {
            return Ok(());
        }
        let mips = crate::hdr_output::bloom_mips(width, height, levels);
        if mips.bytes > self.lighting.bloom.size {
            eprintln!("lighting: bloom chain does not fit");
            return Ok(());
        }
        let Some(first) = mips.levels.first() else {
            return Ok(());
        };
        let modes = crate::hdr_output::bloom_modes(levels);
        let Some(first_mode) = modes.first().copied() else {
            return Ok(());
        };
        self.bloom_dispatch(
            first.width,
            first.height,
            width,
            height,
            0,
            first.word,
            first_mode,
            0,
        );
        for index in 1..mips.levels.len() {
            let Some(src) = mips.levels.get(index - 1) else {
                return Ok(());
            };
            let Some(dst) = mips.levels.get(index) else {
                return Ok(());
            };
            let mode = modes.get(index).copied().unwrap_or(1);
            self.bloom_dispatch(
                dst.width,
                dst.height,
                src.width,
                src.height,
                src.word,
                dst.word,
                mode,
                0,
            );
        }
        let count = mips.levels.len();
        if count > 1 {
            let mut src_index = count - 1;
            let mut mode_index = count;
            for finer in (0..count - 1).rev() {
                let Some(src) = mips.levels.get(src_index) else {
                    return Ok(());
                };
                let Some(dst) = mips.levels.get(finer) else {
                    return Ok(());
                };
                let mode = modes.get(mode_index).copied().unwrap_or(2);
                self.bloom_dispatch(
                    dst.width,
                    dst.height,
                    src.width,
                    src.height,
                    src.word,
                    dst.word,
                    mode,
                    dst.word,
                );
                src_index = finer;
                mode_index = mode_index.saturating_add(1);
            }
        }
        let word = crate::hdr_output::pack_f32(self.lighting.hdr.bloom_intensity);
        let cmd = self.cmd;
        let dst = self.lighting.hdr_params.buffer;
        self.write_device(cmd, dst, 8, &word.to_ne_bytes())?;
        // Display reads the chain and the new intensity. Host reads the full image after the fence.
        self.memory_barrier(0x800 | 0x1000, 0x800 | 0x4000, 0x40 | 0x1000, 0x20 | 0x2000);
        Ok(())
    }

    fn bloom_dispatch(
        &self,
        dst_w: u32,
        dst_h: u32,
        src_w: u32,
        src_h: u32,
        src: u32,
        dst: u32,
        mode: u32,
        finer: u32,
    ) {
        let pc = [
            dst_w, dst_h, src_w, src_h, src, dst, mode, finer, 0, 0, 0, 0,
        ];
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, self.lighting.bloom_pipe);
            (self.fns.cmd_push)(
                self.cmd,
                self.lighting.layout,
                0x20,
                0,
                LIGHTING_PC_BYTES,
                pc.as_ptr() as *const c_void,
            );
            let texels = u64::from(dst_w).saturating_mul(u64::from(dst_h));
            // Word 0 is the RGBA16F mip: one thread per texel, 8×8. Later mips
            // pack two texels per thread; a group is still 64 invocations.
            if dst == 0 {
                (self.fns.cmd_dispatch)(
                    self.cmd,
                    dst_w.div_ceil(8).max(1),
                    dst_h.div_ceil(8).max(1),
                    1,
                );
            } else {
                let pairs = u32::try_from(texels.div_ceil(2)).unwrap_or(1);
                (self.fns.cmd_dispatch)(self.cmd, pairs.div_ceil(64).max(1), 1, 1);
            }
        }
        self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
    }

    /// Host view of the encode buffer: RGBA32F unpacked from RGBA16F PQ or scRGB.
    fn read_hdr_encode_host(&self) -> Result<Vec<f32>, String> {
        if self.lighting.bloom_probe && self.hdr_output_word() == 0.0 {
            return Ok(Vec::new());
        }
        let width = usize::try_from(self.extent_w).unwrap_or(0);
        let height = usize::try_from(self.extent_h).unwrap_or(0);
        self.read_fp16_region(0, width.saturating_mul(height))
    }

    /// Bloom tent the display pass added, before the intensity scale. RGBA32F.
    fn read_bloom_probe(&self) -> Result<Vec<f32>, String> {
        let width = usize::try_from(self.extent_w).unwrap_or(0);
        let height = usize::try_from(self.extent_h).unwrap_or(0);
        self.read_fp16_region(0, width.saturating_mul(height))
    }

    /// `agx_eval` at the first 64 lattice inputs. RGBA32F, one sample per texel.
    fn read_agx_probe(&self) -> Result<Vec<f32>, String> {
        let width = usize::try_from(self.extent_w).unwrap_or(0);
        let height = usize::try_from(self.extent_h).unwrap_or(0);
        let pixels = width.saturating_mul(height);
        self.read_fp16_region(pixels, 64)
    }

    /// Unpack `count` RGBA16F texels starting `texel` texels into the encode buffer.
    fn read_fp16_region(&self, texel: usize, count: usize) -> Result<Vec<f32>, String> {
        if count == 0 || self.lighting.hdr_encode.memory.is_null() {
            return Ok(Vec::new());
        }
        let bytes = texel.saturating_add(count).saturating_mul(8);
        if self.lighting.hdr_encode.size < u64::try_from(bytes).unwrap_or(u64::MAX) {
            return Ok(Vec::new());
        }
        let mut raw = vec![0u8; bytes];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            if (self.fns.map_mem)(
                self.device,
                self.lighting.hdr_encode.memory,
                0,
                self.lighting.hdr_encode.size,
                0,
                &mut mapped,
            ) != 0
            {
                return Ok(Vec::new());
            }
            std::ptr::copy_nonoverlapping(mapped.cast::<u8>(), raw.as_mut_ptr(), bytes);
            (self.fns.unmap_mem)(self.device, self.lighting.hdr_encode.memory);
        }
        let start = texel.saturating_mul(8);
        let end = start.saturating_add(count.saturating_mul(8));
        let slice = raw.get(start..end).unwrap_or(&[]);
        if slice.len() < count.saturating_mul(8) {
            return Ok(Vec::new());
        }
        Ok(crate::hdr_output::rgba_f32_from_rgba16f(slice, count))
    }

    /// Full-resolution bloom, reconstructed by tent-upsampling the finest stored mip.
    fn read_bloom_host(&self) -> Result<Vec<f32>, String> {
        let levels = crate::hdr_output::bloom_level_count(&self.lighting.hdr);
        if levels == 0 || self.lighting.bloom.memory.is_null() || self.lighting.bloom.size == 0 {
            return Ok(Vec::new());
        }
        let size = usize::try_from(self.lighting.bloom.size).unwrap_or(0);
        if size == 0 {
            return Ok(Vec::new());
        }
        let mut raw = vec![0u8; size];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            if (self.fns.map_mem)(
                self.device,
                self.lighting.bloom.memory,
                0,
                self.lighting.bloom.size,
                0,
                &mut mapped,
            ) != 0
            {
                return Ok(Vec::new());
            }
            std::ptr::copy_nonoverlapping(mapped.cast::<u8>(), raw.as_mut_ptr(), size);
            (self.fns.unmap_mem)(self.device, self.lighting.bloom.memory);
        }
        Ok(crate::hdr_output::bloom_rgba_from_chain(
            &raw,
            self.extent_w,
            self.extent_h,
            levels,
        ))
    }

    /// True when the encode side buffer is the full picture. SDR does not allocate one.
    fn post_encode_full(&self) -> bool {
        self.hdr_output_word() > 0.0
    }

    /// Empty post bindings name the params buffer. That buffer is already in the total.
    fn bound_or_standin(&self, buffer: &Buffer) -> Handle {
        if buffer.buffer.is_null() {
            self.lighting.hdr_params.buffer
        } else {
            buffer.buffer
        }
    }

    /// Fallback and Low stay on SDR. Low on a Deck OLED may take listed HDR10.
    /// A higher preset with bloom off is not Fallback: the encode grade uses that shape.
    fn present_locks_sdr(&self, hdr10_listed: bool) -> bool {
        if self.lighting.hdr.legacy_bytes {
            return true;
        }
        let preset = self.lighting.settings.preset;
        let named = matches!(
            preset,
            genos_scene::LightingPreset::Fallback | genos_scene::LightingPreset::Low
        );
        named
            && crate::hdr_output::post_locks_sdr(
                &self.lighting.hdr,
                preset,
                crate::hdr_output::deck_panel(),
                hdr10_listed,
            )
    }

    /// 0 keeps the 8-bit picture. 1 is HDR10. 2 is scRGB. Forced SDR stays 0.
    /// A requested mode that the surface did not list still returns 1 or 2 so
    /// the side buffer can be read back.
    fn hdr_output_word(&self) -> f32 {
        let force_sdr = if self.lighting.budget_hdr {
            crate::hdr_output::post_locks_sdr(
                &self.lighting.hdr,
                self.lighting.budget_preset,
                self.lighting.budget_panel,
                self.lighting.budget_hdr10_listed,
            ) || !matches!(
                self.lighting.hdr.output,
                genos_scene::OutputMode::Hdr10 | genos_scene::OutputMode::Scrgb
            )
        } else {
            self.present_locks_sdr(self.hdr_output == crate::hdr_output::HdrSwapchain::Hdr10)
        };        if force_sdr {
            return 0.0;
        }
        match self.hdr_output {
            crate::hdr_output::HdrSwapchain::Hdr10 => 1.0,
            crate::hdr_output::HdrSwapchain::Scrgb => 2.0,
            crate::hdr_output::HdrSwapchain::Sdr => match self.lighting.hdr.output {
                genos_scene::OutputMode::Hdr10 => 1.0,
                genos_scene::OutputMode::Scrgb => 2.0,
                genos_scene::OutputMode::Auto | genos_scene::OutputMode::Sdr => 0.0,
            },
        }
    }

    fn write_hdr_exposure(&mut self) {
        self.take_meter();
        if self.lighting.hdr_params.memory.is_null() {
            return;
        }
        let scale = self.hdr_exposure_scale();
        self.lighting.exposure_scale = scale;
        self.lighting.exposure_log.push(f64::from(scale));
        if self.lighting.exposure_log.len() > 8 {
            let extra = self.lighting.exposure_log.len() - 8;
            self.lighting.exposure_log.drain(0..extra);
        }
        // `legacy_bytes` forces the shoulder. Mode 2 is ACES, which is not the
        // CTL, so the shader keeps the shoulder for that mode too.
        let mode = if self.lighting.hdr.legacy_bytes {
            0.0
        } else {
            match self.lighting.hdr.display {
                genos_scene::DisplayTransform::Legacy => 0.0,
                genos_scene::DisplayTransform::Agx => 1.0,
                genos_scene::DisplayTransform::Aces => 2.0,
            }
        };
        // Bloom intensity stays 0 until the pyramid has run. The display pass
        // reads the updated word. Compose therefore does not add a stale buffer.
        // `w` is the encode: 0 SDR, 1 HDR10 PQ, 2 scRGB. Fallback, Low, and
        // `legacy_bytes` stay 0. An unlisted mode still encodes into the side
        // buffer; the swapchain image stays the SDR pair.
        let output = self.hdr_output_word();
        let white = crate::hdr_output::pack_f32(crate::hdr_output::graphics_white_linear());
        let peak = crate::hdr_output::pack_f32(crate::hdr_output::HDR_PEAK_NITS);
        let format_code = crate::hdr_output::swapchain_format_code(self.format);
        let mut divisor = if self.bloom_active() {
            f32::from(u16::try_from(crate::hdr_output::bloom_divisor(
                crate::hdr_output::bloom_level_count(&self.lighting.hdr),
            ))
            .unwrap_or(0))
        } else {
            0.0
        };
        // The picture gate reads the tent and the lattice from the encode buffer.
        if self.lighting.bloom_probe && output == 0.0 {
            divisor += 100.0;
        }
        let words = [scale, mode, 0.0_f32, output, white, peak, format_code, divisor];
        unsafe {
            let mut mapped = std::ptr::null_mut();
            if (self.fns.map_mem)(
                self.device,
                self.lighting.hdr_params.memory,
                0,
                32,
                0,
                &mut mapped,
            ) != 0
            {
                return;
            }
            std::ptr::copy_nonoverlapping(words.as_ptr(), mapped as *mut f32, 8);
            (self.fns.unmap_mem)(self.device, self.lighting.hdr_params.memory);
        }
        // Host-coherent upload, finished before submit. Compute (compose) and
        // fragment (output) read it. 0x20 is the tessellation-evaluation
        // stage, not a shader read (VUID-vkCmdPipelineBarrier-dstStageMask-04091).
        self.memory_barrier(0x4000, 0x800 | 0x80, 0x4000, 0x20);
    }

    /// Histogram of every pixel, and a short still check. The header is copied
    /// for the next frame. Luma halves stay on the device. Fixed exposure skips it.
    /// One buffer per frame in flight: both frames filling one buffer is a
    /// WRITE_AFTER_WRITE on mailbox.
    fn record_gpu_meter(&mut self, slot: usize, width: u32, height: u32) {
        if self.lighting.hdr.legacy_bytes
            || self.lighting.hdr.exposure_mode != genos_scene::ExposureMode::Auto
            || self.lighting.meter_pipe.is_null()
            || self.lighting.meter_dev[slot].buffer.is_null()
            || self.lighting.meter_host[slot].buffer.is_null()
            || width == 0
            || height == 0
        {
            return;
        }
        let chain = crate::hdr_output::post_chain(width, height, &self.lighting.hdr, false);
        if chain.bins == 0 || chain.meter_bytes > self.lighting.meter_dev[slot].size {
            return;
        }
        let header = u64::from(chain.bins.saturating_add(4)).saturating_mul(4);
        if header == 0 || self.lighting.meter_host[slot].size < header {
            return;
        }
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        if pixels == 0 {
            return;
        }
        unsafe {
            (self.fns.cmd_fill_buffer)(
                self.cmd,
                self.lighting.meter_dev[slot].buffer,
                0,
                header,
                0,
            );
        }
        self.memory_barrier(0x800 | 0x1000, 0x800, 0x40 | 0x1000, 0x20 | 0x40);
        let stride = if chain.slots == 0 {
            0
        } else {
            u32::try_from(pixels.div_ceil(u64::from(chain.slots))).unwrap_or(u32::MAX)
        };
        let pc = [
            width,
            height,
            chain.bins,
            chain.slots,
            u32::from(self.lighting.meter_has_prev[slot]),
            self.lighting.meter_ping[slot] & 1,
            stride,
            1,
            chain.bins.saturating_add(4),
            0,
            0,
            0,
        ];
        let groups = u32::try_from(pixels.div_ceil(64)).unwrap_or(u32::MAX).max(1);
        unsafe {
            (self.fns.cmd_bind_pipe)(self.cmd, 1, self.lighting.meter_pipe);
            (self.fns.cmd_push)(
                self.cmd,
                self.lighting.layout,
                0x20,
                0,
                LIGHTING_PC_BYTES,
                pc.as_ptr().cast::<c_void>(),
            );
            (self.fns.cmd_dispatch)(self.cmd, groups, 1, 1);
        }
        self.memory_barrier(0x800, 0x1000, 0x40, 0x1000);
        #[repr(C)]
        struct Region {
            src: u64,
            dst: u64,
            size: u64,
        }
        let region = Region {
            src: 0,
            dst: 0,
            size: header,
        };
        unsafe {
            (self.fns.cmd_copy_buffer)(
                self.cmd,
                self.lighting.meter_dev[slot].buffer,
                self.lighting.meter_host[slot].buffer,
                1,
                &region as *const Region as *const u8,
            );
        }
        self.lighting.meter_bins = chain.bins;
        self.lighting.meter_pending[slot] = true;
        self.lighting.meter_ping[slot] ^= 1;
    }

    /// Upload the light tree and the point lamps. Reservoirs stay on the device.
    /// The product gate is the only shade that reads them.
    fn prepare_restir_di(
        &mut self,
        width: u32,
        height: u32,
        changes: Option<&Vec<[f32; 4]>>,
    ) -> bool {
        self.lighting.restir_frame = false;
        if !restir_product_shade(
            self.lighting.settings.restir_di,
            self.lighting.settings.direct.enabled,
            self.lighting.shade_gate,
        )
            || width == 0
            || height == 0
        {
            if !self.lighting.settings.restir_di {
                self.lighting.restir_have_prev = false;
            }
            return false;
        }
        if self.lighting.restir_force_fail {
            self.lighting.restir_have_prev = false;
            eprintln!("lighting: restir prepare failed: the reservoir upload was forced off");
            return false;
        }
        let scale = self.lighting.settings.restir_resolution_scale;
        let (rw, rh) = crate::restir_live::reservoir_size(width, height, scale);
        if rw == 0 || rh == 0 {
            eprintln!("lighting: restir prepare failed: the pixel count does not fit");
            return false;
        }
        let reservoir_pixels = u64::from(rw).saturating_mul(u64::from(rh));
        if u32::try_from(reservoir_pixels).is_err() {
            eprintln!("lighting: restir prepare failed: the pixel count does not fit");
            return false;
        }
        let resized = self.lighting.restir_w != rw || self.lighting.restir_h != rh;
        if self.lighting.restir_direct.is_null() {
            let start = Instant::now();
            match self.make_compute_pipe_ids(
                LIGHTING_DIRECT_RESTIR_SPV,
                self.lighting.layout,
                "restir-direct",
                &Self::moving_lamp_wall_spec(),
            ) {
                Ok(pipe) => self.lighting.restir_direct = pipe,
                Err(err) => {
                    eprintln!("lighting: restir prepare failed: restir-direct pipeline: {err}");
                    return false;
                }
            }
            self.note_pipeline_time("restir-direct", start);
        }
        let need = crate::restir_live::buffer_bytes(reservoir_pixels);
        if self.lighting.restir_buf.size < need {
            eprintln!(
                "lighting: restir prepare failed: reservoir {} bytes does not cover {} bytes at {rw}x{rh} (screen {width}x{height})",
                self.lighting.restir_buf.size,
                need
            );
            self.lighting.restir_have_prev = false;
            return false;
        }
        let geom_reset = changes.is_none();
        let previous = if self.lighting.restir_have_prev {
            self.lighting.restir_prev
        } else {
            crate::restir_live::RestirCam::default()
        };
        let cost = crate::restir_live::RestirCost {
            width: rw,
            height: rh,
            spatial_neighbors: self.lighting.settings.restir_spatial_neighbors,
            spatial_radius: self.lighting.settings.restir_spatial_radius,
            resolution_scale: scale,
            checkerboard: self.lighting.settings.restir_spatial_checkerboard,
            candidates: self.lighting.settings.restir_candidates,
            split_passes: self.lighting.settings.restir_split_passes,
            screen_width: width,
            screen_height: height,
        };
        crate::restir_live::log_cost(rw, rh, &cost);
        let Some(mut prefix) = crate::restir_live::prefix(
            &self.lighting.restir_lamps,
            &self.lighting.restir_prev_lamps,
            &previous,
            self.lighting.frame,
            self.lighting.restir_slot,
            cost,
        ) else {
            eprintln!("lighting: restir prepare failed: the reservoir prefix does not fit");
            return false;
        };
        let reset = geom_reset || prefix.layout_changed || resized || !self.lighting.restir_have_prev;
        crate::restir_live::set_history_reset(&mut prefix.words, reset);
        self.lighting.restir_prev_lamps.clear();
        self.lighting
            .restir_prev_lamps
            .extend(self.lighting.restir_lamps.iter().map(copy_gpu_lamp));
        // One restir buffer, two frames in flight, and each frame waits only on
        // its own slot fence. Frame N still has compute reads and writes in
        // this buffer when frame N+1 records, and a transfer read of header
        // words 43-46 (the stats copy of history_accepts, history_rejects, and
        // restir_point_adds). On one queue a barrier's first scope is every
        // earlier command in submission order. Source is compute and transfer,
        // so that copy finishes before vkCmdUpdateBuffer overwrites those
        // words. A second scratch and prefix would not remove the history
        // dependency, and a barrier waits for the whole source stage either
        // way, so the extra buffers do not start the next frame sooner.
        // The world submit copies pc before bit 21 is set. Every restir_words
        // load sits under restir_pix, which requires that bit, so the async
        // pass does not touch this buffer.
        self.memory_barrier(0x800 | 0x1000, 0x1000, 0x20 | 0x40 | 0x800, 0x1000);
        if self.lighting.restir_clear {
            self.lighting.restir_clear = false;
            unsafe {
                (self.fns.cmd_fill_buffer)(
                    self.cmd,
                    self.lighting.restir_buf.buffer,
                    0,
                    self.lighting.restir_buf.size,
                    0,
                );
            }
            self.memory_barrier(0x1000, 0x1000, 0x1000, 0x1000);
        }
        let mut at = 0usize;
        while at < prefix.words.len() {
            let end = (at + 16384).min(prefix.words.len());
            let Ok(offset) = u64::try_from(at.saturating_mul(4)) else {
                return false;
            };
            let Ok(bytes) = u64::try_from(end.saturating_sub(at).saturating_mul(4)) else {
                return false;
            };
            unsafe {
                (self.fns.cmd_update_buffer)(
                    self.cmd,
                    self.lighting.restir_buf.buffer,
                    offset,
                    bytes,
                    prefix.words[at..end].as_ptr().cast::<std::ffi::c_void>(),
                );
            }
            at = end;
        }
        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20);
        self.lighting.restir_w = rw;
        self.lighting.restir_h = rh;
        let prefix_words = u64::try_from(prefix.words.len()).unwrap_or(u64::MAX);
        self.lighting.restir_frame = prefix_words <= crate::restir_live::PREFIX_CAP_WORDS
            && prefix.total_words >= prefix_words
            && crate::restir_live::buffer_bytes(0) <= self.lighting.restir_buf.size;
        self.lighting.restir_frame
    }


    /// The initial, temporal, and spatial passes are gone. The four stamps
    /// stay so a timing read still has a slot; the intervals are empty.
    /// Shade is `restir-direct`: the sun, the brightest lamp, and the cascade.
    fn dispatch_restir_di(&mut self, slot: usize, pc: &[u32; 12], pool: Handle, first: u32) {
        let _ = pc;
        let stamp = !pool.is_null();
        if !stamp {
            self.lighting.restir_query[slot] = u32::MAX;
        }
        let write_stamp = |this: &Self, at: u32| {
            if stamp {
                unsafe {
                    (this.fns.cmd_write_timestamp)(this.cmd, 0x800, pool, first + RESTIR_STAMP_AT + at);
                }
            }
        };
        if stamp {
            self.lighting.restir_query[slot] = first + RESTIR_STAMP_AT;
        }
        write_stamp(self, 0);
        write_stamp(self, 1);
        write_stamp(self, 2);
        write_stamp(self, 3);
        self.lighting.restir_prev = self.lighting.restir_view;
        self.lighting.restir_have_prev = true;
        self.lighting.restir_slot ^= 1;
    }

    fn lighting_passes(&mut self, slot: usize) -> Result<(), String> {
        self.lighting.hdr_query[slot] = u32::MAX;
        self.write_hdr_exposure();
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
            // Fog is not in the scene bytes. A header that appears or disappears
            // has to recompose the whole picture; a quiet reuse would keep the
            // previous frame's fog on the pixels it does not redraw.
            self.froxel_wrote.hash(&mut hasher);
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
        // Bounds of what moved in the traced world since the last frame.
        // A cache ray drops a stratum that crosses a mover. Direct light shades
        // its samples again each frame, so it does not read this list.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            self.lighting.trace_key.hash(&mut hasher);
            self.mesh_field_key.hash(&mut hasher);
            hasher.finish()
        };
        let frozen = self.lighting_stats().settled;
        let had = !self.lighting.kept_bytes.is_empty() && key == self.lighting.kept_key;
        let changes = if had {
            lighting_changes(&self.lighting.kept_bytes, &self.frame_bytes)
        } else {
            None
        };
        if lighting_debug_on() {
            if frozen {
                eprintln!("lighting frozen");
            } else {
                eprintln!(
                    "lighting kept had {had} changes {}",
                    changes.as_ref().map_or(-1, |c| c.len() as i64)
                );
            }
        }
        let mut block = vec![0f32; 4 * (LIGHTING_CHANGE_HEAD + LIGHTING_CHANGES)];
        // World irradiance cells and the surface-cache flags. Head z is the
        // probe-run bit plus twice the low flags. Head w is mode × 1000 + cap
        // until the mover count is added below. Head 3.w is written after the
        // last-camera copy, which shares that word.
        let world = self.lighting.settings.world_mode();
        let head = lighting_change_head(&self.lighting.settings);
        let flags = head.flags;
        block[2] = ((self.lighting.probe_runs & 1) + 2 * flags) as f32;
        block[3] = head.mode_cap as f32;
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
        // The shaders read this fraction as "the host hash changed". The
        // integer count is what lighting_near_change walks. A lamp does not
        // land in that list; the fraction is how the hold hears it this frame.
        // Occluder motion with the camera and the lamps held is not that mark.
        // The reach pass marks the tiles. A lamp stays picture-wide, this frame.
        // The drawn instances move with the solid, so the draw key changes too.
        // Fog and a traced mesh are not that solid: those stay picture-wide.
        let mover_local = self.lighting.still == 0
            && !self.lighting.kept_bytes.is_empty()
            && scene_mover_only(&self.lighting.kept_bytes, &self.frame_bytes)
            && self.froxel_wrote == self.lighting.kept_froxel
            && self.mesh_field_key == self.lighting.kept_mesh;
        self.lighting.mover_local = mover_local;
        let [fw, fh, _, _] = self.lighting.dims;
        let spheres = changes.as_deref().unwrap_or(&[]);
        // Behind the camera, on it, entirely off-screen, or already half the
        // tiles: not a local mask. The picture unholds.
        let footprint = mover_local && reach_footprint_unproven(&self.frame_bytes, spheres, fw, fh);
        let saturated = mover_local && reach_mask_saturated(self.lighting.reach_tiles, fw, fh);
        // A change that is not occluder-only, or a dropped change list, is
        // already the picture-wide path. The flag says so. The 16-frame catch
        // (shader LIGHTING_HOLD_CATCH) keeps the flag on.
        let picture = self.lighting.still == 0 && !mover_local;
        let dropped = self.lighting.still == 0 && changes.is_none();
        let catch_age = self.lighting.frame.saturating_sub(self.lighting.scene_catch);
        let catching = self.lighting.scene_catch != 0 && catch_age <= 16;
        let wide = self.lighting.picture_unhold || footprint || saturated || picture || dropped || catching;
        self.lighting.reach_wide = wide;
        if mover_local || self.lighting.reach_hot > 0 {
            self.lighting.reach_arm = 4;
        } else {
            self.lighting.reach_arm = self.lighting.reach_arm.saturating_sub(1);
        }
        if self.lighting.still == 0 && (!mover_local || wide) {
            block[0] += LIGHTING_SCENE_CHANGED_MARK;
            if self.lighting.light_scene != self.lighting.light_scene_seen {
                block[0] += LIGHTING_LIT_CHANGED_MARK;
            }
            // The first frames are the fill. Arming the catch there keeps the
            // wavelet off through the frames a still-hall gate scores.
            // The test probe marks this frame only. It does not arm the catch,
            // so a stopped control holds the same way the local path does.
            if self.lighting.frame >= LIGHTING_STILL_REFRESH && !self.lighting.picture_unhold {
                self.lighting.scene_catch = self.lighting.frame;
            }
        }
        self.lighting.light_scene_seen = self.lighting.light_scene;
        sent += used;
        // Movers: KEEP timers still run. Head w / 200000 is the live count
        // the picture uses to unhold. The texel pool is gone.
        if world == 3 && !self.lighting.kept_bytes.is_empty() {
            let packed = lighting_movers(
                &self.lighting.kept_bytes,
                &self.frame_bytes,
                &mut self.lighting.movers,
            );
            let mut records = packed;
            for &i in self.lighting.mesh_movers.keys() {
                if let Some(sph) = self.lighting.mesh_spheres.get(i) {
                    // Centre is the world bounds centre (sphere xyz), not origin.
                    // Centre=[0,0,0] made every draw near the origin match the
                    // first mesh mover and broke the local texel frame.
                    let mesh_id = 10_001u32.saturating_add(u32::try_from(i).unwrap_or(0));
                    records.push([
                        *sph,
                        [sph[0], sph[1], sph[2], f32::from_bits(mesh_id)],
                        [0.0; 4],
                        [1.0, 0.0, 0.0, 0.0],
                    ]);
                }
            }
            let n = records.len();
            // Unhold while a mover is in the frame. Head w / 200000 is that count.
            block[3] += (n * 200000) as f32;
            if lighting_debug_on() {
                eprintln!(
                    "lighting world {world} cap {} flags {flags} movers {n} occs {} meshes {} of {}",
                    head.mode_cap % 1000,
                    lighting_occs(&self.frame_bytes).len(),
                    self.lighting.mesh_movers.len(),
                    self.lighting.mesh_poses.len()
                );
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
        // The camera up's w (head 3.w) is the flags' high bits, the budget
        // shift, the isolate toggles, and the SH texel detail bit. Written
        // after the last-camera copy, which fills this word from the basis.
        block[4 * 3 + 3] = head.head_hi as f32;
        // A frame drawn from held light runs no probes: what it saw is still what
        // the probes last ran on.
        if !frozen {
            self.lighting.kept_key = key;
            self.lighting.kept_bytes.clone_from(&self.frame_bytes);
            self.lighting.kept_froxel = self.froxel_wrote;
            self.lighting.kept_mesh = self.mesh_field_key;
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
        let atrous_ppm = lighting_atrous_ppm(
            self.lighting
                .atrous_dirty
                .unwrap_or(self.lighting.settings.denoiser.dirty_fraction),
        );
        // GI can be on while this buffer is still the 4096-byte stand-in. Probes
        // and the world cache stay off, so the alloc key does not rebuild, and the
        // changes block would start past the end (the cache-ray offset). Bounce 0
        // uploads nothing.
        let bytes = used as u64 * 4;
        if self.lighting.settings.gi.enabled
            && self.lighting.settings.bounce.count > 0
            && block_at + bytes <= self.lighting.work.size
        {
            // The last frame's passes are done with the block before it is written.
            self.memory_barrier(0x800, 0x1000, 0x20 | 0x40, 0x1000);
            let floats = used.min(block.len());
            let cmd = self.cmd;
            let work = self.lighting.work.buffer;
            self.write_device(cmd, work, block_at, pod_slice(&block[..floats]))?;
            if self.lighting.cache_full {
                // S78X: the a-trous dirty-fraction threshold (millionths). A cache
                // clear earlier this frame zeroed the word; the levels read it.
                let keys = self.lighting.cache_keys.buffer;
                self.write_device(
                    cmd,
                    keys,
                    lighting_atrous_ppm_at() * 4,
                    &atrous_ppm.to_ne_bytes(),
                )?;
                // MSME firefly sigma (MultiscaleMeanEstimator.hlsl, default 8).
                // Word 13 of the dispatch args. A zero word is sigma 0, which
                // tightens the clamp, so this is written every full-cache frame.
                let firefly_sigma = self.lighting.settings.surface_cache.firefly_sigma.to_bits();
                let ray_budget = self.lighting.settings.surface_cache.ray_budget;
                let dormant_rays = self.lighting.settings.surface_cache.dormant_rays;
                // Drain check noise test (ADR 0039 stage 1), word 14. A zero
                // word is sigma 0, the 1 % gap alone.
                let drain_sigma = self.lighting.settings.surface_cache.drain_sigma.to_bits();
                self.write_device(
                    cmd,
                    keys,
                    (LIGHTING_ARGS_AT + 11) * 4,
                    &ray_budget.to_ne_bytes(),
                )?;
                self.write_device(
                    cmd,
                    keys,
                    (LIGHTING_ARGS_AT + 12) * 4,
                    &dormant_rays.to_ne_bytes(),
                )?;
                self.write_device(
                    cmd,
                    keys,
                    (LIGHTING_ARGS_AT + 13) * 4,
                    &firefly_sigma.to_ne_bytes(),
                )?;
                self.write_device(
                    cmd,
                    keys,
                    (LIGHTING_ARGS_AT + 14) * 4,
                    &drain_sigma.to_ne_bytes(),
                )?;
            }
            self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
        }
        // The light alone (no camera): while it holds and a full sweep has changed
        // no mature patch, rounds relight young patches only. Any change goes back
        // to full sweeps at once. A small sun step stays on the hash from the last
        // full refresh: the direction is in the scene hash, and taking the new one
        // would relight the room as a whole-scene light change.
        let sun_amortise = self.note_sun_amortise();
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
        let tracked = if sun_amortise { self.lighting.light_hash } else { light };
        let counters_read = self.lighting.async_on
            || lighting_stats_wanted(
            self.lighting.settings.world_mode(),
            self.lighting.settings.experiments.sc_flags,
        );
        if tracked != self.lighting.light_hash {
            self.lighting.light_hash = tracked;
            self.lighting.light_quiet = 0;
        } else if self.lighting.still > 0
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
        let young_only = if sc { self.lighting.light_quiet >= 2 } else { self.lighting.settings.experiments.young_only && self.lighting.light_quiet >= sweep };
        self.lighting.stats_light[slot] = tracked;
        self.lighting.stats_young[slot] = young_only || sc;
        // z: bgra, fills (bit 6), young-only rounds and lamp picks (lighting_common.glsl);
        // the gather and filter passes add their round (bits 1-2) below.
        let picks = self.lighting.settings.experiments.cache_picks;
        let fill = self.lighting.settings.experiments.fill;
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
        let mut lamp = lamp_pick_push(&self.lighting.settings);
        lamp[1] |= lamp_shade_bits(self.lighting.shade_gate);
        if restir_off_is_exact(
            self.lighting.settings.restir_di,
            self.lighting.settings.lamp_pick.mode,
            self.lighting.shade_gate,
        ) {
            lamp[1] |= 1 << 19;
        }
        // The existing diag bit counts fresh shadow rays. It does not change
        // the shade. It is set only while GI metrics are recording.
        if self.gpu_times.as_ref().is_some_and(|times| times.record) {
            lamp[1] |= 1 << 17;
        }
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
        // turns `compose_reuse` off in the settings, which forces the full picture.
        let bounce_filling = self.lighting.bounce_fill < BOUNCE_FILL_FRAMES;
        // The fill walks every frame until the first drift pass. Holding the
        // frame-1 bounce and replacing it on frame 32 was the open-picture step.
        let opening = self.lighting.frame != 0 && self.lighting.frame < LIGHTING_FILL_WALK;
        // The drift pass visits every pixel. A miss that never resolved is
        // stored then, and a block the retry list could not hold is not left
        // black. Later held frames reuse that store.
        let refresh = self.lighting.frame != 0
            && self.lighting.frame.is_multiple_of(LIGHTING_STILL_REFRESH);
        // Per-frame cascades: every visible level-0 voxel and its upper chain
        // are reshaded each frame, so every pixel's bounce can move each frame.
        // Reusing last frame's picture held the hall at the frame-8 fill until
        // the 32-frame drift pass (path error 0.38 at frame 24, 0.29 at 32).
        let cascades = self.lighting.settings.world_mode() == 3 && self.lighting.settings.gi.enabled;
        let compose_full = cascades
            || bounce_filling
            || opening
            || refresh
            || !self.lighting.compose_valid
            || (self.lighting.still == 0 && !sun_amortise && (!self.lighting.mover_local || self.lighting.reach_wide))
            || self.lighting.settings.world_mode() != 3
            || !self.lighting.settings.compose_reuse
            || !self.lighting.settings.gi.enabled
            || blocks > LIGHTING_COMPOSE_CAP
            || (self.particle_stamped && !self.lighting.hdr.legacy_bytes);
        // The raster colour as the base: pixels with no surface keep it. A held
        // frame keeps last frame's picture in the buffer instead.
        if compose_full {
            self.copy_image_buffer(image, 6, self.lighting.out.buffer, w, h, true, 0);
        }
        self.memory_barrier(0x80 | 0x1000, 0x800, 0x40 | 0x1000, 0x20 | 0x40);
        let set = self.lighting.sets[slot];
        // `frozen` is the host settle bit (32 still frames, then either the
        // change counter or `sc_settle`, and the drain flag). It used to cancel
        // the cache round. A 1440p still hall hits that bit around frame 94,
        // and every later frame traced zero cache rays. The surface cache
        // keeps its budget; probe-only still idles.
        self.lighting.held = if frozen { self.lighting.held.saturating_add(1) } else { 0 };
        // A light cache round is due (`experiments.cache_hz` caps them per second). On
        // the async queue (`experiments.cache_async`) it goes there, else into this frame;
        // a frame without one runs the picture's passes only (lighting_mode 1).
        let cache_hz = f64::from(self.lighting.settings.experiments.cache_hz);
        let surface_budget = self.lighting.settings.world_mode() == 3
            && self.lighting.settings.world_cache.enabled
            && self.lighting.settings.gi.enabled
            && self.lighting.settings.bounce.count >= 2;
        let due = (surface_budget || !frozen)
            && (cache_hz <= 0.0
                || self
                    .lighting
                    .last_round
                    .is_none_or(|t| t.elapsed().as_secs_f64() * cache_hz >= 1.0));
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
                    self.lighting.async_failed = true;
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
            let built = if self.lighting.hdr.legacy_bytes {
                self.ensure_sun_pipe()
            } else {
                self.ensure_sun_hdr_pipe()
            };
            if let Err(err) = built {
                eprintln!("lighting: sun compose off: {err}");
                sun_compose = false;
            }
        }
        // legacy_bytes keeps the plain compose shader. The HDR compile is the
        // one that stores scene radiance.
        let mut hdr_compose = !self.lighting.hdr.legacy_bytes;
        if hdr_compose {
            if let Err(err) = self.ensure_hdr_compose() {
                eprintln!("lighting: hdr compose off: {err}");
                hdr_compose = false;
            }
        }
        let mut agx_display = hdr_compose;
        if agx_display {
            if let Err(err) = self.ensure_agx_pipe() {
                eprintln!("lighting: display pass off: {err}");
                agx_display = false;
            }
        }
        let mut gpu_bloom = agx_display && self.bloom_active();
        if gpu_bloom {
            if let Err(err) = self.ensure_bloom_pipe() {
                eprintln!("lighting: gpu bloom off: {err}");
                gpu_bloom = false;
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
        let mut hdr_pool: Handle = std::ptr::null_mut();
        let mut hdr_base = None;
        let ctrl = self.lighting.ctrl_bytes;
        let stats = self.lighting.stats_dev[slot].buffer;
        let cmd = self.cmd;
        self.write_device(cmd, stats, LIGHTING_CTRL_WORD * 4, &ctrl)?;
        unsafe {
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
                let keys = self.lighting.cache_keys.buffer;
                self.write_device(cmd, keys, lighting_sun_at() * 4, pod_bytes(&sun_words))?;
                self.write_device(
                    cmd,
                    keys,
                    LIGHTING_LAMP_CATCH_AT * 4,
                    &self.lighting.scene_catch.to_ne_bytes(),
                )?;
            }
            // The sun words are read by compute and as indirect args: the
            // indirect read needs the draw-indirect stage
            // (VUID-vkCmdPipelineBarrier-dstAccessMask-02816).
            self.memory_barrier(0x1000, 0x800 | 0x2, 0x1000, 0x20 | 0x1);
            let restir_wanted = restir_product_shade(
                self.lighting.settings.restir_di,
                self.lighting.settings.direct.enabled,
                self.lighting.shade_gate,
            );
            let first = slot as u32 * LIGHTING_STAMPS;
            let stamps = self
                .gpu_times
                .as_mut()
                .filter(|t| !t.lighting_pool.is_null())
                .map(|t| {
                    t.lighting_ids[slot] = groups.iter().map(|g| g.0).collect();
                    t.lighting_geom[slot] = groups.iter().map(|g| (g.1, g.2)).collect();
                    t.lighting_pool
                });
            if let Some(pool) = stamps {
                (self.fns.cmd_reset_query)(self.cmd, pool, first, LIGHTING_STAMPS);
                // GPU time of the prefix upload. The close stamp is after prepare.
                (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, first + RESTIR_PREPARE_AT);
            }
            let restir_on = self.prepare_restir_di(w, h, changes.as_ref());
            if let Some(pool) = stamps {
                (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, first + RESTIR_PREPARE_AT + 1);
            }
            if restir_on {
                pc[9] |= 1 << 21;
            } else {
                self.lighting.restir_query[slot] = u32::MAX;
                if restir_wanted {
                    // Labelled exact. The next frame that binds the reservoir
                    // clears this bit, and that frame does not keep the hold.
                    pc[9] |= 1 << 19;
                    self.lighting.restir_prepare_failures = self
                        .lighting
                        .restir_prepare_failures
                        .saturating_add(1);
                    eprintln!(
                        "lighting: restir prepare failed; this frame shades the labelled exact sum"
                    );
                }
            }
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
            // The query reset and the prepare-open stamp are above prepare.
            if restir_on {
                let pool = stamps.unwrap_or(std::ptr::null_mut());
                // Frame N wrote the history slot this frame reads, and wrote the
                // scratch this frame overwrites. The prefix barrier above makes
                // those writes available. This one makes them visible to the
                // passes' reads and writes.
                self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
                self.dispatch_restir_di(slot, &pc, pool, first);
                // Shade reads the spatial store (header word 6). A 1440p pass is
                // one dispatch; without this barrier the 4070 starts the direct
                // shade while that store is still in flight, and the picture
                // changes with the overlap. Lavapipe runs the dispatches in
                // order, so the race does not show there. This is intra-frame:
                // the two barriers above order the other frame.
                self.memory_barrier(0x800, 0x800, 0x40, 0x20);
            }
            if let Some(pool) = stamps {
                (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, first);
            }
            // The reach pass runs after direct and the cache have written this
            // frame's shade counts and cell notes, and before the wavelet or
            // compose reads the mask. Dispatching it here would be a frame late.
            let mut filter_round = 0u32;
            let mut atrous_i = 0usize;
            let mut atrous_ran = false;
            let mut picked = false;
            // The push constants the next pass sees (the gather and filter rounds add bits).
            let mut cur_pc = pc;
            let mut reach_marked = false;
            if self.lighting.cache_full && !self.lighting.cache_keys.buffer.is_null() {
                self.clear_cascade_l0_frame(w, h);
            }
            for (k, (pass, gx, gy)) in groups.iter().copied().enumerate() {
                if !reach_marked && (pass == 7 || pass == 8 || pass == SH_PREP_PASS) {
                    reach_marked = true;
                    if self.lighting.cache_full && !self.lighting.reach_pipe.is_null() {
                        (self.fns.cmd_push)(
                            self.cmd,
                            self.lighting.layout,
                            0x20,
                            0,
                            LIGHTING_PC_BYTES,
                            pc.as_ptr() as *const c_void,
                        );
                        (self.fns.cmd_bind_pipe)(self.cmd, 1, self.lighting.reach_pipe);
                        (self.fns.cmd_dispatch)(self.cmd, 1, 1, 1);
                        self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
                    }
                }
                // The screen pass marks the same blocks afterwards and its
                // exchange drops the field bit. Set it again from the mask
                // once that pass has finished, before compose reads it.
                if pass == 7 && self.lighting.cache_full && !self.lighting.reach_pipe.is_null() {
                    let mut stamp_pc = pc;
                    stamp_pc[2] = 0xffff_fffe;
                    (self.fns.cmd_push)(
                        self.cmd,
                        self.lighting.layout,
                        0x20,
                        0,
                        LIGHTING_PC_BYTES,
                        stamp_pc.as_ptr() as *const c_void,
                    );
                    (self.fns.cmd_bind_pipe)(self.cmd, 1, self.lighting.reach_pipe);
                    (self.fns.cmd_dispatch)(self.cmd, 1, 1, 1);
                    self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
                    (self.fns.cmd_push)(
                        self.cmd,
                        self.lighting.layout,
                        0x20,
                        0,
                        LIGHTING_PC_BYTES,
                        pc.as_ptr() as *const c_void,
                    );
                }
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
                    // The cache pass already read last frame's slot stamps.
                    // Clear them before this frame's wavelet writes its own.
                    if atrous_i == 0
                        && self.lighting.cache_full
                        && !self.lighting.reach_pipe.is_null()
                    {
                        let mut clear_pc = pc;
                        clear_pc[2] = 0xffff_ffff;
                        (self.fns.cmd_push)(
                            self.cmd,
                            self.lighting.layout,
                            0x20,
                            0,
                            LIGHTING_PC_BYTES,
                            clear_pc.as_ptr() as *const c_void,
                        );
                        (self.fns.cmd_bind_pipe)(self.cmd, 1, self.lighting.reach_pipe);
                        (self.fns.cmd_dispatch)(self.cmd, 1, 1, 1);
                        self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
                    }
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
                    } else if kind == LIGHTING_DENSE_ENTRIES {
                        // The fill resolve phases are gone. A cache dispatch
                        // leaves the phase at 0 and does not enter resolve.
                        k_pc[8] = gx;
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
                // Pass 9 is the same-frame gather. It is not one of the nine pipes.
                // Maintenance enqueues, then the drain fills, then compose loads.
                if pass == SH_PREP_PASS {
                    self.dispatch_sh_prep(sun_compose, compose_full, gx, gy);
                    self.memory_barrier(0x800, 0x800 | 0x1000 | 0x2, 0x40, 0x20 | 0x40 | 0x800 | 0x1);
                    if let Some(pool) = stamps {
                        let at = first
                            .saturating_add(1)
                            .saturating_add(u32::try_from(k).unwrap_or(u32::MAX));
                        if at < first + RESTIR_STAMP_AT {
                            (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, at);
                        }
                    }
                    continue;
                }
                // Pass 7 is compose: the sun path binds its own pipeline.
                // Pass 8 is a-trous: the plain kernel has no adaptive branch.
                // Adaptive uses the scalar shader that still reads the bit.
                let pipe = if pass == 3 {
                    self.direct_pipe()
                } else if pass == 7 && sun_compose {
                    if self.lighting.hdr.legacy_bytes {
                        self.lighting.sun_pipe
                    } else {
                        self.lighting.sun_hdr_pipe
                    }
                } else if pass == 7 && hdr_compose {
                    self.lighting.compose_hdr_pipe
                } else {
                    self.lighting.pipes[pass]
                };
                let maint_pipe = self.lighting.compose_maint_pipe;
                // World mode 3 gathered in pass 9. Other modes still write here.
                if pass == 7 && !sun_compose && !maint_pipe.is_null() && self.lighting.settings.world_mode() != 3 {
                    // #142: the cache writes compose used to make, over the same
                    // pixels, before compose reads the cache (and clears the
                    // G-buffer record this pass reads).
                    (self.fns.cmd_bind_pipe)(self.cmd, 1, maint_pipe);
                    if compose_full {
                        (self.fns.cmd_dispatch)(self.cmd, gx.max(1), gy.max(1), 1);
                    } else {
                        (self.fns.cmd_dispatch_indirect)(
                            self.cmd,
                            self.lighting.cache_keys.buffer,
                            lighting_compose_at() * 4,
                        );
                    }
                    self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
                }
                // Level 0 traces and shades every live voxel, then publish
                // blends this level with the next coarser footprint level.
                if pass == 7 {
                    self.record_cone();
                }
                if pass == 7 && self.lighting.cache_full {
                    self.dispatch_cascade_upper();
                    self.dispatch_cascade_l0();
                    self.stamp_cascade(5);
                }
                if pass == 7 && self.lighting.cache_full && !self.lighting.cascade_pub_pipe.is_null()
                {
                    if sun_compose && !compose_full {
                        (self.fns.cmd_fill_buffer)(
                            self.cmd,
                            self.lighting.cache_keys.buffer,
                            (lighting_compose_at() + 3) * 4,
                            4,
                            1,
                        );
                        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
                    }
                    (self.fns.cmd_bind_pipe)(self.cmd, 1, self.lighting.cascade_pub_pipe);
                    if sun_compose || compose_full {
                        let pgx = if sun_compose { w.div_ceil(8).max(1) } else { gx.max(1) };
                        let pgy = if sun_compose { h.div_ceil(8).max(1) } else { gy.max(1) };
                        (self.fns.cmd_dispatch)(self.cmd, pgx, pgy, 1);
                    } else {
                        (self.fns.cmd_dispatch_indirect)(
                            self.cmd,
                            self.lighting.cache_keys.buffer,
                            lighting_compose_at() * 4,
                        );
                    }
                    self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
                }
                if pass == 3 && self.lighting.area_clear && !self.lighting.area_mean.buffer.is_null() {
                    // Allocated after lighting_begin. Zero the mean before the
                    // shade reads it. The next frame's begin covers a resize
                    // that lands before the raster.
                    self.lighting.area_clear = false;
                    (self.fns.cmd_fill_buffer)(
                        self.cmd,
                        self.lighting.area_mean.buffer,
                        0,
                        self.lighting.area_mean.size,
                        0,
                    );
                    self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
                }
                (self.fns.cmd_bind_pipe)(self.cmd, 1, pipe);
                if pass == 7 {
                    self.lighting.compose_dispatches = 1;
                }
                if pass == 7 && self.lighting.bounce_fill < BOUNCE_FILL_FRAMES {
                    // The clear frame stores an empty bounce. Zero the slice
                    // so this frame's walk is what the picture shows.
                    self.clear_bounce_slice(w, h);
                }
                if pass == 7 && sun_compose {
                    // One dispatch. The old path indirect-dispatched the dirty
                    // list (a near-full probe walk under a moving sun) and then
                    // dispatched the whole grid again to paint direct. Both sat
                    // in this timestamp. The slice is cleared once per run so
                    // the first frame stores bounce everywhere; later frames
                    // walk only the band.
                    if !self.lighting.sun_bounce_live {
                        self.clear_bounce_slice(w, h);
                        self.lighting.sun_bounce_live = true;
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
                    let at = first + 1 + k as u32;
                    if at < first + RESTIR_STAMP_AT {
                        (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, at);
                    }
                }
            }
            if !reach_marked && self.lighting.cache_full && !self.lighting.reach_pipe.is_null() {
                (self.fns.cmd_push)(
                    self.cmd,
                    self.lighting.layout,
                    0x20,
                    0,
                    LIGHTING_PC_BYTES,
                    pc.as_ptr() as *const c_void,
                );
                (self.fns.cmd_bind_pipe)(self.cmd, 1, self.lighting.reach_pipe);
                (self.fns.cmd_dispatch)(self.cmd, 1, 1, 1);
                self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
            }
            if agx_display {
                let hdr_at = hdr_stamp_base(groups.len(), first);
                if let (Some(pool), Some(base)) = (stamps, hdr_at) {
                    (self.fns.cmd_write_timestamp)(self.cmd, 0x800, pool, base);
                    hdr_pool = pool;
                    hdr_base = Some(base);
                }
            }
        }
        if compose_full {
            self.lighting.compose_valid = true;
        }
        if agx_display {
            // Compose stored scene radiance. Bloom runs on that image, then this
            // pass replaces the 8-bit picture. The lattice sample is not in the GI shader.
            self.stamp_hdr_time(slot, true);
            self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
            // Cards go into scene_lin before bloom and AgX. A display-space
            // blend after this pass misses exposure and the halo.
            let scene = self.lighting.scene_lin.buffer;
            self.composite_particles_into_scene(w, h, scene)?;
            // The card composite binds its own set. Bloom and AgX use this one.
            unsafe {
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
            }
            self.memory_barrier(0x800, 0x800, 0x40, 0x20 | 0x40);
            if gpu_bloom {
                self.record_gpu_bloom(w, h)?;
            }
            unsafe {
                (self.fns.cmd_bind_pipe)(self.cmd, 1, self.lighting.agx_pipe);
                (self.fns.cmd_push)(
                    self.cmd,
                    self.lighting.layout,
                    0x20,
                    0,
                    LIGHTING_PC_BYTES,
                    pc.as_ptr() as *const c_void,
                );
                (self.fns.cmd_dispatch)(
                    self.cmd,
                    w.div_ceil(8).max(1),
                    h.div_ceil(8).max(1),
                    1,
                );
            }
            self.memory_barrier(0x800, 0x1000 | 0x4000, 0x40, 0x800 | 0x2000);
            self.stamp_hdr_time(slot, false);
            if let Some(base) = hdr_base {
                unsafe {
                    (self.fns.cmd_write_timestamp)(
                        self.cmd,
                        0x800,
                        hdr_pool,
                        base.saturating_add(1),
                    );
                }
            }
        }
        self.image_barrier(image, 6, 7, 0x1000, 0x1000, 0x800, 0x1000);
        if self.format == crate::hdr_output::FORMAT_R16F {
            // The encode buffer starts with tightly packed RGBA16F. BGRA follows it.
            self.copy_image_buffer(image, 7, self.lighting.hdr_encode.buffer, w, h, false, 0);
        } else {
            self.copy_image_buffer(image, 7, self.lighting.out.buffer, w, h, false, 0);
        }
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
            self.lighting.stats_slot = slot;
            // The dirty-list length. The same barrier covers the compute writes.
            // One host buffer: read it only after the device is idle.
            if self.lighting.cache_full && !self.lighting.compose_host.buffer.is_null() {
                let list = Region { src: lighting_compose_at() * 4, dst: 0, size: 4 };
                (self.fns.cmd_copy_buffer)(
                    self.cmd,
                    self.lighting.cache_keys.buffer,
                    self.lighting.compose_host.buffer,
                    1,
                    &list as *const Region as *const u8,
                );
            }
            // History counters live in the restir header, past the prefix fields.
            // The tail sits after the unfilled mask so diag[], chg[], and the
            // mask keep their words. A frame that did not run ReSTIR stores zeros.
            let tail = Region {
                src: (crate::restir_live::HIST_ACCEPT_WORD as u64).saturating_mul(4),
                dst: restir_diag_tail_offset(),
                size: RESTIR_DIAG_TAIL_BYTES,
            };
            let restir_ready = self.lighting.restir_frame
                && self.lighting.restir_buf.size
                    >= tail.src.saturating_add(RESTIR_DIAG_TAIL_BYTES);
            if restir_ready {
                (self.fns.cmd_copy_buffer)(
                    self.cmd,
                    self.lighting.restir_buf.buffer,
                    self.lighting.stats[slot].buffer,
                    1,
                    &tail as *const Region as *const u8,
                );
            } else {
                (self.fns.cmd_fill_buffer)(
                    self.cmd,
                    self.lighting.stats[slot].buffer,
                    tail.dst,
                    RESTIR_DIAG_TAIL_BYTES,
                    0,
                );
            }
        }
        if hdr_compose {
            if let Err(err) = self.ensure_meter_pipe() {
                eprintln!("lighting: gpu meter off: {err}");
            } else {
                self.record_gpu_meter(slot, w, h);
            }
        }
        self.memory_barrier(0x1000, 0x800 | 0x1000 | 0x4000, 0x1000, 0x20 | 0x40 | 0x800 | 0x2000);
        self.lighting_dump_copy();
        if self.lighting.bounce_fill < BOUNCE_FILL_FRAMES {
            self.lighting.bounce_fill = self.lighting.bounce_fill.saturating_add(1);
        }
        Ok(())
    }

    /// Zero the linear-bounce slice. `0` means empty; a real black bounce is
    /// stored as `pack + 1`. The slice sits after the picture and the glow,
    /// and before the screen-wavelet slices.
    fn clear_bounce_slice(&mut self, w: u32, h: u32) {
        let hist = u64::from(self.lighting.settings.direct.refresh_frames > 1);
        let pixels = u64::from(w) * u64::from(h);
        let at = (2 + hist) * pixels * 4;
        unsafe {
            (self.fns.cmd_fill_buffer)(self.cmd, self.lighting.out.buffer, at, pixels * 4, 0);
        }
        self.memory_barrier(0x1000, 0x800, 0x1000, 0x20 | 0x40);
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
        // First GPU free of teardown. `wait_lighting_world` returns without
        // waiting once `lifecycle_release` is set, and the 48 ms drain may
        // have given up while this frame and the world submission are queued.
        self.drain_for_teardown();
        self.wait_lighting_world();
        self.destroy_cone();
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
            if !self.lighting.world_qp.is_null() {
                (self.fns.destroy_query_pool)(self.device, self.lighting.world_qp, std::ptr::null());
                self.lighting.world_qp = std::ptr::null_mut();
            }
            if !self.lighting.cascade_qp.is_null() {
                (self.fns.destroy_query_pool)(self.device, self.lighting.cascade_qp, std::ptr::null());
                self.lighting.cascade_qp = std::ptr::null_mut();
            }
        }
        let dumps = std::mem::take(&mut self.lighting.dump);
        for (_, set) in dumps {
            for (_, mut buf) in set {
                self.destroy_buffer(&mut buf);
            }
        }
        // The world command buffer is freed with the compute command pool.
        let mut gbuf = std::mem::replace(&mut self.lighting.gbuf, Buffer::empty());
        let mut work = std::mem::replace(&mut self.lighting.work, Buffer::empty());
        let mut out = std::mem::replace(&mut self.lighting.out, Buffer::empty());
        let mut scene_lin = std::mem::replace(&mut self.lighting.scene_lin, Buffer::empty());
        let mut scene_host = std::mem::replace(&mut self.lighting.scene_host, Buffer::empty());
        let mut hdr_params = std::mem::replace(&mut self.lighting.hdr_params, Buffer::empty());
        let mut hdr_encode = std::mem::replace(&mut self.lighting.hdr_encode, Buffer::empty());
        let mut bloom = std::mem::replace(&mut self.lighting.bloom, Buffer::empty());
        let mut meter_dev = std::mem::replace(
            &mut self.lighting.meter_dev,
            [Buffer::empty(), Buffer::empty()],
        );
        let mut meter_host = std::mem::replace(
            &mut self.lighting.meter_host,
            [Buffer::empty(), Buffer::empty()],
        );
        let mut lamp_res = std::mem::replace(&mut self.lighting.lamp_res, Buffer::empty());
        let mut area_mean = std::mem::replace(&mut self.lighting.area_mean, Buffer::empty());
        let mut restir_buf = std::mem::replace(&mut self.lighting.restir_buf, Buffer::empty());
        let mut keys = std::mem::replace(&mut self.lighting.cache_keys, Buffer::empty());
        let mut cache = std::mem::replace(&mut self.lighting.cache, Buffer::empty());
        let mut compose_host = std::mem::replace(&mut self.lighting.compose_host, Buffer::empty());
        for k in 0..2 {
            let mut stats_dev = std::mem::replace(&mut self.lighting.stats_dev[k], Buffer::empty());
            self.destroy_buffer(&mut stats_dev);
            let mut stats = std::mem::replace(&mut self.lighting.stats[k], Buffer::empty());
            self.destroy_buffer(&mut stats);
        }
        self.destroy_buffer(&mut gbuf);
        self.destroy_buffer(&mut work);
        self.destroy_buffer(&mut out);
        self.destroy_buffer(&mut scene_lin);
        self.destroy_buffer(&mut scene_host);
        self.destroy_buffer(&mut hdr_params);
        self.destroy_buffer(&mut hdr_encode);
        self.destroy_buffer(&mut bloom);
        for dev in &mut meter_dev {
            self.destroy_buffer(dev);
        }
        for host in &mut meter_host {
            self.destroy_buffer(host);
        }
        self.destroy_buffer(&mut lamp_res);
        self.destroy_buffer(&mut area_mean);
        self.destroy_buffer(&mut restir_buf);
        self.destroy_buffer(&mut keys);
        self.destroy_buffer(&mut cache);
        self.destroy_buffer(&mut compose_host);
        unsafe {
            for pipe in self
                .lighting
                .pipes
                .iter_mut()
                .chain(std::iter::once(&mut self.lighting.atrous_plain))
                .chain(std::iter::once(&mut self.lighting.restir_pipe))
                .chain(std::iter::once(&mut self.lighting.restir_spatial_pipe))
                .chain(std::iter::once(&mut self.lighting.restir_spatial_wide))
                .chain(std::iter::once(&mut self.lighting.restir_direct))
                .chain(std::iter::once(&mut self.lighting.screen_pipe))
                .chain(std::iter::once(&mut self.lighting.reach_pipe))
                .chain(std::iter::once(&mut self.lighting.gbuffer_pipe))
                .chain(std::iter::once(&mut self.lighting.sun_pipe))
                .chain(std::iter::once(&mut self.lighting.agx_pipe))
                .chain(std::iter::once(&mut self.lighting.bloom_pipe))
                .chain(std::iter::once(&mut self.lighting.meter_pipe))
                .chain(std::iter::once(&mut self.lighting.compose_hdr_pipe))
                .chain(std::iter::once(&mut self.lighting.compose_maint_pipe))
                .chain(std::iter::once(&mut self.lighting.sh_drain_pipe))
                .chain(std::iter::once(&mut self.lighting.cascade_pub_pipe))
                .chain(std::iter::once(&mut self.lighting.cascade_l0_trace_pipe))
                .chain(std::iter::once(&mut self.lighting.cascade_l0_shade_pipe))
                .chain(std::iter::once(&mut self.lighting.cascade_upper_claim_pipe))
                .chain(std::iter::once(&mut self.lighting.cascade_upper_pick_pipe))
                .chain(std::iter::once(&mut self.lighting.cascade_upper_trace_pipe))
                .chain(std::iter::once(&mut self.lighting.cascade_upper_shade_pipe))
                .chain(std::iter::once(&mut self.lighting.sun_hdr_pipe))
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
fn lighting_stats_wanted(world: u32, sc_flags: u32) -> bool {
    lighting_debug_on() || !(world == 3 && sc_flags & 8192 != 0)
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

/// The change-block words `lighting_passes` uploads from the settings.
/// `flags` is the low 23 surface-cache bits (head z, above the probe-run bit).
/// `mode_cap` is world mode × 1000 + cap (head w, before mover counts).
/// `head_hi` is head 3.w: high flags, the budget shift, the isolate toggles,
/// ADR 0039 stage 3 bit 17 (`surface_cache.sh_texel_detail`), stage 4
/// bit 18 (`surface_cache.sh_texel_ray_cut`), and the stage 5 slice
/// bit 19 (`surface_cache.sh_probe_retire`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightingChangeHead {
    pub flags: u32,
    pub mode_cap: u32,
    pub head_hi: u32,
}

pub fn lighting_change_head(settings: &genos_scene::LightingSettings) -> LightingChangeHead {
    let world = settings.world_mode();
    let cap = match settings.experiments.world_cap {
        0 if world == 3 => LIGHTING_DEFAULT_SC_CAP,
        0 => 32,
        cap => cap,
    }
    .clamp(1, 999);
    let mut toggles = settings.experiments.toggles;
    // Test-only. A shipping build's guard is always on, so this bit stays
    // whatever the settings asked for. The knockout feature with
    // GENOS_KNOCKOUT=cache_fill freezes cache value writes. The dispatches
    // stay: dropping them faults lavapipe.
    if !crate::gate::guard(crate::gate::CACHE_FILL) {
        toggles |= genos_scene::LIGHTING_TOGGLE_FREEZE_CACHE;
    }
    const GI_ESTIMATOR: u32 = 1 << 20;
    const SURFACE_CACHE_HOLD: u32 = 1 << 30;
    const SH_CELLS: u32 = 1 << 23;
    const SH_TEXEL_DETAIL: u32 = 1 << 17;
    const SH_TEXEL_RAY_CUT: u32 = 1 << 18;
    const SH_PROBE_RETIRE: u32 = 1 << 19;
    let mut flags_all = settings.experiments.sc_flags
        | if toggles & genos_scene::LIGHTING_TOGGLE_DIRECT_ONLY != 0 {
            2097152
        } else {
            0
        };
    flags_all |= GI_ESTIMATOR;
    match settings.surface_cache.blend {
        genos_scene::SurfaceCacheBlend::Msme => flags_all &= !SURFACE_CACHE_HOLD,
        genos_scene::SurfaceCacheBlend::Hold => flags_all |= SURFACE_CACHE_HOLD,
    }
    if settings.surface_cache.sh_cells {
        flags_all |= SH_CELLS;
    } else {
        // Bit 23 of sc_flags is the same word. Leaving it set keeps sh_cells
        // on after the setting is turned off: head_hi bit 0 is that bit.
        flags_all &= !SH_CELLS;
    }
    let budget_shift = settings.gi.budget_shift.min(7);
    let mut head_hi = ((flags_all >> 23) & 511) | (budget_shift << 9) | ((toggles & 31) << 12);
    if settings.surface_cache.sh_texel_detail {
        head_hi |= SH_TEXEL_DETAIL;
    }
    if settings.surface_cache.sh_texel_ray_cut {
        head_hi |= SH_TEXEL_RAY_CUT;
    }
    if settings.surface_cache.sh_probe_retire {
        head_hi |= SH_PROBE_RETIRE;
    }
    LightingChangeHead {
        flags: flags_all & ((1 << 23) - 1),
        mode_cap: world * 1000 + cap,
        head_hi,
    }
}
/// Lighting stats buffer: 24 words, diag[64], chg[32], the a-trous fallback
/// (fraction, mode, threshold, filtered texels) at `LIGHTING_ATROUS_STAT`, the
/// control block at word 124 (16 words), then the unfilled-pixel count and the
/// shaded-pixel count. Two pad words keep the mask on a 144-word boundary. The
/// mask follows, one bit per composed pixel, and is not part of this header.
/// The change-counter enable word stays chg[31] (word 119). Sixteen bytes
/// after the mask cap hold the restir history counters. They are not diag words.
const LIGHTING_STATS_WORDS: usize = 148;
/// `gstats.diag[48]` (pixel shadow rays). The header is 24 words.
const LAMP_PICK_DIAG_WORD: usize = 24 + 48;
const _: () = assert!(LAMP_PICK_DIAG_WORD + 12 < LIGHTING_STATS_WORDS);
const LIGHTING_STATS_BYTES: u64 = LIGHTING_STATS_WORDS as u64 * 4;
/// Three header words plus one pad, after the unfilled mask.
const RESTIR_DIAG_TAIL_BYTES: u64 = 16;
/// Words 140 and 141. The control block occupies 124..140.
const LIGHTING_UNFILLED_WORD: usize = 140;
const LIGHTING_SHADED_WORD: usize = 141;
/// Word 0 of the mask. Matches lighting_cache.glsl.
const LIGHTING_UNFILLED_MASK_AT: usize = 148;
/// Words 144 and 145: the cache round's WRC sample and held luminance sums (x256).
/// Word 146: 1 while the cache is draining or filling (lighting_cache.glsl
/// LIGHTING_DRAIN_FLAG_AT): the samples still differ from the held light.
/// Word 147: the round's batch entries (16 rays each).
const LIGHTING_DRAIN_WORD: usize = 144;
/// Word 16: cache rays traced this round (`CacheStats.rays`). Scouts are word 19.
const LIGHTING_CACHE_RAYS_WORD: usize = 16;
/// (4096/32) * 2160. A larger picture clamps the mask.
const LIGHTING_UNFILLED_MASK_CAP: u32 = 276480;
const _: () = assert!(LIGHTING_UNFILLED_MASK_AT == LIGHTING_STATS_WORDS);
const _: () = assert!(LIGHTING_CACHE_RAYS_WORD < LIGHTING_STATS_WORDS);
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

fn restir_diag_tail_offset() -> u64 {
    LIGHTING_STATS_BYTES + u64::from(LIGHTING_UNFILLED_MASK_CAP) * 4
}

fn lighting_stats_storage_bytes() -> u64 {
    restir_diag_tail_offset() + RESTIR_DIAG_TAIL_BYTES
}
const LIGHTING_CHG_ENABLE_BYTE: u64 = LIGHTING_CHG_ENABLE_WORD * 4;
const LIGHTING_ATROUS_STAT: usize = 120;
/// Query index of the bloom and display pair, after the pass stamps.
/// `None` when those two would fall outside [`LIGHTING_STAMPS`].
fn hdr_stamp_base(groups: usize, first: u32) -> Option<u32> {
    let used = u32::try_from(groups).unwrap_or(u32::MAX).saturating_add(1);
    // ReSTIR owns the queries at RESTIR_STAMP_AT. A long pass list skips the
    // bloom pair rather than sharing those queries.
    if used.saturating_add(2) > RESTIR_STAMP_AT {
        return None;
    }
    Some(first.saturating_add(used))
}

/// S78S: the change counters (lighting_chg) run with GENOS_LIGHTING_SC_TRACE.
#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
fn lighting_chg_on() -> bool {
    genos_scene::lighting_env_os("GENOS_LIGHTING_SC_TRACE").is_some()
}

/// The a-trous dirty fraction (`denoise_dirty`, mirrored by
/// `GENOS_LIGHTING_ATROUS_DIRTY`) in millionths. Default 0.25: levels 1-5 keep
/// the work lists while the fullest level is at or under a quarter of the
/// filtered texels, and run the dense sweep above that. `0` is always dense,
/// `1` keeps the lists up to the storage cap.
fn lighting_atrous_ppm(frac: f32) -> u32 {
    (frac.clamp(0.0, 1.0) * 1_000_000.0).round() as u32
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


impl Gpu {
    /// Stable draw→mover id at submission: occluder KEEP index + 1, or mesh
    /// field KEEP index + 10001. Never matches by translation (that collided
    /// static draws near a mover centre and origin-centred mesh movers).
    pub(crate) fn mover_object_id_for_draw(
        &self,
        occluder_index: Option<u32>,
        world_object: Option<usize>,
    ) -> u32 {
        if let Some(occ) = occluder_index {
            let key = usize::try_from(occ).unwrap_or(usize::MAX);
            if self.lighting.movers.contains_key(&key) {
                return occ.saturating_add(1);
            }
        }
        if let Some(obj) = world_object {
            for &field_i in self.lighting.mesh_movers.keys() {
                if self
                    .lighting
                    .mesh_field_object
                    .get(field_i)
                    .copied()
                    .flatten()
                    == Some(obj)
                {
                    return 10_001u32.saturating_add(u32::try_from(field_i).unwrap_or(0));
                }
            }
        }
        0
    }
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
            let oid = u32::try_from(i.saturating_add(1)).unwrap_or(u32::MAX);
            [
                lighting_occ_bounds(r),
                [f(0), f(1), f(2), f32::from_bits(oid)],
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

/// True when the two scene blocks differ only in occluder records and the
/// occupancy grid those records rebuild. A lamp, the camera, or the sky is a
/// picture-wide change and returns false. The grid header starts at byte 176
/// (`occ_grid` in `scene_data.glsl`); the grid words follow the occluders.
fn scene_mover_only(old: &[u8], new: &[u8]) -> bool {
    const GRID_AT: usize = 176;
    if old.len() < crate::pack::SCENE_TAIL || new.len() < crate::pack::SCENE_TAIL {
        return false;
    }
    if old[..GRID_AT] != new[..GRID_AT] {
        return false;
    }
    let word = |bytes: &[u8], at: usize| -> usize {
        bytes
            .get(at * 4..at * 4 + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) as usize
    };
    let tail = crate::pack::SCENE_TAIL;
    let lamps = word(old, 0);
    let occs = word(old, 1);
    if lamps != word(new, 0) || occs != word(new, 1) {
        return false;
    }
    let lamp_end = tail + lamps * 32;
    let occ_end = lamp_end + occs * 64;
    if occ_end > old.len() || occ_end > new.len() {
        return false;
    }
    if old[tail..lamp_end] != new[tail..lamp_end] {
        return false;
    }
    old[lamp_end..occ_end] != new[lamp_end..occ_end]
}

const REACH_TAN_FLOOR: f32 = 1.0e-4;
const REACH_NEAR_Z: f32 = 0.25;
const REACH_NDC_Z: f32 = 0.05;

fn f32_at(bytes: &[u8], at: usize) -> Option<f32> {
    let b = bytes.get(at..at + 4)?;
    Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn vec3_at(bytes: &[u8], at: usize) -> Option<[f32; 3]> {
    Some([f32_at(bytes, at)?, f32_at(bytes, at + 4)?, f32_at(bytes, at + 8)?])
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// True when last frame's mask already covered half the picture. The next
/// frame unholds rather than pretending the rest is untouched.
fn reach_mask_saturated(tiles: u32, width: u32, height: u32) -> bool {
    let total = width.div_ceil(32) * height.div_ceil(32);
    total > 0 && tiles.saturating_mul(2) >= total
}

/// True when the change-list spheres are not a proved on-screen footprint. A
/// sphere behind the camera, on the camera, or entirely off the picture, or
/// whose union covers half the tiles, unholds. An empty list is unproved.
fn reach_footprint_unproven(frame: &[u8], spheres: &[[f32; 4]], width: u32, height: u32) -> bool {
    if spheres.is_empty() || width == 0 || height == 0 {
        return true;
    }
    let Some(eye) = vec3_at(frame, 16) else {
        return true;
    };
    let Some(right) = vec3_at(frame, 96) else {
        return true;
    };
    let Some(tan) = f32_at(frame, 108) else {
        return true;
    };
    let Some(up) = vec3_at(frame, 112) else {
        return true;
    };
    let Some(fwd) = vec3_at(frame, 128) else {
        return true;
    };
    let Some(aspect) = f32_at(frame, 140) else {
        return true;
    };
    let tw = width.div_ceil(32);
    let th = height.div_ceil(32);
    let total = tw * th;
    if total == 0 || total > 65536 {
        return true;
    }
    let mut bits = vec![0u64; usize::try_from(total).unwrap_or(0).div_ceil(64)];
    let mut marked = 0u32;
    let t = tan.max(REACH_TAN_FLOOR);
    let aspect = aspect.max(REACH_TAN_FLOOR);
    for s in spheres {
        let rel = [s[0] - eye[0], s[1] - eye[1], s[2] - eye[2]];
        let rad = s[3].max(0.0);
        let z = dot3(rel, fwd);
        if z < -rad || z <= REACH_NEAR_Z {
            return true;
        }
        let zc = z.max(REACH_NEAR_Z);
        let (ndc_x, ndc_y) = if z > REACH_NDC_Z {
            (dot3(rel, right) / (z * t * aspect), dot3(rel, up) / (z * t))
        } else {
            (0.0, 0.0)
        };
        let width_px = u32_f32(width);
        let height_px = u32_f32(height);
        let px = (ndc_x + 1.0) * 0.5 * width_px;
        let py = (1.0 - ndc_y) * 0.5 * height_px;
        let pry = (rad / zc) / t * (0.5 * height_px);
        let prx = (rad / zc) / (t * aspect) * (0.5 * width_px);
        let pr = prx.max(pry);
        let x0 = floor_i32((px - pr) / 32.0);
        let y0 = floor_i32((py - pr) / 32.0);
        let x1 = floor_i32((px + pr) / 32.0);
        let y1 = floor_i32((py + pr) / 32.0);
        let tw_i = i32::try_from(tw).unwrap_or(i32::MAX);
        let th_i = i32::try_from(th).unwrap_or(i32::MAX);
        // Entirely off the picture. A disc that only crosses the edge still
        // has its on-screen tiles; measured notes cover the rest.
        if x1 < 0 || y1 < 0 || x0 >= tw_i || y0 >= th_i {
            return true;
        }
        let x0 = x0.max(0);
        let y0 = y0.max(0);
        let x1 = x1.min(tw_i - 1);
        let y1 = y1.min(th_i - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let tile = u32::try_from(y).unwrap_or(0) * tw + u32::try_from(x).unwrap_or(0);
                let i = tile as usize;
                let word = i / 64;
                let bit = 1u64 << (i % 64);
                if bits.get(word).is_some_and(|w| w & bit == 0) {
                    bits[word] |= bit;
                    marked += 1;
                }
            }
        }
    }
    marked.saturating_mul(2) >= total
}

/// `u32` to `f32` with no silent cast. Both halves fit the mantissa.
fn u32_f32(v: u32) -> f32 {
    let hi = u16::try_from(v >> 16).unwrap_or(0);
    let lo = u16::try_from(v & 0xffff).unwrap_or(0);
    f32::from(hi) * 65536.0 + f32::from(lo)
}

/// `floor` as an `i32`. A tile index past 65535 clamps. The reach mask
/// stops at that many words.
fn floor_i32(v: f32) -> i32 {
    let n = v.floor();
    if !n.is_finite() {
        return 0;
    }
    let neg = n < 0.0;
    let mag = if neg { -n } else { n };
    if mag >= 65535.0 {
        return if neg { -65535 } else { 65535 };
    }
    let mut lo = 0u16;
    let mut hi = 65535u16;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if f32::from(mid) < mag {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let mag_i = i32::from(lo);
    if neg { -mag_i } else { mag_i }
}

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
    /// World-cache walks in the last sun compose. The first sun frame stores
    /// bounce for the picture. Later frames are one tile-row band.
    pub walks: u32,
    /// Compose dispatches in that frame. The sun path is one.
    pub dispatches: u32,
    /// Dirty-list length at the end of that frame. The list dispatch's group
    /// count. Zero when the key table was not copied. A full compose can still
    /// report the marks it did not use.
    pub blocks: u32,
    /// Maintenance and the lattice drain were recorded. False on a held frame
    /// that is not the 32-frame backstop.
    pub maint_ran: bool,
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
        LIGHTING_CACHE_SLOTS, LIGHTING_COMPOSE_HDR_SPV, LIGHTING_COMPOSE_MAINT_SPV,
        LIGHTING_COMPOSE_SPV, LIGHTING_SH_DRAIN_SPV, SUN_COMPOSE_HDR_SPV, SUN_COMPOSE_SPV,
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
        // The missing-field read, the path debug view, the texels population
        // record, the shown-mean age tally, the bounce store rule, and the
        // retry list sit in this pass. Compose compiles the slot count as a
        // literal, so the compiler folds it. Specialization constant 1 left
        // every slot mask dynamic and that module was 252_688 bytes.
        // Hitch mesh/on bodies stay out of GENOS_COMPOSE_NO_WALK (pool path
        // + lattice warm-up). A mover tile sets the field bit compose already
        // tests; the tile bitset stays out of this shader. Compose is
        // 230_656 bytes after merging #285. 248_000 still rejects a second
        // occluder walk.
        assert!(
            LIGHTING_COMPOSE_SPV.len() < 248_000,
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

    /// The compose module compiles the slot count as a literal. The host table
    /// and the specialization-constant default are the same number. Changing
    /// one arm and not the others desyncs compose from the buffer.
    #[test]
    #[expect(
        clippy::expect_used,
        reason = "a test aborts when the slot count does not fit in a SPIR-V word"
    )]
    fn compose_slot_literal_matches_the_host() {
        let src = include_str!("../shaders/lighting_cache_slots.glsl");
        let literals = slot_literals(src);
        assert_eq!(
            literals.len(),
            2,
            "compose literal and constant-id default, got {literals:?}"
        );
        let host = LIGHTING_CACHE_SLOTS;
        let probe = u64::from(crate::cache_probe::SLOTS);
        assert_eq!(host, probe, "host table and the cache probe disagree");
        for n in literals {
            assert_eq!(n, host, "shader slot count {n} is not the host {host}");
        }
        let want = u32::try_from(host).expect("slot count fits in a SPIR-V word");
        assert!(
            spirv_has_u32(LIGHTING_COMPOSE_SPV, want),
            "compose SPIR-V does not contain the host slot count {want}"
        );
    }

    /// Every compose specialization, including maintenance and the sun and HDR
    /// compiles. Flags on and off are this same module: the walk is not in it.
    #[test]
    fn default_compose_does_not_call_the_neighbour_walk() {
        for (name, words) in [
            ("compose", LIGHTING_COMPOSE_SPV),
            ("compose maint", LIGHTING_COMPOSE_MAINT_SPV),
            ("compose hdr", LIGHTING_COMPOSE_HDR_SPV),
            ("sun compose", SUN_COMPOSE_SPV),
            ("sun compose hdr", SUN_COMPOSE_HDR_SPV),
        ] {
            assert_spirv_1(name, words);
            let reached = reachable_from_main(words);
            for prefix in ["lighting_sh_gather(", "lighting_sh_read("] {
                assert!(
                    !reached.iter().any(|got| got.starts_with(prefix)),
                    "{name} still reaches {prefix}: {reached:?}"
                );
            }
            let text = String::from_utf8_lossy(words);
            assert!(
                !text.contains("lighting_sh_gather") && !text.contains("lighting_sh_read"),
                "{name} still names the neighbour walk"
            );
        }
        assert_spirv_1("sh drain", LIGHTING_SH_DRAIN_SPV);
        let drain = reachable_from_main(LIGHTING_SH_DRAIN_SPV);
        assert!(
            drain.iter().any(|got| got.starts_with("lighting_sh_gather(")),
            "the gather dispatch lost the neighbour walk: {drain:?}"
        );
    }

    #[test]
    fn sh_cells_off_clears_flag_bit_23() {
        let mut settings = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::High);
        settings.experiments.sc_flags |= 1 << 23;
        settings.surface_cache.sh_cells = false;
        let off = super::lighting_change_head(&settings);
        assert_eq!(off.head_hi & 1, 0, "bit 23 stayed set in head_hi");
        settings.surface_cache.sh_cells = true;
        let on = super::lighting_change_head(&settings);
        assert_eq!(on.head_hi & 1, 1, "sh_cells did not set head_hi bit 0");
    }

    fn assert_spirv_1(name: &str, words: &[u8]) {
        assert!(words.len() >= 8, "{name} is not SPIR-V");
        let version = u32::from_le_bytes([words[4], words[5], words[6], words[7]]);
        assert_eq!(version, 0x0001_0000, "{name} SPIR-V {version:#x}");
    }

fn reachable_from_main(bytes: &[u8]) -> Vec<String> {
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    let mut names = std::collections::HashMap::<u32, String>::new();
    let mut calls = Vec::<(u32, u32)>::new();
    let mut i = 5usize;
    let mut current = 0u32;
    while i < words.len() {
        let word = words[i];
        let count = (word >> 16) as usize;
        let op = word & 0xffff;
        if count == 0 || i + count > words.len() {
            break;
        }
        if op == 5 {
            let mut raw = Vec::new();
            for piece in &words[i + 2..i + count] {
                raw.extend_from_slice(&piece.to_le_bytes());
            }
            let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
            if let Ok(name) = std::str::from_utf8(&raw[..end]) {
                names.insert(words[i + 1], name.to_string());
            }
        } else if op == 54 && count >= 4 {
            current = words[i + 2];
        } else if op == 57 && count >= 4 {
            calls.push((current, words[i + 3]));
        }
        i += count;
    }
    let Some(root) = names.iter().find(|(_, name)| *name == "main").map(|(id, _)| *id) else {
        return Vec::new();
    };
    let mut seen = vec![root];
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        for (caller, callee) in &calls {
            if *caller == id && !seen.contains(callee) {
                seen.push(*callee);
                stack.push(*callee);
            }
        }
    }
    let mut reached = Vec::new();
    for id in seen {
        if let Some(name) = names.get(&id) {
            reached.push(name.clone());
        }
    }
    reached
}

fn slot_literals(src: &str) -> Vec<u64> {
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.starts_with("//") || !line.contains("LIGHTING_CACHE_SLOTS =") {
            continue;
        }
        let Some(eq) = line.rfind('=') else {
            continue;
        };
        let rest = line[eq + 1..].trim().trim_end_matches(';').trim();
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse() {
            out.push(n);
        }
    }
    out
}

fn spirv_has_u32(bytes: &[u8], want: u32) -> bool {
    if bytes.len() < 20 || !bytes.len().is_multiple_of(4) {
        return false;
    }
    let mut words = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let raw = [chunk[0], chunk[1], chunk[2], chunk[3]];
        words.push(u32::from_le_bytes(raw));
    }
    let mut i = 5usize;
    while i < words.len() {
        let word = words[i];
        let count = usize::try_from(word >> 16).unwrap_or(0);
        let op = word & 0xffff;
        if count == 0 || i + count > words.len() {
            return false;
        }
        // OpConstant: result type, result id, literal.
        if op == 43 && count >= 4 && words[i + 3] == want {
            return true;
        }
        i += count;
    }
    false
}
}

include!("lighting_dispatch.rs");

/// One frame of surface-cache scheduling counters (`gstats`).
#[derive(Clone, Copy, Debug)]
pub struct CacheRound {
    /// Occupied cache slots.
    pub live: u32,
    /// Cells claimed this round.
    pub claimed: u32,
    /// Histogram bucket the scheduler cut below. Zero means nothing was cut.
    pub thr: u32,
    /// Cache rays traced (texels and world cells, not the scout count).
    pub rays: u32,
    /// Entries the ranking marked due.
    pub due: u32,
    /// Entries that waited (not due, or a settle turn).
    pub waiting: u32,
    /// Scout rays scheduled this round. Zero when the scout was not due.
    pub scouts: u32,
    /// Lookups that found neither their entry nor a free slot.
    pub missed: u32,
    /// Entries freed this round (distance drop, expiry, or a full-table eviction).
    pub freed: u32,
}

impl CacheRound {
    fn from_words(words: &[u32; LIGHTING_STATS_WORDS]) -> Self {
        Self {
            live: words[2],
            claimed: words[8],
            thr: words[12],
            rays: words[16],
            due: words[17],
            waiting: words[18],
            scouts: words[19],
            missed: words[3],
            freed: words[10],
        }
    }
}

/// The shade's chosen reservoir and the visibility mean for one pixel.
///
/// `mean_kind` 3 is the product mean (`LAMP_GEO_RESTIR_MEAN`). `mean_rgb` is
/// that mean's irradiance. `restir_w` is `w_sum / (M * target)` from the
/// spatial scratch the shade reads.
#[derive(Clone, Copy, Debug)]
pub struct DirectSample {
    pub pixel: u32,
    pub restir_lamp: u32,
    pub restir_w: f32,
    pub restir_m: u32,
    pub restir_target: f32,
    pub mean_kind: u32,
    pub mean_m: u32,
    pub mean_lamp: u32,
    pub mean_vis: bool,
    pub mean_rgb: [f32; 3],
}

impl DirectSample {
    fn empty(pixel: u32) -> Self {
        Self {
            pixel,
            restir_lamp: 0,
            restir_w: 0.0,
            restir_m: 0,
            restir_target: 0.0,
            mean_kind: 0,
            mean_m: 0,
            mean_lamp: 0,
            mean_vis: false,
            mean_rgb: [0.0; 3],
        }
    }
}

/// `LAMP_GEO_RESTIR_MEAN` in `lighting_lamp_pick.glsl`.
const LAMP_GEO_RESTIR_MEAN: u32 = 3;

fn f32_from_u32(v: u32) -> f32 {
    let mut acc = 0.0f32;
    let mut bit = 1.0f32;
    let mut n = v;
    while n != 0 {
        if n & 1 == 1 {
            acc += bit;
        }
        bit += bit;
        n >>= 1;
    }
    acc
}

impl Gpu {
    /// Hash lookup of the surface-cache cells under `queries`, after the device is idle.
    pub(crate) fn read_cache_cells(
        &mut self,
        queries: &[crate::cache_probe::CacheQuery],
    ) -> Result<Vec<crate::cache_probe::StoredCell>, String> {
        if queries.len() > 32 {
            return Err("cache cell read is limited to 32 points".to_string());
        }
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        if !self.lighting.cache_full || self.lighting.cache.buffer.is_null() {
            return Err("surface cache is not allocated".to_string());
        }
        self.idle_device();
        let key_bytes = LIGHTING_CACHE_SLOTS.saturating_mul(4);
        if self.lighting.cache_keys.size < key_bytes || self.lighting.cache_keys.buffer.is_null() {
            return Err("cache key table is shorter than the slot count".to_string());
        }
        let raw_keys = self.copy_device_range(self.lighting.cache_keys.buffer, 0, key_bytes)?;
        let keys = u32_words(&raw_keys);
        let mut hits = Vec::with_capacity(queries.len());
        let mut regions = Vec::new();
        for query in queries {
            let key = crate::cache_probe::cache_key(query.position, query.normal, query.size);
            let hit = crate::cache_probe::classify_window(&keys, key);
            if let Some(slot) = hit.slot {
                let base = regions.len();
                self.push_cell_regions(slot, &mut regions)?;
                hits.push((query, hit, Some(base)));
            } else {
                hits.push((query, hit, None));
            }
        }
        let packed = if regions.is_empty() {
            Vec::new()
        } else {
            self.copy_device_regions(self.lighting.cache.buffer, &regions)?
        };
        let words = f32_vec4s(&packed);
        let mut out = Vec::with_capacity(hits.len());
        for (query, hit, base) in hits {
            let records = base.and_then(|at| seven_vec4s(&words, at));
            out.push(crate::cache_probe::stored_cell(*query, hit, records));
        }
        Ok(out)
    }

    /// Last collected ReSTIR frame, same order as [`Self::lighting_restir_ms`].
    ///
    /// One frame behind the draw that just returned. Zeros until a timestamped
    /// frame lands. Shade includes that dispatch's visibility ray.
    pub(crate) fn lighting_restir_last_ms(&self) -> [f64; 5] {
        self.lighting.restir_ms_last
    }

    /// Bottom-of-pipe lighting span of the last collected frame.
    ///
    /// The same window as the `lighting` column of `GPU_MS`. One frame behind
    /// the draw that just returned.
    pub(crate) fn lighting_span_ms(&self) -> f64 {
        self.lighting.span_ms
    }

    /// Mean of the collected ReSTIR stamps. Zeros until a timestamped frame lands.
    pub(crate) fn lighting_restir_ms(&self) -> [f64; 5] {
        let frames = self.lighting.restir_ms_frames;
        if frames == 0 {
            return [0.0; 5];
        }
        let denom = f64::from(frames);
        let mut out = [0.0; 5];
        for (i, sum) in self.lighting.restir_ms_sum.into_iter().enumerate() {
            out[i] = sum / denom;
        }
        out
    }

    /// Latest cache-round counters. The world round wins when it wrote rays or
    /// scouts; otherwise the frame copy from the last submitted slot.
    pub(crate) fn lighting_cache_round(&mut self) -> Result<CacheRound, String> {
        self.idle_device();
        let mut best: Option<CacheRound> = None;
        let mut take = |words: [u32; LIGHTING_STATS_WORDS]| {
            let round = CacheRound::from_words(&words);
            let rank = round.rays.saturating_add(round.scouts);
            let prev = best.map_or(0, |item| item.rays.saturating_add(item.scouts));
            if best.is_none() || rank > prev {
                best = Some(round);
            }
        };
        if !self.lighting.world_stats.memory.is_null() {
            if let Ok(words) = self.read_stats16(self.lighting.world_stats.memory) {
                take(words);
            }
        }
        if let Some(buf) = self.lighting.stats.get(self.lighting.stats_slot) {
            if !buf.memory.is_null() {
                if let Ok(words) = self.read_stats16(buf.memory) {
                    take(words);
                }
            }
        }
        best.ok_or_else(|| "lighting stats were not readable".to_string())
    }

    /// Scratch reservoir and the visibility mean for each pixel.
    pub(crate) fn read_direct_samples(
        &mut self,
        pixels: &[u32],
    ) -> Result<Vec<DirectSample>, String> {
        if pixels.is_empty() {
            return Ok(Vec::new());
        }
        if pixels.len() > 8 {
            return Err("direct sample read is limited to 8 pixels".to_string());
        }
        self.idle_device();
        let width = self.lighting.restir_w;
        let height = self.lighting.restir_h;
        let count = width.saturating_mul(height);
        if count == 0
            || self.lighting.restir_buf.buffer.is_null()
            || self.lighting.lamp_res.buffer.is_null()
        {
            return Err("direct reservoirs are not allocated".to_string());
        }
        let header = self.copy_device_range(self.lighting.restir_buf.buffer, 0, 256)?;
        let words = u32_words(&header);
        let scratch = words.get(6).copied().unwrap_or(0);
        let mut out = Vec::with_capacity(pixels.len());
        for &pixel in pixels {
            if pixel >= count {
                return Err(format!("pixel {pixel} is outside {width}x{height}"));
            }
            out.push(self.read_one_direct(pixel, scratch, count)?);
        }
        Ok(out)
    }

    fn read_one_direct(&self, pixel: u32, scratch: u32, count: u32) -> Result<DirectSample, String> {
        let mut sample = DirectSample::empty(pixel);
        let scratch_at = u64::from(scratch)
            .saturating_mul(4)
            .saturating_add(u64::from(pixel).saturating_mul(16));
        if scratch != 0 && scratch_at.saturating_add(16) <= self.lighting.restir_buf.size {
            let bytes = self.copy_device_range(self.lighting.restir_buf.buffer, scratch_at, 16)?;
            let words = u32_words(&bytes);
            if words.len() >= 4 && words[2] > 0 {
                let target = f32::from_bits(words[3]);
                sample.restir_lamp = words[0];
                sample.restir_m = words[2];
                sample.restir_target = target;
                if target > 0.0 {
                    let sum = f32::from_bits(words[1]);
                    let denom = f32_from_u32(words[2]) * target;
                    if denom > 0.0 {
                        sample.restir_w = sum / denom;
                    }
                }
            }
        }
        let stride = u64::from(count).saturating_mul(32);
        let mut best_m = 0u32;
        let mut found = false;
        for slot in 0..2u32 {
            let at = u64::from(slot)
                .saturating_mul(stride)
                .saturating_add(u64::from(pixel).saturating_mul(32));
            if at.saturating_add(32) > self.lighting.lamp_res.size {
                break;
            }
            let bytes = self.copy_device_range(self.lighting.lamp_res.buffer, at, 32)?;
            let words = u32_words(&bytes);
            if words.len() < 8 {
                continue;
            }
            let kind = words[6];
            if kind != LAMP_GEO_RESTIR_MEAN {
                if !found {
                    sample.mean_kind = kind;
                }
                continue;
            }
            let mean_m = words[2];
            if found && mean_m < best_m {
                continue;
            }
            found = true;
            best_m = mean_m;
            let pack = words[0];
            sample.mean_kind = kind;
            sample.mean_m = mean_m;
            sample.mean_lamp = pack & 0xffff;
            sample.mean_vis = (pack & 0x1_0000) != 0;
            sample.mean_rgb = [
                f32::from_bits(words[1]),
                f32::from_bits(words[3]),
                f32::from_bits(words[7]),
            ];
        }
        Ok(sample)
    }

    fn push_cell_regions(&self, slot: u32, regions: &mut Vec<(u64, u64)>) -> Result<(), String> {
        let slot_u = u64::from(slot);
        let slots = LIGHTING_CACHE_SLOTS;
        let indices = [
            slot_u.saturating_mul(3),
            slot_u.saturating_mul(3).saturating_add(1),
            slot_u.saturating_mul(3).saturating_add(2),
            slots.saturating_mul(3).saturating_add(slot_u),
            slots.saturating_mul(6).saturating_add(slot_u),
            slots.saturating_mul(8).saturating_add(slot_u),
            slots.saturating_mul(10).saturating_add(slot_u),
        ];
        for index in indices {
            let offset = index.saturating_mul(16);
            if offset.saturating_add(16) > self.lighting.cache.size {
                return Err("cache cell record sits past the cache buffer".to_string());
            }
            regions.push((offset, 16));
        }
        Ok(())
    }

    fn copy_device_range(&self, src: Handle, offset: u64, size: u64) -> Result<Vec<u8>, String> {
        if size == 0 {
            return Ok(Vec::new());
        }
        self.copy_device_regions(src, &[(offset, size)])
    }

    fn copy_device_regions(&self, src: Handle, regions: &[(u64, u64)]) -> Result<Vec<u8>, String> {
        let mut total = 0u64;
        for (_, size) in regions {
            total = total.saturating_add(*size);
        }
        if total == 0 {
            return Ok(Vec::new());
        }
        let mut dst = self.make_buffer(total, 0x2, Memory::Readback)?;
        let copied = self.record_region_copy(src, dst.buffer, regions);
        let bytes = match copied {
            Ok(()) => map_buffer_bytes(self, &dst, total),
            Err(err) => Err(err),
        };
        self.destroy_buffer(&mut dst);
        bytes
    }

    fn oracle_barrier(&self, cmd: Handle, src_stage: u32, dst_stage: u32, src_access: u32, dst_access: u32) {
        #[repr(C)]
        struct MemBar {
            s_type: i32,
            next: *const c_void,
            src_access: u32,
            dst_access: u32,
        }
        let bar = MemBar {
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
                &bar as *const MemBar as *const c_void,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            );
        }
    }

    fn oracle_submit(&self, cmd: Handle) -> Result<(), String> {
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
        unsafe {
            check(
                (self.fns.queue_submit)(
                    self.queue,
                    1,
                    &submit as *const Submit as *const u8,
                    std::ptr::null_mut(),
                ),
                "oracle submit",
            )?;
            check((self.fns.device_wait)(self.device), "oracle wait")?;
        }
        Ok(())
    }

    fn record_region_copy(&self, src: Handle, dst: Handle, regions: &[(u64, u64)]) -> Result<(), String> {
        let pool = self.cache_copy_pool()?;
        let cmd = match self.cache_copy_cmd(pool) {
            Ok(cmd) => cmd,
            Err(err) => {
                self.drop_copy_pool(pool);
                return Err(err);
            }
        };
        self.oracle_barrier(cmd, 0x1_0000, 0x1000, 0x1_8000, 0x800);
        let mut copies = Vec::with_capacity(regions.len());
        let mut dst_at = 0u64;
        for (src_at, size) in regions {
            copies.push(BufferCopy {
                src: *src_at,
                dst: dst_at,
                size: *size,
            });
            dst_at = dst_at.saturating_add(*size);
        }
        unsafe {
            (self.fns.cmd_copy_buffer)(
                cmd,
                src,
                dst,
                u32::try_from(copies.len()).unwrap_or(0),
                copies.as_ptr().cast::<u8>(),
            );
        }
        self.oracle_barrier(cmd, 0x1000, 0x4000, 0x1000, 0x2000);
        let ended = unsafe { check((self.fns.end_cmd)(cmd), "end cache-cell copy") };
        let submitted = ended.and_then(|()| self.oracle_submit(cmd));
        self.drop_copy_pool(pool);
        submitted
    }

    fn cache_copy_pool(&self) -> Result<Handle, String> {
        #[repr(C)]
        struct PoolInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            family: u32,
        }
        let pool_info = PoolInfo {
            s_type: 39,
            next: std::ptr::null(),
            flags: 1,
            family: self.queue_family,
        };
        let mut pool = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.create_pool)(
                    self.device,
                    &pool_info as *const PoolInfo as *const u8,
                    std::ptr::null(),
                    &mut pool,
                ),
                "cache-cell command pool",
            )?;
        }
        Ok(pool)
    }

    fn cache_copy_cmd(&self, pool: Handle) -> Result<Handle, String> {
        #[repr(C)]
        struct AllocInfo {
            s_type: i32,
            next: *const c_void,
            pool: Handle,
            level: u32,
            count: u32,
        }
        #[repr(C)]
        struct BeginInfo {
            s_type: i32,
            next: *const c_void,
            flags: u32,
            inherit: *const c_void,
        }
        let alloc = AllocInfo {
            s_type: 40,
            next: std::ptr::null(),
            pool,
            level: 0,
            count: 1,
        };
        let begin = BeginInfo {
            s_type: 42,
            next: std::ptr::null(),
            flags: 1,
            inherit: std::ptr::null(),
        };
        let mut cmd = std::ptr::null_mut();
        unsafe {
            check(
                (self.fns.alloc_cmd)(self.device, &alloc as *const AllocInfo as *const u8, &mut cmd),
                "cache-cell command",
            )?;
            check(
                (self.fns.begin_cmd)(cmd, &begin as *const BeginInfo as *const u8),
                "begin cache-cell copy",
            )?;
        }
        Ok(cmd)
    }

    fn drop_copy_pool(&self, pool: Handle) {
        unsafe {
            (self.fns.destroy_pool)(self.device, pool, std::ptr::null());
        }
    }
}

#[repr(C)]
struct BufferCopy {
    src: u64,
    dst: u64,
    size: u64,
}

fn u32_words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let mut lane = [0u8; 4];
            lane.copy_from_slice(chunk);
            u32::from_le_bytes(lane)
        })
        .collect()
}

fn f32_vec4s(bytes: &[u8]) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for chunk in bytes.chunks_exact(16) {
        let mut word = [0.0f32; 4];
        for (lane, value) in word.iter_mut().enumerate() {
            let at = lane.saturating_mul(4);
            let mut raw = [0u8; 4];
            raw.copy_from_slice(&chunk[at..at + 4]);
            *value = f32::from_le_bytes(raw);
        }
        out.push(word);
    }
    out
}


fn seven_vec4s(words: &[[f32; 4]], at: usize) -> Option<[[f32; 4]; 7]> {
    let mut records = [[0.0f32; 4]; 7];
    for (i, record) in records.iter_mut().enumerate() {
        *record = *words.get(at.saturating_add(i))?;
    }
    Some(records)
}

fn map_buffer_bytes(gpu: &Gpu, buffer: &Buffer, size: u64) -> Result<Vec<u8>, String> {
    if size > buffer.size {
        return Err("cache cell copy does not fit".to_string());
    }
    let len = usize::try_from(size).unwrap_or(0);
    let mut out = vec![0u8; len];
    unsafe {
        let mut mapped = std::ptr::null_mut();
        check(
            (gpu.fns.map_mem)(gpu.device, buffer.memory, 0, size, 0, &mut mapped),
            "map cache cell",
        )?;
        std::ptr::copy_nonoverlapping(mapped.cast::<u8>(), out.as_mut_ptr(), len);
        (gpu.fns.unmap_mem)(gpu.device, buffer.memory);
    }
    Ok(out)
}

#[cfg(test)]
mod sh_knob_tests {
    use super::*;

    /// The low-spec SH storage knob is one value in two places: the shader's
    /// layout and the buffer Rust allocates for it must agree.
    #[test]
    fn sh_storage_knob_matches_the_shader_and_sizes_the_cache() {
        let glsl = include_str!("../shaders/lighting_cache_slots.glsl");
        let line = glsl
            .lines()
            .find(|l| l.starts_with("const bool LIGHTING_SH_HALF"))
            .expect("LIGHTING_SH_HALF in lighting_cache_slots.glsl");
        assert_eq!(line.contains("true"), LIGHTING_SH_HALF, "{line}");
        let tail = LIGHTING_CACHE_SLOT_BYTES - 23 * 16;
        let sh_bytes = if LIGHTING_SH_HALF { 24 } else { 48 };
        let pad = if LIGHTING_SH_HALF { 24 } else { 0 };
        assert_eq!(tail, sh_bytes + pad + 16 + CASCADE_BIN_BYTES);
        let rays = glsl
            .lines()
            .find(|l| l.starts_with("const uint LIGHTING_CACHE_RAYS"))
            .expect("LIGHTING_CACHE_RAYS in lighting_cache_slots.glsl");
        assert!(rays.contains(&format!("= {LIGHTING_CACHE_RAYS}u;")), "{rays}");
        assert_eq!(u64::from(crate::cache_probe::SLOTS), LIGHTING_CACHE_SLOTS);
    }
}

#[cfg(test)]
mod reach_footprint_tests {
    use super::{
        dot3, f32_at, floor_i32, reach_footprint_unproven, reach_mask_saturated, u32_f32, vec3_at,
        REACH_NDC_Z, REACH_NEAR_Z, REACH_TAN_FLOOR,
    };

    /// Half the diagonal of a unit square. A cell of edge `size` uses
    /// `size * REACH_CELL_DIAG`, matching `REACH_CELL_DIAG` in the shader.
    const REACH_CELL_DIAG: f32 = 0.70710677;

    /// True when a sample a `step` away from a mean of `peak` joins a tile.
    /// A step inside the quarter stays quiet. No residual yet stays quiet:
    /// that sample is the seed. A running residual has to clear `noise_band`.
    /// A cell the mover reaches joins in the shader without this test.
    fn reach_sample_hot(step: f32, peak: f32, noise_known: bool, noise_band: f32) -> bool {
        let bar = 0.25 * peak.max(0.25);
        if step <= bar {
            return false;
        }
        if !noise_known {
            return false;
        }
        step > noise_band
    }

    /// Tiles a world cell's square covers. `radius` is the half-diagonal.
    /// On or behind the camera covers nothing and is not wide. Half the
    /// picture is wide and the tile list is empty. An entirely off-screen
    /// cell covers nothing and is not wide. Same projection as
    /// `lighting_reach_note_bounds`.
    fn reach_cell_span(
        frame: &[u8],
        centre: [f32; 3],
        radius: f32,
        width: u32,
        height: u32,
    ) -> (bool, Vec<u32>) {
        let Some(eye) = vec3_at(frame, 16) else {
            return (true, Vec::new());
        };
        let Some(right) = vec3_at(frame, 96) else {
            return (true, Vec::new());
        };
        let Some(tan) = f32_at(frame, 108) else {
            return (true, Vec::new());
        };
        let Some(up) = vec3_at(frame, 112) else {
            return (true, Vec::new());
        };
        let Some(fwd) = vec3_at(frame, 128) else {
            return (true, Vec::new());
        };
        let Some(aspect) = f32_at(frame, 140) else {
            return (true, Vec::new());
        };
        if width == 0 || height == 0 {
            return (true, Vec::new());
        }
        let tw = width.div_ceil(32);
        let th = height.div_ceil(32);
        let total = tw * th;
        if total == 0 {
            return (true, Vec::new());
        }
        let rel = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
        let rad = radius.max(0.0);
        let z = dot3(rel, fwd);
        if z < -rad || z <= REACH_NEAR_Z {
            return (false, Vec::new());
        }
        let t = tan.max(REACH_TAN_FLOOR);
        let aspect = aspect.max(REACH_TAN_FLOOR);
        let zc = z.max(REACH_NEAR_Z);
        let (ndc_x, ndc_y) = if z > REACH_NDC_Z {
            (dot3(rel, right) / (z * t * aspect), dot3(rel, up) / (z * t))
        } else {
            (0.0, 0.0)
        };
        let width_px = u32_f32(width);
        let height_px = u32_f32(height);
        let px = (ndc_x + 1.0) * 0.5 * width_px;
        let py = (1.0 - ndc_y) * 0.5 * height_px;
        let pry = (rad / zc) / t * (0.5 * height_px);
        let prx = (rad / zc) / (t * aspect) * (0.5 * width_px);
        let pr = prx.max(pry);
        let mut x0 = floor_i32((px - pr) / 32.0);
        let mut y0 = floor_i32((py - pr) / 32.0);
        let mut x1 = floor_i32((px + pr) / 32.0);
        let mut y1 = floor_i32((py + pr) / 32.0);
        let tw_i = i32::try_from(tw).unwrap_or(i32::MAX);
        let th_i = i32::try_from(th).unwrap_or(i32::MAX);
        if x1 < 0 || y1 < 0 || x0 >= tw_i || y0 >= th_i {
            return (false, Vec::new());
        }
        x0 = x0.max(0);
        y0 = y0.max(0);
        x1 = x1.min(tw_i - 1);
        y1 = y1.min(th_i - 1);
        let span = u32::try_from(x1 - x0 + 1).unwrap_or(0) * u32::try_from(y1 - y0 + 1).unwrap_or(0);
        if span.saturating_mul(2) >= total {
            return (true, Vec::new());
        }
        let mut tiles = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                tiles.push(u32::try_from(y).unwrap_or(0) * tw + u32::try_from(x).unwrap_or(0));
            }
        }
        (false, tiles)
    }

    fn put(buf: &mut [u8], at: usize, v: f32) {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Camera at the origin, looking down -Z, 60° vertical, 320×180.
    fn camera() -> Vec<u8> {
        let mut buf = vec![0u8; 256];
        put(&mut buf, 96, 1.0);
        put(&mut buf, 108, 0.57735026);
        put(&mut buf, 116, 1.0);
        put(&mut buf, 136, -1.0);
        put(&mut buf, 140, 320.0 / 180.0);
        buf
    }

    #[test]
    fn an_interior_disc_stays_local_and_an_unproved_one_does_not() {
        let frame = camera();
        let interior = [[0.0, 0.0, -8.0, 0.4]];
        assert!(
            !reach_footprint_unproven(&frame, &interior, 320, 180),
            "a small disc in the middle of the picture was treated as unproved"
        );
        let behind = [[0.0, 0.0, 4.0, 0.4]];
        assert!(reach_footprint_unproven(&frame, &behind, 320, 180));
        let near = [[0.0, 0.0, -0.1, 0.4]];
        assert!(reach_footprint_unproven(&frame, &near, 320, 180));
        let off = [[40.0, 0.0, -8.0, 0.4]];
        assert!(reach_footprint_unproven(&frame, &off, 320, 180));
        assert!(reach_footprint_unproven(&frame, &[], 320, 180));
        assert!(reach_mask_saturated(30, 320, 180));
        assert!(!reach_mask_saturated(9, 320, 180));
    }

    #[test]
    fn a_cell_footprint_covers_every_tile_it_spans() {
        let frame = camera();
        let (wide, one) = reach_cell_span(&frame, [0.8, 0.0, -8.0], 0.02, 320, 180);
        assert!(!wide, "a small on-screen cell was treated as the picture");
        assert_eq!(one.len(), 1, "a cell inside one tile covered {one:?}");
        let (wide, many) = reach_cell_span(&frame, [0.0, 0.0, -8.0], 0.2, 320, 180);
        assert!(!wide);
        assert!(
            many.len() >= 2,
            "the centre tile was the whole footprint, got {many:?}"
        );
        let centre = reach_cell_span(&frame, [0.0, 0.0, -8.0], 0.0, 320, 180).1;
        assert!(
            centre.iter().all(|t| many.contains(t)),
            "the footprint dropped the centre tile"
        );
        let (behind_wide, behind) = reach_cell_span(&frame, [0.0, 0.0, 4.0], 0.2, 320, 180);
        assert!(!behind_wide, "a cell behind the camera unheld the picture");
        assert!(behind.is_empty(), "a cell behind the camera covered a tile");
        let (near_wide, near) = reach_cell_span(&frame, [0.0, 0.0, -0.1], 0.2, 320, 180);
        assert!(!near_wide, "a cell on the camera unheld the picture");
        assert!(near.is_empty(), "a cell on the camera covered a tile");
        assert!(reach_cell_span(&frame, [0.0, 0.0, -8.0], 8.0, 320, 180).0);
        let (wide, diag) =
            reach_cell_span(&frame, [0.0, 0.0, -8.0], 0.25 * REACH_CELL_DIAG, 320, 180);
        assert!(!wide);
        assert!(diag.len() >= 2, "a cell-edge diagonal stayed on the centre tile");
    }

    #[test]
    fn a_seed_sample_stays_quiet_and_a_step_outside_its_noise_joins() {
        assert!(
            !reach_sample_hot(0.4, 1.0, false, 0.0),
            "a cell with no residual joined on its seed sample"
        );
        assert!(
            !reach_sample_hot(0.1, 1.0, false, 0.0),
            "a step inside the quarter joined"
        );
        assert!(
            !reach_sample_hot(0.4, 1.0, true, 0.5),
            "a step inside the cell's own noise joined"
        );
        assert!(reach_sample_hot(0.6, 1.0, true, 0.5));
    }
}



#[cfg(test)]
mod level0_layout {
    use super::{
        lighting_fill_keys_words, lighting_keys_words, lighting_level0_words, lighting_reach_words,
        lighting_upper_words,
    };

    #[test]
    fn the_level0_table_fits_in_a_shader_word() {
        let pixels = 320 * 180;
        let start = lighting_fill_keys_words(pixels) + lighting_reach_words();
        let end = lighting_keys_words(pixels);
        assert_eq!(
            end - start,
            lighting_level0_words(pixels) + lighting_upper_words()
        );
        assert!(end < u64::from(u32::MAX), "key words {end} do not fit in a shader index");
    }
}
