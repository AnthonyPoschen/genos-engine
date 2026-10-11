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
// bit 3 pooling never narrower than 0.6 cells; bit 4 the finest texels (inside
// GI2_SC_NEAR) grow to twice the cap; bit 5 a texel with few sets shares its
// pixels with the probes; bit 6 a texel's lit-from point moving does not restart
// its sets.
uint gi2_sc_flags() {
    return uint(work.v[gi2_change_at()].z + 0.5) >> 1u;
}
// Surface cache (mode 3, GENOS_GI2_WORLD=3): the light cache's patches are the
// texels pixels read. A texel adds a new set of GI2_CACHE_RAYS rays each time it is
// relit and keeps their running mean, up to cap sets; then it holds (converged)
// until the light changes or an object moves near it. Within GI2_SC_NEAR of the
// eye texels are 6.25 cm, to 8 m 12.5 cm.
const float GI2_SC_NEAR = 2.5;
// Sets a texel inside a change, or under a light that is changing, averages over
// (a short running mean on one fixed set: quick, and no flicker).
const float GI2_SC_MOVING = 2.0;
// Sets a texel averages over while the light changes (lamps moving: a running
// mean over a window, the sets turning with the rounds).
const float GI2_SC_LIGHT = 8.0;
// Sets a texel only rays read (not on screen) grows to.
const float GI2_SC_HIT_CAP = 8.0;
// Flag 32: a texel's share of its pixels against the probes, sets / (sets + this).
const float GI2_SC_TRUST_SETS = 6.0;

