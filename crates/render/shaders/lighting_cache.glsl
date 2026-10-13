// GI v2 surface light cache (stand-in for the surface cache cards of phase 4).
//
// A hash table of small surface patches: a cell of the world grid (25 cm near the
// eye, doubling with distance) on one of six facing axes. Each patch keeps a surface
// point, its normal and the irradiance arriving there. Probe rays and cache rays
// that hit a surface read the patch's irradiance as the light already bounced onto
// it, which gives every bounce after the first; a patch is relit from fixed hashed
// rays a batch at a time, so a static scene converges to fixed values and nothing
// crawls. Patches nobody reads for a while are freed.
//
// Keys (binding 9): one uint per slot, 0 when free; then the count of mature live
// slots and one list of SLOTS entries, rebuilt each round by lighting_compact.comp:
// mature slots from the front, young ones (relit fewer than LIGHTING_CACHE_MATURE
// times) from the back; last, the young count. A round relights the young first
// (a newly seen surface gets its light at once) and fills the batch from the
// mature ones in turn. Once the lights and objects have held still long enough
// for a full sweep to change nothing (lighting_gpu.rs), a round takes only the young:
// their light cannot have changed, and a moving camera then costs only the new
// surfaces it sees.
// Patches (binding 10): 3 vec4 per slot: position (w: frame last read), normal
// (w: cell size, 0 until claimed), irradiance (w: times relit, up to
// LIGHTING_CACHE_MATURE; 0 unlit).

layout(std430, set = 0, binding = 9) buffer CacheKeys {
    uint key[];
} ckeys;

layout(std430, set = 0, binding = 10) buffer Cache {
    vec4 v[];
} cache;

// Per frame, read back by the CPU to tell when the light has settled: the largest
// relative change of a relit patch (millionths), patches relit, live patches.
layout(std430, set = 0, binding = 11) buffer CacheStats {
    uint max_change;
    uint relit;
    uint live;
    // Lookups that found neither their patch nor a free slot (the table is full).
    uint missed;
    // The same two for mature patches only: has the light itself stopped changing.
    uint mature_change;
    uint mature_relit;
    // Screen probes: rays they hold this frame (kept strata included) and probes
    // placed (lighting_gather.comp), for the effective sample count.
    uint pad0;
    uint pad1;
    // Surface cache trace (GENOS_LIGHTING_SC_TRACE): texels claimed, sets restarted by
    // an offer move, texels freed, relights of texels already at the cap.
    uint claimed;
    uint restarted;
    uint freed;
    uint atcap;
    // Scheduler and few rays (S58/S59): the threshold bucket this round set,
    // texels that took more than one entry (the rest of their set at once),
    // relights that stored a partial quarter sum, sets committed.
    uint sched_thr;
    uint extra;
    uint partial;
    uint sets;
    // S64A rate-based scheduling: cache rays traced this round (texels, WRC,
    // scouts), batch entries of the texels due, texels not yet due, scout rays.
    uint rays;
    uint due;
    uint waiting;
    uint scouts;
    uint pad2;
    uint pad3;
    uint pad4;
    uint pad5;
    // S76 diagnostics (GENOS_LIGHTING_SC_TRACE): per-round counters, see lighting_gpu.rs.
    uint diag[64];
    // S78S change counters (GENOS_LIGHTING_SC_TRACE, chg[31] the enable word the
    // host sets): which writers changed what, and by how much. lightingchg lines,
    // perf/chg.py; the slots are listed at LIGHTING_CHG_*.
    uint chg[32];
    // S78X dense fallback (lightingatrous lines). Fraction is millionths of the
    // filtered texels, the fullest of levels 1-5. Mode is 1 (work list), 2
    // (dense full-frame sweep), or 0 (the levels did not run). ppm is the
    // threshold in force (GENOS_LIGHTING_ATROUS_DIRTY). active is the filtered
    // texel count that fraction divided by.
    uint atrous_frac;
    uint atrous_mode;
    uint atrous_ppm;
    uint atrous_active;
    // Control block (lighting_dispatch.rs pack_lighting_ctrl), word 124.
    // High writes every bit that today's tuned path assumes, so a set bit
    // falls through and the picture stays the one the passes recorded before.
    uint ctrl_bits;
    uint ctrl_max_lights;
    uint ctrl_bounce;
    uint ctrl_scale;
    uint ctrl_bias;
    uint ctrl_pixel_only;
    uint ctrl_ambient_mode;
    uint ctrl_history;
    float ctrl_shadow_cell;
    float ctrl_ambient_r;
    float ctrl_ambient_g;
    float ctrl_ambient_b;
    float ctrl_ambient_i;
    float ctrl_ground_r;
    float ctrl_ground_g;
    float ctrl_ground_b;
    // Unfilled-on-screen detector (compose). Pixels with no finished set, and
    // shaded surface pixels. The two pads keep the mask at stats word 144
    // (lighting_gpu.rs LIGHTING_UNFILLED_MASK_AT), after the 144-word header.
    // The mask is one bit per pixel, 32 pixels a word, row-major, and is
    // written only while the detector is on.
    uint unfilled;
    uint shaded;
    uint unfilled_pad0;
    uint unfilled_pad1;
    uint mask[];
} gstats;
// 0 pixel or probe read, 1 cache-ray hit, 2 probe hit. The direct pass sets it
// before a cache read. A pixel compose leaves it at 0.
uint lighting_hit_kind = 0u;
bool lighting_gi_on() {
    return (gstats.ctrl_bits & 4u) != 0u;
}
bool lighting_sky_enabled() {
    return (gstats.ctrl_bits & 256u) != 0u;
}
// Bits 16..18. High writes 5, which is also the fallback when the field is 0.
uint lighting_denoise_iters() {
    uint n = (gstats.ctrl_bits >> 16u) & 7u;
    return n == 0u ? 5u : n;
}
// Bits 22..31, steps of 0.0001. Relative luminance deviation sqrt(var) / mean.
float lighting_denoise_variance() {
    return float((gstats.ctrl_bits >> 22u) & 1023u) * 0.0001;
}
// LIGHTING_DENOISE_PLAIN is the high pipeline. Adaptive passes are compiled
// out of it. That branch lives in the other a-trous pipeline. On the 4070 the
// extra branches dropped occupancy of this kernel (11.7 ms vs the gi-v2 7.5 ms)
// even with the bits clear. Half-res and tiles were removed after they lost
// the A/B (S79).
#ifdef LIGHTING_DENOISE_PLAIN
bool lighting_denoise_adaptive() { return false; }
#else
// Bit 20. Fewer wavelet iterations where history is stable. Off unless the
// experimental knob is on. See S79.
bool lighting_denoise_adaptive() {
    return (gstats.ctrl_bits & 1048576u) != 0u;
}
#endif
// Unlimited (bit 16384) is today's cache. Bounce 0 and 1 do not read it: one
// bounce is the direct light on a probe hit, and the cache lookup is not run.
// Bounce 2 and above read it (cache-to-cache).
bool lighting_bounce_ok() {
    if ((gstats.ctrl_bits & 16384u) != 0u) {
        return true;
    }
    return gstats.ctrl_bounce >= 2u;
}
// Kept-hit light writes (light pass): all, changed, by relative luminance delta
// (<0.2 %, 0.2-1 %, 1-5 %, >5 %), and changed by source of the hit's direct light
// (TDL reused, TDL checked / recomputed, lamps traced here).
const uint CHG_KEPT_WRITE = 0u;
const uint CHG_KEPT_CHANGED = 1u;
const uint CHG_KEPT_D0 = 2u;
const uint CHG_KEPT_TDL_REUSE = 6u;
// Kept lights held by the S78T hysteresis (within tolerance, not written).
const uint CHG_KEPT_HELD = 10u;
const uint CHG_KEPT_TDL_CHECK = 7u;
const uint CHG_KEPT_LAMPS = 8u;
// Kept-hit records changed by the trace (new hit).
const uint CHG_REC = 9u;
// Gather (S78R): probes gathered, held (skipped), and why not held: inputs
// changed this frame, moved, reads a neighbour's strata or fills.
const uint CHG_G_RUN = 11u;
const uint CHG_G_HELD = 12u;
const uint CHG_G_DIRTY = 13u;
const uint CHG_G_MOVED = 14u;
const uint CHG_G_PICKS = 15u;
// Surface-cache texel value writes (lighting_cache.comp): all, by relative
// luminance delta (as above).
const uint CHG_TEX_WRITE = 16u;
const uint CHG_TEX_D0 = 17u;
// Kept-hit light writes by where the hit's bounce came from (lighting_cache_
// irradiance_at): fine texel, coarse patch lit, none, coarse unlit -> parent,
// coarse unlit -> neighbours; then changed ones the same.
const uint CHG_KB_FINE = 21u; // S78Z: 21-30 are the verify / wake counters (CHG_V_*)
const uint CHG_KC_FINE = 26u;
const uint CHG_V_TESTS = 21u;    // verification probes tested (rotating set)
const uint CHG_V_FAILS = 22u;    // ... whose surface impact passed their noise bound
const uint CHG_W_SEEDED = 23u;   // wakes seeded (ring probes stamped)
const uint CHG_W_CAPPED = 24u;   // wakes dropped by the per-frame cap
const uint CHG_W_TESTS = 25u;    // woken probes tested
const uint CHG_W_FAILS = 26u;    // ... that passed the wake on
const uint CHG_W_DEDUPE = 27u;   // ring probes already woken by this event
const uint CHG_V_RELIT = 28u;    // kept hits relit by verification / wakes
const uint CHG_W_HOPS = 29u;     // sum of hop depth of woken probes (chain size)
uint lighting_bounce_path = 2u;
// S78S debug isolate toggles (lighting_gpu.rs lighting_toggles): head 3.w bits 12-15.
uint lighting_toggles() {
    return (uint(work.v[lighting_change_at() + 3u].w) >> 12u) & 15u;
}
const uint LIGHTING_TOGGLE_FREEZE_CACHE = 1u;
const uint LIGHTING_TOGGLE_BOUNCE_ONLY = 4u;
const uint LIGHTING_TOGGLE_FREEZE_PROBES = 8u;
void lighting_chg(uint i, uint v) {
    if (gstats.chg[31] != 0u) {
        atomicAdd(gstats.chg[i], v);
    }
}
// Bin of a relative luminance delta: 0 <0.2 %, 1 <1 %, 2 <5 %, 3 more.
uint lighting_chg_bin(vec3 a, vec3 b) {
    float la = dot(a, vec3(0.2126, 0.7152, 0.0722));
    float lb = dot(b, vec3(0.2126, 0.7152, 0.0722));
    float r = abs(la - lb) / max(max(la, lb), 1.0e-4);
    return r < 0.002 ? 0u : (r < 0.01 ? 1u : (r < 0.05 ? 2u : 3u));
}
const bool LIGHTING_DIAG = false;
void lighting_dg(uint i, uint v) { if (LIGHTING_DIAG) atomicAdd(gstats.diag[i], v); }

const float LIGHTING_CACHE_MATURE = 3.0;


uint lighting_frame() {
    return pc.params.w;
}

// Quiet compose (lighting_compose.comp). The picture is kept in the output
// buffer. An 8x8 block is redrawn only when a pass that feeds it writes a new
// value: direct light or the surface point (lighting_direct.comp), a probe's SH
// (lighting_filter.comp), or the filtered field of a texel that block reads
// (the push below). The block list is indirect-dispatched, so a frame whose
// producers write nothing launches no compose groups. Words live at the end of
// the key buffer; lighting_gpu.rs sizes the same range.
const uint LIGHTING_COMPOSE_CAP = 65535u;
const uint LIGHTING_COMPOSE_AT = 2u * LIGHTING_CACHE_SLOTS + 2u + 17600u + 16u + 3u * 65536u + 1024u + 32768u + 16u + 5u * LIGHTING_CACHE_SLOTS + 5u * 65537u + 6u * 65536u + 16u;
const uint LIGHTING_COMPOSE_COUNT = LIGHTING_COMPOSE_AT;
const uint LIGHTING_COMPOSE_FULL = LIGHTING_COMPOSE_AT + 3u;
const uint LIGHTING_COMPOSE_STAMP = LIGHTING_COMPOSE_AT + 4u;
const uint LIGHTING_COMPOSE_LIST = LIGHTING_COMPOSE_STAMP + LIGHTING_COMPOSE_CAP;
const uint LIGHTING_COMPOSE_PACK = LIGHTING_COMPOSE_LIST + LIGHTING_COMPOSE_CAP;
const uint LIGHTING_COMPOSE_MINX = LIGHTING_COMPOSE_PACK + LIGHTING_CACHE_SLOTS;
const uint LIGHTING_COMPOSE_MINY = LIGHTING_COMPOSE_MINX + LIGHTING_CACHE_SLOTS;
const uint LIGHTING_COMPOSE_MAXX = LIGHTING_COMPOSE_MINY + LIGHTING_CACHE_SLOTS;
const uint LIGHTING_COMPOSE_MAXY = LIGHTING_COMPOSE_MAXX + LIGHTING_CACHE_SLOTS;
// Set by the compose pass around the lookups that colour a pixel.
bool lighting_sc_display = false;
uvec2 lighting_compose_px = uvec2(0u);
// Unfilled-on-screen detector. Bits of scene.grid_at.w: 24 count, 25 magenta tint
// (debug_view.rs). Off is a uniform test and no atomics. The mask word count is
// lighting_gpu.rs LIGHTING_UNFILLED_MASK_CAP; word 0 is stats word 144.
const float LIGHTING_UNFILLED_MIN_SETS = 1.0;
const uint LIGHTING_UNFILLED_MASK_CAP = 276480u;
bool lighting_px_unfilled = false;
bool lighting_unfilled_count() {
    return (scene.grid_at.w & 0x1000000u) != 0u;
}
bool lighting_unfilled_tint() {
    return (scene.grid_at.w & 0x2000000u) != 0u;
}
void lighting_unfilled_mark(uvec2 p) {
    uint stride = (pc.dims.x + 31u) / 32u;
    uint at = p.y * stride + (p.x >> 5u);
    if (at < LIGHTING_UNFILLED_MASK_CAP) {
        atomicOr(gstats.mask[at], 1u << (p.x & 31u));
    }
}
// The camera matches the one stored for last frame (change block head 1-4).
// eye.w is the "last camera is valid" flag; up.w is the flag word, not the basis.
bool lighting_view_held() {
    vec4 eye = work.v[lighting_change_at() + 1u];
    if (!(eye.w > 0.5)) {
        return false;
    }
    if (any(notEqual(eye.xyz, scene.eye.xyz))) {
        return false;
    }
    vec4 right = work.v[lighting_change_at() + 2u];
    vec4 up = work.v[lighting_change_at() + 3u];
    vec4 fwd = work.v[lighting_change_at() + 4u];
    return all(equal(right.xyz, scene.view_right.xyz)) && right.w == scene.view_right.w
        && all(equal(up.xyz, scene.view_up.xyz))
        && all(equal(fwd.xyz, scene.view_forward.xyz)) && fwd.w == scene.view_forward.w;
}
// A pixel showed this texel within `window` frames, or the camera has not moved
// since a pixel showed it. Compose skips redraws while the camera is held, so
// the shown-frame stamp stops advancing; the texel is still the one on screen.
bool lighting_sc_pixel_fresh(uint shown, int window) {
    if (shown == 0u) {
        return false;
    }
    if (int(lighting_frame() - shown) <= window) {
        return true;
    }
    return lighting_view_held();
}
bool lighting_sc_expired(uint last, uint shown) {
    return int(lighting_frame() - last) > int(LIGHTING_CACHE_LIFE) && !lighting_sc_pixel_fresh(shown, 60);
}
void lighting_compose_cover(uint slot) {
    if (!lighting_sc_display || slot == ~0u || slot >= LIGHTING_CACHE_SLOTS) {
        return;
    }
    uvec2 b = lighting_compose_px >> 3u;
    atomicMin(ckeys.key[LIGHTING_COMPOSE_MINX + slot], b.x);
    atomicMin(ckeys.key[LIGHTING_COMPOSE_MINY + slot], b.y);
    atomicMax(ckeys.key[LIGHTING_COMPOSE_MAXX + slot], b.x);
    atomicMax(ckeys.key[LIGHTING_COMPOSE_MAXY + slot], b.y);
}
void lighting_compose_mark_block(uint block) {
    if (block >= LIGHTING_COMPOSE_CAP) {
        return;
    }
    if (atomicExchange(ckeys.key[LIGHTING_COMPOSE_STAMP + block], lighting_frame()) == lighting_frame()) {
        return;
    }
    uint at = atomicAdd(ckeys.key[LIGHTING_COMPOSE_COUNT], 1u);
    if (at < LIGHTING_COMPOSE_CAP) {
        ckeys.key[LIGHTING_COMPOSE_LIST + at] = block;
    }
}
void lighting_compose_mark_px(uint idx) {
    uint w = pc.dims.x;
    uint bw = (w + 7u) / 8u;
    uint x = idx % w;
    uint y = idx / w;
    lighting_compose_mark_block((y >> 3u) * bw + (x >> 3u));
}
void lighting_compose_mark_slot(uint slot) {
    if (slot >= LIGHTING_CACHE_SLOTS) {
        return;
    }
    uint x0 = ckeys.key[LIGHTING_COMPOSE_MINX + slot];
    uint y0 = ckeys.key[LIGHTING_COMPOSE_MINY + slot];
    uint x1 = ckeys.key[LIGHTING_COMPOSE_MAXX + slot];
    uint y1 = ckeys.key[LIGHTING_COMPOSE_MAXY + slot];
    if (x0 > x1 || y0 > y1) {
        return;
    }
    uint bw = (pc.dims.x + 7u) / 8u;
    uint bh = (pc.dims.y + 7u) / 8u;
    if (bw == 0u || bh == 0u || x0 >= bw || y0 >= bh) {
        return;
    }
    x1 = min(x1, bw - 1u);
    y1 = min(y1, bh - 1u);
    for (uint y = y0; y <= y1; ++y) {
        for (uint x = x0; x <= x1; ++x) {
            lighting_compose_mark_block(y * bw + x);
        }
    }
}
// Probe cell (cx, cy) is one of the four samples for the pixel cells around it
// (lighting_compose.comp). Mark every 8x8 block those cells cover.
void lighting_compose_mark_probe(uint probe) {
    uint tile = max(pc.params.y, 1u);
    uint cols = pc.dims.z;
    if (cols == 0u) {
        return;
    }
    int cx = int(probe % cols);
    int cy = int(probe / cols);
    int x0 = max(cx - 1, 0);
    int y0 = max(cy - 1, 0);
    int x1 = cx + 2;
    int y1 = cy + 2;
    uint bw = (pc.dims.x + 7u) / 8u;
    uint bh = (pc.dims.y + 7u) / 8u;
    uint bx0 = min(uint(x0) * tile, pc.dims.x) / 8u;
    uint by0 = min(uint(y0) * tile, pc.dims.y) / 8u;
    uint bx1 = min(uint(max(x1, 0)) * tile, pc.dims.x);
    uint by1 = min(uint(max(y1, 0)) * tile, pc.dims.y);
    bx1 = min((bx1 + 7u) / 8u, bw);
    by1 = min((by1 + 7u) / 8u, bh);
    for (uint y = by0; y < by1; ++y) {
        for (uint x = bx0; x < bx1; ++x) {
            lighting_compose_mark_block(y * bw + x);
        }
    }
}
void lighting_compose_note_field(uint slot, uint packed) {
    uint now = packed + 1u;
    uint prev = ckeys.key[LIGHTING_COMPOSE_PACK + slot];
    ckeys.key[LIGHTING_COMPOSE_PACK + slot] = now;
    if (prev != 0u && prev != now) {
        lighting_compose_mark_slot(slot);
    }
}

// Light cache round: which batch of live patches is relit. Rounds are counted on
// their own (lighting_gpu.rs), so a frame without one skips no batch.
uint lighting_round() {
    return pc.params.z >> 12u;
}

// Schedule id. Seed 0 is the slot, so a live frame keeps today's phases.
// A seeded frame uses the cache key: two runs can claim different slots for
// the same patch, and the key does not follow that race.
uint lighting_sched(uint slot) {
    if (pc.seed == 0u) {
        return slot;
    }
    uint key = ckeys.key[slot];
    return key == 0u ? slot : key;
}

// This round relights young patches only (params.z bit 10).
bool lighting_young_only() {
    return ((pc.params.z >> 10u) & 1u) != 0u;
}

uint lighting_young_count() {
    return min(ckeys.key[2u * LIGHTING_CACHE_SLOTS + 1u], LIGHTING_CACHE_SLOTS);
}
uint lighting_mature_count() {
    return min(ckeys.key[LIGHTING_CACHE_SLOTS], LIGHTING_CACHE_SLOTS);
}

// Surface cache scheduler (Lumen's surface cache update priority, FLumenCard
// LastUsed - LastUpdated, under a fixed budget): each compact pass buckets the
// priority of every texel it could relight into LIGHTING_SC_BUCKETS log buckets and
// lists those at or above the threshold the cache pass set from the previous
// round's histogram, so the batch takes the most stale-for-their-use texels
// first. GIBS (2021) weighs a texel still short of its cap up (more effort where
// the estimate is unconverged) and a dormant one down. ckeys after the young
// count: the histogram, then the threshold.
const bool LIGHTING_SC_SCHED = true;
// World space radiance cache (WRC, S61A; Lumen's world space radiance cache,
// SHaRC's hash grid): the 25 cm ray-hit cells. Kept resident and converging
// around the camera, on screen or not, so a surface revealed by a turn or a
// corner starts from converged light: each round LIGHTING_WRC_SCOUT_ENTRIES batch
// entries trace scout rays from the camera (half spread over the sphere, half
// ahead of the motion and turn, a quarter of them from up to 2 m beside the eye
// to see round corners); their hits claim and read WRC cells, which the
// scheduler then relights like any other. A WRC cell keeps a running mean to
// LIGHTING_WRC_CAP sets (was: each relight replaced it, at most 3 sets). A WRC cell a
// pixel needs as a seed (its fine texel has no light) with fewer than
// LIGHTING_WRC_YOUNG sets is relit first with whole sets (the reveal burst); new fine
// texels start from the WRC (bilinear, coplanar cells only) with at most
// LIGHTING_WRC_SEED_SETS sets and take their turn by priority.
const uint LIGHTING_WRC_SCOUT_ENTRIES = 256u;
// S61B: a WRC relight is one batch entry (16 rays, a quarter of a texel's set)
// and counts as one sample; LIGHTING_WRC_CAP is in those (128 = 32 whole sets).
const float LIGHTING_WRC_CAP = 128.0;
const float LIGHTING_WRC_YOUNG = 16.0;
const float LIGHTING_WRC_SEED_SETS = 8.0;
const float LIGHTING_WRC_NEAR = 6.0;
const uint LIGHTING_SC_BUCKETS = 64u;
const uint LIGHTING_SC_HIST_AT = 2u * LIGHTING_CACHE_SLOTS + 2u;
const uint LIGHTING_SC_THR_AT = LIGHTING_SC_HIST_AT + LIGHTING_SC_BUCKETS;
// WRC refresh (S61B, Anthony: a hard, fixed budget): at most LIGHTING_WRC_REFRESH
// batch entries a round (~6 % of 65536) go to WRC cells outside the reveal
// burst, from two lists: change-driven first (a mover nearby, or a lamp that
// reaches the cell moved since its last relight: its history drops to
// LIGHTING_WRC_CHANGE_KEEP samples), then the steady background (a resident cell not
// relit for LIGHTING_WRC_REFRESH_FRAMES, or still short of its cap and read since)
// with what is left. Lumen's world space radiance cache: a fixed budget of probe
// updates, the changed and the nearest first.
const uint LIGHTING_WRC_REFRESH = 4096u;
// S62D (Anthony): a refresh takes a whole 64-ray set (LIGHTING_SC_QUARTERS
// consecutive entries), so the cap above holds LIGHTING_WRC_REFRESH_CELLS cells a
// round (1024; same rays as before).
const uint LIGHTING_WRC_REFRESH_CELLS = LIGHTING_WRC_REFRESH / 4u;
const uint LIGHTING_WRC_REFRESH_FRAMES = 180u;
const uint LIGHTING_WRC_CHANGE_FRAMES = 120u;
const float LIGHTING_WRC_CHANGE_KEEP = 4.0;
const uint LIGHTING_WRC_CHG_COUNT_AT = LIGHTING_SC_HIST_AT + 66u;
const uint LIGHTING_WRC_BG_COUNT_AT = LIGHTING_SC_HIST_AT + 67u;
const uint LIGHTING_WRC_LAMPS_AT = LIGHTING_SC_HIST_AT + 68u;
// S62B: WRC cells bursting this round (zeroed with the two counts above), and
// the most a round may burst (each takes 4 entries).
const uint LIGHTING_WRC_BURST_COUNT_AT = LIGHTING_SC_HIST_AT + 70u;
const uint LIGHTING_WRC_BURST_MAX = 2048u;
// S65A two-tier updates (Anthony): a due texel whose light changes little (the
// estimator's inconsistency under LIGHTING_SC_TRICKLE_THR standard deviations, with
// its own history, no local change) takes a trickle of one 16-ray entry (the smallest the batch
// holds; its quarters add up to a set over updates, Lumen's few rays per texel
// accumulated temporally) from its own list of at most LIGHTING_SC_TRICKLE entries a
// round. The rest are ranked by how much their light changes and take whole sets
// from the main batch, so a big change syncs in a few rounds. Both are ceilings.
const float LIGHTING_SC_TRICKLE_THR = 0.173;
const float LIGHTING_SC_LAMP_K = 1.0;
// S65C anti-lag (ReBLUR / A-SVGF): a set further than LIGHTING_SC_ANTILAG_SD standard
// deviations from the history pulls the blend up in proportion (to 1 at 4x),
// so a big change follows within a few sets and a small one keeps the average.
const float LIGHTING_SC_ANTILAG_SD = 3.0;
const uint LIGHTING_SC_TRICKLE = 4096u;
const uint LIGHTING_SC_TRICKLE_COUNT_AT = LIGHTING_SC_HIST_AT + 71u;
const uint LIGHTING_SC_TRICKLE_AT = LIGHTING_SC_HIST_AT + 13200u;
// Batch entries before the trickle, WRC refresh and scout regions.
// S74A A-SVGF gradient checks (Schied et al. 2018, 2019): batch entries
// before the scouts, one check per ray (LIGHTING_GRAD_ENTRIES x 16 checks).
const uint LIGHTING_GRAD_ENTRIES = 2048u;
uint lighting_sc_room() {
    return LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES - LIGHTING_GRAD_ENTRIES - LIGHTING_WRC_REFRESH - LIGHTING_SC_TRICKLE;
}
// S66A budget boost (Anthony): the texels' share of a round is normally
// LIGHTING_SC_BASE_BATCH entries' worth (the 1M-ray ceiling); while more is due than
// that, for at most LIGHTING_SC_BOOST_ROUNDS rounds in a row, the ceiling rises to
// the whole batch (1.5x). A backlog that lasts longer (a scene that changes
// forever) falls back to the normal ceiling until it clears, so the boost never
// stays on. Decided by the cache pass's thread 0 from the due entries' histogram.
const uint LIGHTING_SC_BASE_BATCH = 65536u;
const uint LIGHTING_SC_BOOST_ROUNDS = 30u;
const uint LIGHTING_SC_BOOST_AT = LIGHTING_SC_HIST_AT + 65u;
uint lighting_sc_base_room() {
    return LIGHTING_SC_BASE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES - LIGHTING_GRAD_ENTRIES - LIGHTING_WRC_REFRESH - LIGHTING_SC_TRICKLE;
}
const uint LIGHTING_WRC_RING_HEAD_AT = LIGHTING_SC_HIST_AT + 69u;
const uint LIGHTING_WRC_RING = 16u;
// S65C: 9 words an entry (old place, new place, range, brightest channel,
// frame), past the trickle list.
const uint LIGHTING_WRC_RING_AT = LIGHTING_SC_HIST_AT + 17300u;
const uint LIGHTING_WRC_RING_W = 9u;
const uint LIGHTING_WRC_SNAP_LAMPS = 128u;
const uint LIGHTING_WRC_SNAP_AT = LIGHTING_SC_HIST_AT + 160u;
const uint LIGHTING_WRC_CHG_AT = LIGHTING_SC_HIST_AT + 680u;
const uint LIGHTING_WRC_BG_AT = LIGHTING_WRC_CHG_AT + LIGHTING_WRC_REFRESH;

// World irradiance cells (experiment, GENOS_LIGHTING_WORLD): the change block's head w
// is mode * 1000 + cap. Mode 1: pixels read the light cache's patches,
// interpolated; mode 2: screen probes average their irradiance into the patch they
// sit in (a running mean of up to cap frames, binding 10 after the patches) and
// pixels read that, interpolated, so what a pixel shows depends on where it is in
// the world, not on where the probe grid fell this frame.
uint lighting_world_mode() {
    return (uint(work.v[lighting_change_at()].w) / 1000u) % 100u;
}
float lighting_world_cap() {
    return float(uint(work.v[lighting_change_at()].w) % 1000u);
}
// Sets texel slot grows to.
float lighting_sc_cap(uint slot);
float lighting_sc_light_window();
float lighting_hit_size(vec3 pos);

