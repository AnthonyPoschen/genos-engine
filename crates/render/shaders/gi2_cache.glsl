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
// slots and one list of SLOTS entries, rebuilt each round by gi2_compact.comp:
// mature slots from the front, young ones (relit fewer than GI2_CACHE_MATURE
// times) from the back; last, the young count. A round relights the young first
// (a newly seen surface gets its light at once) and fills the batch from the
// mature ones in turn. Once the lights and objects have held still long enough
// for a full sweep to change nothing (gi2_gpu.rs), a round takes only the young:
// their light cannot have changed, and a moving camera then costs only the new
// surfaces it sees.
// Patches (binding 10): 3 vec4 per slot: position (w: frame last read), normal
// (w: cell size, 0 until claimed), irradiance (w: times relit, up to
// GI2_CACHE_MATURE; 0 unlit).

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
    // placed (gi2_gather.comp), for the effective sample count.
    uint pad0;
    uint pad1;
    // Surface cache trace (GENOS_GI2_SC_TRACE): texels claimed, sets restarted by
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
} gstats;

const float GI2_CACHE_MATURE = 3.0;


uint gi2_frame() {
    return pc.params.w;
}

// Light cache round: which batch of live patches is relit. Rounds are counted on
// their own (gi2_gpu.rs), so a frame without one skips no batch.
uint gi2_round() {
    return pc.params.z >> 12u;
}

// This round relights young patches only (params.z bit 10).
bool gi2_young_only() {
    return ((pc.params.z >> 10u) & 1u) != 0u;
}

uint gi2_young_count() {
    return min(ckeys.key[2u * GI2_CACHE_SLOTS + 1u], GI2_CACHE_SLOTS);
}
uint gi2_mature_count() {
    return min(ckeys.key[GI2_CACHE_SLOTS], GI2_CACHE_SLOTS);
}

// Surface cache scheduler (Lumen's surface cache update priority, FLumenCard
// LastUsed - LastUpdated, under a fixed budget): each compact pass buckets the
// priority of every texel it could relight into GI2_SC_BUCKETS log buckets and
// lists those at or above the threshold the cache pass set from the previous
// round's histogram, so the batch takes the most stale-for-their-use texels
// first. GIBS (2021) weighs a texel still short of its cap up (more effort where
// the estimate is unconverged) and a dormant one down. ckeys after the young
// count: the histogram, then the threshold.
const bool GI2_SC_SCHED = true;
// World space radiance cache (WRC, S61A; Lumen's world space radiance cache,
// SHaRC's hash grid): the 25 cm ray-hit cells. Kept resident and converging
// around the camera, on screen or not, so a surface revealed by a turn or a
// corner starts from converged light: each round GI2_WRC_SCOUT_ENTRIES batch
// entries trace scout rays from the camera (half spread over the sphere, half
// ahead of the motion and turn, a quarter of them from up to 2 m beside the eye
// to see round corners); their hits claim and read WRC cells, which the
// scheduler then relights like any other. A WRC cell keeps a running mean to
// GI2_WRC_CAP sets (was: each relight replaced it, at most 3 sets). A WRC cell a
// pixel needs as a seed (its fine texel has no light) with fewer than
// GI2_WRC_YOUNG sets is relit first with whole sets (the reveal burst); new fine
// texels start from the WRC (bilinear, coplanar cells only) with at most
// GI2_WRC_SEED_SETS sets and take their turn by priority.
const uint GI2_WRC_SCOUT_ENTRIES = 256u;
// S61B: a WRC relight is one batch entry (16 rays, a quarter of a texel's set)
// and counts as one sample; GI2_WRC_CAP is in those (128 = 32 whole sets).
const float GI2_WRC_CAP = 128.0;
const float GI2_WRC_YOUNG = 16.0;
const float GI2_WRC_SEED_SETS = 8.0;
const float GI2_WRC_NEAR = 6.0;
const uint GI2_SC_BUCKETS = 64u;
const uint GI2_SC_HIST_AT = 2u * GI2_CACHE_SLOTS + 2u;
const uint GI2_SC_THR_AT = GI2_SC_HIST_AT + GI2_SC_BUCKETS;
// WRC refresh (S61B, Anthony: a hard, fixed budget): at most GI2_WRC_REFRESH
// batch entries a round (~6 % of 65536) go to WRC cells outside the reveal
// burst, from two lists: change-driven first (a mover nearby, or a lamp that
// reaches the cell moved since its last relight: its history drops to
// GI2_WRC_CHANGE_KEEP samples), then the steady background (a resident cell not
// relit for GI2_WRC_REFRESH_FRAMES, or still short of its cap and read since)
// with what is left. Lumen's world space radiance cache: a fixed budget of probe
// updates, the changed and the nearest first.
const uint GI2_WRC_REFRESH = 4096u;
// S62D (Anthony): a refresh takes a whole 64-ray set (GI2_SC_QUARTERS
// consecutive entries), so the cap above holds GI2_WRC_REFRESH_CELLS cells a
// round (1024; same rays as before).
const uint GI2_WRC_REFRESH_CELLS = GI2_WRC_REFRESH / 4u;
const uint GI2_WRC_REFRESH_FRAMES = 180u;
const uint GI2_WRC_CHANGE_FRAMES = 120u;
const float GI2_WRC_CHANGE_KEEP = 4.0;
const uint GI2_WRC_CHG_COUNT_AT = GI2_SC_HIST_AT + 66u;
const uint GI2_WRC_BG_COUNT_AT = GI2_SC_HIST_AT + 67u;
const uint GI2_WRC_LAMPS_AT = GI2_SC_HIST_AT + 68u;
// S62B: WRC cells bursting this round (zeroed with the two counts above), and
// the most a round may burst (each takes 4 entries).
const uint GI2_WRC_BURST_COUNT_AT = GI2_SC_HIST_AT + 70u;
const uint GI2_WRC_BURST_MAX = 2048u;
const uint GI2_WRC_RING_HEAD_AT = GI2_SC_HIST_AT + 69u;
const uint GI2_WRC_RING = 16u;
const uint GI2_WRC_RING_AT = GI2_SC_HIST_AT + 72u;
const uint GI2_WRC_SNAP_LAMPS = 128u;
const uint GI2_WRC_SNAP_AT = GI2_SC_HIST_AT + 160u;
const uint GI2_WRC_CHG_AT = GI2_SC_HIST_AT + 680u;
const uint GI2_WRC_BG_AT = GI2_WRC_CHG_AT + GI2_WRC_REFRESH;

// World irradiance cells (experiment, GENOS_GI2_WORLD): the change block's head w
// is mode * 1000 + cap. Mode 1: pixels read the light cache's patches,
// interpolated; mode 2: screen probes average their irradiance into the patch they
// sit in (a running mean of up to cap frames, binding 10 after the patches) and
// pixels read that, interpolated, so what a pixel shows depends on where it is in
// the world, not on where the probe grid fell this frame.
uint gi2_world_mode() {
    return (uint(work.v[gi2_change_at()].w + 0.5) / 1000u) % 100u;
}
float gi2_world_cap() {
    return float(uint(work.v[gi2_change_at()].w + 0.5) % 1000u);
}
// Sets texel slot grows to.
float gi2_sc_cap(uint slot);
float gi2_sc_light_window();
float gi2_hit_size(vec3 pos);