float gi2_cell_size(vec3 pos) {
    float d = length(pos - scene.eye.xyz);
    float level = clamp(floor(log2(max(d / 8.0, 1.0))), 0.0, 6.0);
    // Surface cache (mode 3): 12.5 cm texels to 8 m, 6.25 cm inside GI2_SC_NEAR.
    if (gi2_world_mode() == 3u) {
        return d < GI2_SC_NEAR ? 0.0625 : (d < 8.0 ? 0.125 : 0.25 * exp2(level));
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
    // Flag 16384 (surface cache): a quarter of the cell along the axis, so two
    // parallel surfaces close together (a panel on a wall, a box face by the wall
    // behind it) do not share a texel lit from whichever was nearer its centre.
    if (gi2_world_mode() == 3u && (gi2_sc_flags() & 16384u) != 0u) {
        uint ax = gi2_axis(n) >> 1u;
        c[ax] = int(floor(pos[ax] / (0.25 * size) - inset[ax]));
    }
    uint h = gi2_hash(uint(c.x) * 73856093u ^ gi2_hash(uint(c.y) * 19349663u ^ gi2_hash(uint(c.z) * 83492791u)));
    h = gi2_hash(h ^ (gi2_axis(n) * 0x27d4eb2du) ^ (floatBitsToUint(size) * 0x165667b1u));
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
            cache.v[3u * slot] = vec4(at, uintBitsToFloat(gi2_frame()));
            cache.v[3u * slot + 1u] = vec4(n, size);
            cache.v[3u * slot + 2u] = vec4(0.0);
            cache.v[GI2_WORLD_AT + slot] = vec4(length(at - gi2_sc_centre(pos, n, size)), 0.0, 0.0, 0.0);
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
    float d = length(at - centre);
    if (d < cache.v[GI2_WORLD_AT + slot].x - 1.0e-4) {
        cache.v[GI2_WORLD_AT + slot].x = d;
        // Lit from far from here so far: its sets start again (value kept).
        float size = cache.v[3u * slot + 1u].w;
        if ((gi2_sc_flags() & 64u) == 0u && length(at - cache.v[3u * slot].xyz) > 0.3 * size) {
            float t = cache.v[3u * slot + 2u].w;
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
        uint s = gi2_cache_find_sized(pos, n, false, size, pos);
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

// How far around a change a patch is relit first (gi2_compact.comp), metres.
const float GI2_NEAR_CHANGE = 1.0;

// A point within GI2_NEAR_CHANGE of an object that moved, came or went this frame.
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
float gi2_sc_count(uint slot) {
    float t = floor(max(cache.v[3u * slot + 2u].w, 0.0));
    bool urgent = ((pc.params.z >> 5u) & 1u) != 0u;
    if (urgent && gi2_near_change(cache.v[3u * slot].xyz)) {
        t = min(t, GI2_SC_MOVING);
    } else if (!gi2_sc_light_quiet()) {
        t = min(t, gi2_sc_light_window());
    }
    return t;
}

float gi2_sc_cap(uint slot) {
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
        float d = length(pos - scene.eye.xyz);
        return 0.25 * exp2(clamp(floor(log2(max(d / 8.0, 1.0))), 0.0, 6.0));
    }
    return gi2_hit_size(pos);
}
float gi2_hit_size(vec3 pos) {
    if (gi2_world_mode() == 3u && (gi2_sc_flags() & 256u) != 0u) {
        float d = length(pos - scene.eye.xyz);
        return 0.25 * exp2(clamp(floor(log2(max(d / 8.0, 1.0))), 0.0, 6.0));
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
void gi2_sc_pool(uint slot) {
    vec4 own = cache.v[3u * slot + 2u];
    float t = floor(max(own.w, 0.0));
    bool floor_on = (gi2_sc_flags() & 8u) != 0u;
    if (t >= GI2_SC_POOL_FULL && !floor_on) {
        cache.v[GI2_WORLD_AT + slot].y = uintBitsToFloat(gi2_pack_rgb9e5(own.rgb));
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
            sum += k * c * v.rgb;
            wsum += k * c;
        }
    }
    cache.v[GI2_WORLD_AT + slot].y = uintBitsToFloat(gi2_pack_rgb9e5(wsum > 0.0 ? sum / wsum : vec3(0.0)));
}

// Surface cache: what a texel shows (w 0 when it has nothing yet).
vec4 gi2_sc_shown(uint slot) {
    vec3 pooled = gi2_unpack_rgb9e5(floatBitsToUint(cache.v[GI2_WORLD_AT + slot].y));
    vec4 own = cache.v[3u * slot + 2u];
    if (any(greaterThan(pooled, vec3(0.0)))) {
        return vec4(pooled, 1.0);
    }
    return own.w > 0.5 ? vec4(own.rgb, 1.0) : vec4(0.0);
}

// Irradiance already bounced onto the surface at pos facing n (0 for a new patch).
vec3 gi2_cache_irradiance(vec3 pos, vec3 n) {
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
    uint first = min(gi2_young_count(), GI2_CACHE_BATCH);
    if (j < first) {
        return ckeys.key[2u * GI2_CACHE_SLOTS - j];
    }
    uint count = gi2_mature_count();
    uint k = j - first;
    if ((gi2_young_only() && gi2_world_mode() != 3u) || k >= count) {
        return ~0u;
    }
    uint start = (gi2_round() * GI2_CACHE_BATCH) % count;
    return ckeys.key[GI2_CACHE_SLOTS + 1u + (start + k) % count];
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
vec4 gi2_world_irradiance(vec3 pos, vec3 n) {
    uint mode = gi2_world_mode();
    if (gi2_sc_slanted(n)) {
        return vec4(0.0);
    }
    uint ax = gi2_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    float size = gi2_cell_size(pos);
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
        bool stale = mode == 3u && fract(own_w) > 0.1;
        float sc_trust = stale ? 0.0 : 1.0;
        if ((gi2_sc_flags() & 32u) != 0u) {
            // A texel with few sets shares the pixel with the probes.
            float c = floor(max(own_w, 0.0));
            sc_trust *= c / (c + GI2_SC_TRUST_SETS);
        }
        trust += bil * (mode == 3u ? sc_trust : (mode != 2u ? 1.0 : min(v.w / 4.0, 1.0)));
    }
    // Surface cache: while the light changes or around an object that moves, the
    // probes (no history) show it.
    if (mode == 3u && (!gi2_sc_light_quiet() || gi2_near_change(pos) || (gi2_sc_flags() & 512u) != 0u)) {
        trust = 0.0;
    }
    return vec4(wsum > 0.0 ? sum / wsum : vec3(0.0), trust);
}
