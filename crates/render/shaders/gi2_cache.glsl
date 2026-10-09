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
// Keys (binding 9): one uint per slot, 0 when free; then the count of live slots
// and their list, rebuilt each frame by gi2_compact.comp, so a frame relights live
// patches only (one more bounce per frame while they fit in a batch).
// Patches (binding 10): 3 vec4 per slot: position (w: frame last read), normal
// (w: cell size), irradiance (w: 1 once lit).

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
    uint pad;
} gstats;


uint gi2_frame() {
    return pc.params.w;
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

// Key of the patch holding pos facing n; never 0.
uint gi2_cache_key(vec3 pos, vec3 n, out float size) {
    size = gi2_cell_size(pos);
    ivec3 c = ivec3(floor(pos / size));
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
            return slot;
        }
        if (prev == key) {
            return slot;
        }
    }
    return ~0u;
}

// Irradiance already bounced onto the surface at pos facing n (0 for a new patch).
vec3 gi2_cache_irradiance(vec3 pos, vec3 n) {
    uint slot = gi2_cache_find(pos, n, true);
    if (slot == ~0u) {
        return vec3(0.0);
    }
    cache.v[3u * slot].w = uintBitsToFloat(gi2_frame());
    return cache.v[3u * slot + 2u].rgb;
}

uint gi2_live_count() {
    return min(ckeys.key[GI2_CACHE_SLOTS], GI2_CACHE_SLOTS);
}

// Slot of this frame's batch entry j, or ~0u when the batch has no entry j.
uint gi2_cache_batch_slot(uint j) {
    uint count = gi2_live_count();
    if (j >= count) {
        return ~0u;
    }
    uint start = (gi2_frame() * GI2_CACHE_BATCH) % count;
    return ckeys.key[GI2_CACHE_SLOTS + 1u + (start + j) % count];
}