// Surface cache switches (GENOS_GI2_SC_FLAGS, experiments): bit 0 sets weigh k^2
// (else k); bit 1 a second pass (a texel reaching the cap the first time drops to a
// quarter of it once); bit 2 texels only rays read grow to GI2_SC_HIT_CAP sets;
// bit 3 pooling never narrower than 0.6 cells; bit 4 the finest texels (under 10
// cm; none since S57) grow to twice the cap; bit 5 a texel with few sets shares its
// pixels with the probes; bit 6 a texel's lit-from point moving does not restart
// its sets.
uint gi2_sc_flags() {
    return uint(work.v[gi2_change_at()].z + 0.5) >> 1u;
}
// Surface cache (mode 3, GENOS_GI2_WORLD=3): the light cache's patches are the
// texels pixels read. A texel adds a new set of GI2_CACHE_RAYS rays each time it is
// relit and keeps their running mean, up to cap sets; then it holds (converged)
// until the light changes or an object moves near it.
// Texels are one size everywhere, GI2_SC_TEXEL (S57, Anthony 2026-10-12): the
// camera-distance bands (6.25 cm within 2.5 m, 12.5 cm to 8 m, 25 cm and up past
// it) are gone. Each band edge was a ring that followed the camera, and crossing
// it re-keyed texels, which restarted (grain) however their light was resampled.
// Neither Lumen's cards nor the papers surveyed key texel size to the eye;
// detail and effort come from the light: change-aware history and dormancy
// (GIBS), the relight budget and its order (Lumen), the read rule (SHaRC). The
// camera only decides what stays resident and what is relit first.
const float GI2_SC_TEXEL = 0.125;
// Ray-hit patches (flag 256): one size too, GI2_SC_HIT (was 25 cm doubling past
// 8 m from the eye).
const float GI2_SC_HIT = 0.25;
// Sets a texel inside a change, or under a light that is changing, averages over
// (a short running mean on one fixed set: quick, and no flicker).
const float GI2_SC_MOVING = 2.0;
// Sets a texel averages over while the light changes (lamps moving: a running
// mean over a window, the sets turning with the rounds).
const float GI2_SC_LIGHT = 8.0;
// Sets a texel only rays read (not on screen) grows to.
const float GI2_SC_HIT_CAP = 8.0;
// Flag 262144: caps of sets a texel takes past the cap before it holds.
const float GI2_SC_HOLD = 4.0;
// Flag 32: a texel's share of its pixels against the probes, sets / (sets + this).
const float GI2_SC_TRUST_SETS = 6.0;

float gi2_cell_size_d(float d);
float gi2_cell_size(vec3 pos) {
    return gi2_cell_size_d(length(pos - scene.eye.xyz));
}
float gi2_cell_size_d(float d) {
    float level = clamp(floor(log2(max(d / 8.0, 1.0))), 0.0, 6.0);
    // Surface cache (mode 3): one texel size, whatever the distance.
    if (gi2_world_mode() == 3u) {
        return GI2_SC_TEXEL;
    }
    return 0.25 * exp2(level);
}

uint gi2_axis(vec3 n) {
    vec3 a = abs(n);
    if (a.x >= a.y && a.x >= a.z) {
        return n.x >= 0.0 ? 0u : 1u;
    }
    if (a.y >= a.z) {
        return n.y >= 0.0 ? 2u : 3u;
    }
    return n.z >= 0.0 ? 4u : 5u;
}