// Surface cache switches (GENOS_LIGHTING_SC_FLAGS, experiments): bit 0 sets weigh k^2
// (else k); bit 1 a second pass (a texel reaching the cap the first time drops to a
// quarter of it once); bit 2 texels only rays read grow to LIGHTING_SC_HIT_CAP sets;
// bit 3 pooling never narrower than 0.6 cells; bit 4 the finest texels (under 10
// cm; none since S57) grow to twice the cap; bit 5 a texel with few sets shares its
// pixels with the probes; bit 6 a texel's lit-from point moving does not restart
// its sets.
uint lighting_sc_flags() {
    // S72K: bits 23 and up ride in head 3.w (head z is a float, exact to 2^24).
    return (uint(work.v[lighting_change_at()].z) >> 1u) | ((uint(work.v[lighting_change_at() + 3u].w) & 511u) << 23u);
}
// S72S budget scale (Anthony 14:03: scale down to a weak GPU, nothing may need a
// minimum ray count): every per-round ray budget (texels, trickle, WRC refresh,
// scouts, bursts) is divided by 2^shift (GENOS_LIGHTING_SC_BUDGET_SHIFT, head 3.w
// bits 9-11); the lists keep their layout.
uint lighting_sc_budget_shift() {
    return (uint(work.v[lighting_change_at() + 3u].w) >> 9u) & 7u;
}
// S72O (Anthony: squares still build up; a page must never shift as a unit):
// per-texel commit timing and ray strata, and the stored means rectified
// against their edge-aware neighbourhood. Flag 268435456 off.
uint lighting_world_mode();
// S72R: the GIBS MSME accumulator verbatim (lighting_cache.comp); flag
// 1073741824 the S59-S68 estimator (plain mean warm-up, young floor, anti-lag).
bool lighting_sc_msme() {
    return lighting_world_mode() == 3u && (lighting_sc_flags() & 1073741824u) == 0u;
}
bool lighting_sc_decorrelate() {
    return lighting_world_mode() == 3u && (lighting_sc_flags() & 268435456u) == 0u;
}
// Surface cache (mode 3, GENOS_LIGHTING_WORLD=3): the light cache's patches are the
// texels pixels read. A texel adds a new set of LIGHTING_CACHE_RAYS rays each time it is
// relit and keeps their running mean, up to cap sets; then it holds (converged)
// until the light changes or an object moves near it.
// Texels are one size everywhere, LIGHTING_SC_TEXEL (S57, Anthony 2026-10-12): the
// camera-distance bands (6.25 cm within 2.5 m, 12.5 cm to 8 m, 25 cm and up past
// it) are gone. Each band edge was a ring that followed the camera, and crossing
// it re-keyed texels, which restarted (grain) however their light was resampled.
// Neither Lumen's cards nor the papers surveyed key texel size to the eye;
// detail and effort come from the light: change-aware history and dormancy
// (GIBS), the relight budget and its order (Lumen), the read rule (SHaRC). The
// camera only decides what stays resident and what is relit first.
const float LIGHTING_SC_TEXEL = 0.125;
// Ray-hit patches (flag 256): one size too, LIGHTING_SC_HIT (was 25 cm doubling past
// 8 m from the eye).
const float LIGHTING_SC_HIT = 0.25;
// Sets a texel inside a change, or under a light that is changing, averages over
// (a short running mean on one fixed set: quick, and no flicker).
const float LIGHTING_SC_MOVING = 2.0;
// Sets a texel averages over while the light changes (lamps moving: a running
// mean over a window, the sets turning with the rounds).
const float LIGHTING_SC_LIGHT = 8.0;
// Sets a texel only rays read (not on screen) grows to.
const float LIGHTING_SC_HIT_CAP = 8.0;
// Flag 262144: caps of sets a texel takes past the cap before it holds.
const float LIGHTING_SC_HOLD = 4.0;
// Flag 32: a texel's share of its pixels against the probes, sets / (sets + this).
const float LIGHTING_SC_TRUST_SETS = 6.0;

float lighting_cell_size_d(float d);
float lighting_cell_size(vec3 pos) {
    return lighting_cell_size_d(length(pos - scene.eye.xyz));
}
float lighting_cell_size_d(float d) {
    float level = clamp(floor(log2(max(d / 8.0, 1.0))), 0.0, 6.0);
    // Surface cache (mode 3): one texel size, whatever the distance.
    if (lighting_world_mode() == 3u) {
        return LIGHTING_SC_TEXEL;
    }
    return 0.25 * exp2(level);
}

uint lighting_axis(vec3 n) {
    vec3 a = abs(n);
    if (a.x >= a.y && a.x >= a.z) {
        return n.x >= 0.0 ? 0u : 1u;
    }
    if (a.y >= a.z) {
        return n.y >= 0.0 ? 2u : 3u;
    }
    return n.z >= 0.0 ? 4u : 5u;
}

const uint LIGHTING_WORLD_AT = 3u * LIGHTING_CACHE_SLOTS;
// Surface cache estimator (flag 1048576), 2 vec4s per slot: short-term mean (rgb)
// and short-term luminance variance (w); then the variance-based blend reduction
// (x) and the inconsistency (y). Zero when the slot is claimed.
const uint LIGHTING_SC_EST_AT = 4u * LIGHTING_CACHE_SLOTS;
// Object id + 1 of a texel keyed in a mover's frame (0: a world texel).
const uint LIGHTING_SC_OBJ_AT = 6u * LIGHTING_CACHE_SLOTS;
// S63A sun/sky share (Anthony: day to night the outside ground kept daylight and
// shifted slowly). Per slot: x the share of its light that came from the suns
// and the sky (direct at ray hits, sky at misses, and that share of the cache
// light read at hits), y the global sun/sky light (lighting_global_light) when it was
// last relit. A sun or sky change scales only that share, at once, read or not:
// shown = v ((1 - x) + x now / y). Lamp-bounced light is untouched; rays then
// converge the shape (Lumen relights its surface cache's direct light every
// frame, the radiosity converges over frames).
const uint LIGHTING_SC_SUN_AT = 7u * LIGHTING_CACHE_SLOTS;
// S72A sparse fill per slot: x non-zero once promoted to its own rays.
const uint LIGHTING_SC_SPARSE_AT = 8u * LIGHTING_CACHE_SLOTS;
// Frame of a texel's last relight (0: never), beside its object id.
// S62E global light scale (Anthony: day to night, unread texels kept daytime
// light and faded as rays came in). The pixel's own direct light is already
// separate (G-buffer w, every frame); the cache holds the gathered light, which
// for sun and sky scales with them. Once a round the cache pass logs the global
// light (each sun's height times its brightest channel, plus the sky's: as
// the old v1 up_light) against the frame (a ring, LIGHTING_LS_RING rounds); a
// texel or WRC cell is shown, and its history carried, times
// (now + eps) / (then + eps), "then" being its last relight. An intensity change
// is instant everywhere, read or not; only the bounce's shape converges by rays
// (Lumen re-lights its surface cache's direct term each frame and lets the
// radiosity converge).
const uint LIGHTING_LS_AT = LIGHTING_SC_HIST_AT + 9000u;
const uint LIGHTING_LS_RING = 2048u;
const float LIGHTING_LS_EPS = 0.05;
const float LIGHTING_LS_MAX = 16.0;
// S62F: off. One scale for a texel's whole value also scales the light lamps
// bounced into it: the stress room at night (lamp-lit) went to half its light on
// the first frame (daynight check, first-frame error 0.49). Needs the cache's
// sun/sky share kept apart from the lamps' (queued). The ratio still drives the
// refresh of unread texels (lighting_compact.comp).
const bool LIGHTING_LS_RESCALE = false;
float lighting_global_light() {
    float s = max(max(scene.sky.r, scene.sky.g), scene.sky.b);
    uint n = scene.lamp_count;
    [[dont_unroll]] for (uint i = 0u; i < n; i++) {
        Lamp l = scene_lamp(i);
        if (l.pos.w > 0.5) {
            s += max(-normalize(l.pos.xyz).y, 0.0) * max(max(l.color.r, l.color.g), l.color.b);
        }
    }
    return s;
}
void lighting_ls_note() {
    uint f = lighting_frame();
    // S69B: a rise (more than twice the lowest of the last 16 frames logged).
    float cur = lighting_global_light();
    float low = 1.0e30;
    [[dont_unroll]] for (uint k = 1u; k <= 16u && k < f; k++) {
        uint at = LIGHTING_LS_AT + 2u * ((f - k) % LIGHTING_LS_RING);
        if (ckeys.key[at] == f - k) {
            low = min(low, uintBitsToFloat(ckeys.key[at + 1u]));
        }
    }
    if (low < 1.0e29 && cur + LIGHTING_LS_EPS > 2.0 * (low + LIGHTING_LS_EPS)) {
        ckeys.key[LIGHTING_LS_AT + 2u * LIGHTING_LS_RING + 1u] = f;
    }
    uint at = LIGHTING_LS_AT + 2u * (f % LIGHTING_LS_RING);
    ckeys.key[at] = f;
    ckeys.key[at + 1u] = floatBitsToUint(lighting_global_light());
    ckeys.key[LIGHTING_LS_AT + 2u * LIGHTING_LS_RING] = f;
}
#ifdef GENOS_SUN_SHADER
bool lighting_sun_small();
float lighting_sun_committed_light();
#endif
// The global sun/sky light as of the last round (-1 when unknown).
// A small sun step (GENOS_SUN_SHADER only) keeps the committed value, so one
// tick does not rescale every texel. The default compose does not compile this.
float lighting_ls_now() {
#ifdef GENOS_SUN_SHADER
    if (lighting_sun_small()) {
        return max(lighting_sun_committed_light(), 0.0);
    }
#endif
    uint g = ckeys.key[LIGHTING_LS_AT + 2u * LIGHTING_LS_RING];
    uint ga = LIGHTING_LS_AT + 2u * (g % LIGHTING_LS_RING);
    return (g != 0u && ckeys.key[ga] == g) ? uintBitsToFloat(ckeys.key[ga + 1u]) : -1.0;
}
// The ratio of the sun/sky light now to when slot was last relit.
float lighting_sc_sun_ratio(uint slot) {
    vec4 sa = cache.v[LIGHTING_SC_SUN_AT + slot];
    float now = lighting_ls_now();
    if (now < 0.0 || sa.y <= 0.0) {
        return 1.0;
    }
    return clamp((now + 1.0e-4) / (sa.y + 1.0e-4), 0.0, LIGHTING_LS_MAX);
}
// What slot's stored light is multiplied by to show it now.
float lighting_sc_sun_gain(uint slot) {
    float f = clamp(cache.v[LIGHTING_SC_SUN_AT + slot].x, 0.0, 1.0);
    return 1.0 - f + f * lighting_sc_sun_ratio(slot);
}
// The sun/sky share of the cache light last read at a ray hit (light pass).
float lighting_hit_sunfrac = 0.0;
float lighting_sc_sunfrac_now(uint slot) {
    float f = clamp(cache.v[LIGHTING_SC_SUN_AT + slot].x, 0.0, 1.0);
    float r = lighting_sc_sun_ratio(slot);
    float g = 1.0 - f + f * r;
    return g > 1.0e-6 ? f * r / g : 0.0;
}
// The ratio to carry light last relit at frame f to now (1 when unknown).
float lighting_ls_ratio(uint f) {
    uint at = LIGHTING_LS_AT + 2u * (f % LIGHTING_LS_RING);
    if (f == 0u || ckeys.key[at] != f) {
        return 1.0;
    }
    float then = uintBitsToFloat(ckeys.key[at + 1u]);
    uint g = ckeys.key[LIGHTING_LS_AT + 2u * LIGHTING_LS_RING];
    uint ga = LIGHTING_LS_AT + 2u * (g % LIGHTING_LS_RING);
    if (ckeys.key[ga] != g) {
        return 1.0;
    }
    float now = uintBitsToFloat(ckeys.key[ga + 1u]);
    return clamp((now + LIGHTING_LS_EPS) / (then + LIGHTING_LS_EPS), 0.0, LIGHTING_LS_MAX);
}
uint lighting_sc_updated(uint slot) {
    return floatBitsToUint(cache.v[LIGHTING_SC_OBJ_AT + slot].y);
}
// S69A (Anthony: night to day left hard black texels everywhere, cleaning up
// slowly). A texel relit at night holds night light; the sun/sky share rescale
// (S63A) cannot brighten it (share ~0, or a tiny value times a ratio capped at
// LIGHTING_LS_MAX), and the two-tier scheduler ranked it by its estimator's old
// change, so it waited in the trickle tier. The global sun/sky light rising more
// than LIGHTING_SC_STALE_UP times since its relight makes it stale: due now, ranked
// by that change, whole sets, and not shown (the pixel takes the probes, which
// carry this frame's light) until it is relit.
const float LIGHTING_SC_STALE_UP = 2.0;
bool lighting_sc_stale(uint slot);
float lighting_sc_global_change(uint slot) {
    float r = lighting_ls_ratio(lighting_sc_updated(slot));
    float d = max(r, 1.0 / max(r, 1.0e-3)) - 1.0;
    return lighting_sc_stale(slot) ? max(d, LIGHTING_SC_STALE_UP - 1.0) : d;
}
// S69B (Anthony: after S69A the black squares stayed only where the camera
// looked at the change): those texels were relit within frames of the jump,
// with rays lit before the new sun reached them, and their stamp said "day".
// lighting_ls_note marks a rise (the global light over twice the lowest of the 16
// frames before); a texel relit within LIGHTING_SC_STALE_WINDOW frames after a
// rise stays stale (due, not shown) and is relit again once the light held.
const uint LIGHTING_SC_STALE_WINDOW = 16u;
// Frame of the last rise (lighting_ls_note): the global light more than
// LIGHTING_SC_STALE_UP times the lowest of the window before it.
const uint LIGHTING_LS_RISE_AT = LIGHTING_LS_AT + 2u * LIGHTING_LS_RING + 1u;
bool lighting_sc_stale(uint slot) {
    uint u = lighting_sc_updated(slot);
    if (lighting_world_mode() != 3u || u == 0u) {
        return false;
    }
    uint rise = ckeys.key[LIGHTING_LS_RISE_AT];
    if (rise != 0u && int(u - rise) >= 0 && int(u - rise) <= int(LIGHTING_SC_STALE_WINDOW)) {
        return true;
    }
    return lighting_ls_ratio(u) > LIGHTING_SC_STALE_UP;
}
// Set while a lookup runs in a mover's frame: id + 1 salts the key.
float lighting_obj_salt = 0.0;

vec3 lighting_axis_vec(uint a) {
    vec3 v = vec3(0.0);
    v[a >> 1u] = (a & 1u) == 0u ? 1.0 : -1.0;
    return v;
}

// Key of the patch of the given size holding pos facing n; never 0.
uint lighting_cache_key_sized(vec3 pos, vec3 n, float size) {
    // World cells on: half a cell in from the surface, so a floor or wall on a cell
    // boundary falls in one cell, not either by rounding.
    vec3 inset = lighting_world_mode() != 0u ? 0.5 * lighting_axis_vec(lighting_axis(n)) : vec3(0.0);
    ivec3 c = ivec3(floor(pos / size - inset));
    uint h = lighting_hash(uint(c.x) * 73856093u ^ lighting_hash(uint(c.y) * 19349663u ^ lighting_hash(uint(c.z) * 83492791u)));
    h = lighting_hash(h ^ (lighting_axis(n) * 0x27d4eb2du) ^ (floatBitsToUint(size) * 0x165667b1u));
    if (lighting_obj_salt > 0.5) {
        h = lighting_hash(h ^ (uint(lighting_obj_salt) * 0x85ebca6bu));
    }
    return max(h, 1u);
}

// Key of the patch holding pos facing n; never 0.
uint lighting_cache_key(vec3 pos, vec3 n, out float size) {
    size = lighting_cell_size(pos);
    return lighting_cache_key_sized(pos, n, size);
}

// Surface cache: where a texel is lit from, the centre of its cell moved along
// its axis onto the surface through pos (so the texel's light does not depend on
// which reader claimed it first).
vec3 lighting_sc_centre(vec3 pos, vec3 n, float size) {
    uint ax = lighting_axis(n) >> 1u;
    vec3 inset = 0.5 * lighting_axis_vec(lighting_axis(n));
    vec3 c = (floor(pos / size - inset) + 0.5 + inset) * size;
    vec3 q = pos;
    uint a1 = (ax + 1u) % 3u;
    uint a2 = (ax + 2u) % 3u;
    q[a1] = c[a1];
    q[a2] = c[a2];
    q[ax] = pos[ax] - ((q[a1] - pos[a1]) * n[a1] + (q[a2] - pos[a2]) * n[a2]) / n[ax];
    return q;
}

// Slot of the patch for (pos, n), claiming a free one when `claim`; ~0u if none.
// The whole search window is checked for the key before anything is claimed: a
// freed slot can sit in front of a live patch's slot, and claiming it would make a
// second, unlit copy of that patch.
uint lighting_cache_find_sized(vec3 pos, vec3 n, bool claim, float size, vec3 at) {
    uint key = lighting_cache_key_sized(pos, n, size);
    uint start = lighting_hash(key ^ 0x9e3779b9u);
    uint free_at = LIGHTING_CACHE_SEARCH;
    [[dont_unroll]] for (uint i = 0u; i < LIGHTING_CACHE_SEARCH; i++) {
        uint k = ckeys.key[(start + i) & (LIGHTING_CACHE_SLOTS - 1u)];
        if (k == key) {
            uint found = (start + i) & (LIGHTING_CACHE_SLOTS - 1u);
            lighting_compose_cover(found);
            return found;
        }
        if (k == 0u && free_at == LIGHTING_CACHE_SEARCH) {
            free_at = i;
        }
    }
    if (!claim) {
        return ~0u;
    }
    [[dont_unroll]] for (uint i = free_at; i < LIGHTING_CACHE_SEARCH; i++) {
        uint slot = (start + i) & (LIGHTING_CACHE_SLOTS - 1u);
        uint prev = atomicCompSwap(ckeys.key[slot], 0u, key);
        if (prev == 0u) {
            atomicAdd(gstats.claimed, 1u);
            // Flag 16384 (surface cache): every texel and hit patch is lit from its
            // cell's centre on the claimer's plane, whoever claimed it, and never
            // moves (Lumen: card texels are placed by the geometry), so its light
            // does not depend on the camera's path. (Was: a quarter-cell depth key,
            // rejected 2026-10-11.)
            if (lighting_world_mode() == 3u && (lighting_sc_flags() & 16384u) != 0u) {
                at = lighting_sc_centre(pos, n, size);
            }
            cache.v[3u * slot] = vec4(at, uintBitsToFloat(lighting_frame()));
            cache.v[3u * slot + 1u] = vec4(n, size);
            cache.v[3u * slot + 2u] = vec4(0.0);
            cache.v[LIGHTING_WORLD_AT + slot] = vec4(length(at - lighting_sc_centre(pos, n, size)), 0.0, 0.0, 0.0);
            cache.v[LIGHTING_SC_EST_AT + 2u * slot] = vec4(0.0);
            cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u] = vec4(0.0);
            cache.v[LIGHTING_SC_SPARSE_AT + slot] = vec4(0.0);
            if (lighting_world_mode() == 3u) {
                cache.v[LIGHTING_SC_OBJ_AT + slot] = vec4(lighting_obj_salt, 0.0, 0.0, 0.0);
            }
            lighting_compose_cover(slot);
            return slot;
        }
        if (prev == key) {
            lighting_compose_cover(slot);
            return slot;
        }
    }
    return ~0u;
}

uint lighting_cache_find(vec3 pos, vec3 n, bool claim) {
    return lighting_cache_find_sized(pos, n, claim, lighting_cell_size(pos), pos);
}

// Surface cache: a reader at pos (a point on the texel's surface) moves the point
// the texel is lit from to itself when it is nearer the cell's centre, so a texel
// ends up lit from the surface point nearest its centre whoever claimed it first
// (the centre itself can lie inside a wall at a corner).
void lighting_sc_offer(uint slot, vec3 at, vec3 centre) {
    if (lighting_world_mode() != 3u) {
        return;
    }
    // Flag 524288 (camera only orders updates): the point still moves toward the
    // cell's centre (a fixed point of the geometry, as Lumen's card texels), but
    // the sets taken so far are kept: a camera move never restarts a texel.
    if ((lighting_sc_flags() & 16384u) != 0u) {
        return;
    }
    bool keep_sets = (lighting_sc_flags() & 524288u) != 0u;
    float d = length(at - centre);
    if (d < cache.v[LIGHTING_WORLD_AT + slot].x - 1.0e-4) {
        cache.v[LIGHTING_WORLD_AT + slot].x = d;
        if (keep_sets) {
            cache.v[3u * slot].xyz = at;
            return;
        }
        // Lit from far from here so far: its sets start again (value kept).
        float size = cache.v[3u * slot + 1u].w;
        if ((lighting_sc_flags() & 64u) == 0u && length(at - cache.v[3u * slot].xyz) > 0.3 * size) {
            float t = cache.v[3u * slot + 2u].w;
            if (t > 1.5) {
                atomicAdd(gstats.restarted, 1u);
            }
            cache.v[3u * slot + 2u].w = min(t, 1.0);
        }
        cache.v[3u * slot].xyz = at;
    }
}

// Surface cache: the lit texel twice the size holding pos (the first fill of a
// texel not lit yet); w 0 when there is none.
vec4 lighting_sc_parent(vec3 pos, vec3 n, float size) {
    [[dont_unroll]] for (uint l = 0u; l < 2u; l++) {
        size *= 2.0;
        // The WRC cell (25 cm) is claimed if missing and marked as a seed a
        // pixel needs (the reveal burst, lighting_compact.comp).
        bool seed = lighting_world_mode() == 3u && l == 0u && size > 0.2;
        uint s = lighting_cache_find_sized(pos, n, seed, size, pos);
        if (s != ~0u && seed) {
            cache.v[3u * s].w = uintBitsToFloat(lighting_frame());
            cache.v[LIGHTING_WORLD_AT + s].z = uintBitsToFloat(lighting_frame());
        }
        if (s != ~0u) {
            vec4 v = cache.v[3u * s + 2u];
            if (v.w > 0.5) {
                cache.v[3u * s].w = uintBitsToFloat(lighting_frame());
                return vec4(v.rgb, 1.0);
            }
        }
    }
    return vec4(0.0);
}

// Surface cache estimator (flag 1048576): a converged texel whose recent sets
// agree with its mean (inconsistency low) is dormant: the sweep relights it one
// round in LIGHTING_SC_DORMANT, enough to notice a change (GIBS 2021: converged surfels
// send only change-detection rays).
const uint LIGHTING_SC_DORMANT = 8u;
const float LIGHTING_SC_DORMANT_SETS = 128.0;
// Agreement a dormant texel needs (S59E: 0.3 -> 0.15 standard deviations).
const float LIGHTING_SC_DORMANT_SD = 0.15;
bool lighting_sc_dormant(uint slot) {
    vec4 b = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u];
    // Only once its mean holds LIGHTING_SC_DORMANT_SETS sets: a texel still warming
    // up keeps its full rate whatever its inconsistency says.
    return b.x > 0.0 && b.y < LIGHTING_SC_DORMANT_SD && b.w >= LIGHTING_SC_DORMANT_SETS;
}

// Surface cache, flag 524288: a new texel's first light (Lumen's "resample
// lighting on realloc", SurfaceCache.ResampleLighting): bilinear at its centre
// from the lit texels of the next size up (the 25 cm ray-hit patches, flag 256)
// or down, with the sample count they hold (to the cap), so it does not start
// from nothing. Since the size bands went (S57) this only fills texels of
// surfaces seen for the first time. w 0 when nothing lit covers it.
vec4 lighting_sc_shown(uint slot);
bool lighting_sc_sparse_on();
// S64B: the sun/sky share (as of now) of the last bilerp's sources.
float lighting_bilerp_sunfrac = 0.0;
vec4 lighting_sc_bilerp_cover(vec3 pos, vec3 n, float size, float cover);
vec4 lighting_sc_bilerp_sized(vec3 pos, vec3 n, float size) {
    vec4 r = lighting_sc_bilerp_cover(pos, n, size, 0.5);
    return r.w < 0.0 ? vec4(0.0) : r;
}
// Bilinear over the lit coplanar cells of one size; w their mean sample count,
// -1 when they cover less than `cover` of the footprint.
vec4 lighting_sc_bilerp_cover(vec3 pos, vec3 n, float size, float cover) {
    float fsum = 0.0;
    uint ax = lighting_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    vec2 uv = vec2(dot(pos, ta), dot(pos, tb)) / size - 0.5;
    vec2 g0 = floor(uv);
    vec2 f = uv - g0;
    vec3 sum = vec3(0.0);
    float count = 0.0;
    float wsum = 0.0;
    [[dont_unroll]] for (uint k = 0u; k < 4u; k++) {
        vec2 c = g0 + vec2(float(k & 1u), float(k >> 1u)) + 0.5;
        vec3 q = pos + ta * (c.x * size - dot(pos, ta)) + tb * (c.y * size - dot(pos, tb));
        float bil = ((k & 1u) == 1u ? f.x : 1.0 - f.x) * ((k >> 1u) == 1u ? f.y : 1.0 - f.y);
        uint s = lighting_cache_find_sized(q, n, false, size, q);
        if (s == ~0u || bil <= 0.0) {
            continue;
        }
        vec4 v = cache.v[3u * s + 2u];
        // Only cells on the same plane (a step or a ledge is another surface).
        if (abs(dot(cache.v[3u * s].xyz - pos, n)) > 0.1) {
            continue;
        }
        // S72K: a texel with no sets yet that shows its keys' interpolation
        // counts too (it used to drop out, the pixel took the other corners, and
        // it popped in with its first set).
        vec4 shown_s = vec4(0.0);
        if (v.w < 0.5 && lighting_world_mode() == 3u && lighting_sc_sparse_on()) {
            shown_s = lighting_sc_shown(s);
            if (shown_s.w > 0.5) {
                cache.v[3u * s].w = uintBitsToFloat(lighting_frame());
                sum += bil * shown_s.rgb;
                fsum += bil * lighting_sc_sunfrac_now(s);
                wsum += bil;
            }
            continue;
        }
        if (v.w > 0.5 && fract(v.w) < 0.1) {
            cache.v[3u * s].w = uintBitsToFloat(lighting_frame());
            sum += bil * lighting_sc_shown(s).rgb;
            fsum += bil * lighting_sc_sunfrac_now(s);
            // WRC samples are 16-ray quarters (S61B): four to a set.
            count += bil * floor(v.w) * (size > 0.2 && lighting_world_mode() == 3u ? 0.25 : 1.0);
            wsum += bil;
        }
    }
    // Most of the footprint lit: a texel at the edge of what was lit keeps the
    // light it has, but one mostly over unlit cells starts as new.
    if (wsum < cover || wsum <= 0.0) {
        return vec4(0.0, 0.0, 0.0, -1.0);
    }
    lighting_bilerp_sunfrac = fsum / wsum;
    return vec4(sum / wsum, count / wsum);
}
const float LIGHTING_SC_RESAMPLE_SETS = 4.0;
bool lighting_near_change(vec3 p);
bool lighting_sc_light_quiet();
float lighting_seed_sunfrac = 0.0;
// S72N: first fill from the coarser sizes (2x, then 4x), bilinear over the lit
// coplanar cells there (any coverage), so it has no cell-shaped steps. The 25 cm
// cell holding pos is still claimed as a seed a pixel needs (reveal burst).
vec4 lighting_sc_parent_bilerp(vec3 pos, vec3 n, float size) {
    float s2 = size * 2.0;
    if (s2 > 0.2) {
        uint c = lighting_cache_find_sized(pos, n, true, s2, pos);
        if (c != ~0u) {
            cache.v[3u * c].w = uintBitsToFloat(lighting_frame());
            cache.v[LIGHTING_WORLD_AT + c].z = uintBitsToFloat(lighting_frame());
        }
    }
    [[dont_unroll]] for (uint l = 0u; l < 2u; l++) {
        vec4 r = lighting_sc_bilerp_cover(pos, n, s2, 1.0e-3);
        if (r.w > -0.5) {
            return vec4(r.rgb, 1.0);
        }
        s2 *= 2.0;
    }
    return vec4(0.0);
}
vec4 lighting_sc_resample(vec3 pos, vec3 n, float size, uint slot) {
    lighting_seed_sunfrac = 0.0;
    vec4 best = vec4(0.0);
    [[dont_unroll]] for (uint l = 0u; l < 4u; l++) {
        float s2 = size * ((l & 1u) == 0u ? 2.0 : 0.5) * ((l & 2u) == 0u ? 1.0 : ((l & 1u) == 0u ? 2.0 : 0.5));
        vec4 r = lighting_sc_bilerp_sized(pos, n, s2);
        // The next size up or down with the most samples; the sizes beyond only
        // when neither has light.
        if (r.w > best.w) {
            best = r;
            lighting_seed_sunfrac = lighting_bilerp_sunfrac;
        }
        if (l == 1u && best.w > 0.5) {
            break;
        }
    }
    if (best.w < 0.5) {
        return vec4(0.0);
    }
    // Near a change (an object moved, came or went) or under changing light the
    // light it starts from may be stale (a box's shadow where it stood): a short
    // count, so its own sets replace it fast.
    float cap = min(lighting_sc_cap(slot), LIGHTING_WRC_SEED_SETS);
    bool urgent = ((pc.params.z >> 5u) & 1u) != 0u;
    if ((urgent && lighting_near_change(pos)) || !lighting_sc_light_quiet()) {
        cap = min(cap, LIGHTING_SC_RESAMPLE_SETS);
    }
    return vec4(best.rgb, max(min(floor(best.w), cap), 1.0));
}

