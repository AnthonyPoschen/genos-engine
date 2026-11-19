// Level 0 of the sparse cascade. Rebuilt from the G-buffer each frame.
// A ring doubles the Chebyshev distance and the cell size together, so the
// projected size at the ring start stays CASCADE_PROJ. The outer
// CASCADE_OVERLAP of a ring also inserts the next ring, and the read blends
// them. GI v1's radiance cascades stay removed.
//
// The record sits after the reach words. Word 0 is unused. Each record is
// CASCADE_LEVEL0_STRIDE words: key, stamp, cell x y z, ring, position xyz,
// packed normal, radiance, weight. The host sizes the same tail.

const uint CASCADE_LEVEL0_CAP = 16384u;
const uint CASCADE_LEVEL0_STRIDE = 12u;
const uint CASCADE_LEVEL0_PROBE = 16u;
const uint CASCADE_LEVEL0_SALT = 0xC35Cu;
const float CASCADE_PROJ = 0.05;
const float CASCADE_RING_NEAR = 1.0;
const uint CASCADE_RING_MAX = 6u;
const float CASCADE_OVERLAP = 0.1;

uint lighting_cascade_base() {
    return lighting_reach_base()
        + LIGHTING_REACH_MASK_WORDS
        + LIGHTING_REACH_CHANGE_WORDS
        + (LIGHTING_CACHE_SLOTS >> 5u);
}

float lighting_cascade_cheb(vec3 pos) {
    vec3 rel = abs(pos - scene.eye.xyz);
    return max(rel.x, max(rel.y, rel.z));
}

uint lighting_cascade_ring(float cheb) {
    float level = floor(log2(max(cheb, CASCADE_RING_NEAR) / CASCADE_RING_NEAR));
    return uint(clamp(level, 0.0, float(CASCADE_RING_MAX)));
}

float lighting_cascade_cell_size(uint ring) {
    return CASCADE_PROJ * CASCADE_RING_NEAR * exp2(float(min(ring, CASCADE_RING_MAX)));
}

float lighting_cascade_blend(float cheb) {
    float level = log2(max(cheb, CASCADE_RING_NEAR) / CASCADE_RING_NEAR);
    if (level >= float(CASCADE_RING_MAX)) {
        return 0.0;
    }
    float frac = level - floor(level);
    float start = 1.0 - CASCADE_OVERLAP;
    if (frac <= start) {
        return 0.0;
    }
    return (frac - start) / CASCADE_OVERLAP;
}

void lighting_cascade_insert_ring(vec3 pos, vec3 n, uint ring) {
    float size = lighting_cascade_cell_size(ring);
    ivec3 cell = ivec3(floor(pos / size));
    uint key = hash_cell3(cell, CASCADE_LEVEL0_SALT ^ ring);
    if (key == 0u) {
        key = 1u;
    }
    uint frame = lighting_frame() + 1u;
    uint mask = CASCADE_LEVEL0_CAP - 1u;
    uint home = key & mask;
    uint base = lighting_cascade_base() + 1u;
    [[dont_unroll]] for (uint i = 0u; i < CASCADE_LEVEL0_PROBE; i++) {
        uint slot = (home + i) & mask;
        uint word = base + slot * CASCADE_LEVEL0_STRIDE;
        uint cur = ckeys.key[word];
        uint stamp = ckeys.key[word + 1u];
        bool live = stamp == frame;
        if (cur == key && live
            && ckeys.key[word + 2u] == uint(cell.x)
            && ckeys.key[word + 3u] == uint(cell.y)
            && ckeys.key[word + 4u] == uint(cell.z)
            && ckeys.key[word + 5u] == ring) {
            return;
        }
        if (live && cur != 0u) {
            continue;
        }
        uint prev = atomicCompSwap(ckeys.key[word], cur, key);
        if (prev != cur && !(prev == key && ckeys.key[word + 1u] == frame)) {
            continue;
        }
        ckeys.key[word + 1u] = frame;
        ckeys.key[word + 2u] = uint(cell.x);
        ckeys.key[word + 3u] = uint(cell.y);
        ckeys.key[word + 4u] = uint(cell.z);
        ckeys.key[word + 5u] = ring;
        ckeys.key[word + 6u] = floatBitsToUint(pos.x);
        ckeys.key[word + 7u] = floatBitsToUint(pos.y);
        ckeys.key[word + 8u] = floatBitsToUint(pos.z);
        ckeys.key[word + 9u] = lighting_pack_normal(n);
        ckeys.key[word + 10u] = 0u;
        ckeys.key[word + 11u] = 0u;
        return;
    }
}

void lighting_cascade_insert(vec3 pos, vec3 n) {
    float cheb = lighting_cascade_cheb(pos);
    uint ring = lighting_cascade_ring(cheb);
    lighting_cascade_insert_ring(pos, n, ring);
    float blend = lighting_cascade_blend(cheb);
    if (blend > 0.0 && ring < CASCADE_RING_MAX) {
        lighting_cascade_insert_ring(pos, n, ring + 1u);
    }
}