const uint GI2_WORLD_AT = 3u * GI2_CACHE_SLOTS;
// Surface cache estimator (flag 1048576), 2 vec4s per slot: short-term mean (rgb)
// and short-term luminance variance (w); then the variance-based blend reduction
// (x) and the inconsistency (y). Zero when the slot is claimed.
const uint GI2_SC_EST_AT = 4u * GI2_CACHE_SLOTS;
// Object id + 1 of a texel keyed in a mover's frame (0: a world texel).
const uint GI2_SC_OBJ_AT = 6u * GI2_CACHE_SLOTS;
// Frame of a texel's last relight (0: never), beside its object id.
// S62E global light scale (Anthony: day to night, unread texels kept daytime
// light and faded as rays came in). The pixel's own direct light is already
// separate (G-buffer w, every frame); the cache holds the gathered light, which
// for sun and sky scales with them. Once a round the cache pass logs the global
// light (each sun's height times its brightest channel, plus the sky's: as
// probe_tier.rs up_light) against the frame (a ring, GI2_LS_RING rounds); a
// texel or WRC cell is shown, and its history carried, times
// (now + eps) / (then + eps), "then" being its last relight. An intensity change
// is instant everywhere, read or not; only the bounce's shape converges by rays
// (Lumen re-lights its surface cache's direct term each frame and lets the
// radiosity converge).
const uint GI2_LS_AT = GI2_SC_HIST_AT + 9000u;
const uint GI2_LS_RING = 2048u;
const float GI2_LS_EPS = 0.05;
const float GI2_LS_MAX = 16.0;
// S62F: off. One scale for a texel's whole value also scales the light lamps
// bounced into it: the stress room at night (lamp-lit) went to half its light on
// the first frame (daynight check, first-frame error 0.49). Needs the cache's
// sun/sky share kept apart from the lamps' (queued). The ratio still drives the
// refresh of unread texels (gi2_compact.comp).
const bool GI2_LS_RESCALE = false;
float gi2_global_light() {
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
void gi2_ls_note() {
    uint f = gi2_frame();
    uint at = GI2_LS_AT + 2u * (f % GI2_LS_RING);
    ckeys.key[at] = f;
    ckeys.key[at + 1u] = floatBitsToUint(gi2_global_light());
    ckeys.key[GI2_LS_AT + 2u * GI2_LS_RING] = f;
}
// The ratio to carry light last relit at frame f to now (1 when unknown).
float gi2_ls_ratio(uint f) {
    uint at = GI2_LS_AT + 2u * (f % GI2_LS_RING);
    if (f == 0u || ckeys.key[at] != f) {
        return 1.0;
    }
    float then = uintBitsToFloat(ckeys.key[at + 1u]);
    uint g = ckeys.key[GI2_LS_AT + 2u * GI2_LS_RING];
    uint ga = GI2_LS_AT + 2u * (g % GI2_LS_RING);
    if (ckeys.key[ga] != g) {
        return 1.0;
    }
    float now = uintBitsToFloat(ckeys.key[ga + 1u]);
    return clamp((now + GI2_LS_EPS) / (then + GI2_LS_EPS), 0.0, GI2_LS_MAX);
}
uint gi2_sc_updated(uint slot) {
    return floatBitsToUint(cache.v[GI2_SC_OBJ_AT + slot].y);
}
// Set while a lookup runs in a mover's frame: id + 1 salts the key.
float gi2_obj_salt = 0.0;

vec3 gi2_axis_vec(uint a) {
    vec3 v = vec3(0.0);
    v[a >> 1u] = (a & 1u) == 0u ? 1.0 : -1.0;
    return v;
}

// Key of the patch of the given size holding pos facing n; never 0.
uint gi2_cache_key_sized(vec3 pos, vec3 n, float size) {
    // World cells on: half a cell in from the surface, so a floor or wall on a cell
    // boundary falls in one cell, not either by rounding.
    vec3 inset = gi2_world_mode() != 0u ? 0.5 * gi2_axis_vec(gi2_axis(n)) : vec3(0.0);
    ivec3 c = ivec3(floor(pos / size - inset));
    uint h = gi2_hash(uint(c.x) * 73856093u ^ gi2_hash(uint(c.y) * 19349663u ^ gi2_hash(uint(c.z) * 83492791u)));
    h = gi2_hash(h ^ (gi2_axis(n) * 0x27d4eb2du) ^ (floatBitsToUint(size) * 0x165667b1u));
    if (gi2_obj_salt > 0.5) {
        h = gi2_hash(h ^ (uint(gi2_obj_salt) * 0x85ebca6bu));
    }
    return max(h, 1u);
}

// Key of the patch holding pos facing n; never 0.
uint gi2_cache_key(vec3 pos, vec3 n, out float size) {
    size = gi2_cell_size(pos);
    return gi2_cache_key_sized(pos, n, size);
}

// Surface cache: where a texel is lit from, the centre of its cell moved along
// its axis onto the surface through pos (so the texel's light does not depend on
// which reader claimed it first).
vec3 gi2_sc_centre(vec3 pos, vec3 n, float size) {
    uint ax = gi2_axis(n) >> 1u;
    vec3 inset = 0.5 * gi2_axis_vec(gi2_axis(n));
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
uint gi2_cache_find_sized(vec3 pos, vec3 n, bool claim, float size, vec3 at) {
    uint key = gi2_cache_key_sized(pos, n, size);
    uint start = gi2_hash(key ^ 0x9e3779b9u);
    uint free_at = GI2_CACHE_SEARCH;
    [[dont_unroll]] for (uint i = 0u; i < GI2_CACHE_SEARCH; i++) {
        uint k = ckeys.key[(start + i) & (GI2_CACHE_SLOTS - 1u)];
        if (k == key) {
            return (start + i) & (GI2_CACHE_SLOTS - 1u);
        }
        if (k == 0u && free_at == GI2_CACHE_SEARCH) {
            free_at = i;
        }
    }
    if (!claim) {
        return ~0u;
    }
    [[dont_unroll]] for (uint i = free_at; i < GI2_CACHE_SEARCH; i++) {
        uint slot = (start + i) & (GI2_CACHE_SLOTS - 1u);
        uint prev = atomicCompSwap(ckeys.key[slot], 0u, key);
        if (prev == 0u) {
            atomicAdd(gstats.claimed, 1u);
            // Flag 16384 (surface cache): every texel and hit patch is lit from its
            // cell's centre on the claimer's plane, whoever claimed it, and never
            // moves (Lumen: card texels are placed by the geometry), so its light
            // does not depend on the camera's path. (Was: a quarter-cell depth key,
            // rejected 2026-10-11.)
            if (gi2_world_mode() == 3u && (gi2_sc_flags() & 16384u) != 0u) {
                at = gi2_sc_centre(pos, n, size);
            }
            cache.v[3u * slot] = vec4(at, uintBitsToFloat(gi2_frame()));
            cache.v[3u * slot + 1u] = vec4(n, size);
            cache.v[3u * slot + 2u] = vec4(0.0);
            cache.v[GI2_WORLD_AT + slot] = vec4(length(at - gi2_sc_centre(pos, n, size)), 0.0, 0.0, 0.0);
            cache.v[GI2_SC_EST_AT + 2u * slot] = vec4(0.0);
            cache.v[GI2_SC_EST_AT + 2u * slot + 1u] = vec4(0.0);
            if (gi2_world_mode() == 3u) {
                cache.v[GI2_SC_OBJ_AT + slot] = vec4(gi2_obj_salt, 0.0, 0.0, 0.0);
            }
            return slot;
        }
        if (prev == key) {
            return slot;
        }
    }
    return ~0u;
}

uint gi2_cache_find(vec3 pos, vec3 n, bool claim) {
    return gi2_cache_find_sized(pos, n, claim, gi2_cell_size(pos), pos);
}

// Surface cache: a reader at pos (a point on the texel's surface) moves the point
// the texel is lit from to itself when it is nearer the cell's centre, so a texel
// ends up lit from the surface point nearest its centre whoever claimed it first
// (the centre itself can lie inside a wall at a corner).
void gi2_sc_offer(uint slot, vec3 at, vec3 centre) {
    if (gi2_world_mode() != 3u) {
        return;
    }
    // Flag 524288 (camera only orders updates): the point still moves toward the
    // cell's centre (a fixed point of the geometry, as Lumen's card texels), but
    // the sets taken so far are kept: a camera move never restarts a texel.
    if ((gi2_sc_flags() & 16384u) != 0u) {
        return;
    }
    bool keep_sets = (gi2_sc_flags() & 524288u) != 0u;
    float d = length(at - centre);
    if (d < cache.v[GI2_WORLD_AT + slot].x - 1.0e-4) {
        cache.v[GI2_WORLD_AT + slot].x = d;
        if (keep_sets) {
            cache.v[3u * slot].xyz = at;
            return;
        }
        // Lit from far from here so far: its sets start again (value kept).
        float size = cache.v[3u * slot + 1u].w;
        if ((gi2_sc_flags() & 64u) == 0u && length(at - cache.v[3u * slot].xyz) > 0.3 * size) {
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
vec4 gi2_sc_parent(vec3 pos, vec3 n, float size) {
    [[dont_unroll]] for (uint l = 0u; l < 2u; l++) {
        size *= 2.0;
        // The WRC cell (25 cm) is claimed if missing and marked as a seed a
        // pixel needs (the reveal burst, gi2_compact.comp).
        bool seed = gi2_world_mode() == 3u && l == 0u && size > 0.2;
        uint s = gi2_cache_find_sized(pos, n, seed, size, pos);
        if (s != ~0u && seed) {
            cache.v[3u * s].w = uintBitsToFloat(gi2_frame());
            cache.v[GI2_WORLD_AT + s].z = uintBitsToFloat(gi2_frame());
        }
        if (s != ~0u) {
            vec4 v = cache.v[3u * s + 2u];
            if (v.w > 0.5) {
                cache.v[3u * s].w = uintBitsToFloat(gi2_frame());
                return vec4(v.rgb, 1.0);
            }
        }
    }
    return vec4(0.0);
}

// Surface cache estimator (flag 1048576): a converged texel whose recent sets
// agree with its mean (inconsistency low) is dormant: the sweep relights it one
// round in GI2_SC_DORMANT, enough to notice a change (GIBS 2021: converged surfels
// send only change-detection rays).
const uint GI2_SC_DORMANT = 8u;
const float GI2_SC_DORMANT_SETS = 128.0;
// Agreement a dormant texel needs (S59E: 0.3 -> 0.15 standard deviations).
const float GI2_SC_DORMANT_SD = 0.15;
bool gi2_sc_dormant(uint slot) {
    vec4 b = cache.v[GI2_SC_EST_AT + 2u * slot + 1u];
    // Only once its mean holds GI2_SC_DORMANT_SETS sets: a texel still warming
    // up keeps its full rate whatever its inconsistency says.
    return b.x > 0.0 && b.y < GI2_SC_DORMANT_SD && b.w >= GI2_SC_DORMANT_SETS;
}

// Surface cache, flag 524288: a new texel's first light (Lumen's "resample
// lighting on realloc", SurfaceCache.ResampleLighting): bilinear at its centre
// from the lit texels of the next size up (the 25 cm ray-hit patches, flag 256)
// or down, with the sample count they hold (to the cap), so it does not start
// from nothing. Since the size bands went (S57) this only fills texels of
// surfaces seen for the first time. w 0 when nothing lit covers it.
vec4 gi2_sc_shown(uint slot);
vec4 gi2_sc_bilerp_sized(vec3 pos, vec3 n, float size) {
    uint ax = gi2_axis(n) >> 1u;
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
        uint s = gi2_cache_find_sized(q, n, false, size, q);
        if (s == ~0u || bil <= 0.0) {
            continue;
        }
        vec4 v = cache.v[3u * s + 2u];
        // Only cells on the same plane (a step or a ledge is another surface).
        if (abs(dot(cache.v[3u * s].xyz - pos, n)) > 0.1) {
            continue;
        }
        if (v.w > 0.5 && fract(v.w) < 0.1) {
            cache.v[3u * s].w = uintBitsToFloat(gi2_frame());
            sum += bil * gi2_sc_shown(s).rgb;
            // WRC samples are 16-ray quarters (S61B): four to a set.
            count += bil * floor(v.w) * (size > 0.2 && gi2_world_mode() == 3u ? 0.25 : 1.0);
            wsum += bil;
        }
    }
    // Most of the footprint lit: a texel at the edge of what was lit keeps the
    // light it has, but one mostly over unlit cells starts as new.
    if (wsum < 0.5) {
        return vec4(0.0);
    }
    return vec4(sum / wsum, count / wsum);
}
const float GI2_SC_RESAMPLE_SETS = 4.0;
bool gi2_near_change(vec3 p);
bool gi2_sc_light_quiet();
vec4 gi2_sc_resample(vec3 pos, vec3 n, float size, uint slot) {
    vec4 best = vec4(0.0);
    [[dont_unroll]] for (uint l = 0u; l < 4u; l++) {
        float s2 = size * ((l & 1u) == 0u ? 2.0 : 0.5) * ((l & 2u) == 0u ? 1.0 : ((l & 1u) == 0u ? 2.0 : 0.5));
        vec4 r = gi2_sc_bilerp_sized(pos, n, s2);
        // The next size up or down with the most samples; the sizes beyond only
        // when neither has light.
        if (r.w > best.w) {
            best = r;
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
    float cap = min(gi2_sc_cap(slot), GI2_WRC_SEED_SETS);
    bool urgent = ((pc.params.z >> 5u) & 1u) != 0u;
    if ((urgent && gi2_near_change(pos)) || !gi2_sc_light_quiet()) {
        cap = min(cap, GI2_SC_RESAMPLE_SETS);
    }
    return vec4(best.rgb, max(min(floor(best.w), cap), 1.0));
}

// How far around a change a patch is relit first (gi2_compact.comp), metres.
const float GI2_NEAR_CHANGE = 1.0;

// Movers (gi2_gpu.rs gi2_movers): occluders that moved in the last
// GI2_MOVER_KEEP frames, listed after the changes, four vec4 each: bounding
// sphere; centre and id + 1; half extents (x, y, z); cosine and sine of the yaw.
// Count in head w above 200000, the object-local texel switch at 100000.
const uint GI2_MOVER_VEC4 = 4u;
uint gi2_movers_first() {
    vec4 head = work.v[gi2_change_at()];
    return head.y > 0.5 ? 0u : min(uint(head.x + 0.5), GI2_CHANGES);
}
uint gi2_movers_count() {
    uint n = uint(work.v[gi2_change_at()].w + 0.5) / 200000u;
    return min(n, (GI2_CHANGES - gi2_movers_first()) / GI2_MOVER_VEC4);
}
vec4 gi2_mover(uint m, uint k) {
    return work.v[gi2_change_at() + GI2_CHANGE_HEAD + gi2_movers_first() + GI2_MOVER_VEC4 * m + k];
}
// Object-local texels (GIBS 2021: surfels keep a transform id and a local
// position): GENOS_GI2_SC_LOCAL=1.
bool gi2_sc_local_on() {
    return gi2_world_mode() == 3u && ((uint(work.v[gi2_change_at()].w + 0.5) / 100000u) & 1u) != 0u;
}

// A point inside the bounds of a mover.
bool gi2_in_change(vec3 p) {
    uint n = gi2_movers_count();
    [[dont_unroll]] for (uint m = 0u; m < n; m++) {
        vec4 b = gi2_mover(m, 0u);
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
bool gi2_mover_mesh(uint m, out MfInst mi) {
    float idp = gi2_mover(m, 1u).w;
    if (idp < 10000.5) {
        return false;
    }
    mi = mf_inst(uint(idp + 0.5) - 10001u);
    return true;
}
vec3 gi2_mover_to_local(uint m, vec3 p, bool dir) {
    MfInst mi;
    if (gi2_mover_mesh(m, mi)) {
        return dir ? normalize(mf_dir_to_object(mi, p)) : mf_to_object(mi, p);
    }
    vec4 c = gi2_mover(m, 1u);
    vec4 r = gi2_mover(m, 3u);
    vec3 d = dir ? p : p - c.xyz;
    return vec3(r.x * d.x - r.y * d.z, d.y, r.y * d.x + r.x * d.z);
}
vec3 gi2_mover_to_world(uint m, vec3 l, bool dir) {
    MfInst mi;
    if (gi2_mover_mesh(m, mi)) {
        mat3 a = transpose(mat3(mi.r0.xyz, mi.r1.xyz, mi.r2.xyz));
        mat3 inv = inverse(a);
        if (dir) {
            return normalize(transpose(a) * l);
        }
        return inv * (l - vec3(mi.r0.w, mi.r1.w, mi.r2.w));
    }
    vec4 c = gi2_mover(m, 1u);
    vec4 r = gi2_mover(m, 3u);
    vec3 d = vec3(r.x * l.x + r.y * l.z, l.y, -r.y * l.x + r.x * l.z);
    return dir ? d : d + c.xyz;
}
// The mover whose surface pos (facing n) lies on, or ~0. An occluder: within its
// local box plus 3 cm, on the face its normal points out of (not the floor under
// or beside it). A mesh: within 3 cm of its distance field.
uint gi2_mover_on(vec3 pos, vec3 n) {
    if (!gi2_sc_local_on()) {
        return ~0u;
    }
    uint cnt = gi2_movers_count();
    [[dont_unroll]] for (uint m = 0u; m < cnt; m++) {
        vec4 b = gi2_mover(m, 0u);
        vec3 d = pos - b.xyz;
        if (dot(d, d) >= b.w * b.w) {
            continue;
        }
        MfInst mi;
        if (gi2_mover_mesh(m, mi)) {
            if (any(lessThan(pos, mi.lo - 0.03)) || any(greaterThan(pos, mi.hi + 0.03))) {
                continue;
            }
            float dist = mf_sample(mi, mf_to_object(mi, pos)).x * mi.dscale;
            if (abs(dist) < 0.04) {
                return m;
            }
            continue;
        }
        vec3 l = gi2_mover_to_local(m, pos, false);
        vec3 e = gi2_mover(m, 2u).xyz;
        if (any(greaterThan(abs(l), e + 0.03))) {
            continue;
        }
        vec3 ln = gi2_mover_to_local(m, n, true);
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
uint gi2_mover_by_id(float idp) {
    uint n = gi2_movers_count();
    [[dont_unroll]] for (uint m = 0u; m < n; m++) {
        if (abs(gi2_mover(m, 1u).w - idp) < 0.5) {
            return m;
        }
    }
    return ~0u;
}
// Texel slot's surface point and normal in the world now: a texel of a moving
// object stores them in the object's frame (GI2_SC_OBJ_AT holds id + 1). False
// for an object texel whose object is no longer listed.
bool gi2_sc_texel_world(uint slot, out vec3 p, out vec3 n) {
    p = cache.v[3u * slot].xyz;
    n = cache.v[3u * slot + 1u].xyz;
    float idp = cache.v[GI2_SC_OBJ_AT + slot].x;
    if (idp < 0.5) {
        return true;
    }
    uint m = gi2_mover_by_id(idp);
    if (m == ~0u) {
        return false;
    }
    p = gi2_mover_to_world(m, p, false);
    n = gi2_mover_to_world(m, n, true);
    return true;
}
vec3 gi2_sc_texel_pos(uint slot) {
    vec3 p;
    vec3 n;
    gi2_sc_texel_world(slot, p, n);
    return p;
}

bool gi2_near_change(vec3 p) {
    vec4 head = work.v[gi2_change_at()];
    uint changes = head.y > 0.5 ? 0u : min(uint(head.x + 0.5), GI2_CHANGES);
    [[dont_unroll]] for (uint c = 0u; c < changes; c++) {
        vec4 b = work.v[gi2_change_at() + GI2_CHANGE_HEAD + c];
        vec3 d = p - b.xyz;
        float r = b.w + GI2_NEAR_CHANGE;
        if (dot(d, d) < r * r) {
            return true;
        }
    }
    return false;
}

// Surface cache: the light has held still long enough (params.z bit 10, which
// means young-only rounds in the other modes).
bool gi2_sc_light_quiet() {
    return ((pc.params.z >> 10u) & 1u) != 0u;
}

// Surface cache: sets the texel in slot has averaged going into this relight
// (its running mean weighs the new set 1 / (count + 1)). Inside a change or
// under changing light, at most GI2_SC_MOVING.
// S62C (Anthony: a change anywhere held every texel at low detail): only a
// local change shortens history: a mover nearby (GI2_SC_MOVING), or a lamp that
// reaches the texel moved since its last relight (the light window). Light that
// changes everywhere a little (the sun turning, any box moving somewhere) no
// longer does: the estimator tracks it (gi2_cache.comp, a floor on its blend
// while the light changes, GI2_SC_CHANGE_ALPHA).
bool gi2_wrc_lamp_changed(vec3 p, uint updated);
// The cache pass sets this to the texel's last relight before it stamps the new
// one (~0u: read the stamp).
uint gi2_sc_upd_override = ~0u;
bool gi2_sc_local_change(uint slot, out bool mover) {
    bool urgent = ((pc.params.z >> 5u) & 1u) != 0u;
    vec3 p = gi2_sc_texel_pos(slot);
    mover = urgent && gi2_near_change(p);
    if (mover) {
        return true;
    }
    uint upd = gi2_sc_upd_override != ~0u ? gi2_sc_upd_override : gi2_sc_updated(slot);
    return !gi2_sc_light_quiet() && gi2_wrc_lamp_changed(p, upd);
}
float gi2_sc_count(uint slot) {
    float t = floor(max(cache.v[3u * slot + 2u].w, 0.0));
    if (gi2_world_mode() == 3u && cache.v[3u * slot + 1u].w > 0.2) {
        // S62D: WRC cells keep their history whatever the light does (they
        // track it with an EMA floor, gi2_cache.comp).
        return t;
    }
    bool mover;
    if (gi2_sc_local_change(slot, mover)) {
        t = min(t, mover ? GI2_SC_MOVING : gi2_sc_light_window());
    }
    return t;
}
// Blend floor of a texel's long-term mean while the light changes anywhere
// (an exponential moving average that keeps up; Lumen's surface cache and
// radiance cache keep a short temporal accumulation the same way).
const float GI2_SC_CHANGE_ALPHA = 1.0 / 24.0;
// WRC: per 16-ray sample (a refresh adds 4): 1/32 while the light changes
// anywhere (1/8 a set), 1/8 near a mover or a moved lamp (1/2 a set).
const float GI2_WRC_CHANGE_ALPHA = 1.0 / 32.0;
const float GI2_WRC_LOCAL_ALPHA = 1.0 / 8.0;

float gi2_sc_cap(uint slot) {
    if (gi2_world_mode() == 3u && cache.v[3u * slot + 1u].w > 0.2) {
        return GI2_WRC_CAP;
    }
    float cap = max(gi2_world_cap(), 1.0);
    if ((gi2_sc_flags() & 16u) != 0u && cache.v[3u * slot + 1u].w < 0.1) {
        cap *= 2.0;
    }
    return cap;
}

// Sets a texel averages while the light changes: GI2_SC_LIGHT, or 0 (each relight
// replaces it) with flag 128.
float gi2_sc_light_window() {
    return (gi2_sc_flags() & (128u | 1024u)) != 0u ? 0.0 : GI2_SC_LIGHT;
}
// Cell of the light cache a ray hit reads: flag 256 keeps the 25 cm patches of the
// other modes for hits (relit fast), the fine texels for pixels.
// Flag 2048: a surface not facing along an axis (a turned box) has no texels of
// its own; it keeps the 25 cm patches of the other modes and the probes.
bool gi2_sc_slanted(vec3 n) {
    vec3 a = abs(n);
    return gi2_world_mode() == 3u && (gi2_sc_flags() & 2048u) != 0u && max(a.x, max(a.y, a.z)) < 0.999;
}
float gi2_hit_size_n(vec3 pos, vec3 n) {
    if (gi2_sc_slanted(n)) {
        return GI2_SC_HIT;
    }
    return gi2_hit_size(pos);
}
float gi2_hit_size(vec3 pos) {
    if (gi2_world_mode() == 3u && (gi2_sc_flags() & 256u) != 0u) {
        return GI2_SC_HIT;
    }
    return gi2_cell_size(pos);
}

// Hash a texel's rays hang on: the set it is on (a new set each relight up to the
// cap, so the mean takes new rays; the same set while it relights under change).
uint gi2_sc_ray_key(uint slot, uint key) {
    if (gi2_world_mode() != 3u) {
        return key;
    }
    float t = gi2_sc_count(slot);
    uint set = uint(t);
    if (!gi2_sc_light_quiet() && t >= gi2_sc_light_window() - 0.5 && (gi2_sc_flags() & 1024u) == 0u) {
        // Under changing light the count holds, so the set turns with the rounds.
        set += gi2_round();
    } else if ((gi2_sc_flags() & 8192u) != 0u && t >= gi2_sc_cap(slot) - 0.5) {
        // Relit at the cap (flag 8192): a new set each time, not set 0 over again
        // (one set's noise would be all the running mean ends up with).
        set += gi2_round() * 7919u + 1u;
    }
    set %= uint(gi2_sc_cap(slot));
    return gi2_hash(key ^ (set * 0x2545f491u + 0x68e31da4u));
}

// Surface cache: what a texel shows while it has few sets (its first fill, then
// refining): the count-weighted mean of the texels around it on the same plane, a
// Gaussian of GI2_SC_POOL / sqrt(sets + 1) cells (at most 2), so a new texel
// starts from its lit neighbours and its own noise fades as its sets come in.
// Kept in the world slot's y as rgb9e5 (x: gi2_sc_offer's distance; z: the frame
// a pixel last read the texel); 0 when not pooled.
const float GI2_SC_POOL = 2.4;
const float GI2_SC_POOL_FULL = 24.0;
// S62A denoise (SVGF, Schied et al. 2017: variance-guided, edge-stopping; in
// surface space, so it is anchored to the world and cannot swim): from
// GI2_DN_MIN_SETS sets on, the pool's radius follows the texel's relative noise
// (the GIBS estimator's per-set variance over its sets, sqrt(var / n) / mean):
// sigma = GI2_DN_SIGMA * noise / target, 0.4 to 2 cells; each neighbour also
// weighs exp(-|dL| / (GI2_DN_LUM * the noise in luminance)), so shadow edges and
// contact darkening (real luminance steps) stop it, noise does not.
// Perceptual stop rule (gi2_sc_settled): a texel with at least GI2_DN_MIN_SETS
// sets whose relative noise is under GI2_DN_STOP (looser in the dark: under
// GI2_DN_DARK luminance the target grows as GI2_DN_DARK / L, at most 4x)
// counts as settled.
const float GI2_DN_MIN_SETS = 8.0;
const float GI2_DN_STOP = 0.015;
const float GI2_DN_DARK = 0.05;
const float GI2_DN_SIGMA = 0.6;
const float GI2_DN_LUM = 3.0;
// Pool radius under changing light (cells; under 0.9 the kernel is 3 x 3).
const float GI2_DN_CHANGE_SIGMA = 0.7;
// Relative noise of slot's mean, and its luminance noise (w); x < 0 when the
// estimator is not running (new, or the light changing).
vec2 gi2_sc_noise(uint slot) {
    vec4 a = cache.v[GI2_SC_EST_AT + 2u * slot];
    vec4 b = cache.v[GI2_SC_EST_AT + 2u * slot + 1u];
    if (!(b.x > 0.0) || !(b.w > 0.5)) {
        return vec2(-1.0, 0.0);
    }
    float lum = dot(cache.v[3u * slot + 2u].rgb, vec3(0.2126, 0.7152, 0.0722));
    float sd = sqrt(max(a.w, 0.0) / max(b.w, 1.0));
    return vec2(sd / max(lum, 1.0e-3), sd);
}
float gi2_dn_target(uint slot) {
    float lum = dot(cache.v[3u * slot + 2u].rgb, vec3(0.2126, 0.7152, 0.0722));
    return GI2_DN_STOP * clamp(GI2_DN_DARK / max(lum, 1.0e-4), 1.0, 4.0);
}
void gi2_sc_pool(uint slot) {
    vec4 own = cache.v[3u * slot + 2u];
    // A texel in a mover's frame pools with its neighbours in that frame: its
    // position and normal are stored there, and the same salt finds them.
    gi2_obj_salt = cache.v[GI2_SC_OBJ_AT + slot].x > 0.5 ? cache.v[GI2_SC_OBJ_AT + slot].x : 0.0;
    float t = floor(max(own.w, 0.0));
    bool floor_on = (gi2_sc_flags() & 8u) != 0u;
    if (t >= GI2_SC_POOL_FULL && !floor_on) {
        cache.v[GI2_WORLD_AT + slot].y = uintBitsToFloat(gi2_pack_rgb9e5(own.rgb));
        gi2_obj_salt = 0.0;
        return;
    }
    vec3 p = cache.v[3u * slot].xyz;
    vec4 nb = cache.v[3u * slot + 1u];
    vec3 n = nb.xyz;
    float size = nb.w;
    float sigma = min(GI2_SC_POOL / sqrt(t + 1.0), 2.0);
    if (floor_on) {
        sigma = max(sigma, 0.6);
    }
    // S62A: variance-guided radius and luminance edge stop once warmed up.
    vec2 nz = gi2_sc_noise(slot);
    bool guided = t >= GI2_DN_MIN_SETS && nz.x >= 0.0;
    float own_lum = dot(own.rgb, vec3(0.2126, 0.7152, 0.0722));
    if (guided) {
        sigma = clamp(GI2_DN_SIGMA * nz.x / gi2_dn_target(slot), 0.4, 2.0);
    }
    // S62B: under changing light the history is short and stays short, so the
    // pool keeps to the nearest texels (detail from the texels' own sets).
    if (!gi2_sc_light_quiet()) {
        sigma = min(sigma, GI2_DN_CHANGE_SIGMA);
    }
    uint ax = gi2_axis(n) >> 1u;
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
                s2 = gi2_cache_find_sized(p + (float(i) * ta + float(j) * tb) * size, n, false, size, p);
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
                k *= exp(-dl / (GI2_DN_LUM * nz.y + 1.0e-4));
            }
            sum += k * c * v.rgb;
            wsum += k * c;
        }
    }
    cache.v[GI2_WORLD_AT + slot].y = uintBitsToFloat(gi2_pack_rgb9e5(wsum > 0.0 ? sum / wsum : vec3(0.0)));
    gi2_obj_salt = 0.0;
}

// Surface cache: what a texel shows (w 0 when it has nothing yet).
vec4 gi2_sc_shown(uint slot) {
    vec3 pooled = gi2_unpack_rgb9e5(floatBitsToUint(cache.v[GI2_WORLD_AT + slot].y));
    vec4 own = cache.v[3u * slot + 2u];
    float ls = GI2_LS_RESCALE && gi2_world_mode() == 3u ? gi2_ls_ratio(gi2_sc_updated(slot)) : 1.0;
    if (any(greaterThan(pooled, vec3(0.0)))) {
        return vec4(pooled * ls, 1.0);
    }
    return own.w > 0.5 ? vec4(own.rgb * ls, 1.0) : vec4(0.0);
}

// Surface cache read rule at a ray hit t from the ray's origin (SHaRC: a cache
// cell answers a ray only if it is small against the distance, t > size sqrt 3,
// and holds enough samples): a near hit reads the finest lit texel with
// t > size sqrt 3 and at least GI2_SC_READ_SETS sets; otherwise the 25 cm patch
// as before, which falls back to its lit neighbours, never black.
const float GI2_SC_READ_SETS = 4.0;
vec3 gi2_cache_irradiance(vec3 pos, vec3 n);
vec3 gi2_cache_irradiance_at(vec3 pos, vec3 n, float t) {
    if (gi2_world_mode() == 3u && (gi2_sc_flags() & 2097152u) == 0u && !gi2_sc_slanted(n)) {
        float hs = gi2_hit_size_n(pos, n);
        if (t < hs * 1.7320508) {
            float fs = GI2_SC_TEXEL;
            while (fs * 2.0 < hs && t >= fs * 2.0 * 1.7320508) {
                fs *= 2.0;
            }
            uint s = gi2_cache_find_sized(pos, n, false, fs, pos);
            if (s != ~0u) {
                vec4 v = cache.v[3u * s + 2u];
                if (v.w >= GI2_SC_READ_SETS && fract(v.w) < 0.1) {
                    cache.v[3u * s].w = uintBitsToFloat(gi2_frame());
                    return v.rgb;
                }
            }
        }
    }
    return gi2_cache_irradiance(pos, n);
}

// Irradiance already bounced onto the surface at pos facing n (0 for a new patch).
vec3 gi2_cache_irradiance(vec3 pos, vec3 n) {
    // Flag 2097152 (diagnostic): no light from the cache at ray hits, so the
    // picture has one bounce (compare with GENOS_GI_CHECK_BOUNCES=1 references).
    if ((gi2_sc_flags() & 2097152u) != 0u) {
        return vec3(0.0);
    }
    float hsize = gi2_hit_size_n(pos, n);
    uint slot = gi2_cache_find_sized(pos, n, true, hsize, pos);
    if (slot == ~0u) {
        atomicAdd(gstats.missed, 1u);
        return vec3(0.0);
    }
    cache.v[3u * slot].w = uintBitsToFloat(gi2_frame());
    // Flag 65536: hit patches stay lit from where they were claimed (as in the
    // other modes), no offer.
    if (gi2_world_mode() == 3u && !gi2_sc_slanted(n) && (gi2_sc_flags() & 65536u) == 0u) {
        gi2_sc_offer(slot, pos, gi2_sc_centre(pos, n, hsize));
    }
    vec4 e = cache.v[3u * slot + 2u];
    // Not lit yet (claimed this round, params.z bit 7): the lit patches beside it
    // on the same surface, so a surface coming into view or a box face entering
    // new cells is not black until the next round relights it.
    // Flag 32768 (surface cache): every round, not only the round that claimed it.
    bool fill_any = gi2_world_mode() == 3u && (gi2_sc_flags() & 32768u) != 0u;
    if (e.w < 0.5 && (fill_any || ((pc.params.z >> 7u) & 1u) != 0u)) {
        float size = cache.v[3u * slot + 1u].w;
        if (gi2_world_mode() == 3u) {
            vec4 up = gi2_sc_parent(pos, n, hsize);
            if (up.w > 0.0) {
                return up.rgb;
            }
        }
        vec3 t1 = normalize(abs(n.x) < 0.9 ? cross(n, vec3(1.0, 0.0, 0.0)) : cross(n, vec3(0.0, 1.0, 0.0)));
        vec3 t2 = cross(n, t1);
        vec4 sum = vec4(0.0);
        [[dont_unroll]] for (uint i = 0u; i < 4u; i++) {
            vec3 o = (i < 2u ? t1 : t2) * ((i & 1u) == 0u ? size : -size);
            uint s2 = gi2_cache_find_sized(pos + o, n, false, hsize, pos + o);
            if (s2 != ~0u) {
                vec4 v = cache.v[3u * s2 + 2u];
                if (v.w > 0.5) {
                    sum += vec4(v.rgb, 1.0);
                }
            }
        }
        return sum.w > 0.0 ? sum.rgb / sum.w : vec3(0.0);
    }
    return e.rgb;
}

uint gi2_live_count() {
    return min(gi2_young_count() + gi2_mature_count(), GI2_CACHE_SLOTS);
}

// Slot of this round's batch entry j, or ~0u when the batch has no entry j: the
// young first, then the mature ones in turn.
uint gi2_cache_batch_slot(uint j) {
    uint room = GI2_CACHE_BATCH;
    if (gi2_world_mode() == 3u) {
        if (j >= GI2_CACHE_BATCH - GI2_WRC_SCOUT_ENTRIES) {
            return ~0u;
        }
        room = GI2_CACHE_BATCH - GI2_WRC_SCOUT_ENTRIES - GI2_WRC_REFRESH;
        if (j >= room) {
            // The WRC refresh entries: change-driven, then background, each
            // cell a whole set (4 consecutive entries).
            uint k = (j - room) / 4u;
            uint c = min(ckeys.key[GI2_WRC_CHG_COUNT_AT], GI2_WRC_REFRESH_CELLS);
            if (k < c) {
                return ckeys.key[GI2_WRC_CHG_AT + k];
            }
            k -= c;
            if (k < min(ckeys.key[GI2_WRC_BG_COUNT_AT], GI2_WRC_REFRESH_CELLS)) {
                return ckeys.key[GI2_WRC_BG_AT + k];
            }
            return ~0u;
        }
    }
    uint first = min(gi2_young_count(), room);
    if (j < first) {
        return ckeys.key[2u * GI2_CACHE_SLOTS - j];
    }
    uint count = gi2_mature_count();
    uint k = j - first;
    if ((gi2_young_only() && gi2_world_mode() != 3u) || k >= count) {
        return ~0u;
    }
    // The scheduler lists only what this round should take (from the front);
    // otherwise the sweep turns with the rounds.
    uint start = (gi2_world_mode() == 3u && GI2_SC_SCHED) ? 0u : (gi2_round() * GI2_CACHE_BATCH) % count;
    return ckeys.key[GI2_CACHE_SLOTS + 1u + (start + k) % count];
}

// Few rays a relight (S59; Lumen: a few rays per texel per update, accumulated
// over updates): a batch entry traces GI2_CACHE_RAYS rays, a quarter of a
// GI2_SC_SET_RAYS set (its stratified pattern split in GI2_SC_QUARTERS). A
// surface-cache texel adds its quarters' light to a partial sum (GI2_SC_OBJ_AT
// .z, rgb9e5; .w the quarters so far) and commits the set to its mean when the
// last quarter is in, so every history rule still counts whole sets. A texel
// takes one entry a relight, or the rest of its set at once (consecutive
// entries) when young, near a change, short of GI2_SC_FULL_SETS sets or
// inconsistent (GIBS 2021: more rays where the estimate is unconverged or its
// variance high). The other modes take whole sets.
const uint GI2_SC_SET_RAYS = 64u;
const uint GI2_SC_QUARTERS = GI2_SC_SET_RAYS / GI2_CACHE_RAYS;
// Tunables (S59E, Anthony: "bump up the quality before it settles"): whole sets
// until GI2_SC_FULL_SETS sets AND agreement within GI2_SC_SETTLE_SD standard
// deviations; an unsettled texel's priority is at least GI2_SC_UNSETTLED_W times
// a settled one's; dormancy needs the same agreement at the cap.
const float GI2_SC_FULL_SETS = 64.0;
const float GI2_SC_SETTLE_SD = 0.15;
const float GI2_SC_UNSETTLED_W = 4.0;
const float GI2_SC_FULL_INC = 1.0;
// Settled (S59D, Anthony: "settling too soon ... splotchy"): GI2_SC_FULL_SETS
// whole sets in its mean (or, while the light changes, the whole window) AND the
// GIBS estimator running with its short-term mean within 0.3 standard deviations
// of the long-term one (the dormancy test's agreement). Under changing light the
// estimator restarts every relight, so nothing settles: whole sets throughout.
bool gi2_sc_settled(uint slot, float t) {
    float reach = GI2_SC_FULL_SETS;
    if (cache.v[3u * slot + 1u].w > 0.2) {
        // A WRC cell: settled at its cap (or the window under changing light).
        return floor(t) >= GI2_WRC_CAP - 0.5;
    }
    vec4 b = cache.v[GI2_SC_EST_AT + 2u * slot + 1u];
    // S62A perceptual stop: quiet enough to the eye counts as settled.
    vec2 nz = gi2_sc_noise(slot);
    if (gi2_sc_light_quiet() && floor(t) >= GI2_DN_MIN_SETS && nz.x >= 0.0 && nz.x < gi2_dn_target(slot) && b.y < GI2_SC_SETTLE_SD) {
        return true;
    }
    return floor(t) >= reach - 0.5 && b.x > 0.0 && b.y < GI2_SC_SETTLE_SD;
}
uint gi2_sc_quarter0(uint slot) {
    if (gi2_world_mode() != 3u) {
        return 0u;
    }
    return min(floatBitsToUint(cache.v[GI2_SC_OBJ_AT + slot].w), GI2_SC_QUARTERS - 1u);
}
uint gi2_sc_entries(uint slot, bool full) {
    return (full || gi2_world_mode() != 3u) ? GI2_SC_QUARTERS - gi2_sc_quarter0(slot) : 1u;
}
// WRC scout ray s of this round (entries past the batch's texels): from the eye
// (a quarter from up to 2 m beside it), half over the sphere and half within
// ~50 degrees of where the camera is heading (forward, plus its motion and turn
// since the last frame the probes ran).
void gi2_wrc_scout(uint s, out vec3 origin, out vec3 dir) {
    uint h = gi2_hash(s * 0x9e3779b9u ^ gi2_round() * 0x85ebca6bu);
    float u1 = gi2_unit(h);
    float u2 = gi2_unit(h ^ 0x68e31da4u);
    vec4 head_at = vec4(0.0);
    vec3 eye = scene.eye.xyz;
    vec3 fwd = normalize(scene.view_forward.xyz);
    vec4 last_eye = work.v[gi2_change_at() + 1u];
    vec3 last_fwd = work.v[gi2_change_at() + 4u].xyz;
    vec3 ahead = fwd;
    if (last_eye.w > 0.5 && dot(last_fwd, last_fwd) > 0.5) {
        ahead = fwd + 8.0 * (eye - last_eye.xyz) + 4.0 * (fwd - normalize(last_fwd));
    }
    ahead = normalize(ahead + vec3(0.0, 1.0e-4, 0.0));
    float z = (s & 1u) == 0u ? 1.0 - 2.0 * u1 : 1.0 - u1 * 0.36;
    float r = sqrt(max(1.0 - z * z, 0.0));
    float a = GI2_TAU * u2;
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
        float u3 = gi2_unit(h ^ 0x27d4eb2fu);
        float b3 = GI2_TAU * gi2_unit(h ^ 0x165667b1u);
        origin += vec3(cos(b3), 0.0, sin(b3)) * (2.0 * u3);
    }
}
// WRC: did a lamp that reaches p move since frame `updated` (within the last
// GI2_WRC_CHANGE_FRAMES)? The ring holds moved lamps' spheres (both places,
// range), written once a round by gi2_wrc_note_lamps.
bool gi2_wrc_lamp_changed(vec3 p, uint updated) {
    [[dont_unroll]] for (uint i = 0u; i < GI2_WRC_RING; i++) {
        uint at = GI2_WRC_RING_AT + 5u * i;
        uint f = ckeys.key[at + 4u];
        if (f == 0u || int(gi2_frame() - f) > int(GI2_WRC_CHANGE_FRAMES) || int(updated - f) >= 0) {
            continue;
        }
        vec4 s = vec4(uintBitsToFloat(ckeys.key[at]), uintBitsToFloat(ckeys.key[at + 1u]),
                      uintBitsToFloat(ckeys.key[at + 2u]), uintBitsToFloat(ckeys.key[at + 3u]));
        if (length(p - s.xyz) < s.w) {
            return true;
        }
    }
    return false;
}
// Once a round (cache pass, thread 0): lamps that moved or changed range since
// the last round go into the ring (suns are left to the light-change rule).
void gi2_wrc_note_lamps() {
    uint n = min(scene.lamp_count, GI2_WRC_SNAP_LAMPS);
    uint had = ckeys.key[GI2_WRC_LAMPS_AT];
    for (uint i = 0u; i < n; i++) {
        Lamp l = scene_lamp(i);
        uint at = GI2_WRC_SNAP_AT + 4u * i;
        vec4 o = vec4(uintBitsToFloat(ckeys.key[at]), uintBitsToFloat(ckeys.key[at + 1u]),
                      uintBitsToFloat(ckeys.key[at + 2u]), uintBitsToFloat(ckeys.key[at + 3u]));
        bool fresh = i >= had;
        bool moved = fresh || length(o.xyz - l.pos.xyz) > 1.0e-3 || abs(o.w - l.color.w) > 1.0e-3;
        if (moved && l.color.w > 0.0) {
            vec3 c = fresh ? l.pos.xyz : 0.5 * (o.xyz + l.pos.xyz);
            float r = l.color.w + (fresh ? 0.0 : 0.5 * length(o.xyz - l.pos.xyz));
            uint h = ckeys.key[GI2_WRC_RING_HEAD_AT] % GI2_WRC_RING;
            ckeys.key[GI2_WRC_RING_HEAD_AT] = h + 1u;
            uint ra = GI2_WRC_RING_AT + 5u * h;
            ckeys.key[ra] = floatBitsToUint(c.x);
            ckeys.key[ra + 1u] = floatBitsToUint(c.y);
            ckeys.key[ra + 2u] = floatBitsToUint(c.z);
            ckeys.key[ra + 3u] = floatBitsToUint(r);
            ckeys.key[ra + 4u] = max(gi2_frame(), 1u);
        }
        ckeys.key[at] = floatBitsToUint(l.pos.x);
        ckeys.key[at + 1u] = floatBitsToUint(l.pos.y);
        ckeys.key[at + 2u] = floatBitsToUint(l.pos.z);
        ckeys.key[at + 3u] = floatBitsToUint(l.color.w);
    }
    ckeys.key[GI2_WRC_LAMPS_AT] = n;
}
// Quarter of its set batch entry j (of slot) traces.
uint gi2_cache_entry_quarter(uint j, uint slot) {
    uint i = 0u;
    while (i + 1u < GI2_SC_QUARTERS && j > i && gi2_cache_batch_slot(j - i - 1u) == slot) {
        i++;
    }
    return min(gi2_sc_quarter0(slot) + i, GI2_SC_QUARTERS - 1u);
}

// Mode 2: a screen probe at pos facing n adds its irradiance e to its world cell.
// Inside a change (a sphere of an object that moved) the mean restarts short.
void gi2_world_add(vec3 pos, vec3 n, vec3 e, bool changed) {
    uint slot = gi2_cache_find(pos, n, true);
    if (slot == ~0u) {
        return;
    }
    cache.v[3u * slot].w = uintBitsToFloat(gi2_frame());
    vec4 h = cache.v[GI2_WORLD_AT + slot];
    float k = min(changed ? min(h.w, 1.0) + 1.0 : h.w + 1.0, max(gi2_world_cap(), 1.0));
    cache.v[GI2_WORLD_AT + slot] = vec4(h.rgb + (e - h.rgb) / k, k);
}

// The world cells around pos facing n, interpolated across the surface (the four
// cell centres nearest pos in the plane of its axis). Returns the irradiance and,
// in w, how much of it to trust (0 to 1: the share of cells that have light,
// counted up to 4 samples).
// Experiment S54K: texels stay shown next to a moving object too (Lumen keeps
// the surface cache and relights it; no method swap to the probes, whose
// difference from the texels was the bright contact strip of the spin check).
const bool GI2_SC_KEEP_NEAR = true;
vec4 gi2_world_irradiance_in(vec3 pos, vec3 n, vec3 wpos, bool obj);
vec4 gi2_world_irradiance(vec3 pos, vec3 n) {
    uint mode = gi2_world_mode();
    // Object-local texels (GENOS_GI2_SC_LOCAL=1, GIBS): a pixel on a mover's
    // surface reads texels keyed in the mover's frame, which move and turn with
    // it and keep their light (relit fast while it moves, as near any change).
    uint om = mode == 3u ? gi2_mover_on(pos, n) : ~0u;
    if (om != ~0u) {
        gi2_obj_salt = gi2_mover(om, 1u).w;
        vec4 r = gi2_world_irradiance_in(gi2_mover_to_local(om, pos, false), gi2_mover_to_local(om, n, true), pos, true);
        gi2_obj_salt = 0.0;
        return r;
    }
    // Flag 4194304 (surface cache): a pixel on or inside the bounds of a mover
    // (an object that moved in the last GI2_MOVER_KEEP frames) takes the screen
    // probes, which trace it as it is now, instead of world texels it sweeps
    // through.
    if (mode == 3u && !GI2_SC_KEEP_NEAR && (gi2_sc_flags() & 4194304u) != 0u && gi2_in_change(pos)) {
        return vec4(0.0);
    }
    return gi2_world_irradiance_in(pos, n, pos, false);
}
// Debug view "texels" (gi2_compose.comp mode 9): the size a pixel at pos facing n
// reads and the sets its texel holds (0 when unclaimed).
vec2 gi2_sc_debug_size(vec3 pos, vec3 n) {
    float size = gi2_cell_size(pos);
    uint s = gi2_cache_find_sized(pos, n, false, size, pos);
    return vec2(size, s == ~0u ? 0.0 : floor(max(cache.v[3u * s + 2u].w, 0.0)));
}
vec4 gi2_world_irradiance_in(vec3 pos, vec3 n, vec3 wpos, bool obj) {
    uint mode = gi2_world_mode();
    if (gi2_sc_slanted(n)) {
        return vec4(0.0);
    }
    uint ax = gi2_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    float size = gi2_cell_size(wpos);
    vec2 uv = vec2(dot(pos, ta), dot(pos, tb)) / size - 0.5;
    vec2 g0 = floor(uv);
    vec2 f = uv - g0;
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    float trust = 0.0;
    [[dont_unroll]] for (uint k = 0u; k < 4u; k++) {
        vec2 c = g0 + vec2(float(k & 1u), float(k >> 1u)) + 0.5;
        vec3 q = pos + ta * (c.x * size - dot(pos, ta)) + tb * (c.y * size - dot(pos, tb));
        float bil = ((k & 1u) == 1u ? f.x : 1.0 - f.x) * ((k >> 1u) == 1u ? f.y : 1.0 - f.y);
        uint slot = gi2_cache_find_sized(q, n, mode != 2u, size, pos);
        if (slot == ~0u) {
            continue;
        }
        cache.v[3u * slot].w = uintBitsToFloat(gi2_frame());
        if (mode == 3u && (gi2_sc_flags() & 524288u) != 0u && cache.v[3u * slot + 2u].w < 0.5) {
            vec4 rs = gi2_sc_resample(q, n, size, slot);
            if (rs.w > 0.5) {
                // S62E: the seed is light as of now; stored in the slot's scale.
                if (GI2_LS_RESCALE) {
                    rs.rgb /= max(gi2_ls_ratio(gi2_sc_updated(slot)), 1.0e-3);
                }
                cache.v[3u * slot + 2u] = rs;
            }
        }
        if (mode == 3u) {
            gi2_sc_offer(slot, pos, gi2_sc_centre(q, n, size));
            cache.v[GI2_WORLD_AT + slot].z = uintBitsToFloat(gi2_frame());
        }
        vec4 v = mode == 3u ? gi2_sc_shown(slot) : (mode != 2u ? cache.v[3u * slot + 2u] : cache.v[GI2_WORLD_AT + slot]);
        if (v.w < 0.5 && mode == 3u) {
            v = gi2_sc_parent(q, n, size);
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
        bool est = (gi2_sc_flags() & 1048576u) != 0u;
        bool stale = mode == 3u && fract(own_w) > 0.1 && !est;
        float sc_trust = stale ? 0.0 : 1.0;
        if ((gi2_sc_flags() & 32u) != 0u || obj) {
            // A texel with few sets shares the pixel with the probes.
            float c = floor(max(own_w, 0.0));
            sc_trust *= c / (c + GI2_SC_TRUST_SETS);
        }
        trust += bil * (mode == 3u ? sc_trust : (mode != 2u ? 1.0 : min(v.w / 4.0, 1.0)));
    }
    // Surface cache: while the light changes or around an object that moves, the
    // probes (no history) show it.
    bool light_swap = !gi2_sc_light_quiet() && (gi2_sc_flags() & 1048576u) == 0u;
    if (mode == 3u && (light_swap || (!obj && !GI2_SC_KEEP_NEAR && gi2_near_change(pos)) || (gi2_sc_flags() & 512u) != 0u)) {
        trust = 0.0;
    }
    return vec4(wsum > 0.0 ? sum / wsum : vec3(0.0), trust);
}