// How far around a change a patch is relit first (lighting_compact.comp), metres.
const float LIGHTING_NEAR_CHANGE = 1.0;

// Movers (lighting_gpu.rs lighting_movers): occluders that moved in the last
// LIGHTING_MOVER_KEEP frames, listed after the changes, four vec4 each: bounding
// sphere; centre and id + 1; half extents (x, y, z); cosine and sine of the yaw.
// Count in head w above 200000, the object-local texel switch at 100000.
const uint LIGHTING_MOVER_VEC4 = 4u;
uint lighting_movers_first() {
    vec4 head = work.v[lighting_change_at()];
    return head.y > 0.5 ? 0u : min(uint(head.x + 0.5), LIGHTING_CHANGES);
}
uint lighting_movers_count() {
    uint n = uint(work.v[lighting_change_at()].w) / 200000u;
    return min(n, (LIGHTING_CHANGES - lighting_movers_first()) / LIGHTING_MOVER_VEC4);
}
vec4 lighting_mover(uint m, uint k) {
    return work.v[lighting_change_at() + LIGHTING_CHANGE_HEAD + lighting_movers_first() + LIGHTING_MOVER_VEC4 * m + k];
}
// Object-local texels (GIBS 2021: surfels keep a transform id and a local
// position): GENOS_LIGHTING_SC_LOCAL=1.
bool lighting_sc_local_on() {
    return lighting_world_mode() == 3u && ((uint(work.v[lighting_change_at()].w) / 100000u) & 1u) != 0u;
}

// A point inside the bounds of a mover.
bool lighting_in_change(vec3 p) {
    uint n = lighting_movers_count();
    [[dont_unroll]] for (uint m = 0u; m < n; m++) {
        vec4 b = lighting_mover(m, 0u);
        vec3 d = p - b.xyz;
        if (dot(d, d) < b.w * b.w) {
            return true;
        }
    }
    return false;
}

// Mover m's frame: world point to local and back. An occluder turns about +Y
// (scene_rays.glsl occ_local and occ_world); a traced mesh instance (id + 1 past
// 10000) uses its mesh field rows (world to object) and their inverse.
bool lighting_mover_mesh(uint m, out MfInst mi) {
    float idp = lighting_mover(m, 1u).w;
    if (idp < 10000.5) {
        return false;
    }
    mi = mf_inst(uint(idp + 0.5) - 10001u);
    return true;
}
vec3 lighting_mover_to_local(uint m, vec3 p, bool dir) {
    MfInst mi;
    if (lighting_mover_mesh(m, mi)) {
        return dir ? normalize(mf_dir_to_object(mi, p)) : mf_to_object(mi, p);
    }
    vec4 c = lighting_mover(m, 1u);
    vec4 r = lighting_mover(m, 3u);
    vec3 d = dir ? p : p - c.xyz;
    return vec3(r.x * d.x - r.y * d.z, d.y, r.y * d.x + r.x * d.z);
}
vec3 lighting_mover_to_world(uint m, vec3 l, bool dir) {
    MfInst mi;
    if (lighting_mover_mesh(m, mi)) {
        mat3 a = transpose(mat3(mi.r0.xyz, mi.r1.xyz, mi.r2.xyz));
        mat3 inv = inverse(a);
        if (dir) {
            return normalize(transpose(a) * l);
        }
        return inv * (l - vec3(mi.r0.w, mi.r1.w, mi.r2.w));
    }
    vec4 c = lighting_mover(m, 1u);
    vec4 r = lighting_mover(m, 3u);
    vec3 d = vec3(r.x * l.x + r.y * l.z, l.y, -r.y * l.x + r.x * l.z);
    return dir ? d : d + c.xyz;
}
// The mover whose surface pos (facing n) lies on, or ~0. An occluder: within its
// local box plus 3 cm, on the face its normal points out of (not the floor under
// or beside it). A mesh: within 3 cm of its distance field.
uint lighting_mover_on(vec3 pos, vec3 n) {
    if (!lighting_sc_local_on()) {
        return ~0u;
    }
    uint cnt = lighting_movers_count();
    [[dont_unroll]] for (uint m = 0u; m < cnt; m++) {
        vec4 b = lighting_mover(m, 0u);
        vec3 d = pos - b.xyz;
        if (dot(d, d) >= b.w * b.w) {
            continue;
        }
        MfInst mi;
        if (lighting_mover_mesh(m, mi)) {
            if (any(lessThan(pos, mi.lo - 0.03)) || any(greaterThan(pos, mi.hi + 0.03))) {
                continue;
            }
            float dist = mf_sample(mi, mf_to_object(mi, pos)).x * mi.dscale;
            if (abs(dist) < 0.04) {
                return m;
            }
            continue;
        }
        vec3 l = lighting_mover_to_local(m, pos, false);
        vec3 e = lighting_mover(m, 2u).xyz;
        if (any(greaterThan(abs(l), e + 0.03))) {
            continue;
        }
        vec3 ln = lighting_mover_to_local(m, n, true);
        vec3 f = abs(l) - e;
        uint k = f.x > f.y ? (f.x > f.z ? 0u : 2u) : (f.y > f.z ? 1u : 2u);
        if (sign(l[k]) * ln[k] > 0.5) {
            return m;
        }
    }
    return ~0u;
}
// Mover list position of the object with id + 1 = idp, or ~0 (it has rested and
// left the list).
uint lighting_mover_by_id(float idp) {
    uint n = lighting_movers_count();
    [[dont_unroll]] for (uint m = 0u; m < n; m++) {
        if (abs(lighting_mover(m, 1u).w - idp) < 0.5) {
            return m;
        }
    }
    return ~0u;
}
// Texel slot's surface point and normal in the world now: a texel of a moving
// object stores them in the object's frame (LIGHTING_SC_OBJ_AT holds id + 1). False
// for an object texel whose object is no longer listed.
bool lighting_sc_texel_world(uint slot, out vec3 p, out vec3 n) {
    p = cache.v[3u * slot].xyz;
    n = cache.v[3u * slot + 1u].xyz;
    float idp = cache.v[LIGHTING_SC_OBJ_AT + slot].x;
    if (idp < 0.5) {
        return true;
    }
    uint m = lighting_mover_by_id(idp);
    if (m == ~0u) {
        return false;
    }
    p = lighting_mover_to_world(m, p, false);
    n = lighting_mover_to_world(m, n, true);
    return true;
}
vec3 lighting_sc_texel_pos(uint slot) {
    vec3 p;
    vec3 n;
    lighting_sc_texel_world(slot, p, n);
    return p;
}

bool lighting_near_change(vec3 p) {
    vec4 head = work.v[lighting_change_at()];
    uint changes = head.y > 0.5 ? 0u : min(uint(head.x + 0.5), LIGHTING_CHANGES);
    [[dont_unroll]] for (uint c = 0u; c < changes; c++) {
        vec4 b = work.v[lighting_change_at() + LIGHTING_CHANGE_HEAD + c];
        vec3 d = p - b.xyz;
        float r = b.w + LIGHTING_NEAR_CHANGE;
        if (dot(d, d) < r * r) {
            return true;
        }
    }
    return false;
}

// Surface cache: the light has held still long enough (params.z bit 10, which
// means young-only rounds in the other modes).
bool lighting_sc_light_quiet() {
    return ((pc.params.z >> 10u) & 1u) != 0u;
}

// Surface cache: sets the texel in slot has averaged going into this relight
// (its running mean weighs the new set 1 / (count + 1)). Inside a change or
// under changing light, at most LIGHTING_SC_MOVING.
// S62C (Anthony: a change anywhere held every texel at low detail): only a
// local change shortens history: a mover nearby (LIGHTING_SC_MOVING), or a lamp that
// reaches the texel moved since its last relight (the light window). Light that
// changes everywhere a little (the sun turning, any box moving somewhere) no
// longer does: the estimator tracks it (lighting_cache.comp, a floor on its blend
// while the light changes, LIGHTING_SC_CHANGE_ALPHA).
bool lighting_wrc_lamp_changed(vec3 p, uint updated);
// The cache pass sets this to the texel's last relight before it stamps the new
// one (~0u: read the stamp).
uint lighting_sc_upd_override = ~0u;
bool lighting_sc_local_change(uint slot, out bool mover) {
    bool urgent = ((pc.params.z >> 5u) & 1u) != 0u;
    vec3 p = lighting_sc_texel_pos(slot);
    mover = urgent && lighting_near_change(p);
    if (mover) {
        return true;
    }
    // S65B: a moved lamp no longer counts as a local change here (it shortened
    // the history of every texel in its range, so a lamp moving slowly kept a
    // whole room young): the texel is due (lighting_sc_lamp_touched) and its
    // estimator measures how much its light actually changed.
    return false;
}
// The lamps' change at slot relative to the light it shows (0 when none).
float lighting_lamp_delta(vec3 p, uint updated);
float lighting_sc_lamp_change(uint slot) {
    if (lighting_sc_light_quiet()) {
        return 0.0;
    }
    uint upd = lighting_sc_upd_override != ~0u ? lighting_sc_upd_override : lighting_sc_updated(slot);
    float d = lighting_lamp_delta(lighting_sc_texel_pos(slot), upd);
    if (d <= 0.0) {
        return 0.0;
    }
    float lum = dot(cache.v[3u * slot + 2u].rgb, vec3(0.2126, 0.7152, 0.0722));
    // Direct light reaching the texel is roughly pi times its bounce share;
    // the ratio is what matters.
    return d / (lum + 0.02);
}
bool lighting_sc_lamp_touched(uint slot) {
    uint upd = lighting_sc_upd_override != ~0u ? lighting_sc_upd_override : lighting_sc_updated(slot);
    return !lighting_sc_light_quiet() && lighting_wrc_lamp_changed(lighting_sc_texel_pos(slot), upd);
}
bool lighting_sc_asvgf_on();
float lighting_sc_count(uint slot) {
    float t = floor(max(cache.v[3u * slot + 2u].w, 0.0));
    if (lighting_world_mode() == 3u && cache.v[3u * slot + 1u].w > 0.2) {
        // S62D: WRC cells keep their history whatever the light does (they
        // track it with an EMA floor, lighting_cache.comp).
        return t;
    }
    bool mover;
    // S76: under A-SVGF a nearby change only nominates a check; the history stays.
    if (!lighting_sc_asvgf_on() && lighting_sc_local_change(slot, mover)) {
        t = min(t, mover ? LIGHTING_SC_MOVING : lighting_sc_light_window());
    }
    return t;
}
// Blend floor of a texel's long-term mean while the light changes anywhere
// (an exponential moving average that keeps up; Lumen's surface cache and
// radiance cache keep a short temporal accumulation the same way).
const float LIGHTING_SC_CHANGE_ALPHA = 1.0 / 24.0;
// WRC: per 16-ray sample (a refresh adds 4): 1/32 while the light changes
// anywhere (1/8 a set), 1/8 near a mover or a moved lamp (1/2 a set).
const float LIGHTING_WRC_CHANGE_ALPHA = 1.0 / 32.0;
const float LIGHTING_WRC_LOCAL_ALPHA = 1.0 / 8.0;

float lighting_sc_cap(uint slot) {
    if (lighting_world_mode() == 3u && cache.v[3u * slot + 1u].w > 0.2) {
        return LIGHTING_WRC_CAP;
    }
    float cap = max(lighting_world_cap(), 1.0);
    if ((lighting_sc_flags() & 16u) != 0u && cache.v[3u * slot + 1u].w < 0.1) {
        cap *= 2.0;
    }
    return cap;
}

// Sets a texel averages while the light changes: LIGHTING_SC_LIGHT, or 0 (each relight
// replaces it) with flag 128.
float lighting_sc_light_window() {
    return (lighting_sc_flags() & (128u | 1024u)) != 0u ? 0.0 : LIGHTING_SC_LIGHT;
}
// Cell of the light cache a ray hit reads: flag 256 keeps the 25 cm patches of the
// other modes for hits (relit fast), the fine texels for pixels.
// Flag 2048: a surface not facing along an axis (a turned box) has no texels of
// its own; it keeps the 25 cm patches of the other modes and the probes.
bool lighting_sc_slanted(vec3 n) {
    vec3 a = abs(n);
    return lighting_world_mode() == 3u && (lighting_sc_flags() & 2048u) != 0u && max(a.x, max(a.y, a.z)) < 0.999;
}
float lighting_hit_size_n(vec3 pos, vec3 n) {
    if (lighting_sc_slanted(n)) {
        return LIGHTING_SC_HIT;
    }
    return lighting_hit_size(pos);
}
float lighting_hit_size(vec3 pos) {
    if (lighting_world_mode() == 3u && (lighting_sc_flags() & 256u) != 0u) {
        return LIGHTING_SC_HIT;
    }
    return lighting_cell_size(pos);
}

// Hash a texel's rays hang on: the set it is on (a new set each relight up to the
// cap, so the mean takes new rays; the same set while it relights under change).
uint lighting_sc_ray_key(uint slot, uint key) {
    if (lighting_world_mode() != 3u) {
        return key;
    }
    float t = lighting_sc_count(slot);
    uint set = uint(t);
    if (!lighting_sc_light_quiet() && t >= lighting_sc_light_window() - 0.5 && (lighting_sc_flags() & 1024u) == 0u) {
        // Under changing light the count holds, so the set turns with the rounds.
        set += lighting_round();
    } else if ((lighting_sc_flags() & 8192u) != 0u && t >= lighting_sc_cap(slot) - 0.5) {
        // Relit at the cap (flag 8192): a new set each time, not set 0 over again
        // (one set's noise would be all the running mean ends up with).
        set += lighting_round() * 7919u + 1u;
    }
    set %= uint(lighting_sc_cap(slot));
    // S72O: after the second pass (flag 2) drops a texel to a quarter of its
    // sets, the sets it takes again are new rays, not the ones it already has.
    uint pass = cache.v[LIGHTING_WORLD_AT + slot].w > 0.5 && lighting_sc_decorrelate() ? 0x7feb352du : 0u;
    return lighting_hash(key ^ (set * 0x2545f491u + 0x68e31da4u) ^ pass);
}

// Surface cache: what a texel shows while it has few sets (its first fill, then
// refining): the count-weighted mean of the texels around it on the same plane, a
// Gaussian of LIGHTING_SC_POOL / sqrt(sets + 1) cells (at most 2), so a new texel
// starts from its lit neighbours and its own noise fades as its sets come in.
// Kept in the world slot's y as rgb9e5 (x: lighting_sc_offer's distance; z: the frame
// a pixel last read the texel); 0 when not pooled.
const float LIGHTING_SC_POOL = 2.4;
const float LIGHTING_SC_POOL_FULL = 24.0;
// S62A denoise (SVGF, Schied et al. 2017: variance-guided, edge-stopping; in
// surface space, so it is anchored to the world and cannot swim): from
// LIGHTING_DN_MIN_SETS sets on, the pool's radius follows the texel's relative noise
// (the GIBS estimator's per-set variance over its sets, sqrt(var / n) / mean):
// sigma = LIGHTING_DN_SIGMA * noise / target, 0.4 to 2 cells; each neighbour also
// weighs exp(-|dL| / (LIGHTING_DN_LUM * the noise in luminance)), so shadow edges and
// contact darkening (real luminance steps) stop it, noise does not.
// Perceptual stop rule (lighting_sc_settled): a texel with at least LIGHTING_DN_MIN_SETS
// sets whose relative noise is under LIGHTING_DN_STOP (looser in the dark: under
// LIGHTING_DN_DARK luminance the target grows as LIGHTING_DN_DARK / L, at most 4x)
// counts as settled.
const float LIGHTING_DN_MIN_SETS = 8.0;
const float LIGHTING_DN_STOP = 0.015;
const float LIGHTING_DN_DARK = 0.05;
const float LIGHTING_DN_SIGMA = 0.6;
const float LIGHTING_DN_LUM = 3.0;
// Pool radius under changing light (cells; under 0.9 the kernel is 3 x 3).
const float LIGHTING_DN_CHANGE_SIGMA = 0.7;
// Relative noise of slot's mean, and its luminance noise (w); x < 0 when the
// estimator is not running (new, or the light changing).
vec2 lighting_sc_noise(uint slot) {
    vec4 a = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
    vec4 b = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u];
    if (!(b.x > 0.0) || !(b.w > 0.5)) {
        return vec2(-1.0, 0.0);
    }
    float lum = dot(cache.v[3u * slot + 2u].rgb, vec3(0.2126, 0.7152, 0.0722));
    float sd = sqrt(max(a.w, 0.0) / max(b.w, 1.0));
    return vec2(sd / max(lum, 1.0e-3), sd);
}
float lighting_dn_target(uint slot) {
    float lum = dot(cache.v[3u * slot + 2u].rgb, vec3(0.2126, 0.7152, 0.0722));
    return LIGHTING_DN_STOP * clamp(LIGHTING_DN_DARK / max(lum, 1.0e-4), 1.0, 4.0);
}
// S72L wide regularisation (Anthony: squares compound as each region converges
// to its own noisy mean; anchor convergence to the wider area). The shown value
// is the wide same-plane mean (16 taps 2-16 cells out, weighed by their sets)
// plus the texel's own difference shrunk by its significance: d^2 / (d^2 + (K
// sigma)^2), sigma the standard error of its mean (the estimator's per-set
// variance / sets; 30 % a set when it is not running). Noise-level
// differences stay with the neighbourhood; a real step (a shadow edge, a
// contact) is many sigma and passes; as sets come in sigma falls and the
// texel's own detail comes through. Flag 33554432 turns it off.
const float LIGHTING_SC_REG_K = 2.0;
// S72O: edge-aware and connected (Anthony: a-trous / pull-push over the stored
// values, plane, normal and visibility aware, stopping at real edges). Four
// directions a texel (axis or diagonal by its hash, so no grid shows), each
// walked out 2, 4, 8, 16 cells (a-trous steps); a direction stops at the first
// cell missing or off the plane (a wall, a gap: what is past it is not seen
// from here) or across a significant step (its luminance more than
// LIGHTING_SC_REG_EDGE standard errors from this texel's: a shadow edge or a
// light gradient). Taps weigh by their sets and a Gaussian in standard errors.
const float LIGHTING_SC_REG_EDGE = 4.0;
vec3 lighting_sc_regularise(uint slot, vec3 value) {
    if (lighting_world_mode() != 3u || (lighting_sc_flags() & 33554432u) != 0u) {
        return value;
    }
    const vec3 lw = vec3(0.2126, 0.7152, 0.0722);
    vec4 own = cache.v[3u * slot + 2u];
    float t = floor(max(own.w, 0.0));
    if (t < 0.5) {
        return value;
    }
    vec3 p = cache.v[3u * slot].xyz;
    vec4 nb = cache.v[3u * slot + 1u];
    vec3 n = nb.xyz;
    float size = nb.w;
    uint ax = lighting_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    vec4 ea = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
    vec4 eb = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u];
    float lum = max(dot(own.rgb, lw), 1.0e-4);
    float var = eb.x > 0.0 ? max(ea.w, 0.0) : pow(0.3 * lum, 2.0);
    float se2 = var / t;
    bool edge_aware = lighting_sc_decorrelate();
    float vlum = dot(value, lw);
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    float rot = edge_aware ? 0.7853982 * float(lighting_hash(lighting_sched(slot) * 0x27d4eb2du) & 1u) : 0.0;
    [[dont_unroll]] for (uint d = 0u; d < 4u; d++) {
        [[dont_unroll]] for (uint l = 0u; l < 4u; l++) {
            uint i = l * 4u + d;
            float r = float(2u << l);
            float a = edge_aware ? 1.5707963 * float(d) + rot : 1.5707963 * float(i & 3u) + 0.7853982 * float((i >> 2u) & 1u);
            vec3 q = p + (cos(a) * ta + sin(a) * tb) * r * size;
            uint s2 = lighting_cache_find_sized(q, n, false, size, p);
            if (s2 == ~0u || abs(dot(cache.v[3u * s2].xyz - p, n)) > 0.5 * size) {
                if (edge_aware) {
                    break;
                }
                continue;
            }
            vec4 v = cache.v[3u * s2 + 2u];
            float c = min(floor(max(v.w, 0.0)), 64.0);
            if (c < 0.5) {
                continue;
            }
            float w = c;
            if (edge_aware) {
                // Standard error of the difference: this texel's and the tap's
                // (same per-set variance, its sets).
                float sd2 = se2 + var / c + 1.0e-10;
                float dl = dot(v.rgb, lw) - vlum;
                float z2 = dl * dl / sd2;
                if (z2 > LIGHTING_SC_REG_EDGE * LIGHTING_SC_REG_EDGE) {
                    break;
                }
                w *= exp(-0.5 * z2 / 4.0);
            }
            sum += w * v.rgb;
            wsum += w;
        }
    }
    if (wsum < 4.0) {
        return value;
    }
    vec3 mu = sum / wsum;
    vec3 d = value - mu;
    float dl = dot(d, lw);
    float s = dl * dl / (dl * dl + LIGHTING_SC_REG_K * LIGHTING_SC_REG_K * se2 + 1.0e-12);
    return max(mu + d * s, vec3(0.0));
}
// S72O history rectification (Anthony: a page's early noise locks in and the
// squares drift): the stored mean itself moves toward its regularised value
// (the edge-aware neighbourhood plus its own significant difference) by one
// set's worth, 1 / (sets + 1), while it is short of 64 sets. Noise-level
// offsets a region took early are pulled to the neighbourhood instead of being
// averaged down at 1 / n; real steps are many standard errors and stay.
void lighting_sc_rectify(uint slot, vec3 reg) {
    vec4 own = cache.v[3u * slot + 2u];
    float t = floor(max(own.w, 0.0));
    if (!lighting_sc_decorrelate() || t < 0.5 || t >= 64.0 || fract(own.w) > 0.1 || cache.v[3u * slot + 1u].w > 0.2) {
        return;
    }
    cache.v[3u * slot + 2u].rgb = mix(own.rgb, reg, 1.0 / (t + 1.0));
}
// S72P two layers (Anthony 13:48): layer 1 is the traced data (each texel's
// own mean, the WRC), never shown as is; layer 2, shown, is a field rebuilt
// from layer 1 on each surface (irradiance caching / DDGI-style interpolation in
// surface space), kept in WORLD_AT .y (rgb9e5) with its support in SPARSE_AT .w
// (sets, negative when an edge stopped it within 4 cells: detail). Per texel:
// eight directions (turned 22.5 degrees on half the texels, by hash), each walked
// out 1, 2, 4, 8, 16 cells (a-trous steps). Only the same plane (the key holds
// the normal; plane distance under half a texel) and mutually visible: a
// direction stops at the first cell missing or off the plane (a wall, a gap).
// It also stops at a real step: from the last tap it took, or from where it
// started, more than LIGHTING_SC_L2_Z standard errors AND LIGHTING_SC_L2_REL
// of the light (shadow edges, contacts, creases stay sharp), so the radius
// shrinks where the light changes steeply and widens on flat light. Taps weigh
// by their sets (to 64), r (the ring's area, a-trous) and a Gaussian of
// LIGHTING_SC_L2_SIGMA cells; a stale tap (lit under change) a quarter.
// Flag 536870912: off (the S62A pool and S72L/O regularisation).
const float LIGHTING_SC_L2_Z = 3.0;
const float LIGHTING_SC_L2_REL = 0.12;
const float LIGHTING_SC_L2_SIGMA = 6.0;
const float LIGHTING_SC_L2_FLAT = 96.0;
bool lighting_sc_layer2() {
    return lighting_world_mode() == 3u && (lighting_sc_flags() & 536870912u) == 0u;
}
float lighting_sc_l2_var(uint s, float lum) {
    vec4 ea = cache.v[LIGHTING_SC_EST_AT + 2u * s];
    vec4 eb = cache.v[LIGHTING_SC_EST_AT + 2u * s + 1u];
    return eb.x > 0.0 ? max(ea.w, 1.0e-10) : pow(0.3 * lum, 2.0) + 1.0e-10;
}
void lighting_sc_recon(uint slot) {
    const vec3 lw = vec3(0.2126, 0.7152, 0.0722);
    vec4 own = cache.v[3u * slot + 2u];
    float t = floor(max(own.w, 0.0));
    vec3 p = cache.v[3u * slot].xyz;
    vec4 nb = cache.v[3u * slot + 1u];
    vec3 n = nb.xyz;
    float size = nb.w;
    uint ax = lighting_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    float support = 0.0;
    float ref = -1.0;
    float ref_se2 = 0.0;
    if (t >= 0.5) {
        float c0 = min(t, 64.0) * (fract(own.w) > 0.1 ? 0.25 : 1.0);
        sum = c0 * own.rgb;
        wsum = c0;
        support = c0;
        ref = dot(own.rgb, lw);
        ref_se2 = lighting_sc_l2_var(slot, ref) / t;
    }
    bool edge = false;
    float rot = 0.3926991 * float(lighting_hash(lighting_sched(slot) * 0x27d4eb2du) & 1u);
    [[dont_unroll]] for (uint d = 0u; d < 8u; d++) {
        float a = 0.7853982 * float(d) + rot;
        vec3 dir = cos(a) * ta + sin(a) * tb;
        float prev = ref;
        float prev_se2 = ref_se2;
        float trans = 1.0;
        [[dont_unroll]] for (uint l = 0u; l < 5u; l++) {
            float r = float(1u << l);
            vec3 q = p + dir * r * size;
            uint s2 = lighting_cache_find_sized(q, n, false, size, p);
            if (s2 == ~0u || abs(dot(cache.v[3u * s2].xyz - p, n)) > 0.5 * size) {
                break;
            }
            vec4 v = cache.v[3u * s2 + 2u];
            float c = min(floor(max(v.w, 0.0)), 64.0);
            if (c < 0.5) {
                continue;
            }
            float L = dot(v.rgb, lw);
            float se2 = lighting_sc_l2_var(s2, L) / c;
            // S72Q: the stops are smooth (a hard break flipped a far tap's
            // whole weight in or out as noise crossed the threshold: pops). A
            // step from the last tap scales what the direction passes on
            // (transmittance); the difference from the start scales this tap.
            float w0 = 1.0;
            if (prev >= 0.0) {
                float dl = L - prev;
                float x = abs(dl) / max(LIGHTING_SC_L2_Z * sqrt(se2 + prev_se2), LIGHTING_SC_L2_REL * max(L, prev) + 1.0e-4);
                trans *= exp(-x * x);
            }
            if (ref >= 0.0) {
                float d0 = L - ref;
                float x0 = abs(d0) / max((LIGHTING_SC_L2_Z + 1.0) * sqrt(se2 + ref_se2), 2.0 * LIGHTING_SC_L2_REL * max(L, ref) + 1.0e-4);
                w0 = exp(-x0 * x0);
            } else {
                // A texel with no sets yet: the first tap it reaches is its reference.
                ref = L;
                ref_se2 = se2;
            }
            prev = L;
            prev_se2 = se2;
            if (r <= 4.0 && trans * w0 < 0.5) {
                edge = true;
            }
            float g = exp(-0.5 * r * r / (LIGHTING_SC_L2_SIGMA * LIGHTING_SC_L2_SIGMA));
            float cw = c * (fract(v.w) > 0.1 ? 0.25 : 1.0) * g * trans * w0;
            if (trans < 0.01) {
                break;
            }
            sum += cw * r * v.rgb;
            wsum += cw * r;
            support += cw;
        }
    }
    cache.v[LIGHTING_WORLD_AT + slot].y = uintBitsToFloat(lighting_pack_rgb9e5(wsum > 0.0 ? sum / wsum : vec3(0.0)));
    cache.v[LIGHTING_SC_SPARSE_AT + slot].w = edge ? -support : support;
}
bool lighting_sc_svgf();
void lighting_sc_pool(uint slot) {
    vec4 own = cache.v[3u * slot + 2u];
    // A texel in a mover's frame pools with its neighbours in that frame: its
    // position and normal are stored there, and the same salt finds them.
    lighting_obj_salt = cache.v[LIGHTING_SC_OBJ_AT + slot].x > 0.5 ? cache.v[LIGHTING_SC_OBJ_AT + slot].x : 0.0;
    float t = floor(max(own.w, 0.0));
    if (lighting_sc_svgf()) {
        // S72V: layer 2 is the SVGF stages (lighting_atrous.comp).
        lighting_obj_salt = 0.0;
        return;
    }
    if (lighting_sc_layer2() && cache.v[3u * slot + 1u].w <= 0.2) {
        // S72P: layer 2 for texels a pixel showed in the last half second.
        uint shown = floatBitsToUint(cache.v[LIGHTING_WORLD_AT + slot].z);
        if (lighting_sc_pixel_fresh(shown, 30)) {
            lighting_sc_recon(slot);
        }
        lighting_obj_salt = 0.0;
        return;
    }
    bool floor_on = (lighting_sc_flags() & 8u) != 0u;
    if (t >= LIGHTING_SC_POOL_FULL && !floor_on) {
        vec3 rg = lighting_sc_regularise(slot, own.rgb);
        lighting_sc_rectify(slot, rg);
        cache.v[LIGHTING_WORLD_AT + slot].y = uintBitsToFloat(lighting_pack_rgb9e5(rg));
        lighting_obj_salt = 0.0;
        return;
    }
    vec3 p = cache.v[3u * slot].xyz;
    vec4 nb = cache.v[3u * slot + 1u];
    vec3 n = nb.xyz;
    float size = nb.w;
    float sigma = min(LIGHTING_SC_POOL / sqrt(t + 1.0), 2.0);
    if (floor_on) {
        sigma = max(sigma, 0.6);
    }
    // S62A: variance-guided radius and luminance edge stop once warmed up.
    vec2 nz = lighting_sc_noise(slot);
    bool guided = t >= LIGHTING_DN_MIN_SETS && nz.x >= 0.0;
    float own_lum = dot(own.rgb, vec3(0.2126, 0.7152, 0.0722));
    if (guided) {
        sigma = clamp(LIGHTING_DN_SIGMA * nz.x / lighting_dn_target(slot), 0.4, 2.0);
    }
    // S62B: under changing light the history is short and stays short, so the
    // pool keeps to the nearest texels (detail from the texels' own sets).
    // S67A: only where the light changes locally (a mover, or a moved lamp's
    // delta above the trickle threshold); the sun or a far lamp moving no
    // longer narrows every texel's pool.
    bool mover_p;
    if (lighting_sc_local_change(slot, mover_p) || LIGHTING_SC_LAMP_K * lighting_sc_lamp_change(slot) >= LIGHTING_SC_TRICKLE_THR) {
        sigma = min(sigma, LIGHTING_DN_CHANGE_SIGMA);
    }
    uint ax = lighting_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    int r = sigma > 0.9 ? 2 : 1;
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    [[dont_unroll]] for (int j = -r; j <= r; j++) {
        [[dont_unroll]] for (int i = -r; i <= r; i++) {
            float k = exp(-float(i * i + j * j) / (2.0 * sigma * sigma));
            uint s2 = slot;
            if (i != 0 || j != 0) {
                s2 = lighting_cache_find_sized(p + (float(i) * ta + float(j) * tb) * size, n, false, size, p);
                if (s2 == ~0u) {
                    continue;
                }
                if (abs(dot(cache.v[3u * s2].xyz - p, n)) > 0.5 * size) {
                    continue;
                }
            }
            vec4 v = cache.v[3u * s2 + 2u];
            float c = floor(max(v.w, 0.0));
            if (c < 0.5) {
                continue;
            }
            if (guided && s2 != slot) {
                float dl = abs(dot(v.rgb, vec3(0.2126, 0.7152, 0.0722)) - own_lum);
                k *= exp(-dl / (LIGHTING_DN_LUM * nz.y + 1.0e-4));
            }
            sum += k * c * v.rgb;
            wsum += k * c;
        }
    }
    cache.v[LIGHTING_WORLD_AT + slot].y = uintBitsToFloat(lighting_pack_rgb9e5(wsum > 0.0 ? lighting_sc_regularise(slot, sum / wsum) : vec3(0.0)));
    if (t >= 0.5) {
        lighting_sc_rectify(slot, lighting_sc_regularise(slot, own.rgb));
    }
    lighting_obj_salt = 0.0;
}

