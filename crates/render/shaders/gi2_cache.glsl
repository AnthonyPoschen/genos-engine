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

float gi2_cell_size(vec3 pos) {
    float d = length(pos - scene.eye.xyz);
    float level = clamp(floor(log2(max(d / 8.0, 1.0))), 0.0, 6.0);
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

// World irradiance cells (experiment, GENOS_GI2_WORLD): the change block's head w
// is mode * 1000 + cap. Mode 1: pixels read the light cache's patches,
// interpolated; mode 2: screen probes average their irradiance into the patch they
// sit in (a running mean of up to cap frames, binding 10 after the patches) and
// pixels read that, interpolated, so what a pixel shows depends on where it is in
// the world, not on where the probe grid fell this frame.
uint gi2_world_mode() {
    return uint(work.v[gi2_change_at()].w + 0.5) / 1000u;
}
float gi2_world_cap() {
    return float(uint(work.v[gi2_change_at()].w + 0.5) % 1000u);
}
const uint GI2_WORLD_AT = 3u * GI2_CACHE_SLOTS;

vec3 gi2_axis_vec(uint a) {
    vec3 v = vec3(0.0);
    v[a >> 1u] = (a & 1u) == 0u ? 1.0 : -1.0;
    return v;
}

// Key of the patch holding pos facing n; never 0.
uint gi2_cache_key(vec3 pos, vec3 n, out float size) {
    size = gi2_cell_size(pos);
    // World cells on: half a cell in from the surface, so a floor or wall on a cell
    // boundary falls in one cell, not either by rounding.
    vec3 inset = gi2_world_mode() != 0u ? 0.5 * gi2_axis_vec(gi2_axis(n)) : vec3(0.0);
    ivec3 c = ivec3(floor(pos / size - inset));
    uint h = gi2_hash(uint(c.x) * 73856093u ^ gi2_hash(uint(c.y) * 19349663u ^ gi2_hash(uint(c.z) * 83492791u)));
    h = gi2_hash(h ^ (gi2_axis(n) * 0x27d4eb2du) ^ (floatBitsToUint(size) * 0x165667b1u));
    return max(h, 1u);
}

// Slot of the patch for (pos, n), claiming a free one when `claim`; ~0u if none.
// The whole search window is checked for the key before anything is claimed: a
// freed slot can sit in front of a live patch's slot, and claiming it would make a
// second, unlit copy of that patch.
uint gi2_cache_find(vec3 pos, vec3 n, bool claim) {
    float size;
    uint key = gi2_cache_key(pos, n, size);
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
            cache.v[3u * slot] = vec4(pos, uintBitsToFloat(gi2_frame()));
            cache.v[3u * slot + 1u] = vec4(n, size);
            cache.v[3u * slot + 2u] = vec4(0.0);
            cache.v[GI2_WORLD_AT + slot] = vec4(0.0);
            return slot;
        }
        if (prev == key) {
            return slot;
        }
    }
    return ~0u;
}

// How far around a change a patch is relit first (gi2_compact.comp), metres.
const float GI2_NEAR_CHANGE = 1.0;

// Irradiance already bounced onto the surface at pos facing n (0 for a new patch).
vec3 gi2_cache_irradiance(vec3 pos, vec3 n) {
    uint slot = gi2_cache_find(pos, n, true);
    if (slot == ~0u) {
        atomicAdd(gstats.missed, 1u);
        return vec3(0.0);
    }
    cache.v[3u * slot].w = uintBitsToFloat(gi2_frame());
    vec4 e = cache.v[3u * slot + 2u];
    // Not lit yet (claimed this round, params.z bit 7): the lit patches beside it
    // on the same surface, so a surface coming into view or a box face entering
    // new cells is not black until the next round relights it.
    if (e.w < 0.5 && ((pc.params.z >> 7u) & 1u) != 0u) {
        float size = cache.v[3u * slot + 1u].w;
        vec3 t1 = normalize(abs(n.x) < 0.9 ? cross(n, vec3(1.0, 0.0, 0.0)) : cross(n, vec3(0.0, 1.0, 0.0)));
        vec3 t2 = cross(n, t1);
        vec4 sum = vec4(0.0);
        [[dont_unroll]] for (uint i = 0u; i < 4u; i++) {
            vec3 o = (i < 2u ? t1 : t2) * ((i & 1u) == 0u ? size : -size);
            uint s2 = gi2_cache_find(pos + o, n, false);
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
    if (gi2_young_only() || k >= count) {
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
        uint slot = gi2_cache_find(q, n, mode == 1u);
        if (slot == ~0u) {
            continue;
        }
        cache.v[3u * slot].w = uintBitsToFloat(gi2_frame());
        vec4 v = mode == 1u ? cache.v[3u * slot + 2u] : cache.v[GI2_WORLD_AT + slot];
        if (v.w < 0.5) {
            continue;
        }
        sum += bil * v.rgb;
        wsum += bil;
        trust += bil * (mode == 1u ? 1.0 : min(v.w / 4.0, 1.0));
    }
    return vec4(wsum > 0.0 ? sum / wsum : vec3(0.0), trust);
}