// S67A neighbourhood clamp (TAA-style variance box, as Lumen's and ReBLUR's
// history clamps): while the light changes, a texel's long-term mean m is held
// within the luminance box of its own and its four neighbours' short-term means
// (the estimator's, this plane) widened by LIGHTING_SC_CLAMP_SD standard
// deviations; colour kept. It follows a change at once where the neighbourhood
// agrees, and noise stays inside the box.
const bool LIGHTING_SC_NCLAMP = true;
const float LIGHTING_SC_CLAMP_SD = 2.0;
vec3 lighting_sc_nclamp(uint slot, vec3 m, vec3 sm, float dev) {
    const vec3 lw = vec3(0.2126, 0.7152, 0.0722);
    vec3 p = cache.v[3u * slot].xyz;
    vec4 nb = cache.v[3u * slot + 1u];
    vec3 n = nb.xyz;
    float size = nb.w;
    uint ax = lighting_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    float lo = dot(sm, lw);
    float hi = lo;
    float salt = lighting_obj_salt;
    lighting_obj_salt = cache.v[LIGHTING_SC_OBJ_AT + slot].x > 0.5 ? cache.v[LIGHTING_SC_OBJ_AT + slot].x : 0.0;
    [[dont_unroll]] for (uint i = 0u; i < 4u; i++) {
        vec3 o = (i < 2u ? ta : tb) * ((i & 1u) == 0u ? size : -size);
        uint s2 = lighting_cache_find_sized(p + o, n, false, size, p);
        if (s2 == ~0u || abs(dot(cache.v[3u * s2].xyz - p, n)) > 0.5 * size) {
            continue;
        }
        vec4 b2 = cache.v[LIGHTING_SC_EST_AT + 2u * s2 + 1u];
        if (b2.x <= 0.0) {
            continue;
        }
        float l2 = dot(cache.v[LIGHTING_SC_EST_AT + 2u * s2].rgb, lw);
        lo = min(lo, l2);
        hi = max(hi, l2);
    }
    lighting_obj_salt = salt;
    lo = max(lo - LIGHTING_SC_CLAMP_SD * dev, 0.0);
    hi += LIGHTING_SC_CLAMP_SD * dev;
    float lm = dot(m, lw);
    if (lm <= 1.0e-6) {
        return m;
    }
    return m * (clamp(lm, lo, hi) / lm);
}

// S72A sparse fill + neighbour propagation (irradiance caching, Ward 1988; Lumen's
// adaptive screen probes; GIBS surfels). Fine texels on a 2 x 2 lattice per plane:
// the key texel (even cell on both in-plane axes) traces its own rays; the other
// three show the bilinear light of the 1-4 key texels around them on the same
// plane and are never scheduled (lighting_compact.comp), so a reveal traces a
// quarter of the texels. A texel is promoted to its own rays for good when a key
// it needs is missing or off its plane (edges, small faces) or the keys disagree
// by more than LIGHTING_SC_SPARSE_RATIO in luminance (contacts, shadow edges).
// Flag 16777216 turns it off.
const float LIGHTING_SC_SPARSE_RATIO = 1.3;
bool lighting_sc_svgf();
bool lighting_sc_sparse_on() {
    return lighting_world_mode() == 3u && (lighting_sc_flags() & 16777216u) == 0u && !lighting_sc_svgf();
}
// S72D levels (coarse to fine): 2 on every 4th cell of both in-plane axes (and
// texels of 25 cm and up), 1 on every 2nd (the S72A keys), else 0.
uint lighting_sc_level(uint slot) {
    vec4 nb = cache.v[3u * slot + 1u];
    float size = nb.w;
    if (size <= 0.0 || size > 0.2) {
        return 2u;
    }
    vec3 p = cache.v[3u * slot].xyz;
    uint ax = lighting_axis(nb.xyz) >> 1u;
    int a = int(floor(p[(ax + 1u) % 3u] / size));
    int b = int(floor(p[(ax + 2u) % 3u] / size));
    return ((a | b) & 3) == 0 ? 2u : (((a | b) & 1) == 0 ? 1u : 0u);
}
bool lighting_sc_key(uint slot) {
    return lighting_sc_level(slot) >= 1u;
}
// S72D: sets after which a texel shows only its own light; below, its own mean
// is mixed with the interpolation from the coarser level by sets / this.
const float LIGHTING_SC_SPARSE_OWN = 4.0;
// S72K handover: interpolation fades out over this many own sets (weight K).
const float LIGHTING_SC_HANDOVER = 16.0;
const float LIGHTING_SC_HANDOVER_K = 2.0;
// More due than the base budget (the boost counter runs): fine texels wait.
bool lighting_sc_backlog() {
    return ckeys.key[LIGHTING_SC_BOOST_AT] > 0u;
}
bool lighting_sc_promoted(uint slot) {
    return floatBitsToUint(cache.v[LIGHTING_SC_SPARSE_AT + slot].x) != 0u;
}
// Light from the keys `step` cells apart around a texel, sun/sky gain applied: w 1
// when interpolated, 0 when a key is not lit yet, -1 when one is missing or off
// the plane, -2 when they differ by more than LIGHTING_SC_SPARSE_RATIO (an edge).
vec4 lighting_sc_sparse(uint slot, float step) {
    const vec3 lw = vec3(0.2126, 0.7152, 0.0722);
    vec3 p = cache.v[3u * slot].xyz;
    vec4 nb = cache.v[3u * slot + 1u];
    vec3 n = nb.xyz;
    float size = nb.w;
    uint ax = lighting_axis(n) >> 1u;
    uint ua = (ax + 1u) % 3u;
    uint ub = (ax + 2u) % 3u;
    vec2 c = floor(vec2(p[ua], p[ub]) / size);
    vec2 base = c - mod(c, step);
    vec2 f = (c - base) / step;
    vec3 sum = vec3(0.0);
    float ws = 0.0;
    float lo = 1.0e30;
    float hi = 0.0;
    float result = 1.0;
    float cs = 0.0;
    float salt = lighting_obj_salt;
    lighting_obj_salt = cache.v[LIGHTING_SC_OBJ_AT + slot].x > 0.5 ? cache.v[LIGHTING_SC_OBJ_AT + slot].x : 0.0;
    [[dont_unroll]] for (uint i = 0u; i < 4u; i++) {
        float w = ((i & 1u) == 1u ? f.x : 1.0 - f.x) * ((i >> 1u) == 1u ? f.y : 1.0 - f.y);
        if (w <= 0.0) {
            continue;
        }
        vec2 k = base + step * vec2(float(i & 1u), float(i >> 1u));
        vec3 q = p;
        q[ua] = (k.x + 0.5) * size;
        q[ub] = (k.y + 0.5) * size;
        uint s2 = lighting_cache_find_sized(q, n, false, size, p);
        if (s2 == ~0u || abs(dot(cache.v[3u * s2].xyz - p, n)) > 0.5 * size) {
            result = -1.0;
            break;
        }
        vec4 o = cache.v[3u * s2 + 2u];
        if (o.w < 0.5) {
            result = 0.0;
            continue;
        }
        vec3 l = o.rgb * lighting_sc_sun_gain(s2);
        float y = dot(l, lw);
        lo = min(lo, y);
        hi = max(hi, y);
        sum += w * l;
        ws += w;
        cs += w * min(floor(o.w), LIGHTING_SC_SPARSE_OWN) / LIGHTING_SC_SPARSE_OWN;
    }
    lighting_obj_salt = salt;
    if (result < 0.0) {
        return vec4(0.0, 0.0, 0.0, -1.0);
    }
    if (result < 0.5 || ws <= 0.0) {
        return vec4(0.0);
    }
    if (hi > 1.0e-3 && hi > LIGHTING_SC_SPARSE_RATIO * lo) {
        // S72K: an edge; the blend still comes back (w -2 - confidence).
        return vec4(sum / ws, -2.0 - cs / ws);
    }
    // S72I: w is 1 + the keys' confidence (their sets over LIGHTING_SC_SPARSE_OWN).
    return vec4(sum / ws, 1.0 + cs / ws);
}

// Surface cache: what a texel shows (w 0 when it has nothing yet).
// S72N (Anthony: seeded light must not count as a texel's own samples): a
// texel's first light from elsewhere (the resample of the next sizes, the
// coarser cells, the pixel's probes) is kept apart from its own mean, in
// SPARSE_AT .y (rgb9e5, light as of the seed) and .z (confidence 0..1, 0 none).
// Its own sets start from nothing and take over by count, as the coarse-level
// interpolation does (lighting_sc_shown). Flag 134217728: the old way (the seed
// is the texel's mean with its sample count).
// S72N dark fill (lighting_cache.comp, lighting_compact.comp); flag 67108864 off.
bool lighting_sc_dark_fill() {
    return lighting_world_mode() == 3u && (lighting_sc_flags() & 67108864u) == 0u;
}
bool lighting_sc_seed_split() {
    return lighting_world_mode() == 3u && (lighting_sc_flags() & 134217728u) == 0u;
}
bool lighting_sc_has_seed(uint slot) {
    return cache.v[LIGHTING_SC_SPARSE_AT + slot].z > 0.0;
}
void lighting_sc_set_seed(uint slot, vec3 rgb, float conf, float sunfrac) {
    if (cache.v[3u * slot + 2u].w < 0.5) {
        cache.v[LIGHTING_SC_SUN_AT + slot] = vec4(sunfrac, max(lighting_ls_now(), 0.0), 0.0, 0.0);
    }
    cache.v[LIGHTING_SC_SPARSE_AT + slot].y = uintBitsToFloat(lighting_pack_rgb9e5(max(rgb, vec3(0.0))));
    cache.v[LIGHTING_SC_SPARSE_AT + slot].z = clamp(conf, 0.05, 1.0);
}
vec4 lighting_sc_seed(uint slot) {
    vec4 sp = cache.v[LIGHTING_SC_SPARSE_AT + slot];
    if (!(sp.z > 0.0)) {
        return vec4(0.0);
    }
    return vec4(lighting_unpack_rgb9e5(floatBitsToUint(sp.y)) * lighting_sc_sun_gain(slot), sp.z);
}

// Surface cache: what a texel shows (w 0 when it has nothing yet).
vec4 lighting_sc_own_shown(uint slot);
// S72V10: the 25 cm WRC cells' light as of now, bilinear over the coplanar
// cells with sets (no recursion into lighting_sc_shown); w 1 when any.
vec4 lighting_sc_wrc_now(vec3 p, vec3 n) {
    const float cs = 0.25;
    uint ax = lighting_axis(n) >> 1u;
    uint ua = (ax + 1u) % 3u;
    uint ub = (ax + 2u) % 3u;
    vec2 uv = vec2(p[ua], p[ub]) / cs - 0.5;
    vec2 g0 = floor(uv);
    vec2 fr = uv - g0;
    vec3 sum = vec3(0.0);
    float ws = 0.0;
    [[dont_unroll]] for (uint k = 0u; k < 4u; k++) {
        vec2 c = g0 + vec2(float(k & 1u), float(k >> 1u));
        vec3 q = p;
        q[ua] = (c.x + 0.5) * cs;
        q[ub] = (c.y + 0.5) * cs;
        float bil = ((k & 1u) == 1u ? fr.x : 1.0 - fr.x) * ((k >> 1u) == 1u ? fr.y : 1.0 - fr.y);
        uint s = lighting_cache_find_sized(q, n, false, cs, p);
        if (s == ~0u || bil <= 0.0 || abs(dot(cache.v[3u * s].xyz - p, n)) > 0.5 * cs) {
            continue;
        }
        vec4 v = cache.v[3u * s + 2u];
        if (v.w < 0.5) {
            continue;
        }
        sum += bil * v.rgb * lighting_sc_sun_gain(s);
        ws += bil;
    }
    return ws > 0.0 ? vec4(sum / ws, 1.0) : vec4(0.0);
}
vec4 lighting_sc_shown(uint slot) {
    // S72D coarse to fine: a texel shows the interpolation of the next level up
    // (level 0 from the 2-apart keys, else the 4-apart ones) until its own sets
    // take over. S72K (Anthony: still-scene refinement popped): the handover is
    // by sample count, never a switch: own weight n / (n + K c f), c the keys'
    // confidence (half for 4-apart keys, half again across an edge), f fading
    // to 0 at LIGHTING_SC_HANDOVER sets, so the value moves continuously. An edge
    // (keys disagree) still promotes the texel to rays of its own (scheduling),
    // but its light fades in the same way.
    // S72N: the texel's seed (lighting_sc_seed) joins the prior by its
    // confidence, in the same handover.
    vec4 own = cache.v[3u * slot + 2u];
    // Denoiser off: the a-trous push never writes the display field, so compose
    // fell through to parent and world-cache walks on every pixel and got
    // slower than with the filter on. The texel's own mean is one load.
    // High sets the denoise bit, so this branch is not taken.
    if ((gstats.ctrl_bits & 64u) == 0u && lighting_world_mode() == 3u) {
        if (own.w > 0.5) {
            return vec4(own.rgb * lighting_sc_sun_gain(slot), 1.0);
        }
        return vec4(0.0);
    }
    float n = floor(max(own.w, 0.0));
    // S72P: layer 2 (the rebuilt field) is what shows; its support (sets around)
    // is the count in the handover from the texel's seed.
    if (lighting_sc_layer2() && cache.v[3u * slot + 1u].w <= 0.2) {
        vec3 field = lighting_unpack_rgb9e5(floatBitsToUint(cache.v[LIGHTING_WORLD_AT + slot].y));
        float sup = abs(cache.v[LIGHTING_SC_SPARSE_AT + slot].w);
        bool have = any(greaterThan(field, vec3(0.0))) && sup > 0.0;
        float ls = lighting_sc_sun_gain(slot);
        vec4 sd = lighting_sc_seed_split() ? lighting_sc_seed(slot) : vec4(0.0);
        if (lighting_sc_svgf()) {
            // S72V: the pushed SVGF field covers what it reaches; the seed only
            // where nothing reached yet.
            // The field is light as of now (S72V7), no gain.
            // S72V10: only a field the push wrote this frame or the last (stamp
            // in the a-trous buffer B .w); a texel coming back on screen after
            // more than 30 frames off it held the field from when it left. Until
            // its field is rebuilt (next frame) it shows the WRC as of now
            // (parent bilinear), not that old light.
            uint pf = floatBitsToUint(cache.v[10u * LIGHTING_CACHE_SLOTS + slot].w);
            // While the camera is held, the field last written stays what pixels
            // show. The push idle and a frozen frame do not rewrite the stamp, and
            // dropping the field for the WRC on that timer would change a still
            // picture that nothing else changed.
            bool fresh = pf != 0u && (int(lighting_frame() - pf) <= 2 || lighting_view_held());
            have = fresh && any(greaterThan(field, vec3(0.0)));
            if (have) {
                return vec4(field, 1.0);
            }
            if (!fresh && own.w > 0.5) {
                vec4 nbs = cache.v[3u * slot + 1u];
                vec4 pb = lighting_sc_wrc_now(cache.v[3u * slot].xyz, nbs.xyz);
                if (pb.w > 0.5) {
                    return vec4(pb.rgb, 1.0);
                }
            }
            // S72V8: no field yet: the texel's own sets before its seed (a seed is
            // a first-fill guess and must never outlast real samples; S72V-V5
            // showed seeds wherever the field was missing, frozen from the first
            // fill).
            if (own.w > 0.5) {
                return vec4(own.rgb * ls, 1.0);
            }
        } else if (have) {
            float f2 = clamp(1.0 - sup / LIGHTING_SC_HANDOVER, 0.0, 1.0);
            float a = sd.w > 0.0 ? sup / max(sup + LIGHTING_SC_HANDOVER_K * sd.w * f2, 1.0e-4) : 1.0;
            return vec4(mix(sd.rgb, field * ls, a), 1.0);
        }
        if (sd.w > 0.0) {
            return vec4(sd.rgb, 1.0);
        }
        if (own.w > 0.5) {
            return vec4(own.rgb * ls, 1.0);
        }
        // Nothing rebuilt yet: the coarse levels (S72D) below.
    }
    float fade = clamp(1.0 - n / LIGHTING_SC_HANDOVER, 0.0, 1.0);
    if (fade > 0.0) {
        vec3 prior = vec3(0.0);
        float pc = 0.0;
        uint lvl = lighting_sc_level(slot);
        if (lighting_sc_sparse_on() && lvl < 2u) {
            vec4 sp = vec4(0.0);
            float ck = 0.0;
            if (lvl == 0u) {
                sp = lighting_sc_sparse(slot, 2.0);
                if (sp.w < -1.5) {
                    if (lighting_sc_display && !lighting_sc_promoted(slot)) {
                        cache.v[LIGHTING_SC_SPARSE_AT + slot].x = uintBitsToFloat(lighting_frame() | 1u);
                    }
                    ck = 0.5 * (-sp.w - 2.0);
                } else if (sp.w > 0.5) {
                    ck = sp.w - 1.0;
                }
            }
            if (sp.w > -1.5 && sp.w < 0.5) {
                sp = lighting_sc_sparse(slot, 4.0);
                ck = sp.w < -1.5 ? 0.25 * (-sp.w - 2.0) : 0.5 * (sp.w - 1.0);
            }
            if (abs(sp.w) > 0.5) {
                ck = max(ck, 1.0e-3);
                prior = sp.rgb * ck;
                pc = ck;
            }
        }
        if (lighting_sc_seed_split()) {
            vec4 sd = lighting_sc_seed(slot);
            if (sd.w > 0.0) {
                prior += sd.rgb * sd.w;
                pc += sd.w;
            }
        }
        if (pc > 0.0) {
            prior /= pc;
            vec4 mine = n > 0.5 ? lighting_sc_own_shown(slot) : vec4(0.0);
            float k = LIGHTING_SC_HANDOVER_K * min(pc, 1.0) * fade;
            float a = mine.w > 0.5 ? n / max(n + k, 1.0e-4) : 0.0;
            return vec4(mix(prior, mine.rgb, a), 1.0);
        }
    }
    return lighting_sc_own_shown(slot);
}
vec4 lighting_sc_own_shown(uint slot) {
    vec3 pooled = lighting_unpack_rgb9e5(floatBitsToUint(cache.v[LIGHTING_WORLD_AT + slot].y));
    vec4 own = cache.v[3u * slot + 2u];
    float ls = lighting_world_mode() == 3u ? lighting_sc_sun_gain(slot) : 1.0;
    if (any(greaterThan(pooled, vec3(0.0)))) {
        return vec4(pooled * ls, 1.0);
    }
    return own.w > 0.5 ? vec4(own.rgb * ls, 1.0) : vec4(0.0);
}

// Surface cache read rule at a ray hit t from the ray's origin (SHaRC: a cache
// cell answers a ray only if it is small against the distance, t > size sqrt 3,
// and holds enough samples): a near hit reads the finest lit texel with
// t > size sqrt 3 and at least LIGHTING_SC_READ_SETS sets; otherwise the 25 cm patch
// as before, which falls back to its lit neighbours, never black.
const float LIGHTING_SC_READ_SETS = 4.0;
vec3 lighting_cache_irradiance(vec3 pos, vec3 n);
vec3 lighting_cache_irradiance_at(vec3 pos, vec3 n, float t) {
    if (!lighting_bounce_ok()) {
        return vec3(0.0);
    }
    if (lighting_world_mode() == 3u && (lighting_sc_flags() & 2097152u) == 0u && !lighting_sc_slanted(n)) {
        float hs = lighting_hit_size_n(pos, n);
        if (t < hs * 1.7320508) {
            float fs = LIGHTING_SC_TEXEL;
            while (fs * 2.0 < hs && t >= fs * 2.0 * 1.7320508) {
                fs *= 2.0;
            }
            uint s = lighting_cache_find_sized(pos, n, false, fs, pos);
            if (s != ~0u) {
                vec4 v = cache.v[3u * s + 2u];
                if (v.w >= LIGHTING_SC_READ_SETS && fract(v.w) < 0.1) {
                    lighting_bounce_path = 0u;
                    cache.v[3u * s].w = uintBitsToFloat(lighting_frame());
                    lighting_hit_sunfrac = lighting_sc_sunfrac_now(s);
                    return v.rgb * lighting_sc_sun_gain(s);
                }
            }
        }
    }
    return lighting_cache_irradiance(pos, n);
}

// Irradiance already bounced onto the surface at pos facing n (0 for a new patch).
vec3 lighting_cache_irradiance(vec3 pos, vec3 n) {
    if (!lighting_bounce_ok()) {
        return vec3(0.0);
    }
    // Flag 2097152 (diagnostic): no light from the cache at ray hits, so the
    // picture has one bounce (compare with GENOS_GI_CHECK_BOUNCES=1 references).
    if ((lighting_sc_flags() & 2097152u) != 0u) {
        return vec3(0.0);
    }
    float hsize = lighting_hit_size_n(pos, n);
    uint slot = lighting_cache_find_sized(pos, n, true, hsize, pos);
    if (slot == ~0u) {
        atomicAdd(gstats.missed, 1u);
        return vec3(0.0);
    }
    cache.v[3u * slot].w = uintBitsToFloat(lighting_frame());
    lighting_bounce_path = 1u;
    // Flag 65536: hit patches stay lit from where they were claimed (as in the
    // other modes), no offer.
    if (lighting_world_mode() == 3u && !lighting_sc_slanted(n) && (lighting_sc_flags() & 65536u) == 0u) {
        lighting_sc_offer(slot, pos, lighting_sc_centre(pos, n, hsize));
    }
    vec4 e = cache.v[3u * slot + 2u];
    // Not lit yet (claimed this round, params.z bit 7): the lit patches beside it
    // on the same surface, so a surface coming into view or a box face entering
    // new cells is not black until the next round relights it.
    // Flag 32768 (surface cache): every round, not only the round that claimed it.
    bool fill_any = lighting_world_mode() == 3u && (lighting_sc_flags() & 32768u) != 0u;
    if (e.w < 0.5 && (fill_any || ((pc.params.z >> 7u) & 1u) != 0u)) {
        float size = cache.v[3u * slot + 1u].w;
        if (lighting_world_mode() == 3u) {
            vec4 up = lighting_sc_parent(pos, n, hsize);
            if (up.w > 0.0) {
                lighting_bounce_path = 3u;
                return up.rgb;
            }
        }
        vec3 t1 = normalize(abs(n.x) < 0.9 ? cross(n, vec3(1.0, 0.0, 0.0)) : cross(n, vec3(0.0, 1.0, 0.0)));
        vec3 t2 = cross(n, t1);
        vec4 sum = vec4(0.0);
        float fill_f = 0.0;
        [[dont_unroll]] for (uint i = 0u; i < 4u; i++) {
            vec3 o = (i < 2u ? t1 : t2) * ((i & 1u) == 0u ? size : -size);
            uint s2 = lighting_cache_find_sized(pos + o, n, false, hsize, pos + o);
            if (s2 != ~0u) {
                vec4 v = cache.v[3u * s2 + 2u];
                if (v.w > 0.5) {
                    // S64B: scaled to the sun/sky of now, share noted.
                    sum += vec4(v.rgb * (lighting_world_mode() == 3u ? lighting_sc_sun_gain(s2) : 1.0), 1.0);
                    fill_f += lighting_world_mode() == 3u ? lighting_sc_sunfrac_now(s2) : 0.0;
                }
            }
        }
        lighting_hit_sunfrac = sum.w > 0.0 ? fill_f / sum.w : 0.0;
        lighting_bounce_path = 4u;
        return sum.w > 0.0 ? sum.rgb / sum.w : vec3(0.0);
    }
    if (lighting_world_mode() == 3u) {
        lighting_hit_sunfrac = lighting_sc_sunfrac_now(slot);
        return e.rgb * lighting_sc_sun_gain(slot);
    }
    return e.rgb;
}

// S70A indirect dispatch (Anthony: GPU-resident, no work sized on the CPU): a
// round's light cache rays run in their own dispatches sized by the GPU. The args
// pass (lighting_compact.comp, params.z bit 4) counts the batch entries that can hold a
// texel (scheduled/young, trickle, WRC refresh, scouts when due), writes the
// dispatch sizes past the WRC lists (LIGHTING_ARGS_AT: rays x,y,z; entries x,y,z;
// dense count; segment ends) and the ray and cache passes map their dense thread
// to the entry (lighting_dense_entry). Nothing past the live entries is dispatched.
bool lighting_wrc_scout_due();
const uint LIGHTING_ARGS_AT = 2u * LIGHTING_CACHE_SLOTS + 2u + 17600u;
// S72J region refinement (Anthony: uniform within a region, no sharp islands in
// blotchy surroundings; Lumen refines surface cache pages). A page is 8 x 8
// texels of one size on one plane (axis, plane offset); a hashed table of
// LIGHTING_SC_PAGES counters per round, three rotating (written this round,
// read from the last, cleared for the next) holds each page's fewest sets
// among its read texels (stored 1000 - sets, atomicMax; 0 none). A texel more
// than LIGHTING_SC_PAGE_LEAD sets ahead of its page waits, so a page's texels
// refine together and the budget goes to the pages that are behind.
// Flag 128 turns it off.
const uint LIGHTING_SC_PAGES = 65536u;
const uint LIGHTING_SC_PAGE_AT = LIGHTING_ARGS_AT + 16u;
const float LIGHTING_SC_PAGE_LEAD = 2.0;
bool lighting_sc_pages_on() {
    return lighting_world_mode() == 3u && (lighting_sc_flags() & 128u) == 0u && !lighting_sc_svgf();
}
// A texel's cell on its plane (xy), the plane's key (z: offset, w: axis), so
// pages and their neighbours hash alike.
ivec4 lighting_sc_cell(uint slot) {
    vec3 p = cache.v[3u * slot].xyz;
    vec4 nb = cache.v[3u * slot + 1u];
    float size = max(nb.w, 1.0e-3);
    uint axd = lighting_axis(nb.xyz);
    uint ax = axd >> 1u;
    return ivec4(int(floor(p[(ax + 1u) % 3u] / size)), int(floor(p[(ax + 2u) % 3u] / size)), int(floor(p[ax] / max(size, 0.0625) + 0.5)), int(axd));
}
uint lighting_sc_page_at(uint slot, ivec4 c, ivec2 page) {
    float size = max(cache.v[3u * slot + 1u].w, 1.0e-3);
    uint h = lighting_hash(uint(page.x) * 73856093u ^ uint(page.y) * 19349663u ^ uint(c.z) * 83492791u ^ (uint(c.w) * 2654435761u) ^ floatBitsToUint(size) ^ floatBitsToUint(cache.v[LIGHTING_SC_OBJ_AT + slot].x));
    return h % LIGHTING_SC_PAGES;
}
uint lighting_sc_page(uint slot) {
    ivec4 c = lighting_sc_cell(slot);
    return lighting_sc_page_at(slot, c, c.xy >> 3);
}
float lighting_sc_page_min(uint page);
// S72K (video: 8 x 8 pages popped as squares, neighbouring pages at different
// stages): the bar a texel is held to is the page minima interpolated
// bilinearly between page centres (a page unknown takes the texel's own), plus
// a per-texel dither in [0, 1) sets, so the gate has no page edges and a page's
// texels do not step together. -1 when its own page is unknown.
float lighting_sc_page_bar(uint slot) {
    ivec4 c = lighting_sc_cell(slot);
    float own = lighting_sc_page_min(lighting_sc_page_at(slot, c, c.xy >> 3));
    if (own < 0.0) {
        return -1.0;
    }
    vec2 cf = (vec2(c.xy) + 0.5) / 8.0 - 0.5;
    vec2 p0 = floor(cf);
    vec2 f = cf - p0;
    float acc = 0.0;
    [[dont_unroll]] for (uint i = 0u; i < 4u; i++) {
        ivec2 pg = ivec2(p0) + ivec2(int(i & 1u), int(i >> 1u));
        float m = lighting_sc_page_min(lighting_sc_page_at(slot, c, pg));
        float w = ((i & 1u) == 1u ? f.x : 1.0 - f.x) * ((i >> 1u) == 1u ? f.y : 1.0 - f.y);
        acc += w * (m < 0.0 ? own : m);
    }
    // S72O: the dither spans LIGHTING_SC_PAGE_LEAD sets (was 1), so a page's
    // texels are held at independent points and never step as one.
    return acc + lighting_unit(slot * 0x9e3779b9u) * (lighting_sc_decorrelate() ? LIGHTING_SC_PAGE_LEAD : 1.0);
}
// The page's fewest sets last round (-1: none known).
float lighting_sc_page_min(uint page) {
    uint r = lighting_round();
    uint v = ckeys.key[LIGHTING_SC_PAGE_AT + ((r + 2u) % 3u) * LIGHTING_SC_PAGES + page];
    return v == 0u || v > 1000u ? -1.0 : float(1000u - v);
}
void lighting_sc_page_note(uint page, float t) {
    uint r = lighting_round();
    atomicMax(ckeys.key[LIGHTING_SC_PAGE_AT + (r % 3u) * LIGHTING_SC_PAGES + page], 1000u - uint(clamp(floor(t), 0.0, 999.0)));
}
void lighting_sc_page_clear(uint slot) {
    // Thread per slot clears the next round's counters (slots outnumber pages).
    if (slot < LIGHTING_SC_PAGES) {
        ckeys.key[LIGHTING_SC_PAGE_AT + ((lighting_round() + 1u) % 3u) * LIGHTING_SC_PAGES + slot] = 0u;
    }
}
uint lighting_grad_first() {
    return LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES - LIGHTING_GRAD_ENTRIES;
}
uint lighting_grad_count();
void lighting_shadow_epochs();
// S76D persistent check words, after the gradient check list (LIGHTING_GRAD_LIST_AT + 32768).
const uint LIGHTING_CHECK_P_AT = LIGHTING_ARGS_AT + 16u + 3u * 65536u + 1024u + 32768u;
bool lighting_dense() {
    return ((pc.params.z >> 4u) & 1u) != 0u;
}
uint lighting_dense_count() {
    return ckeys.key[LIGHTING_ARGS_AT + 6u];
}
uint lighting_dense_entry(uint d) {
    uint a = ckeys.key[LIGHTING_ARGS_AT + 7u];
    uint b = ckeys.key[LIGHTING_ARGS_AT + 8u];
    uint c = ckeys.key[LIGHTING_ARGS_AT + 9u];
    if (d < a) {
        return d;
    }
    uint room = lighting_world_mode() == 3u ? lighting_sc_room() : LIGHTING_CACHE_BATCH;
    if (d < b) {
        return room + (d - a);
    }
    if (d < c) {
        return room + LIGHTING_SC_TRICKLE + (d - b);
    }
    uint g = ckeys.key[LIGHTING_ARGS_AT + 10u];
    if (d < g) {
        return lighting_grad_first() + (d - c);
    }
    return LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES + (d - g);
}
// Seeded admission. The list body is 0xffffffff for an empty bucket. A pack
// keeps the top bit clear, so atomicMin prefers any real candidate over empty.
// Low 20 bits are the slot (the cache has 2^20 slots). Seed 0 does not call these.
// A run pack also stores the copy count: the cache pass sums consecutive copies
// of one slot on the thread that owns the first copy.
uint lighting_seed_pack(uint slot, uint salt) {
    uint rank = lighting_hash(lighting_sched(slot) ^ salt);
    return (rank & 0x7ff00000u) | (slot & 0x000fffffu);
}
uint lighting_seed_run_pack(uint slot, uint n, uint salt) {
    uint rank = lighting_hash(lighting_sched(slot) ^ salt);
    uint copies = min(max(n, 1u), 8u);
    return ((rank >> 24u) << 23u) | ((copies - 1u) << 20u) | (slot & 0x000fffffu);
}
uint lighting_seed_run_n(uint pack) {
    return ((pack >> 20u) & 7u) + 1u;
}
uint lighting_wrc_list_cap() {
    return max(LIGHTING_WRC_REFRESH_CELLS >> lighting_sc_budget_shift(), 1u);
}
uint lighting_trickle_list_cap() {
    return max(LIGHTING_SC_TRICKLE >> lighting_sc_budget_shift(), 1u);
}
uint lighting_grad_list_cap() {
    return max((LIGHTING_GRAD_ENTRIES * LIGHTING_CACHE_RAYS) >> lighting_sc_budget_shift(), 1u);
}
uint lighting_seed_room() {
    uint sh = lighting_sc_budget_shift();
    if (lighting_world_mode() == 3u) {
        return max(lighting_sc_base_room() >> sh, 1u);
    }
    return LIGHTING_CACHE_BATCH;
}
// Matches LIGHTING_GRAD_LIST_AT, which is declared further down.
const uint LIGHTING_SEED_GRAD_LIST_AT = LIGHTING_ARGS_AT + 16u + 3u * 65536u + 1024u;

void lighting_put_capped(uint count_at, uint list_at, uint cap, uint seeded_cap, uint slot, uint salt) {
    if (pc.seed == 0u) {
        uint at = atomicAdd(ckeys.key[count_at], 1u);
        if (at < cap) {
            ckeys.key[list_at + at] = slot;
        }
        return;
    }
    uint room = max(seeded_cap, 1u);
    uint pack = lighting_seed_pack(slot, salt);
    atomicMin(ckeys.key[list_at + (lighting_hash(pack ^ salt) % room)], pack);
}
void lighting_put_forward(uint count_at, uint list_at, uint slot, uint n, uint salt) {
    if (pc.seed == 0u) {
        uint at = atomicAdd(ckeys.key[count_at], n);
        for (uint i = 0u; i < n; i++) {
            ckeys.key[list_at + at + i] = slot;
        }
        return;
    }
    uint pack = lighting_seed_run_pack(slot, n, salt);
    uint room = lighting_seed_room();
    atomicMin(ckeys.key[list_at + (lighting_hash(pack) % room)], pack);
}
void lighting_put_backward(uint count_at, uint base, uint slot, uint n, uint salt) {
    if (pc.seed == 0u) {
        uint at = atomicAdd(ckeys.key[count_at], n);
        for (uint i = 0u; i < n; i++) {
            ckeys.key[base - at - i] = slot;
        }
        return;
    }
    uint pack = lighting_seed_run_pack(slot, n, salt);
    uint room = lighting_seed_room();
    atomicMin(ckeys.key[base - (lighting_hash(pack) % room)], pack);
}
void lighting_seal_forward(uint count_at, uint list_at, uint cap) {
    uint n = 0u;
    [[dont_unroll]] for (uint i = 0u; i < cap; i++) {
        uint pack = ckeys.key[list_at + i];
        if (pack != 0xffffffffu) {
            ckeys.key[list_at + n] = pack & 0x000fffffu;
            n++;
        }
    }
    ckeys.key[count_at] = n;
}
// Compact rank buckets, then expand each winner into consecutive copies.
// The cache pass sums a run on one thread, so the copies have to sit together.
// Expanding from the end keeps an unread pack ahead of the write cursor.
void lighting_seal_forward_runs(uint count_at, uint list_at, uint cap, uint limit) {
    uint u = 0u;
    [[dont_unroll]] for (uint i = 0u; i < cap; i++) {
        uint pack = ckeys.key[list_at + i];
        if (pack != 0xffffffffu) {
            ckeys.key[list_at + u] = pack;
            u++;
        }
    }
    uint used = 0u;
    uint covered = 0u;
    [[dont_unroll]] for (uint i = 0u; i < u && used < limit; i++) {
        uint pack = ckeys.key[list_at + i];
        uint n = lighting_seed_run_n(pack);
        uint take = min(n, limit - used);
        if (take < n) {
            pack = (pack & ~0x00700000u) | ((take - 1u) << 20u);
            ckeys.key[list_at + i] = pack;
        }
        used += take;
        covered++;
    }
    uint dst = used;
    uint i = covered;
    [[dont_unroll]] while (i > 0u && dst > 0u) {
        i--;
        uint pack = ckeys.key[list_at + i];
        uint slot = pack & 0x000fffffu;
        uint n = lighting_seed_run_n(pack);
        [[dont_unroll]] for (uint k = 0u; k < n; k++) {
            dst--;
            ckeys.key[list_at + dst] = slot;
        }
    }
    ckeys.key[count_at] = used;
}
void lighting_seal_backward_runs(uint count_at, uint base, uint cap, uint limit) {
    uint u = 0u;
    [[dont_unroll]] for (uint i = 0u; i < cap; i++) {
        uint pack = ckeys.key[base - i];
        if (pack != 0xffffffffu) {
            ckeys.key[base - u] = pack;
            u++;
        }
    }
    uint used = 0u;
    uint covered = 0u;
    [[dont_unroll]] for (uint i = 0u; i < u && used < limit; i++) {
        uint pack = ckeys.key[base - i];
        uint n = lighting_seed_run_n(pack);
        uint take = min(n, limit - used);
        if (take < n) {
            pack = (pack & ~0x00700000u) | ((take - 1u) << 20u);
            ckeys.key[base - i] = pack;
        }
        used += take;
        covered++;
    }
    uint dst = used;
    uint i = covered;
    [[dont_unroll]] while (i > 0u && dst > 0u) {
        i--;
        uint pack = ckeys.key[base - i];
        uint slot = pack & 0x000fffffu;
        uint n = lighting_seed_run_n(pack);
        [[dont_unroll]] for (uint k = 0u; k < n; k++) {
            dst--;
            ckeys.key[base - dst] = slot;
        }
    }
    ckeys.key[count_at] = used;
}
void lighting_seal_seeded_lists() {
    if (pc.seed == 0u) {
        return;
    }
    uint room = lighting_seed_room();
    uint span = LIGHTING_CACHE_BATCH;
    lighting_seal_backward_runs(2u * LIGHTING_CACHE_SLOTS + 1u, 2u * LIGHTING_CACHE_SLOTS, room, span);
    lighting_seal_forward_runs(LIGHTING_CACHE_SLOTS, LIGHTING_CACHE_SLOTS + 1u, room, span);
    lighting_seal_forward(LIGHTING_WRC_CHG_COUNT_AT, LIGHTING_WRC_CHG_AT, lighting_wrc_list_cap());
    lighting_seal_forward(LIGHTING_WRC_BG_COUNT_AT, LIGHTING_WRC_BG_AT, lighting_wrc_list_cap());
    lighting_seal_forward(LIGHTING_SC_TRICKLE_COUNT_AT, LIGHTING_SC_TRICKLE_AT, lighting_trickle_list_cap());
    lighting_seal_forward(LIGHTING_SC_HIST_AT + 72u, LIGHTING_SEED_GRAD_LIST_AT, lighting_grad_list_cap());
}

void lighting_write_args() {
    lighting_seal_seeded_lists();
    uint a;
    uint b = 0u;
    uint c = 0u;
    uint s = 0u;
    uint g = 0u;
    if (lighting_world_mode() == 3u) {
        uint sh = lighting_sc_budget_shift();
        uint room = lighting_sc_room() >> sh;
        uint first = min(lighting_young_count(), lighting_sc_base_room() >> sh);
        a = min(room, first + lighting_mature_count());
        b = min(ckeys.key[LIGHTING_SC_TRICKLE_COUNT_AT], LIGHTING_SC_TRICKLE >> sh);
        // S77A: change-list cells are checks, one 16-ray entry each; the
        // background list (and escalated cells) take whole sets.
        c = min(min(ckeys.key[LIGHTING_WRC_CHG_COUNT_AT], LIGHTING_WRC_REFRESH_CELLS >> sh)
            + 4u * min(ckeys.key[LIGHTING_WRC_BG_COUNT_AT], LIGHTING_WRC_REFRESH_CELLS >> sh), LIGHTING_WRC_REFRESH >> sh);
        s = lighting_wrc_scout_due() ? LIGHTING_WRC_SCOUT_ENTRIES >> sh : 0u;
        g = (lighting_grad_count() + LIGHTING_CACHE_RAYS - 1u) / LIGHTING_CACHE_RAYS;
        lighting_shadow_epochs();
        // S76D: last round's demand for fair service, and the global boost when
        // a large share of the checks since the last write escalated.
        ckeys.key[LIGHTING_CHECK_P_AT + 3u] = ckeys.key[LIGHTING_SC_HIST_AT + 72u]; // LIGHTING_GRAD_COUNT_AT
        uint esc = ckeys.key[LIGHTING_CHECK_P_AT];
        uint chk = ckeys.key[LIGHTING_CHECK_P_AT + 1u];
        if (chk >= 64u) {
            if (float(esc) > 0.2 * float(chk)) { // LIGHTING_BOOST_FRAC
                ckeys.key[LIGHTING_CHECK_P_AT + 2u] = lighting_frame() + 120u; // LIGHTING_BOOST_FRAMES
            }
            ckeys.key[LIGHTING_CHECK_P_AT] = 0u;
            ckeys.key[LIGHTING_CHECK_P_AT + 1u] = 0u;
        }
    } else {
        uint first = min(lighting_young_count(), LIGHTING_CACHE_BATCH);
        a = lighting_young_only() ? first : min(LIGHTING_CACHE_BATCH, first + lighting_mature_count());
    }
    uint e = a + b + c + g + s;
    ckeys.key[LIGHTING_ARGS_AT + 10u] = a + b + c + g;
    ckeys.key[LIGHTING_ARGS_AT + 0u] = max((e * LIGHTING_CACHE_RAYS + 63u) / 64u, 1u);
    ckeys.key[LIGHTING_ARGS_AT + 1u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 2u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 3u] = max((e + 63u) / 64u, 1u);
    ckeys.key[LIGHTING_ARGS_AT + 4u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 5u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 6u] = e;
    ckeys.key[LIGHTING_ARGS_AT + 7u] = a;
    ckeys.key[LIGHTING_ARGS_AT + 8u] = a + b;
    ckeys.key[LIGHTING_ARGS_AT + 9u] = a + b + c;
}

uint lighting_live_count() {
    return min(lighting_young_count() + lighting_mature_count(), LIGHTING_CACHE_SLOTS);
}

// Slot of this round's batch entry j, or ~0u when the batch has no entry j: the
// young first, then the mature ones in turn.
uint lighting_cache_batch_slot(uint j) {
    uint room = LIGHTING_CACHE_BATCH;
    if (lighting_world_mode() == 3u) {
        if (j >= LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES - LIGHTING_GRAD_ENTRIES) {
            return ~0u;
        }
        room = lighting_sc_room();
        if (j >= room && j < room + LIGHTING_SC_TRICKLE) {
            // S65A trickle entries: one a texel.
            uint k = j - room;
            return k < min(ckeys.key[LIGHTING_SC_TRICKLE_COUNT_AT], LIGHTING_SC_TRICKLE >> lighting_sc_budget_shift()) ? ckeys.key[LIGHTING_SC_TRICKLE_AT + k] : ~0u;
        }
        if (j >= room) {
            // The WRC refresh entries: change-driven, then background, each
            // cell a whole set (4 consecutive entries).
            uint k = j - room - LIGHTING_SC_TRICKLE;
            uint rc = LIGHTING_WRC_REFRESH_CELLS >> lighting_sc_budget_shift();
            uint c = min(ckeys.key[LIGHTING_WRC_CHG_COUNT_AT], rc);
            if (k < c) {
                return ckeys.key[LIGHTING_WRC_CHG_AT + k];
            }
            k = (k - c) / 4u;
            if (k < min(ckeys.key[LIGHTING_WRC_BG_COUNT_AT], rc)) {
                return ckeys.key[LIGHTING_WRC_BG_AT + k];
            }
            return ~0u;
        }
    }
    uint first = min(lighting_young_count(), lighting_world_mode() == 3u ? lighting_sc_base_room() >> lighting_sc_budget_shift() : room);
    if (j < first) {
        return ckeys.key[2u * LIGHTING_CACHE_SLOTS - j];
    }
    uint count = lighting_mature_count();
    uint k = j - first;
    if ((lighting_young_only() && lighting_world_mode() != 3u) || k >= count) {
        return ~0u;
    }
    // The scheduler lists only what this round should take (from the front);
    // otherwise the sweep turns with the rounds.
    uint start = (lighting_world_mode() == 3u && LIGHTING_SC_SCHED) ? 0u : (lighting_round() * LIGHTING_CACHE_BATCH) % count;
    return ckeys.key[LIGHTING_CACHE_SLOTS + 1u + (start + k) % count];
}

// Few rays a relight (S59; Lumen: a few rays per texel per update, accumulated
// over updates): a batch entry traces LIGHTING_CACHE_RAYS rays, a quarter of a
// LIGHTING_SC_SET_RAYS set (its stratified pattern split in LIGHTING_SC_QUARTERS). A
// surface-cache texel adds its quarters' light to a partial sum (LIGHTING_SC_OBJ_AT
// .z, rgb9e5; .w the quarters so far) and commits the set to its mean when the
// last quarter is in, so every history rule still counts whole sets. A texel
// takes one entry a relight, or the rest of its set at once (consecutive
// entries) when young, near a change, short of LIGHTING_SC_FULL_SETS sets or
// inconsistent (GIBS 2021: more rays where the estimate is unconverged or its
// variance high). The other modes take whole sets.
const uint LIGHTING_SC_SET_RAYS = 64u;
const uint LIGHTING_SC_QUARTERS = LIGHTING_SC_SET_RAYS / LIGHTING_CACHE_RAYS;
// Tunables (S59E, Anthony: "bump up the quality before it settles"): whole sets
// until LIGHTING_SC_FULL_SETS sets AND agreement within LIGHTING_SC_SETTLE_SD standard
// deviations; an unsettled texel's priority is at least LIGHTING_SC_UNSETTLED_W times
// a settled one's; dormancy needs the same agreement at the cap.
const float LIGHTING_SC_FULL_SETS = 64.0;
const float LIGHTING_SC_SETTLE_SD = 0.15;
const float LIGHTING_SC_UNSETTLED_W = 4.0;
const float LIGHTING_SC_FULL_INC = 1.0;
// Settled (S59D, Anthony: "settling too soon ... splotchy"): LIGHTING_SC_FULL_SETS
// whole sets in its mean (or, while the light changes, the whole window) AND the
// GIBS estimator running with its short-term mean within 0.3 standard deviations
// of the long-term one (the dormancy test's agreement). Under changing light the
// estimator restarts every relight, so nothing settles: whole sets throughout.
bool lighting_sc_settled(uint slot, float t) {
    float reach = LIGHTING_SC_FULL_SETS;
    if (cache.v[3u * slot + 1u].w > 0.2) {
        // A WRC cell: settled at its cap (or the window under changing light).
        return floor(t) >= LIGHTING_WRC_CAP - 0.5;
    }
    vec4 b = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u];
    // S62A perceptual stop: quiet enough to the eye counts as settled.
    vec2 nz = lighting_sc_noise(slot);
    if (lighting_sc_light_quiet() && floor(t) >= LIGHTING_DN_MIN_SETS && nz.x >= 0.0 && nz.x < lighting_dn_target(slot) && b.y < LIGHTING_SC_SETTLE_SD) {
        return true;
    }
    return floor(t) >= reach - 0.5 && b.x > 0.0 && b.y < LIGHTING_SC_SETTLE_SD;
}
// S64A rate-based scheduling (Anthony: "the budget is a ceiling, not a target").
// A texel is relit only once it is due: its interval (frames) comes from how
// much its light is changing. Young texels (short of LIGHTING_SC_FULL_SETS sets) and
// texels with a local change (a mover nearby, a moved lamp) are due every
// round; the rest every LIGHTING_SC_RATE_SLOW / (1 + LIGHTING_SC_RATE_K inconsistency)
// frames (the estimator's short/long divergence in standard deviations, GIBS),
// at most LIGHTING_SC_RATE_CHANGING while the light changes anywhere and
// LIGHTING_SC_RATE_DORMANT once dormant. Only when more is due than the batch holds
// does the priority order choose among them; otherwise the rest of the batch
// stays empty and the GPU idles.
const bool LIGHTING_SC_RATE = true;
const float LIGHTING_SC_RATE_SLOW = 240.0;
const float LIGHTING_SC_RATE_K = 16.0;
const float LIGHTING_SC_RATE_CHANGING = 30.0;
const float LIGHTING_SC_RATE_DORMANT = 600.0;
const float LIGHTING_SC_RATE_FAST = 1.0;
// S68A: a texel read this recently counts as read since its relight.
const uint LIGHTING_SC_READ_SLACK = 2u;
// S68D: EMA floor on a young texel's long mean (lighting_cache.comp).
const float LIGHTING_SC_YOUNG_ALPHA = 1.0 / 16.0;
const float LIGHTING_SC_YOUNG_SETS = 32.0;
bool lighting_sc_dormant(uint slot);
uint lighting_sc_interval(uint slot, float t) {
    if (!LIGHTING_SC_RATE) {
        return 0u;
    }
    vec4 b = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u];
    // S64C: not yet settled (lighting_sc_settled: the perceptual stop or the full
    // sets with agreement) = due every round. S64A asked for LIGHTING_SC_FULL_SETS
    // sets as well, which texels the perceptual stop settles early never
    // reach: nothing ever waited (stats: waiting 0).
    if (!lighting_sc_settled(slot, t) || lighting_sc_stale(slot)) {
        return 0u;
    }
    bool mover;
    if (lighting_sc_local_change(slot, mover) || lighting_sc_lamp_touched(slot)) {
        return 0u;
    }
    float iv = LIGHTING_SC_RATE_SLOW / (1.0 + LIGHTING_SC_RATE_K * max(b.y, 0.0));
    if ((lighting_sc_flags() & 1048576u) != 0u && t >= lighting_sc_cap(slot) - 0.5 && lighting_sc_dormant(slot)) {
        iv = LIGHTING_SC_RATE_DORMANT;
    }
    if (!lighting_sc_light_quiet()) {
        iv = min(iv, LIGHTING_SC_RATE_CHANGING);
    }
    return uint(clamp(iv, LIGHTING_SC_RATE_FAST, LIGHTING_SC_RATE_DORMANT));
}
// Scouts (WRC pre-seeding) only while the camera moves or turns, else one
// round in LIGHTING_WRC_SCOUT_STILL.
const uint LIGHTING_WRC_SCOUT_STILL = 8u;
bool lighting_wrc_scout_due() {
    if ((gstats.ctrl_bits & 1024u) == 0u) {
        return false;
    }
    vec4 last_eye = work.v[lighting_change_at() + 1u];
    vec3 last_fwd = work.v[lighting_change_at() + 4u].xyz;
    bool moving = last_eye.w < 0.5 || length(scene.eye.xyz - last_eye.xyz) > 0.01
        || dot(normalize(scene.view_forward.xyz), normalize(last_fwd + vec3(0.0, 1.0e-6, 0.0))) < 0.9999;
    return moving || lighting_round() % LIGHTING_WRC_SCOUT_STILL == 0u;
}
uint lighting_sc_quarter0(uint slot) {
    if (lighting_world_mode() != 3u) {
        return 0u;
    }
    return min(floatBitsToUint(cache.v[LIGHTING_SC_OBJ_AT + slot].w), LIGHTING_SC_QUARTERS - 1u);
}
uint lighting_sc_entries(uint slot, bool full) {
    return (full || lighting_world_mode() != 3u) ? LIGHTING_SC_QUARTERS - lighting_sc_quarter0(slot) : 1u;
}
// WRC scout ray s of this round (entries past the batch's texels): from the eye
// (a quarter from up to 2 m beside it), half over the sphere and half within
// ~50 degrees of where the camera is heading (forward, plus its motion and turn
// since the last frame the probes ran).
void lighting_wrc_scout(uint s, out vec3 origin, out vec3 dir) {
    uint h = lighting_hash(s * 0x9e3779b9u ^ lighting_round() * 0x85ebca6bu);
    float u1 = lighting_unit(h);
    float u2 = lighting_unit(h ^ 0x68e31da4u);
    vec4 head_at = vec4(0.0);
    vec3 eye = scene.eye.xyz;
    vec3 fwd = normalize(scene.view_forward.xyz);
    vec4 last_eye = work.v[lighting_change_at() + 1u];
    vec3 last_fwd = work.v[lighting_change_at() + 4u].xyz;
    vec3 ahead = fwd;
    if (last_eye.w > 0.5 && dot(last_fwd, last_fwd) > 0.5) {
        ahead = fwd + 8.0 * (eye - last_eye.xyz) + 4.0 * (fwd - normalize(last_fwd));
    }
    ahead = normalize(ahead + vec3(0.0, 1.0e-4, 0.0));
    float z = (s & 1u) == 0u ? 1.0 - 2.0 * u1 : 1.0 - u1 * 0.36;
    float r = sqrt(max(1.0 - z * z, 0.0));
    float a = LIGHTING_TAU * u2;
    vec3 local = vec3(r * cos(a), r * sin(a), z);
    if ((s & 1u) == 0u) {
        dir = local;
    } else {
        vec3 t = normalize(abs(ahead.y) < 0.9 ? cross(ahead, vec3(0.0, 1.0, 0.0)) : cross(ahead, vec3(1.0, 0.0, 0.0)));
        vec3 b = cross(ahead, t);
        dir = normalize(t * local.x + b * local.y + ahead * local.z);
    }
    origin = eye;
    if ((s & 3u) == 3u) {
        float u3 = lighting_unit(h ^ 0x27d4eb2fu);
        float b3 = LIGHTING_TAU * lighting_unit(h ^ 0x165667b1u);
        origin += vec3(cos(b3), 0.0, sin(b3)) * (2.0 * u3);
    }
}
// WRC: did a lamp that reaches p move since frame `updated` (within the last
// LIGHTING_WRC_CHANGE_FRAMES)? The ring holds moved lamps' spheres (both places,
// range), written once a round by lighting_wrc_note_lamps.
bool lighting_wrc_lamp_changed(vec3 p, uint updated) {
    return lighting_lamp_delta(p, updated) >= 0.0;
}
// S65C: how much the lamps that moved since frame `updated` changed the
// unshadowed light at p (sum over moved lamps of brightest channel times
// |1/d_new^2 - 1/d_old^2| inside their range; -1 when none reaches p). The
// light's delta in power and place at that point (Anthony, item 3).
float lighting_lamp_delta(vec3 p, uint updated) {
    float delta = -1.0;
    [[dont_unroll]] for (uint i = 0u; i < LIGHTING_WRC_RING; i++) {
        uint at = LIGHTING_WRC_RING_AT + LIGHTING_WRC_RING_W * i;
        uint f = ckeys.key[at + 8u];
        if (f == 0u || int(lighting_frame() - f) > int(LIGHTING_WRC_CHANGE_FRAMES) || int(updated - f) >= 0) {
            continue;
        }
        vec3 a = vec3(uintBitsToFloat(ckeys.key[at]), uintBitsToFloat(ckeys.key[at + 1u]), uintBitsToFloat(ckeys.key[at + 2u]));
        vec3 b = vec3(uintBitsToFloat(ckeys.key[at + 3u]), uintBitsToFloat(ckeys.key[at + 4u]), uintBitsToFloat(ckeys.key[at + 5u]));
        float range = uintBitsToFloat(ckeys.key[at + 6u]);
        float pw = uintBitsToFloat(ckeys.key[at + 7u]);
        float da = length(p - a);
        float db = length(p - b);
        if (min(da, db) >= range) {
            continue;
        }
        float ia = da < range ? 1.0 / max(da * da, 0.25) : 0.0;
        float ib = db < range ? 1.0 / max(db * db, 0.25) : 0.0;
        delta = max(delta, 0.0) + pw * abs(ib - ia);
    }
    return delta;
}
// Once a round (cache pass, thread 0): lamps that moved or changed range since
// the last round go into the ring (suns are left to the light-change rule).
void lighting_wrc_note_lamps() {
    uint n = min(scene.lamp_count, LIGHTING_WRC_SNAP_LAMPS);
    uint had = ckeys.key[LIGHTING_WRC_LAMPS_AT];
    for (uint i = 0u; i < n; i++) {
        Lamp l = scene_lamp(i);
        uint at = LIGHTING_WRC_SNAP_AT + 4u * i;
        vec4 o = vec4(uintBitsToFloat(ckeys.key[at]), uintBitsToFloat(ckeys.key[at + 1u]),
                      uintBitsToFloat(ckeys.key[at + 2u]), uintBitsToFloat(ckeys.key[at + 3u]));
        bool fresh = i >= had;
        bool moved = fresh || length(o.xyz - l.pos.xyz) > 1.0e-3 || abs(o.w - l.color.w) > 1.0e-3;
        if (moved && l.color.w > 0.0) {
            vec3 a = fresh ? l.pos.xyz : o.xyz;
            uint h = ckeys.key[LIGHTING_WRC_RING_HEAD_AT] % LIGHTING_WRC_RING;
            ckeys.key[LIGHTING_WRC_RING_HEAD_AT] = h + 1u;
            uint ra = LIGHTING_WRC_RING_AT + LIGHTING_WRC_RING_W * h;
            ckeys.key[ra] = floatBitsToUint(a.x);
            ckeys.key[ra + 1u] = floatBitsToUint(a.y);
            ckeys.key[ra + 2u] = floatBitsToUint(a.z);
            ckeys.key[ra + 3u] = floatBitsToUint(l.pos.x);
            ckeys.key[ra + 4u] = floatBitsToUint(l.pos.y);
            ckeys.key[ra + 5u] = floatBitsToUint(l.pos.z);
            ckeys.key[ra + 6u] = floatBitsToUint(max(l.color.w, o.w));
            // A new lamp: all of its light is new (the old place is far away).
            ckeys.key[ra + 7u] = floatBitsToUint(max(max(l.color.r, l.color.g), l.color.b));
            ckeys.key[ra + 8u] = max(lighting_frame(), 1u);
        }
        ckeys.key[at] = floatBitsToUint(l.pos.x);
        ckeys.key[at + 1u] = floatBitsToUint(l.pos.y);
        ckeys.key[at + 2u] = floatBitsToUint(l.pos.z);
        ckeys.key[at + 3u] = floatBitsToUint(l.color.w);
    }
    ckeys.key[LIGHTING_WRC_LAMPS_AT] = n;
}
// Quarter of its set batch entry j (of slot) traces.
uint lighting_cache_entry_quarter(uint j, uint slot) {
    uint i = 0u;
    while (i + 1u < LIGHTING_SC_QUARTERS && j > i && lighting_cache_batch_slot(j - i - 1u) == slot) {
        i++;
    }
    return min(lighting_sc_quarter0(slot) + i, LIGHTING_SC_QUARTERS - 1u);
}

// Mode 2: a screen probe at pos facing n adds its irradiance e to its world cell.
// Inside a change (a sphere of an object that moved) the mean restarts short.
void lighting_world_add(vec3 pos, vec3 n, vec3 e, bool changed) {
    uint slot = lighting_cache_find(pos, n, true);
    if (slot == ~0u) {
        return;
    }
    cache.v[3u * slot].w = uintBitsToFloat(lighting_frame());
    vec4 h = cache.v[LIGHTING_WORLD_AT + slot];
    float k = min(changed ? min(h.w, 1.0) + 1.0 : h.w + 1.0, max(lighting_world_cap(), 1.0));
    cache.v[LIGHTING_WORLD_AT + slot] = vec4(h.rgb + (e - h.rgb) / k, k);
}

// The world cells around pos facing n, interpolated across the surface (the four
// cell centres nearest pos in the plane of its axis). Returns the irradiance and,
// in w, how much of it to trust (0 to 1: the share of cells that have light,
// counted up to 4 samples).
// Experiment S54K: texels stay shown next to a moving object too (Lumen keeps
// the surface cache and relights it; no method swap to the probes, whose
// difference from the texels was the bright contact strip of the spin check).
const bool LIGHTING_SC_KEEP_NEAR = true;
vec4 lighting_world_irradiance_in(vec3 pos, vec3 n, vec3 wpos, bool obj);
// S72M: the pixel's screen-probe irradiance (lighting_compose.comp), -1 when none.
vec3 lighting_px_probes = vec3(-1.0);
vec4 lighting_world_irradiance(vec3 pos, vec3 n) {
    lighting_px_unfilled = false;
    uint mode = lighting_world_mode();
    // Object-local texels (GENOS_LIGHTING_SC_LOCAL=1, GIBS): a pixel on a mover's
    // surface reads texels keyed in the mover's frame, which move and turn with
    // it and keep their light (relit fast while it moves, as near any change).
    uint om = mode == 3u ? lighting_mover_on(pos, n) : ~0u;
    if (om != ~0u) {
        lighting_obj_salt = lighting_mover(om, 1u).w;
        vec4 r = lighting_world_irradiance_in(lighting_mover_to_local(om, pos, false), lighting_mover_to_local(om, n, true), pos, true);
        lighting_obj_salt = 0.0;
        return r;
    }
    // Flag 4194304 (surface cache): a pixel on or inside the bounds of a mover
    // (an object that moved in the last LIGHTING_MOVER_KEEP frames) takes the screen
    // probes, which trace it as it is now, instead of world texels it sweeps
    // through.
    if (mode == 3u && !LIGHTING_SC_KEEP_NEAR && (lighting_sc_flags() & 4194304u) != 0u && lighting_in_change(pos)) {
        return vec4(0.0);
    }
    return lighting_world_irradiance_in(pos, n, pos, false);
}
// Debug view "texels" (lighting_compose.comp mode 9): the size a pixel at pos facing n
// reads and the sets its texel holds (0 when unclaimed).
vec2 lighting_sc_debug_size(vec3 pos, vec3 n) {
    float size = lighting_cell_size(pos);
    uint s = lighting_cache_find_sized(pos, n, false, size, pos);
    return vec2(size, s == ~0u ? 0.0 : floor(max(cache.v[3u * s + 2u].w, 0.0)));
}
vec4 lighting_world_irradiance_in(vec3 pos, vec3 n, vec3 wpos, bool obj) {
    uint mode = lighting_world_mode();
    if (lighting_sc_slanted(n)) {
        return vec4(0.0);
    }
    uint ax = lighting_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    float size = lighting_cell_size(wpos);
    vec2 uv = vec2(dot(pos, ta), dot(pos, tb)) / size - 0.5;
    vec2 g0 = floor(uv);
    vec2 f = uv - g0;
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    float trust = 0.0;
    // Finished sets, weighted like the bilinear corners. A missing corner counts
    // as zero finished sets. Only the compose display path tallies.
    float unfilled_cov = 0.0;
    float unfilled_sets = 0.0;
    bool tally = lighting_sc_display && mode == 3u && lighting_unfilled_count();
    [[dont_unroll]] for (uint k = 0u; k < 4u; k++) {
        vec2 c = g0 + vec2(float(k & 1u), float(k >> 1u)) + 0.5;
        vec3 q = pos + ta * (c.x * size - dot(pos, ta)) + tb * (c.y * size - dot(pos, tb));
        float bil = ((k & 1u) == 1u ? f.x : 1.0 - f.x) * ((k >> 1u) == 1u ? f.y : 1.0 - f.y);
        if (tally) {
            unfilled_cov += bil;
        }
        uint slot = lighting_cache_find_sized(q, n, mode != 2u, size, pos);
        if (slot == ~0u) {
            continue;
        }
        if (tally) {
            unfilled_sets += bil * floor(max(cache.v[3u * slot + 2u].w, 0.0));
        }
        cache.v[3u * slot].w = uintBitsToFloat(lighting_frame());
        bool split = mode == 3u && lighting_sc_seed_split() && size <= 0.2;
        if (mode == 3u && (lighting_sc_flags() & 524288u) != 0u && cache.v[3u * slot + 2u].w < 0.5 && !(split && lighting_sc_has_seed(slot))) {
            vec4 rs = lighting_sc_resample(q, n, size, slot);
            if (rs.w > 0.5 && split) {
                // S72N: a seed beside the texel's own (empty) mean, confidence
                // by the samples it came from (8 = whole).
                lighting_sc_set_seed(slot, rs.rgb, min(rs.w / LIGHTING_WRC_SEED_SETS, 1.0), lighting_seed_sunfrac);
            } else if (rs.w > 0.5) {
                // S62E: the seed is light as of now; stored in the slot's scale.
                // S64B: the seed is light as of now, with its seeds' sun/sky share.
                cache.v[LIGHTING_SC_SUN_AT + slot] = vec4(lighting_seed_sunfrac, max(lighting_ls_now(), 0.0), 0.0, 0.0);
                cache.v[3u * slot + 2u] = rs;
            }
        }
        if (mode == 3u) {
            lighting_sc_offer(slot, pos, lighting_sc_centre(q, n, size));
            cache.v[LIGHTING_WORLD_AT + slot].z = uintBitsToFloat(lighting_frame());
        }
        vec4 v = mode == 3u ? lighting_sc_shown(slot) : (mode != 2u ? cache.v[3u * slot + 2u] : cache.v[LIGHTING_WORLD_AT + slot]);
        if (mode == 3u && lighting_sc_stale(slot)) {
            continue;
        }
        if (v.w < 0.5 && mode == 3u) {
            if (split) {
                // S72N: the coarser cells bilinear (a nearest parent showed its
                // 50 cm / 1 m squares), kept as a low-confidence seed.
                v = lighting_sc_parent_bilerp(q, n, size);
                if (v.w > 0.5) {
                    lighting_sc_set_seed(slot, v.rgb, 0.25, lighting_bilerp_sunfrac);
                }
            } else {
                v = lighting_sc_parent(q, n, size);
            }
        }
        if (v.w < 0.5 && mode == 3u && (lighting_sc_flags() & 33554432u) == 0u) {
            // S72L first fill: no texel, no parent lit yet; the hit-size patch
            // or its lit neighbours (what the rays read), not the probes.
            vec3 hc = lighting_cache_irradiance(q, n);
            if (any(greaterThan(hc, vec3(0.0)))) {
                v = vec4(hc, 1.0);
                if (split) {
                    lighting_sc_set_seed(slot, hc, 0.25, lighting_hit_sunfrac);
                }
            } else if (split && lighting_px_probes.x >= 0.0 && !obj && cache.v[3u * slot + 2u].w < 0.5) {
                // S72N: the S72M probe seed, as a seed (not an own set).
                lighting_sc_set_seed(slot, lighting_px_probes, 0.25, 0.0);
                v = vec4(lighting_px_probes, 1.0);
            } else if (lighting_px_probes.x >= 0.0 && !obj && cache.v[3u * slot + 1u].w <= 0.2 && cache.v[3u * slot + 2u].w < 0.5) {
                // S72M (video: first reveal tore along the screen, the new strip
                // on the probes, the rest on texels): nothing lit here yet, so the
                // texel starts from the probes' light at this pixel, one set's
                // worth; its own sets replace it (count-weighted, as any mean).
                cache.v[LIGHTING_SC_SUN_AT + slot] = vec4(0.0, max(lighting_ls_now(), 0.0), 0.0, 0.0);
                cache.v[3u * slot + 2u] = vec4(lighting_px_probes, 1.0);
                v = vec4(lighting_px_probes, 1.0);
            }
        }
        if (v.w < 0.5) {
            continue;
        }
        sum += bil * v.rgb;
        wsum += bil;
        float own_w = cache.v[3u * slot + 2u].w;
        // With the estimator (flag 1048576) a texel lit under changing light is
        // still shown (its short mean lags a little, as Lumen's surface cache does);
        // swapping the pixel to the probes swaps methods, and their difference was
        // most of the mix error. Around a moving object the probes still take over.
        bool est = (lighting_sc_flags() & 1048576u) != 0u;
        bool stale = mode == 3u && fract(own_w) > 0.1 && !est;
        float sc_trust = stale ? 0.0 : 1.0;
        if ((lighting_sc_flags() & 32u) != 0u || obj) {
            // A texel with few sets shares the pixel with the probes.
            float c = floor(max(own_w, 0.0));
            sc_trust *= c / (c + LIGHTING_SC_TRUST_SETS);
        }
        trust += bil * (mode == 3u ? sc_trust : (mode != 2u ? 1.0 : min(v.w / 4.0, 1.0)));
    }
    // Surface cache: while the light changes or around an object that moves, the
    // probes (no history) show it.
    // S72L: corners with no light are left out of the blend, not handed to the
    // probes texel by texel (first fill tore as corners flipped between the two
    // methods); the probes take the pixel only where most of it has nothing.
    if (mode == 3u && wsum > 0.0 && (lighting_sc_flags() & 33554432u) == 0u) {
        trust = trust / wsum * min(4.0 * wsum, 1.0);
    }
    bool light_swap = !lighting_sc_light_quiet() && (lighting_sc_flags() & 1048576u) == 0u;
    if (mode == 3u && (light_swap || (!obj && !LIGHTING_SC_KEEP_NEAR && lighting_near_change(pos)) || (lighting_sc_flags() & 512u) != 0u)) {
        trust = 0.0;
    }
    if (tally && unfilled_cov > 0.0) {
        atomicAdd(gstats.shaded, 1u);
        if (unfilled_sets < unfilled_cov * LIGHTING_UNFILLED_MIN_SETS) {
            atomicAdd(gstats.unfilled, 1u);
            lighting_unfilled_mark(lighting_compose_px);
            lighting_px_unfilled = true;
        }
    }
    return vec4(wsum > 0.0 ? sum / wsum : vec3(0.0), trust);
}

// S72V layer 2 as SVGF (Anthony 14:15: the papers' definitions and structure).
// Pipeline per round, after the cache pass (layer 1 updated), one dispatch per
// stage over the slots (lighting_atrous.comp, stage in params.z bits 1-3):
//   stage 0  variance (Schied et al. 2017, sec. 4.2): a texel's luminance
//            variance is its MSME per-set variance (moments, Barre-Brisebois
//            2019); with fewer than 4 sets (or none kept, the WRC) it is
//            estimated spatially from the luminance moments of a 7 x 7
//            neighbourhood weighted by w_z w_n, boosted by 4 / sets (Falcor's
//            SVGF reference). Input value and mask weight m (min(1, sets / 4),
//            WRC sets / 16 [inferred]; 0 for no sets or a non-site, sec. IC
//            below) go to buffer B.
//   stages 1-5  five a-trous iterations (Dammertz et al. 2010), step 2^i
//            cells, 5 x 5 B3-spline kernel h = (1/16, 1/4, 3/8, 1/4, 1/16),
//            B -> A -> B -> A -> B -> A; SVGF weights w = h w_z w_n w_l with
//            w_l = exp(-|l_p - l_q| / (sigma_l sqrt(g3x3(var_p)) + eps)),
//            sigma_l = 4, g3x3 a 3 x 3 Gaussian of the input variance;
//            w_n = max(0, n_p . n_q)^sigma_n, sigma_n = 128; w_z = exp(-|z_p -
//            z_q| / (sigma_z s + eps)), sigma_z = 1, z the plane offset and s
//            half a texel [deviation: the screen-space depth gradient term has no
//            surface-space counterpart]. Variance filtered with w^2 / (sum w)^2.
//            Texels without data take part with their mask m as normalized
//            convolution (Knutsson & Westin 1993) [deviation: SVGF has a sample
//            at every pixel]; m carries through the levels.
//   stage 6  push (Gortler et al. 1996, pull-push): the fine texel's filtered
//            value with weight W = its mask after the last level, the WRC (25 cm
//            cells, filtered by the same five levels on their own grid) bilinear
//            for 1 - W. Written to WORLD_AT .y, what pixels show. Layer 1 is
//            never written (no SVGF feedback of the first level into history
//            [deviation, Anthony's rule]).
//   Also in stage 6, irradiance-caching placement for the next rounds (Ward
//   et al. 1988; Ward & Heckbert 1992; Krivanek et al. 2006, sec. 5.1-5.2):
//   R = min(R_geo, E / |grad_t E|) (gradient limit), neighbour clamping
//   R_i = min(R_i, R_j + |p_i - p_j|) against the stored R of texels 4 cells
//   away, stored unclamped in SPARSE_AT .w; the scheduler clamps it to
//   [R_min, R_max] and traces a texel only where its cell lies on the grid of
//   spacing a R' (record influence radius a R, Krivanek 2006 sec. 5.2),
//   a power of two from 1 to 8. R_geo is twice the distance to where the
//   surface ends [deviation: Ward's harmonic mean ray distance is not kept];
//   grad_t E is the central difference of the filtered field [deviation: Ward &
//   Heckbert's hemisphere gradient needs per-ray distances].
// Flag 2147483648: off (S72Q layer 2).
const uint LIGHTING_SC_ATROUS_A = 9u * LIGHTING_CACHE_SLOTS;
const uint LIGHTING_SC_ATROUS_B = 10u * LIGHTING_CACHE_SLOTS;
const float LIGHTING_SVGF_SIGMA_L = 4.0;
const float LIGHTING_SVGF_SIGMA_N = 128.0;
const float LIGHTING_SVGF_SIGMA_Z = 1.0;
const float LIGHTING_IC_A = 0.25;
const float LIGHTING_IC_RMIN = 1.0;
const float LIGHTING_IC_RMAX = 64.0;
const float LIGHTING_IC_GLOBAL_STALE = 0.05;
bool lighting_sc_svgf() {
    return lighting_world_mode() == 3u && (lighting_sc_flags() & 2147483648u) == 0u;
}
uint lighting_atrous_stage() {
    return (pc.params.z >> 1u) & 7u;
}
// The slot of the texel `off` cells from slot's cell on its plane (same size,
// same salt), ~0u when none.
// S72V5: the texel's own cell is found from its key. Its stored position is
// where its claimer stood (a pixel's bilinear corner claims the cell next to
// the pixel), so floor(position / size) is a cell off for about half the
// texels and every tap of theirs was shifted: the stripes and smears.
uint lighting_svgf_memo_slot = ~0u;
vec2 lighting_svgf_memo_cell = vec2(0.0);
vec2 lighting_sc_true_cell(uint slot) {
    if (slot == lighting_svgf_memo_slot) {
        return lighting_svgf_memo_cell;
    }
    vec3 p = cache.v[3u * slot].xyz;
    vec4 nb = cache.v[3u * slot + 1u];
    float size = nb.w;
    uint ax = lighting_axis(nb.xyz) >> 1u;
    uint ua = (ax + 1u) % 3u;
    uint ub = (ax + 2u) % 3u;
    vec2 c0 = floor(vec2(p[ua], p[ub]) / size);
    uint key = ckeys.key[slot];
    vec2 c = c0;
    [[dont_unroll]] for (uint k = 0u; k < 9u; k++) {
        uint kk = (k + 4u) % 9u;
        vec2 cc = c0 + vec2(float(kk % 3u) - 1.0, float(kk / 3u) - 1.0);
        vec3 q = p;
        q[ua] = (cc.x + 0.5) * size;
        q[ub] = (cc.y + 0.5) * size;
        if (lighting_cache_key_sized(q, nb.xyz, size) == key) {
            c = cc;
            break;
        }
    }
    lighting_svgf_memo_slot = slot;
    lighting_svgf_memo_cell = c;
    return c;
}
uint lighting_sc_cell_slot(uint slot, ivec2 off) {
    vec3 p = cache.v[3u * slot].xyz;
    vec4 nb = cache.v[3u * slot + 1u];
    float size = nb.w;
    uint ax = lighting_axis(nb.xyz) >> 1u;
    uint ua = (ax + 1u) % 3u;
    uint ub = (ax + 2u) % 3u;
    vec2 c = lighting_sc_true_cell(slot) + vec2(off);
    vec3 q = p;
    q[ua] = (c.x + 0.5) * size;
    q[ub] = (c.y + 0.5) * size;
    return lighting_cache_find_sized(q, nb.xyz, false, size, p);
}
// S72V4 IC density as a rate: a texel is relit one round in g^2 (at most 16),
// g the IC record spacing a R' (a power of two, 1-8 cells), so the rays per area
// match a site lattice of spacing g but every texel keeps samples of its own.
// S72V-V3 traced only the lattice and left the rest without data: SVGF has a
// sample at every pixel, and a-trous over a sparse lattice left the lattice in
// the picture (Anthony 14:26: regular wavy stripes on floor and ceiling)
// [deviation: IC places records in space, here their density sets a rate].
uint lighting_ic_period(uint slot) {
    float r = cache.v[LIGHTING_SC_SPARSE_AT + slot].w;
    if (!(r > 0.0)) {
        return 1u;
    }
    float rc = clamp(r, LIGHTING_IC_RMIN, LIGHTING_IC_RMAX);
    uint g = 1u << uint(clamp(floor(log2(max(LIGHTING_IC_A * rc, 1.0))), 0.0, 3.0));
    return min(g * g, 16u);
}
// The texels layer 2 filters: WRC cells, and fine texels a pixel showed in
// the last half second (as S72P) [inferred: SVGF filters only the visible pixels].
bool lighting_svgf_active(uint s) {
    if (cache.v[3u * s + 1u].w > 0.2) {
        return true;
    }
    uint shown = floatBitsToUint(cache.v[LIGHTING_WORLD_AT + s].z);
    return lighting_sc_pixel_fresh(shown, 30);
}
#ifdef GENOS_SUN_SHADER
// Sun amortise words, after the a-trous meta (391 words). The default compose
// does not compile this block, so its SPIR-V stays the gi-v2 compose.
const uint LIGHTING_SUN_META = 391u;
const uint LIGHTING_SUN_AT = LIGHTING_COMPOSE_AT + 4u + 2u * LIGHTING_COMPOSE_CAP + 5u * LIGHTING_CACHE_SLOTS + LIGHTING_SUN_META;
const uint LIGHTING_SUN_SMALL = 1u;
bool lighting_stratum_held(uint probe);

uint lighting_sun_flags() {
    return ckeys.key[LIGHTING_SUN_AT + 1u];
}
bool lighting_sun_small() {
    return (lighting_sun_flags() & LIGHTING_SUN_SMALL) != 0u;
}
uint lighting_sun_n() {
    return max((lighting_sun_flags() >> 8u) & 255u, 1u);
}
uint lighting_sun_slice_id() {
    return (lighting_sun_flags() >> 16u) & 255u;
}
// This frame's band. The nearest probe row, or the page column of the surface
// cell (page_x = cell.x >> 3, the same split as lighting_sun_slot_slice, at the
// one texel size). A pixel on either walks probes. The rest reuse stored bounce.
bool lighting_sun_pixel_band(uvec2 p, vec3 pos, vec3 n) {
    uint bands = lighting_sun_n();
    uint slice = lighting_sun_slice_id();
    float tile = float(max(pc.params.y, 1u));
    float gy = (float(p.y) + 0.5) / tile - 0.5;
    int row = int(floor(gy + 0.5));
    int rows = int(max(pc.dims.w, 1u));
    row = clamp(row, 0, rows - 1);
    if (uint(row) % bands == slice) {
        return true;
    }
    uint axd = lighting_axis(n);
    uint ax = axd >> 1u;
    int cell_x = int(floor(pos[(ax + 1u) % 3u] / LIGHTING_SC_TEXEL));
    uint page_x = uint(cell_x) >> 3u;
    return page_x % bands == slice;
}
vec3 lighting_sun_committed_dir() {
    return vec3(uintBitsToFloat(ckeys.key[LIGHTING_SUN_AT + 2u]), uintBitsToFloat(ckeys.key[LIGHTING_SUN_AT + 3u]), uintBitsToFloat(ckeys.key[LIGHTING_SUN_AT + 4u]));
}
float lighting_sun_committed_light() {
    return uintBitsToFloat(ckeys.key[LIGHTING_SUN_AT + 5u]);
}
float lighting_sun_live_light() {
    return uintBitsToFloat(ckeys.key[LIGHTING_SUN_AT + 6u]);
}
bool lighting_catch_up() {
    return ckeys.key[LIGHTING_SUN_AT + 7u] != 0u;
}
bool lighting_sun_probe_slice(uint probe) {
    uint cols = max(pc.dims.z, 1u);
    return (probe / cols) % lighting_sun_n() == lighting_sun_slice_id();
}
bool lighting_sun_slot_slice(uint slot) {
    ivec4 c = lighting_sc_cell(slot);
    uint page_x = uint(c.x) >> 3u;
    return page_x % lighting_sun_n() == lighting_sun_slice_id();
}
bool lighting_sun_hold_kept(uint h) {
    if (!lighting_sun_small()) {
        return false;
    }
    uint rays = lighting_rays();
    uint pk = h / rays / LIGHTING_STRATA;
    if (lighting_sun_probe_slice(pk) || !lighting_stratum_held(pk)) {
        return false;
    }
    if (floatBitsToUint(work.v[lighting_probe_at(pk) + 1u].w) == lighting_frame() + LIGHTING_STRATA) {
        return false;
    }
    return floatBitsToUint(work.v[lighting_kept_light_at() + h / 4u][h % 4u]) != 0u;
}
bool lighting_sun_hold_fill(uint h) {
    if (!lighting_sun_small()) {
        return false;
    }
    uint pk = h / lighting_rays() / LIGHTING_STRATA;
    if (lighting_sun_probe_slice(pk)) {
        return false;
    }
    return floatBitsToUint(work.v[lighting_fill_light_at() + h / 4u][h % 4u]) != 0u;
}
// One thread records that this dispatch took the sun path. Bit 0: a small sun
// step. Bit 1: the corner catch-up. The slice id is the band this frame shades
// with the live sun. The pads are otherwise unused (unfilled mask alignment).
void lighting_sun_mark() {
    if (gl_GlobalInvocationID.x != 0u || gl_GlobalInvocationID.y != 0u || gl_GlobalInvocationID.z != 0u) {
        return;
    }
    uint mark = 0u;
    if (lighting_sun_small()) {
        mark |= 1u;
    }
    if (lighting_catch_up()) {
        mark |= 2u;
    }
    gstats.unfilled_pad0 = mark;
    gstats.unfilled_pad1 = lighting_sun_slice_id();
}
#endif

// Layer-1 input: rgb, luminance variance (-1 unknown), mask weight.
vec4 lighting_svgf_in(uint s, out float var) {
    vec4 own = cache.v[3u * s + 2u];
    float t = floor(max(own.w, 0.0));
    var = -1.0;
    bool wrc = cache.v[3u * s + 1u].w > 0.2;
    if (t < 0.5 || !lighting_svgf_active(s)) {
        return vec4(0.0);
    }
    // S72V7: each texel's light is carried to now (its sun/sky gain, as the
    // pixels show it) before the levels mix them: texels relit at different
    // times under a moving sun held different scales, and the luminance stop
    // kept those groups apart (camouflage plateaus at dawn).
    float g = lighting_sc_sun_gain(s);
#ifdef GENOS_SUN_SHADER
    // The page column catching up to the live sun. The rest keep the committed scale.
    if (lighting_sun_small() && lighting_sun_slot_slice(s)) {
        float c = max(lighting_sun_committed_light(), 1.0e-4);
        g *= clamp(lighting_sun_live_light() / c, 0.0, LIGHTING_LS_MAX);
    }
#endif
    if (!wrc) {
        vec4 eb = cache.v[LIGHTING_SC_EST_AT + 2u * s + 1u];
        if (eb.x > 0.0 && t >= 4.0) {
            var = max(cache.v[LIGHTING_SC_EST_AT + 2u * s].w, 0.0) * g * g;
        }
    }
    // S72V5: a texel with samples counts fully, as a pixel does in SVGF (its
    // filter has no count weights); S72V's t / 4 mask weighed the long-settled
    // texels over the ones under changing light (kept short), which darkened
    // lit areas, and left the push to the WRC.
    return vec4(own.rgb * g, 1.0);
}
float lighting_svgf_wzn(uint p, uint q) {
    vec4 np = cache.v[3u * p + 1u];
    vec4 nq = cache.v[3u * q + 1u];
    float dz = abs(dot(cache.v[3u * q].xyz - cache.v[3u * p].xyz, np.xyz));
    float wz = exp(-dz / (LIGHTING_SVGF_SIGMA_Z * 0.5 * np.w + 1.0e-6));
    float wn = pow(max(dot(np.xyz, nq.xyz), 0.0), LIGHTING_SVGF_SIGMA_N);
    return wz * wn;
}
void lighting_svgf_store(uint at, uint s, vec3 rgb, float var, float m) {
    cache.v[at + s] = vec4(uintBitsToFloat(lighting_pack_rgb9e5(max(rgb, vec3(0.0)))), var, m, 0.0);
}
vec4 lighting_svgf_load(uint at, uint s, out float var) {
    vec4 v = cache.v[at + s];
    var = v.y;
    return vec4(lighting_unpack_rgb9e5(floatBitsToUint(v.x)), v.z);
}
const vec3 LIGHTING_SVGF_LW = vec3(0.2126, 0.7152, 0.0722);
// S77D dirty-page a-trous (Anthony 17:31, 17:53): every level of the filter is
// kept per slot (LIGHTING_SC_LVL_AT + k: level 0 the variance stage, 1-5 the
// a-trous levels), so a texel whose inputs did not change keeps its value at every
// level and its neighbours still read the right level from it. Level 0: a texel
// is recomputed when its own input (rgb x sun gain, mask, temporal variance)
// moved more than LIGHTING_DIRTY_EPS (1 %) from the stored one, when it was not
// active last frame, or while its variance is the spatial estimate (young
// texels; their 7 x 7 inputs change). A level k value that changed by more than
// LIGHTING_DIRTY_EPS marks every texel that reads it at level k + 1 (its 5 x 5
// taps at step 2^k, which hold the 3 x 3 variance blur) dirty for that level
// (scatter: neighbour lookups only around what changed). A dirty texel is
// recomputed; a clean one keeps its level. The push (stage 6) runs for all, as
// before. Deviation: SVGF re-filters every pixel every frame; changes under 1 %
// are held until they add up past it (bounded 1 % lag).
const bool LIGHTING_DIRTY_ATROUS = true;
const float LIGHTING_DIRTY_EPS = 0.01;
const bool LIGHTING_DIRTY_NOISE = false; // S78X: off (lum a 0.348 -> 0.345, a lag toward darker)
const float LIGHTING_DIRTY_SE = 2.0;
const uint LIGHTING_SC_LVL_AT = 15u * LIGHTING_CACHE_SLOTS;
const uint LIGHTING_SC_DIRTY_AT = 21u * LIGHTING_CACHE_SLOTS;
uint lighting_svgf_lvl(uint k) {
    return LIGHTING_SC_LVL_AT + k * LIGHTING_CACHE_SLOTS;
}
// Dirty stamp of level k (1..5) of slot s.
uint lighting_svgf_dirty_word(uint k) {
    return k - 1u;
}
bool lighting_svgf_is_dirty(uint s, uint k) {
    uint w = k - 1u;
    vec4 d = cache.v[LIGHTING_SC_DIRTY_AT + (w >> 2u) * LIGHTING_CACHE_SLOTS + s];
    return floatBitsToUint(d[w & 3u]) == lighting_frame();
}
// S78X quiet a-trous: work lists. A mark of level k (1-5) appends the slot to
// that level's list once a frame (atomic exchange on a per-slot, per-level
// stamp); the level's dispatch then runs only the listed slots (one uniform
// count read for the rest), instead of every thread of 1M slots reading its
// stamps. Still scene: ~5-18k dirty slots a level against ~108k filtered.
//
// That list is slower once most of the filtered set is dirty. Building it is
// a contended atomicAdd on one counter (plus a scattered slot id) inside the
// filter, and the dispatch then walks slots in append order, so the 5x5 taps
// miss the coalescing of a dense slot sweep. S78A was rejected for the same
// locality cost. The moving scene paid both and got the dense sweep anyway
// once a list passed LIGHTING_ATROUS_LIST_CAP.
//
// The fallback uses last frame's fullest level (LIGHTING_ATROUS_PREV_AT,
// millionths of the filtered texels). Above the threshold the level does not
// build a list and every thread keeps its own slot, skipping clean stamps as
// before. A list that still overflows the cap does the same. Threshold 0 is
// always dense; a threshold of 1 keeps the lists up to the cap.
const bool LIGHTING_ATROUS_LISTS = true;
const uint LIGHTING_ATROUS_LIST_CAP = 65536u;
const uint LIGHTING_ATROUS_MARK_AT = LIGHTING_CHECK_P_AT + 16u;
const uint LIGHTING_ATROUS_LIST_AT = LIGHTING_ATROUS_MARK_AT + 5u * LIGHTING_CACHE_SLOTS;
// Past the S78Z verify/wake reservation. Host writes the threshold; the rest
// is this frame's list decision and the dense-path recount.
// After the compose words (count, y, z, full, stamps, list, packs, rects); this used to alias LIGHTING_COMPOSE_AT.
const uint LIGHTING_ATROUS_PPM_AT = LIGHTING_COMPOSE_AT + 4u + 2u * LIGHTING_COMPOSE_CAP + 5u * LIGHTING_CACHE_SLOTS;
const uint LIGHTING_ATROUS_PREV_AT = LIGHTING_ATROUS_PPM_AT + 1u;
const uint LIGHTING_ATROUS_BIN_AT = LIGHTING_ATROUS_PREV_AT + 1u;
const uint LIGHTING_ATROUS_MODE_AT = LIGHTING_ATROUS_BIN_AT + 64u;
const uint LIGHTING_ATROUS_DBIN_AT = LIGHTING_ATROUS_MODE_AT + 5u;
uint lighting_atrous_list(uint k) {
    return LIGHTING_ATROUS_LIST_AT + (k - 1u) * (LIGHTING_ATROUS_LIST_CAP + 1u);
}
// True when this frame's levels 1-5 should append work lists. Last frame's
// fraction is what decides: the mark that would cross the threshold is already
// inside the filter, so this frame cannot both count it cheaply and avoid the
// atomic. The next frame then takes the dense sweep.
bool lighting_atrous_listing() {
    if (!LIGHTING_ATROUS_LISTS) {
        return false;
    }
    uint ppm = ckeys.key[LIGHTING_ATROUS_PPM_AT];
    if (ppm == 0u) {
        return false;
    }
    return ckeys.key[LIGHTING_ATROUS_PREV_AT] <= ppm;
}
void lighting_svgf_mark(uint s, uint k) {
    if (s == ~0u) {
        return;
    }
    if (LIGHTING_ATROUS_LISTS && lighting_atrous_listing()) {
        uint list = lighting_atrous_list(k);
        // Past the cap the dispatch falls back; stop adding to the one counter.
        if (ckeys.key[list] <= LIGHTING_ATROUS_LIST_CAP && atomicExchange(ckeys.key[LIGHTING_ATROUS_MARK_AT + (k - 1u) * LIGHTING_CACHE_SLOTS + s], lighting_frame()) != lighting_frame()) {
            uint at = atomicAdd(ckeys.key[list], 1u);
            if (at < LIGHTING_ATROUS_LIST_CAP) {
                ckeys.key[list + 1u + at] = s;
            }
        }
    }
    uint w = k - 1u;
    cache.v[LIGHTING_SC_DIRTY_AT + (w >> 2u) * LIGHTING_CACHE_SLOTS + s][w & 3u] = uintBitsToFloat(lighting_frame());
}
// Stage 6, one thread: the fullest level's fraction, the mode the levels
// actually ran, and the fraction the next frame's list decision reads.
void lighting_atrous_publish() {
    uint nlive = 0u;
    [[dont_unroll]] for (uint i = 0u; i < 64u; i++) {
        nlive += ckeys.key[LIGHTING_ATROUS_BIN_AT + i];
    }
    uint frac = 0u;
    uint mode = 0u;
    [[dont_unroll]] for (uint k = 1u; k <= 5u; k++) {
        uint m = ckeys.key[LIGHTING_ATROUS_MODE_AT + k - 1u];
        if (m == 0u) {
            continue;
        }
        uint cnt = 0u;
        if (m == 2u) {
            mode = 2u;
            [[dont_unroll]] for (uint i = 0u; i < 64u; i++) {
                cnt += ckeys.key[LIGHTING_ATROUS_DBIN_AT + (k - 1u) * 64u + i];
            }
        } else {
            cnt = ckeys.key[lighting_atrous_list(k)];
            if (mode == 0u) {
                mode = 1u;
            }
        }
        uint f = 0u;
        if (nlive > 0u) {
            f = uint(min(1000000.0, 1000000.0 * float(cnt) / float(nlive)));
        } else if (cnt > 0u) {
            f = 1000000u;
        }
        if (f > frac) {
            frac = f;
        }
    }
    ckeys.key[LIGHTING_ATROUS_PREV_AT] = frac;
    gstats.atrous_frac = frac;
    gstats.atrous_mode = mode;
    gstats.atrous_ppm = ckeys.key[LIGHTING_ATROUS_PPM_AT];
    gstats.atrous_active = nlive;
}
bool lighting_svgf_moved(vec4 a, float va, vec4 b, float vb) {
    float la = dot(a.rgb, vec3(0.2126, 0.7152, 0.0722));
    float lb = dot(b.rgb, vec3(0.2126, 0.7152, 0.0722));
    return abs(la - lb) > LIGHTING_DIRTY_EPS * max(max(la, lb), 1.0e-4)
        || abs(a.w - b.w) > LIGHTING_DIRTY_EPS
        || abs(va - vb) > 2.0 * LIGHTING_DIRTY_EPS * max(max(abs(va), abs(vb)), 1.0e-8);
}
// Kernel step in cells. Defined below the activity stamps.
int lighting_svgf_step_for(uint it);
// Level k of slot changed: its readers at level k + 1 are dirty.
void lighting_svgf_scatter(uint slot, uint k) {
    if (k >= 5u) {
        return;
    }
    int step = lighting_svgf_step_for(k);
    [[dont_unroll]] for (int j = -2; j <= 2; j++) {
        [[dont_unroll]] for (int i = -2; i <= 2; i++) {
            lighting_svgf_mark((i == 0 && j == 0) ? slot : lighting_sc_cell_slot(slot, ivec2(-i, -j) * step), k + 1u);
        }
    }
}
// Activity last frame (slot 22.y: 1 active).
bool lighting_svgf_was_active(uint s) {
    return cache.v[LIGHTING_SC_DIRTY_AT + LIGHTING_CACHE_SLOTS + s].y > 0.5;
}
void lighting_svgf_set_active(uint s, bool a) {
    cache.v[LIGHTING_SC_DIRTY_AT + LIGHTING_CACHE_SLOTS + s].y = a ? 1.0 : 0.0;
}
// S78B stamps (slot 22.z: the spatial variance's 7 x 7 inputs changed; 22.w:
// level 5 or its +-1 neighbours changed, the push reads them). A stamp holds
// this frame and the next (a mark made by a neighbour of the same dispatch may
// land after the texel looked).
bool lighting_svgf_stamp_recent(uint s, uint c) {
    float f = cache.v[LIGHTING_SC_DIRTY_AT + LIGHTING_CACHE_SLOTS + s][c];
    return f != 0.0 && lighting_frame() - floatBitsToUint(f) <= 1u;
}
void lighting_svgf_stamp(uint s, uint c) {
    if (s != ~0u) {
        cache.v[LIGHTING_SC_DIRTY_AT + LIGHTING_CACHE_SLOTS + s][c] = uintBitsToFloat(lighting_frame());
    }
}
// Kernel step in cells. Dammertz HPG 2010 doubles the step each iteration.
int lighting_svgf_step_for(uint it) {
    return 1 << int(it);
}
// How many wavelet iterations this texel still needs.
//
// Schied et al. 2017 (SVGF) sec. 4.2 use the variance to set the luminance
// edge-stop. They still run every iteration. ReLAX (Zhdan, NVIDIA Real-Time
// Denoisers) reduces the a-trous iteration count as history length grows.
// This takes the smaller of the two limits, so either a long sample age or a
// variance under the threshold drops iterations.
//
// Deviations: sample age is the surface-cache set count, not a screen-space
// history length. No previous frame is kept. SVGF's own filter does not drop
// iterations; the drop is the ReLAX rule, gated by the SVGF variance as well.
// Bands: 16 sets or variance at the threshold -> 1 iteration; 8 sets or four
// times the threshold -> 3; otherwise the configured count.
uint lighting_svgf_pass_limit(uint slot, float var, float lum) {
    uint full = lighting_denoise_iters();
    if (!lighting_denoise_adaptive()) {
        return full;
    }
    float sets = floor(max(cache.v[3u * slot + 2u].w, 0.0));
    float rel = sqrt(max(var, 0.0)) / max(lum, 1.0e-3);
    float thr = lighting_denoise_variance();
    uint by_age = full;
    if (sets >= 16.0) {
        by_age = 1u;
    } else if (sets >= 8.0) {
        by_age = min(full, 3u);
    }
    uint by_var = full;
    // The knob tops out at 0.1 (1000 steps of 0.0001). That value forces one
    // iteration. A young texel's relative deviation sits above 0.1, so the
    // old test never entered the skip the settled cache takes (16 sets).
    // ReLAX keys the drop off history length; this stop is the short-run
    // equivalent, flagged as a deviation in S79.
    uint thr_q = (gstats.ctrl_bits >> 22u) & 1023u;
    if (thr_q >= 1000u) {
        by_var = 1u;
    } else if (thr > 0.0 && rel <= thr) {
        by_var = 1u;
    } else if (thr > 0.0 && rel <= thr * 4.0) {
        by_var = min(full, 3u);
    }
    return min(full, min(by_age, by_var));
}
// The push reads buffer A. A skipped iteration must publish the last level
// that actually ran (level `applied`), even when this level is clean. The
// dirty early-out used to return first, so A kept an older filter or the
// zeros from allocation, and the push blended that hole toward the 25 cm
// WRC: darker and blotchy. Lavapipe zeros the cache and the old test warmed
// up with the full filter, so A was already the reference and the miss was
// invisible. A driver that does not zero, or a run that starts in the skip,
// shows it.
void lighting_svgf_publish_applied(uint slot, uint applied) {
    vec4 now = cache.v[lighting_svgf_lvl(applied) + slot];
    vec4 prev = cache.v[LIGHTING_SC_ATROUS_A + slot];
    cache.v[LIGHTING_SC_ATROUS_A + slot] = now;
    if (prev.x != now.x || prev.y != now.y || prev.z != now.z) {
        lighting_svgf_stamp(slot, 3u);
        lighting_svgf_stamp(lighting_sc_cell_slot(slot, ivec2(1, 0)), 3u);
        lighting_svgf_stamp(lighting_sc_cell_slot(slot, ivec2(-1, 0)), 3u);
        lighting_svgf_stamp(lighting_sc_cell_slot(slot, ivec2(0, 1)), 3u);
        lighting_svgf_stamp(lighting_sc_cell_slot(slot, ivec2(0, -1)), 3u);
    }
}
// An inactive texel: mask 0 at every level (a flip marks its readers once).
void lighting_svgf_inactive(uint slot) {
    bool was = lighting_svgf_was_active(slot);
    [[dont_unroll]] for (uint k = 0u; k <= 5u; k++) {
        lighting_svgf_store(lighting_svgf_lvl(k), slot, vec3(0.0), 0.0, 0.0);
    }
    if (was) {
        lighting_svgf_set_active(slot, false);
        lighting_svgf_scatter(slot, 0u);
    }
}
void lighting_svgf_variance(uint slot) {
    float var;
    vec4 v = lighting_svgf_in(slot, var);
    bool spatial = var < 0.0;
    bool flip = LIGHTING_DIRTY_ATROUS && !lighting_svgf_was_active(slot);
    if (LIGHTING_DIRTY_ATROUS && !flip) {
        float vo;
        vec4 o = lighting_svgf_load(lighting_svgf_lvl(0u), slot, vo);
        // S78B: a spatial estimate is redone only when its value or one of its
        // 7 x 7 inputs moved (stamp 22.z), not every frame.
        if (spatial ? (!lighting_svgf_moved(v, vo, o, vo) && !lighting_svgf_stamp_recent(slot, 2u)) : !lighting_svgf_moved(v, var, o, vo)) {
            return;
        }
        // S78X: noise-aware level 0 (Anthony: rewrite thresholds from the
        // texel's own noise, not a fixed %): a texel whose new mean is within
        // LIGHTING_DIRTY_SE standard errors of the stored one, same mask, keeps
        // the stored value; refinement inside its own noise no longer dirties
        // the five levels around it.
        if (LIGHTING_DIRTY_NOISE && !spatial && abs(v.w - o.w) <= LIGHTING_DIRTY_EPS) {
            vec2 nz = lighting_sc_noise(slot);
            float dl = abs(dot(v.rgb - o.rgb, vec3(0.2126, 0.7152, 0.0722)));
            if (nz.x >= 0.0 && dl <= LIGHTING_DIRTY_SE * nz.y * lighting_sc_sun_gain(slot)) {
                return;
            }
        }
    }
    if (var < 0.0) {
        // Spatial estimate, 7 x 7 moments weighted by w_z w_n.
        float s1 = 0.0;
        float s2 = 0.0;
        float sw = 0.0;
        [[dont_unroll]] for (int j = -3; j <= 3; j++) {
            [[dont_unroll]] for (int i = -3; i <= 3; i++) {
                uint q = (i == 0 && j == 0) ? slot : lighting_sc_cell_slot(slot, ivec2(i, j));
                if (q == ~0u) {
                    continue;
                }
                float vq;
                vec4 x = lighting_svgf_in(q, vq);
                if (x.w <= 0.0) {
                    continue;
                }
                float w = lighting_svgf_wzn(slot, q) * x.w;
                float l = dot(x.rgb, LIGHTING_SVGF_LW);
                s1 += w * l;
                s2 += w * (l * l + max(vq, 0.0));
                sw += w;
            }
        }
        float t = floor(max(cache.v[3u * slot + 2u].w, 0.0));
        var = sw > 0.0 ? max(s2 / sw - (s1 / sw) * (s1 / sw), 0.0) * 4.0 / max(t, 1.0) : 0.0;
    }
    if (!LIGHTING_DIRTY_ATROUS) {
        lighting_svgf_store(LIGHTING_SC_ATROUS_B, slot, v.rgb, var, v.w);
        return;
    }
    float vo;
    vec4 o = lighting_svgf_load(lighting_svgf_lvl(0u), slot, vo);
    lighting_svgf_store(lighting_svgf_lvl(0u), slot, v.rgb, var, v.w);
    if (flip) {
        lighting_svgf_set_active(slot, true);
    }
    // Stored packed (rgb9e5): compare what the readers will see.
    float vn;
    vec4 nw = lighting_svgf_load(lighting_svgf_lvl(0u), slot, vn);
    if (flip || lighting_svgf_moved(nw, vn, o, vo)) {
        lighting_svgf_scatter(slot, 0u);
    }
    // S78B: the value itself moved: the spatial variances that read it.
    if (flip || lighting_svgf_moved(vec4(nw.rgb, nw.w), 0.0, vec4(o.rgb, o.w), 0.0)) {
        [[dont_unroll]] for (int j = -3; j <= 3; j++) {
            [[dont_unroll]] for (int i = -3; i <= 3; i++) {
                lighting_svgf_stamp((i == 0 && j == 0) ? slot : lighting_sc_cell_slot(slot, ivec2(i, j)), 2u);
            }
        }
    }
}
void lighting_svgf_iter(uint slot, uint it) {
#ifndef LIGHTING_DENOISE_PLAIN
    // Before the dirty early-out. A clean texel still has to publish the last
    // applied level, or the push keeps a stale or empty A. Not in the plain
    // kernel: high leaves adaptive off (S79).
    if (lighting_denoise_adaptive()) {
        float v0;
        vec4 s0 = lighting_svgf_load(lighting_svgf_lvl(0u), slot, v0);
        uint limit = lighting_svgf_pass_limit(slot, v0, dot(s0.rgb, LIGHTING_SVGF_LW));
        if (it + 1u > limit) {
            if (it + 1u == lighting_denoise_iters()) {
                lighting_svgf_publish_applied(slot, limit);
            }
            return;
        }
    }
#endif
    if (LIGHTING_DIRTY_ATROUS && !lighting_svgf_is_dirty(slot, it + 1u)) {
        return;
    }
    uint src = LIGHTING_DIRTY_ATROUS ? lighting_svgf_lvl(it) : ((it & 1u) == 0u ? LIGHTING_SC_ATROUS_B : LIGHTING_SC_ATROUS_A);
    uint dst = LIGHTING_DIRTY_ATROUS ? lighting_svgf_lvl(it + 1u) : ((it & 1u) == 0u ? LIGHTING_SC_ATROUS_A : LIGHTING_SC_ATROUS_B);
    float vold;
    vec4 old = lighting_svgf_load(dst, slot, vold);
    int step = lighting_svgf_step_for(it);
    float varp;
    vec4 vp = lighting_svgf_load(src, slot, varp);
    float lp = dot(vp.rgb, LIGHTING_SVGF_LW);
    // g3x3: 3 x 3 Gaussian of the input variance.
    float gv = 0.0;
    float gw = 0.0;
    [[dont_unroll]] for (int j = -1; j <= 1; j++) {
        [[dont_unroll]] for (int i = -1; i <= 1; i++) {
            uint q = (i == 0 && j == 0) ? slot : lighting_sc_cell_slot(slot, ivec2(i, j));
            if (q == ~0u) {
                continue;
            }
            float vq;
            vec4 x = lighting_svgf_load(src, q, vq);
            float k = (i == 0 ? 0.5 : 0.25) * (j == 0 ? 0.5 : 0.25);
            gv += k * vq;
            gw += k;
        }
    }
    gv = gw > 0.0 ? gv / gw : varp;
    float phi = LIGHTING_SVGF_SIGMA_L * sqrt(max(gv, 0.0)) + 1.0e-10;
    const float h1[5] = float[5](1.0 / 16.0, 0.25, 0.375, 0.25, 1.0 / 16.0);
    vec3 sum = vec3(0.0);
    float wm = 0.0;
    float wall = 0.0;
    float vsum = 0.0;
    [[dont_unroll]] for (int j = -2; j <= 2; j++) {
        [[dont_unroll]] for (int i = -2; i <= 2; i++) {
            uint q = (i == 0 && j == 0) ? slot : lighting_sc_cell_slot(slot, ivec2(i, j) * step);
            if (q == ~0u) {
                continue;
            }
            float vq;
            vec4 x = lighting_svgf_load(src, q, vq);
            float w = h1[i + 2] * h1[j + 2] * lighting_svgf_wzn(slot, q);
            if (q != slot && vp.w > 0.0 && x.w > 0.0) {
                w *= exp(-abs(lp - dot(x.rgb, LIGHTING_SVGF_LW)) / phi);
            }
            float a = w * x.w;
            sum += a * x.rgb;
            wm += a;
            wall += w;
            vsum += a * a * max(vq, 0.0);
        }
    }
    if (wm > 0.0) {
        // S72V3 certainty as Gortler et al. 1996's pull, w' = min(1, sum h w)
        // with the unnormalized 2D kernel (4 x the normalized one): a level whose
        // footprint holds a quarter of full support is fully certain. S72V took
        // the normalized certainty (Knutsson & Westin), so with IC sites 2 apart
        // every texel stayed at 1/4 and the push gave 3/4 to the 25 cm WRC:
        // the big splotches, and the WRC's lower level darkened the picture.
        lighting_svgf_store(dst, slot, sum / wm, vsum / (wm * wm), wall > 0.0 ? min(1.0, 4.0 * wm / wall) : 0.0);
    } else {
        lighting_svgf_store(dst, slot, vec3(0.0), 0.0, 0.0);
    }
    if (LIGHTING_DIRTY_ATROUS) {
        float vn;
        vec4 nw = lighting_svgf_load(dst, slot, vn);
        if (it + 1u == lighting_denoise_iters()) {
            // The push and the IC placement read the last level from A.
            // Five iterations land on level 5. Fewer iterations used to jump
            // to the push while A still held an earlier level, so compose
            // showed an unfiltered field (white boxes, noisy surfaces).
            cache.v[LIGHTING_SC_ATROUS_A + slot] = cache.v[dst + slot];
        }
        if (lighting_svgf_moved(nw, vn, old, vold)) {
            lighting_svgf_scatter(slot, it + 1u);
            if (it + 1u == lighting_denoise_iters()) {
                // S78B: the push of this texel and of its +-1 neighbours (IC gradient).
                lighting_svgf_stamp(slot, 3u);
                lighting_svgf_stamp(lighting_sc_cell_slot(slot, ivec2(1, 0)), 3u);
                lighting_svgf_stamp(lighting_sc_cell_slot(slot, ivec2(-1, 0)), 3u);
                lighting_svgf_stamp(lighting_sc_cell_slot(slot, ivec2(0, 1)), 3u);
                lighting_svgf_stamp(lighting_sc_cell_slot(slot, ivec2(0, -1)), 3u);
            }
        }
    }
}
float lighting_svgf_lum_at(uint slot, ivec2 off, out bool ok) {
    // A zero offset is this slot. cell_slot(0, 0) hashes a point that is a
    // cell off for about half the texels (S72V), so the center tap would miss.
    uint q = (off.x == 0 && off.y == 0) ? slot : lighting_sc_cell_slot(slot, off);
    ok = false;
    if (q == ~0u) {
        return 0.0;
    }
    float vq;
    vec4 x = lighting_svgf_load(LIGHTING_SC_ATROUS_A, q, vq);
    ok = x.w > 0.0;
    return dot(x.rgb, LIGHTING_SVGF_LW);
}
const uint LIGHTING_PUSH_IDLE = 16u;
void lighting_svgf_push(uint slot) {
    vec4 nb = cache.v[3u * slot + 1u];
    if (nb.w > 0.2) {
        return;
    }
    float varp = 0.0;
    vec4 f = lighting_svgf_load(LIGHTING_SC_ATROUS_A, slot, varp);
    float W = clamp(f.w, 0.0, 1.0);
    // S78B: the push is redone when this texel's last level (or a +-1
    // neighbour's) moved, when it leans on the WRC (W < 1: the cells change
    // without telling their texels), and otherwise one frame in
    // LIGHTING_PUSH_IDLE by hash (the +-4 R clamp, a texel just shown).
    if (LIGHTING_DIRTY_ATROUS && W >= 0.999 && !lighting_svgf_stamp_recent(slot, 3u)
        && cache.v[LIGHTING_SC_ATROUS_B + slot].w != 0.0
        && ((lighting_hash(lighting_sched(slot) ^ 0x7f4a7c15u) + lighting_frame()) % LIGHTING_PUSH_IDLE) != 0u) {
        // S72V10 trusts the field for two frames from this stamp. The idle skip
        // does not rewrite the field, so while the camera is held the stamp stays
        // current and compose keeps showing that field instead of dropping it
        // for the WRC on a timer. A later push that writes a different field
        // marks the blocks (lighting_compose_note_field).
        if (lighting_view_held()) {
            uint pf = floatBitsToUint(cache.v[LIGHTING_SC_ATROUS_B + slot].w);
            if (pf != 0u && lighting_frame() - pf >= 2u) {
                cache.v[LIGHTING_SC_ATROUS_B + slot].w = uintBitsToFloat(lighting_frame());
            }
        }
        return;
    }
    // Coarse: the WRC cells (2x size) bilinear, their filtered values and masks.
    vec3 p = cache.v[3u * slot].xyz;
    vec3 n = nb.xyz;
    float cs = 0.25;
    uint ax = lighting_axis(n) >> 1u;
    uint ua = (ax + 1u) % 3u;
    uint ub = (ax + 2u) % 3u;
    vec2 tc = (lighting_sc_true_cell(slot) + 0.5) * nb.w;
    vec2 uv = tc / cs - 0.5;
    vec2 g0 = floor(uv);
    vec2 fr = uv - g0;
    vec3 csum = vec3(0.0);
    float cw = 0.0;
    [[dont_unroll]] for (uint k = 0u; k < 4u; k++) {
        vec2 c = g0 + vec2(float(k & 1u), float(k >> 1u));
        vec3 q = p;
        q[ua] = (c.x + 0.5) * cs;
        q[ub] = (c.y + 0.5) * cs;
        float bil = ((k & 1u) == 1u ? fr.x : 1.0 - fr.x) * ((k >> 1u) == 1u ? fr.y : 1.0 - fr.y);
        uint s = lighting_cache_find_sized(q, n, false, cs, p);
        if (s == ~0u || bil <= 0.0 || abs(dot(cache.v[3u * s].xyz - p, n)) > 0.5 * cs) {
            continue;
        }
        float vq;
        vec4 x = lighting_svgf_load(LIGHTING_SC_ATROUS_A, s, vq);
        csum += bil * x.w * x.rgb;
        cw += bil * x.w;
    }
    vec3 outc;
    if (cw > 0.0) {
        outc = W * f.rgb + (1.0 - W) * (csum / cw);
    } else if (W > 0.0) {
        outc = f.rgb;
    } else {
        outc = vec3(0.0);
    }
    uint field_pack = lighting_pack_rgb9e5(outc);
    cache.v[LIGHTING_WORLD_AT + slot].y = uintBitsToFloat(field_pack);
    cache.v[LIGHTING_SC_ATROUS_B + slot].w = uintBitsToFloat(lighting_frame());
    lighting_compose_note_field(slot, field_pack);
    // IC placement for the next rounds.
    if (W > 0.0 || cw > 0.0) {
        float e = dot(outc, LIGHTING_SVGF_LW);
        bool o1, o2, o3, o4;
        float lx1 = lighting_svgf_lum_at(slot, ivec2(1, 0), o1);
        float lx0 = lighting_svgf_lum_at(slot, ivec2(-1, 0), o2);
        float ly1 = lighting_svgf_lum_at(slot, ivec2(0, 1), o3);
        float ly0 = lighting_svgf_lum_at(slot, ivec2(0, -1), o4);
        float fl = dot(f.rgb, LIGHTING_SVGF_LW);
        float gx = (o1 && o2) ? 0.5 * (lx1 - lx0) : (o1 ? lx1 - fl : (o2 ? fl - lx0 : 0.0));
        float gy = (o3 && o4) ? 0.5 * (ly1 - ly0) : (o3 ? ly1 - fl : (o4 ? fl - ly0 : 0.0));
        float grad = length(vec2(gx, gy));
        float r = LIGHTING_IC_RMAX;
        if (grad > 1.0e-6) {
            r = min(r, e / grad);
        }
        // S72V2: no R_geo term. Texels exist only where pixels see the surface,
        // so a missing neighbour is not where the surface ends; S72V's "twice
        // the distance to a missing texel" held R at 4-8 cells everywhere and
        // nothing thinned [deviation: Ward's harmonic mean ray distance would
        // need the per-ray hit distances kept].
        // Neighbour clamping (Krivanek 2006, 5.2), against the stored R 4 cells away.
        [[dont_unroll]] for (uint d = 0u; d < 4u; d++) {
            ivec2 dir = d == 0u ? ivec2(4, 0) : (d == 1u ? ivec2(-4, 0) : (d == 2u ? ivec2(0, 4) : ivec2(0, -4)));
            uint q = lighting_sc_cell_slot(slot, dir);
            if (q != ~0u) {
                float rq = cache.v[LIGHTING_SC_SPARSE_AT + q].w;
                if (rq > 0.0) {
                    r = min(r, rq + 4.0);
                }
            }
        }
        cache.v[LIGHTING_SC_SPARSE_AT + slot].w = max(r, 0.5);
    }
}

// ---------------------------------------------------------------------------
// S74A A-SVGF change-adaptive rays (Schied, Peters & Dachsbacher 2018,
// "Gradient Estimation for Real-Time Adaptive Temporal Filtering"; Schied et
// al. 2019) on the surface-cache texels, with GIBS's MSME as the history.
// Every round: in each 3 x 3 block of texels one (by hash of block and round,
// A-SVGF's random pick per stratum) that is resolved re-traces ONE ray of its
// last relight with the same seed (direction, lamp pick, quarter): what that ray
// brings now against what it brought then is the temporal gradient (a still
// scene gives exactly 0: same ray, same hit, same light). lambda = min(1,
// |delta| / max(L_now, L_then)) (A-SVGF eq. 6), written to the 9 texels of the
// block; alpha' = (1 - lambda) alpha + lambda (A-SVGF eq. 7) in the MSME
// update. A texel is resolved while lambda < LIGHTING_ASVGF_EPS and its mean's
// standard error sqrt(var / n) is under LIGHTING_ASVGF_SE of its mean (ReBLUR-
// style convergence); a resolved texel takes no rays besides its checks. One
// with lambda >= eps is relit with probability min(1, lambda / FULL) a round.
// Lamp and mover hints only nominate: every resolved texel there is checked,
// not just its block's pick. Age floor (Lumen: oldest pages refreshed slowly):
// a texel not relit for LIGHTING_ASVGF_AGE frames is scheduled as before.
const bool LIGHTING_SC_ASVGF = true;
bool lighting_sc_asvgf_on() { return LIGHTING_SC_ASVGF && lighting_sc_svgf(); }
const bool LIGHTING_GRAD_CHECKS_ON = true;
const uint LIGHTING_SC_GRAD_AT = 11u * LIGHTING_CACHE_SLOTS;
const uint LIGHTING_SC_GRAD2_AT = 12u * LIGHTING_CACHE_SLOTS;
const uint LIGHTING_GRAD_COUNT_AT = LIGHTING_SC_HIST_AT + 72u;
const uint LIGHTING_EXT_AT = LIGHTING_ARGS_AT + 16u + 3u * 65536u;
const uint LIGHTING_SHADOW_EPOCH_AT = LIGHTING_EXT_AT;
const uint LIGHTING_GRAD_LIST_AT = LIGHTING_EXT_AT + 1024u;
const float LIGHTING_ASVGF_EPS = 0.05;
const float LIGHTING_ASVGF_SE = 0.05;
const float LIGHTING_ASVGF_FULL = 0.25;
const uint LIGHTING_ASVGF_KEEP = 8u;
const uint LIGHTING_ASVGF_AGE = 900u;
uint lighting_grad_count() {
    uint cap = (LIGHTING_GRAD_ENTRIES * LIGHTING_CACHE_RAYS) >> lighting_sc_budget_shift();
    return LIGHTING_GRAD_CHECKS_ON ? min(ckeys.key[LIGHTING_GRAD_COUNT_AT], cap) : 0u;
}
bool lighting_sc_grad_stored(uint slot) {
    return floatBitsToUint(cache.v[LIGHTING_SC_GRAD_AT + slot].z) != 0u;
}
float lighting_sc_lambda(uint slot) {
    uint f = floatBitsToUint(cache.v[LIGHTING_SC_GRAD2_AT + slot].x);
    if (f == 0u || int(lighting_frame() - f) > int(LIGHTING_ASVGF_KEEP)) {
        return 0.0;
    }
    return abs(cache.v[LIGHTING_SC_GRAD_AT + slot].x);
}
// S76: lambda not yet spent on rays (spent lambda is kept negated for the blend).
bool lighting_sc_lambda_fresh(uint slot) {
    return cache.v[LIGHTING_SC_GRAD_AT + slot].x > 0.0;
}
bool lighting_sc_resolved(uint slot, float t, float lam) {
    if (lam >= LIGHTING_ASVGF_EPS || !lighting_sc_grad_stored(slot)) {
        return false;
    }
    vec4 a = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
    vec4 b = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u];
    if (!(b.x > 0.0) || t < LIGHTING_DN_MIN_SETS - 0.5) {
        return false;
    }
    float mean = dot(cache.v[3u * slot + 2u].rgb, vec3(0.2126, 0.7152, 0.0722));
    float se = sqrt(max(a.w, 0.0) / max(t, 1.0));
    return se < LIGHTING_ASVGF_SE * mean + 1.0e-4;
}
// This round's pick of the texel's 3 x 3 block.
bool lighting_sc_grad_rep(uint slot) {
    vec2 c = lighting_sc_true_cell(slot);
    ivec2 b = ivec2(floor(c / 3.0));
    ivec2 m = ivec2(c) - 3 * b;
    uint h = lighting_hash(uint(b.x) * 0x8da6b343u ^ uint(b.y) * 0xd8163841u ^ lighting_round() * 0x9e3779b9u);
    return uint(m.y * 3 + m.x) == h % 9u;
}
void lighting_sc_grad_put(uint s, float lam) {
    if (s == ~0u) {
        return;
    }
    float old = lighting_sc_lambda_fresh(s) ? lighting_sc_lambda(s) : 0.0;
    cache.v[LIGHTING_SC_GRAD_AT + s].x = max(lam, old);
    cache.v[LIGHTING_SC_GRAD2_AT + s].x = uintBitsToFloat(lighting_frame());
}
// S76 the check is the gate (Anthony 16:58). Check i's result: the relative
// same-seed difference d = |L_now - L_then| / max(L_now, L_then, 0.1 mean)
// (A-SVGF eq. 6), less twice the texel's own check noise sigma (an EMA of d^2 over
// its checks, capped at LIGHTING_CHECK_SIGMA_MAX; Schied et al. 2019 normalise
// the gradient by the sample's own variance so noise is not read as change):
// lambda = max(0, d - 2 sigma). Under LIGHTING_ASVGF_EPS the check confirms the
// texel: no rays, and its next check is deferred twice as far (LIGHTING_CHECK_MIN
// to LIGHTING_CHECK_MAX frames). Otherwise lambda goes to its 3 x 3 block and the
// scheduler escalates rays in proportion (lighting_compact.comp).
const uint LIGHTING_CHECK_MIN = 8u;
const uint LIGHTING_CHECK_MAX = 128u;
const float LIGHTING_CHECK_SIGMA_MAX = 0.05;
uint lighting_sc_check_next(uint slot) {
    return floatBitsToUint(cache.v[LIGHTING_SC_GRAD2_AT + slot].y);
}
// S76D persistent words (never cleared per frame): escalations and checks since
// the last args write, the frame the global boost ends, last round's check demand.
const float LIGHTING_BOOST_FRAC = 0.2;
const uint LIGHTING_BOOST_FRAMES = 120u;
bool lighting_check_boost() {
    return int(ckeys.key[LIGHTING_CHECK_P_AT + 2u] - lighting_frame()) > 0;
}
bool lighting_sc_check_due(uint slot) {
    uint nx = lighting_sc_check_next(slot);
    if (nx == 0u || int(lighting_frame() - nx) >= 0 || int(nx - lighting_frame()) > int(LIGHTING_CHECK_MAX)) {
        return true;
    }
    // S76D: while the boost holds (a scene-wide change), every texel is checked
    // at least every LIGHTING_CHECK_MIN frames whatever its deferral.
    if (lighting_check_boost()) {
        uint iv = floatBitsToUint(cache.v[LIGHTING_SC_GRAD2_AT + slot].z);
        return int(lighting_frame() - (nx - iv)) >= int(LIGHTING_CHECK_MIN);
    }
    return false;
}
// Nominate (persistent: the texel stays due until a check runs on it).
void lighting_sc_nominate(uint slot) {
    if (slot == ~0u) {
        return;
    }
    uint nx = lighting_sc_check_next(slot);
    if (nx == 0u || int(nx - lighting_frame()) > 0) {
        cache.v[LIGHTING_SC_GRAD2_AT + slot].y = uintBitsToFloat(lighting_frame());
        cache.v[LIGHTING_SC_GRAD2_AT + slot].z = uintBitsToFloat(LIGHTING_CHECK_MIN);
    }
}
// S76D fair service: when last round's checks due exceeded the room, a texel is
// served on its turn (one round in k, by hash) unless it has waited more than
// LIGHTING_CHECK_MIN frames, so every nomination runs within ~k rounds.
bool lighting_sc_check_turn(uint slot) {
    uint room = (LIGHTING_GRAD_ENTRIES * LIGHTING_CACHE_RAYS) >> lighting_sc_budget_shift();
    uint dem = ckeys.key[LIGHTING_CHECK_P_AT + 3u];
    if (dem <= room) {
        return true;
    }
    uint nx = lighting_sc_check_next(slot);
    if (nx != 0u && int(lighting_frame() - nx) > int(LIGHTING_CHECK_MIN)) {
        return true;
    }
    uint k = (dem + room - 1u) / room;
    return ((lighting_hash(lighting_sched(slot) ^ 0x7feb352du) + lighting_round()) % k) == 0u;
}
void lighting_sc_check_soon(uint slot) {
    cache.v[LIGHTING_SC_GRAD2_AT + slot].y = uintBitsToFloat(lighting_frame() + 1u);
    cache.v[LIGHTING_SC_GRAD2_AT + slot].z = uintBitsToFloat(LIGHTING_CHECK_MIN);
}
void lighting_sc_grad_take(uint slot, float l_now) {
    vec4 g = cache.v[LIGHTING_SC_GRAD_AT + slot];
    vec4 g2 = cache.v[LIGHTING_SC_GRAD2_AT + slot];
    float mean = dot(cache.v[3u * slot + 2u].rgb, vec3(0.2126, 0.7152, 0.0722));
    float d = min(1.0, abs(l_now - g.y) / max(max(l_now, g.y), 0.1 * mean + 1.0e-4));
    float n2 = clamp(g2.w, 0.0, LIGHTING_CHECK_SIGMA_MAX * LIGHTING_CHECK_SIGMA_MAX);
    float lam = max(0.0, d - 2.0 * sqrt(n2));
    cache.v[LIGHTING_SC_GRAD2_AT + slot].w = min(mix(n2, d * d, 0.125), LIGHTING_CHECK_SIGMA_MAX * LIGHTING_CHECK_SIGMA_MAX);
    atomicAdd(gstats.pad4, 1u);
    atomicAdd(ckeys.key[LIGHTING_CHECK_P_AT + 1u], 1u);
    if (lam < LIGHTING_ASVGF_EPS) {
        // Confirmed: defer the next check (doubling).
        uint iv = clamp(floatBitsToUint(g2.z), LIGHTING_CHECK_MIN / 2u, LIGHTING_CHECK_MAX / 2u) * 2u;
        cache.v[LIGHTING_SC_GRAD2_AT + slot].z = uintBitsToFloat(iv);
        cache.v[LIGHTING_SC_GRAD2_AT + slot].y = uintBitsToFloat(lighting_frame() + iv);
        lighting_dg(42u, 1u);
        return;
    }
    lighting_dg(43u, 1u);
    atomicAdd(gstats.pad5, 1u);
    atomicAdd(ckeys.key[LIGHTING_CHECK_P_AT], 1u);
    vec2 c = lighting_sc_true_cell(slot);
    ivec2 b = ivec2(floor(c / 3.0)) * 3 - ivec2(c);
    [[dont_unroll]] for (int k = 0; k < 9; k++) {
        ivec2 off = b + ivec2(k % 3, k / 3);
        lighting_sc_grad_put(off == ivec2(0) ? slot : lighting_sc_cell_slot(slot, off), lam);
    }
    // S76D: a confirmed change nominates the ring around its block (5 x 5 less
    // the 3 x 3, same surface) for checks.
    [[dont_unroll]] for (int k = 0; k < 25; k++) {
        ivec2 off = b + ivec2(k % 5, k / 5) - ivec2(1);
        if (off.x >= b.x && off.x < b.x + 3 && off.y >= b.y && off.y < b.y + 3) {
            continue;
        }
        lighting_sc_nominate(lighting_sc_cell_slot(slot, off));
        lighting_dg(56u, 1u);
    }
}
// Shadow reuse for static lamps (S74A, Anthony 16:11): a light-cache or probe
// hit's shadow ray to a lamp is kept per 12.5 cm cell, facing axis and lamp, with
// the lamp's epoch; the epoch moves when the lamp moves or changes, or when a
// change of the traced world (this frame's change list, old and new bounds, or
// a whole-world change) falls within its range (suns: anywhere). A moving lamp
// moves its epoch every frame, so it re-shadows as before.
const bool LIGHTING_SHADOW_CACHE = true;
const uint LIGHTING_SHADOW_AT = 13u * LIGHTING_CACHE_SLOTS;
void lighting_shadow_epochs() {
    if (!LIGHTING_SHADOW_CACHE) {
        return;
    }
    uint n = min(scene.lamp_count, LIGHTING_WRC_SNAP_LAMPS);
    vec4 head = work.v[lighting_change_at()];
    bool all = head.y > 0.5;
    uint changes = all ? 0u : min(uint(head.x + 0.5), LIGHTING_CHANGES);
    for (uint i = 0u; i < n; i++) {
        Lamp l = scene_lamp(i);
        uint at = LIGHTING_SHADOW_EPOCH_AT + 8u * i;
        vec4 last = vec4(uintBitsToFloat(ckeys.key[at + 1u]), uintBitsToFloat(ckeys.key[at + 2u]), uintBitsToFloat(ckeys.key[at + 3u]), uintBitsToFloat(ckeys.key[at + 4u]));
        vec4 now = vec4(l.pos.xyz, l.color.w);
#ifdef GENOS_SUN_SHADER
        // A small sun step keeps this epoch. Pixels still trace the live sun.
        if (lighting_sun_small() && l.pos.w > 0.5) {
            continue;
        }
#endif
        // S75: colour too (the texel direct-light cache reads the epoch).
        uint col = lighting_hash(floatBitsToUint(l.color.x) ^ lighting_hash(floatBitsToUint(l.color.y) ^ lighting_hash(floatBitsToUint(l.color.z))));
        bool bump = all || any(notEqual(last, now)) || ckeys.key[at + 5u] != col;
        ckeys.key[at + 5u] = col;
        for (uint c = 0u; c < changes && !bump; c++) {
            vec4 b = work.v[lighting_change_at() + LIGHTING_CHANGE_HEAD + c];
            bump = l.color.w <= 0.0 || length(b.xyz - l.pos.xyz) < l.color.w + b.w;
        }
        if (bump) {
            ckeys.key[at] += 1u;
            ckeys.key[at + 1u] = floatBitsToUint(now.x);
            ckeys.key[at + 2u] = floatBitsToUint(now.y);
            ckeys.key[at + 3u] = floatBitsToUint(now.z);
            ckeys.key[at + 4u] = floatBitsToUint(now.w);
        }
    }
}
// The cached entry for (o, n, lamp li): 1 lit, 0 occluded, -1 none (trace it).
uint lighting_shadow_cell(vec3 o, vec3 n, uint li, out uint tag) {
    ivec3 c;
    // The tuned cell is the constant. Dividing by a uniform that holds the
    // same 0.125 quantises boundaries differently: block-shaped differences
    // and a darker still frame. A custom cell is the only variable division.
    if (!(gstats.ctrl_shadow_cell > 0.0) || gstats.ctrl_shadow_cell == LIGHTING_SC_TEXEL) {
        c = ivec3(floor(o / LIGHTING_SC_TEXEL));
    } else {
        c = ivec3(floor(o / gstats.ctrl_shadow_cell));
    }
    uint h = lighting_hash(uint(c.x) * 73856093u ^ uint(c.y) * 19349663u ^ uint(c.z) * 83492791u ^ lighting_axis(n) * 0x27d4eb2fu ^ (li + 1u) * 0x9e3779b9u);
    tag = h | 1u;
    return LIGHTING_SHADOW_AT + (lighting_hash(h ^ 0x5bd1e995u) % LIGHTING_CACHE_SLOTS);
}
float lighting_shadow_get(vec3 o, vec3 n, uint li) {
    // Shadows off, or the hard filter: trace, do not read the cell.
    if ((gstats.ctrl_bits & 2u) == 0u || (gstats.ctrl_bits & 2048u) != 0u) {
        return -1.0;
    }
    if (!LIGHTING_SHADOW_CACHE || li >= LIGHTING_WRC_SNAP_LAMPS || li >= scene.lamp_count) {
        return -1.0;
    }
    uint tag;
    uint at = lighting_shadow_cell(o, n, li, tag);
    vec4 e = cache.v[at];
    if (floatBitsToUint(e.x) == tag && floatBitsToUint(e.z) == ckeys.key[LIGHTING_SHADOW_EPOCH_AT + 8u * li]) {
        return e.y;
    }
    return -1.0;
}
// S77C (lighting_compact.comp).
const bool LIGHTING_COMPACT_LIST = true;
const uint LIGHTING_COMPACT_IDLE = 8u;
void lighting_shadow_put(vec3 o, vec3 n, uint li, bool lit) {
    if ((gstats.ctrl_bits & 2u) == 0u || (gstats.ctrl_bits & 2048u) != 0u) {
        return;
    }
    if (!LIGHTING_SHADOW_CACHE || li >= LIGHTING_WRC_SNAP_LAMPS || li >= scene.lamp_count) {
        return;
    }
    uint tag;
    uint at = lighting_shadow_cell(o, n, li, tag);
    cache.v[at] = vec4(uintBitsToFloat(tag), lit ? 1.0 : 0.0, uintBitsToFloat(ckeys.key[LIGHTING_SHADOW_EPOCH_AT + 8u * li]), 0.0);
}

// S75 Lumen surface-cache direct lighting (Wright et al. 2022, "Lumen: Real-time
// Global Illumination in Unreal Engine 5", surface cache direct lighting): each fine
// texel keeps its exact (all lamps, shadow-traced) direct irradiance, read by GI ray
// hits (light-cache rays, probe hits) instead of their lamp loop. The screen's own
// direct light stays per pixel. Slot 14: x rgb9e5 irradiance, y sun share, z
// signature, w frame last updated. Valid while the signature (texel key, its lamp
// list with each lamp's epoch, the flame) matches and the entry is younger than
// LIGHTING_TDL_STALE; otherwise the hit relights the texel (at its centre) if this
// frame's budget LIGHTING_TDL_BUDGET has room, else runs its own lamp loop as before.
const bool LIGHTING_TDL = true;
const uint LIGHTING_TDL_AT = 14u * LIGHTING_CACHE_SLOTS;
const uint LIGHTING_TDL_STALE = 600u;
const uint LIGHTING_TDL_BUDGET = 32768u;
const bool LIGHTING_TDL_CHECK = true;
const float LIGHTING_TDL_TOL = 0.1;
const uint LIGHTING_TDL_COUNT_AT = LIGHTING_SC_HIST_AT + 73u;
uint lighting_lamps_sig(uint seed, LampList list);
uint lighting_tdl_sig(uint slot, LampList list) {
    return lighting_lamps_sig(lighting_hash(ckeys.key[slot] ^ 0x68e31da4u), list);
}
uint lighting_lamps_sig(uint seed, LampList list) {
    uint sig = seed;
    uint lamps = list.suns.y + list.near.y;
    uint count = lamp_list_size(list);
    for (uint k = 0u; k < count; k++) {
        uint e;
        if (k >= lamps) {
            e = lighting_hash(floatBitsToUint(scene.fire_pos.x) ^ lighting_hash(floatBitsToUint(scene.fire_pos.y) ^ lighting_hash(floatBitsToUint(scene.fire_pos.z) ^ lighting_hash(floatBitsToUint(scene.fire_pos.w) ^ lighting_hash(floatBitsToUint(scene.fire_color.x) ^ floatBitsToUint(scene.fire_color.y) * 31u ^ floatBitsToUint(scene.fire_color.z) * 131u)))));
        } else {
            uint li = lamp_list_index(list, k);
            if (li >= LIGHTING_WRC_SNAP_LAMPS || li >= scene.lamp_count) {
                return 0u;
            }
            e = lighting_hash(li * 0x9e3779b9u ^ ckeys.key[LIGHTING_SHADOW_EPOCH_AT + 8u * li]);
        }
        sig = lighting_hash(sig ^ e);
    }
    return sig | 1u;
}

// S76B WRC change refresh, confirm then escalate (Anthony 17:06): a change
// nearby (mover, moved lamp) only nominates a cell; it is refreshed when its
// check is due. The refresh's sample against the cell's mean is the check: within
// twice the cell's own sample noise (an EMA of d^2 over its refreshes, capped at
// LIGHTING_WRC_SIGMA_MAX) plus LIGHTING_ASVGF_EPS confirms, and the next nominated
// refresh waits twice as long (LIGHTING_CHECK_MIN..MAX frames); otherwise the
// cell is refreshed again next frame while nominated.
const float LIGHTING_WRC_SIGMA_MAX = 0.2;
void lighting_wrc_check_take(uint slot, vec3 e, vec3 old) {
    float a = dot(e, vec3(0.2126, 0.7152, 0.0722));
    float b = dot(old, vec3(0.2126, 0.7152, 0.0722));
    float d = min(1.0, abs(a - b) / max(max(a, b), 1.0e-3));
    vec4 g2 = cache.v[LIGHTING_SC_GRAD2_AT + slot];
    float n2 = clamp(g2.w, 0.0, LIGHTING_WRC_SIGMA_MAX * LIGHTING_WRC_SIGMA_MAX);
    cache.v[LIGHTING_SC_GRAD2_AT + slot].w = min(mix(n2, d * d, 0.125), LIGHTING_WRC_SIGMA_MAX * LIGHTING_WRC_SIGMA_MAX);
    // S77A: escalated (GRAD.x = 1): its next refresh is a whole set.
    cache.v[LIGHTING_SC_GRAD_AT + slot].x = 0.0;
    if (d < 2.0 * sqrt(n2) + LIGHTING_ASVGF_EPS) {
        uint iv = clamp(floatBitsToUint(g2.z), LIGHTING_CHECK_MIN / 2u, LIGHTING_CHECK_MAX / 2u) * 2u;
        cache.v[LIGHTING_SC_GRAD2_AT + slot].z = uintBitsToFloat(iv);
        cache.v[LIGHTING_SC_GRAD2_AT + slot].y = uintBitsToFloat(lighting_frame() + iv);
        lighting_dg(44u, 1u);
    } else {
        lighting_sc_check_soon(slot);
        cache.v[LIGHTING_SC_GRAD_AT + slot].x = 1.0;
        lighting_dg(45u, 1u);
    }
}

// S78J per-pixel direct light reuse: a pixel whose surface point (1 mm), normal
// and sidedness and whose lamps' epochs (lighting_lamps_sig) are those of last
// frame, in a frame with no change and no mover listed, shows last frame's
// direct light: no lamp loop, no shadow rays. Any mover anywhere relights every
// pixel (a moving shadow can reach any of them).
const bool LIGHTING_PIX_REUSE = true;
uint lighting_pix_at() {
    return lighting_fill_light_at() + lighting_kept_hits() / 4u;
}
// Mode 3 is the surface cache. Probe-only (world cache bit off) used to skip
// this and relight every pixel every still frame, which costs more than the
// cache being on. High keeps the mode-3 check: the cache bit is set there.
bool lighting_pix_cache_ok() {
    return lighting_world_mode() == 3u || (gstats.ctrl_bits & 32u) == 0u;
}
bool lighting_pix_reuse_on() {
    // No probes and no world cache is a 4096-byte stand-in. Pixel reuse indexes
    // the work buffer past that (the same rule as the shadow cells).
    bool stored = (gstats.ctrl_bits & 16u) != 0u || (gstats.ctrl_bits & 32u) != 0u;
    return LIGHTING_PIX_REUSE && lighting_gi_on() && stored && lighting_pix_cache_ok();
}
bool lighting_pix_quiet() {
    vec4 head = work.v[lighting_change_at()];
    return lighting_pix_cache_ok() && !(head.y > 0.5) && uint(head.x) == 0u && lighting_movers_count() == 0u;
}
uint lighting_pix_sig(vec3 pos, vec3 n, bool two_sided, LampList list) {
    ivec3 q = ivec3(floor(pos * 1000.0));
    uint h = lighting_hash(uint(q.x) * 0x8da6b343u ^ uint(q.y) * 0xd8163841u ^ uint(q.z) * 0xcb1ab31fu ^ lighting_pack_normal(n) ^ (two_sided ? 0x5bd1e995u : 0u));
    return lighting_lamps_sig(h, list);
}

// S78Q (held-probe gather): per probe two vec4 (x the frame its gather inputs
// last changed, y its kept samples at its last gather; then its place there), then the gather's raw cell
// table (R per probe), which the filter's first pass reads.
const bool LIGHTING_GATHER_HOLD = true; // S78R (re-tested on S78W)
uint lighting_gdirty_at() {
    return lighting_pix_at() + (pc.dims.x * pc.dims.y + 1u) / 2u;
}
uint lighting_graw_at(uint probe, uint k) {
    return lighting_gdirty_at() + 2u * lighting_probes() + probe * lighting_rays() + k;
}
void lighting_gdirty_mark(uint probe) {
    work.v[lighting_gdirty_at() + 2u * probe].x = uintBitsToFloat(lighting_frame());
}
// A write to a kept hit's slot (record or light) that changed it marks its probe.
void lighting_gdirty_kept_at(uint at) {
    uint base = lighting_kept_at(0u);
    if (at >= base && at < base + lighting_kept_hits()) {
        lighting_gdirty_mark((at - base) / lighting_rays() / LIGHTING_STRATA);
    }
}
