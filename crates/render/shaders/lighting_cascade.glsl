// Level 0 of the sparse cascade. Rebuilt from the G-buffer each frame.
// A ring doubles the Chebyshev distance and the cell size together, so the
// projected size at the ring start stays CASCADE_PROJ. The outer
// CASCADE_OVERLAP of a ring also inserts the next ring, and the read blends
// them. GI v1 stays removed.
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
const float CASCADE_CELL_CENTRE = 0.5;
const float CASCADE_PARENT_SCALE = 2.0;

uint lighting_cascade_base() {
    return lighting_reach_base()
        + LIGHTING_REACH_MASK_WORDS
        + LIGHTING_REACH_CHANGE_WORDS
        + (LIGHTING_CACHE_SLOTS >> 5u);
}

// Picture compose loads these two words. The hash walk stays out of that
// compile: running it there faults lavapipe. The publish pass writes them
// after the lattice fill.
uint lighting_cascade_pixel_at(uint pixel) {
    return lighting_cascade_base() + 1u + CASCADE_LEVEL0_CAP * CASCADE_LEVEL0_STRIDE + pixel * 2u;
}

vec4 lighting_cascade_published(uint pixel) {
    uint at = lighting_cascade_pixel_at(pixel);
    if (ckeys.key[at + 1u] != lighting_frame() + 1u) {
        return vec4(0.0);
    }
    return vec4(max(lighting_unpack_rgb9e5(ckeys.key[at]), vec3(0.0)), 1.0);
}

#ifndef GENOS_COMPOSE_NO_WALK

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

uint lighting_cascade_insert_ring(vec3 pos, vec3 n, uint ring) {
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
            return ~0u;
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
        return word;
    }
    return ~0u;
}

// A parent corner the leak gates refuse contributes nothing. Plane, facing,
// a stale cell, and an empty mean are the same refusals as the lattice.
vec4 lighting_cascade_gated(uint s, vec3 q, vec3 n, float size) {
    if (s == ~0u) {
        return vec4(0.0);
    }
    if (lighting_sc_stale(s)) {
        return vec4(0.0);
    }
    if (dot(cache.v[3u * s + 1u].xyz, n) <= 0.0) {
        return vec4(0.0);
    }
    if (abs(dot(cache.v[3u * s].xyz - q, n)) > LIGHTING_CELL_HALF * size) {
        return vec4(0.0);
    }
    vec4 v = cache.v[3u * s + 2u];
    if (!(v.w > LIGHTING_CACHE_HAS_W)) {
        return vec4(0.0);
    }
    return vec4(max(v.rgb, vec3(0.0)), 1.0);
}

// Sparse trilinear of the parent cells that exist. A missing corner is left
// out and the weights that remain are renormalised. No corner means black.
vec4 lighting_cascade_trilinear(vec3 pos, vec3 n, float size) {
    vec3 g = floor(pos / size);
    vec3 f = pos / size - g;
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    [[dont_unroll]] for (uint i = 0u; i < 8u; i++) {
        vec3 corner = g + vec3(float(i & 1u), float((i >> 1u) & 1u), float(i >> 2u));
        vec3 q = (corner + vec3(CASCADE_CELL_CENTRE)) * size;
        float bx = (i & 1u) == 0u ? (1.0 - f.x) : f.x;
        float by = ((i >> 1u) & 1u) == 0u ? (1.0 - f.y) : f.y;
        float bz = (i >> 2u) == 0u ? (1.0 - f.z) : f.z;
        float bil = bx * by * bz;
        if (!(bil > 0.0)) {
            continue;
        }
        uint s = lighting_cache_find_sized(q, n, false, size, q);
        vec4 tap = lighting_cascade_gated(s, pos, n, size);
        if (!(tap.w > LIGHTING_CACHE_HAS_W)) {
            continue;
        }
        sum += bil * tap.rgb;
        wsum += bil;
    }
    if (!(wsum > 0.0)) {
        return vec4(0.0);
    }
    return vec4(sum / wsum, 1.0);
}

// L(a,c) = L(a,b) + beta(a,b) * L(b,c). Beta is 0 when the nearer sample
// exists, so that sample occludes the parent. A missing nearer sample shows
// the parent in full. Black only when neither has data.
vec4 lighting_cascade_merge(vec3 pos, vec3 n) {
    vec4 near = lighting_sh_compose_shown(pos, n);
    vec4 far = lighting_cascade_trilinear(pos, n, LIGHTING_SC_HIT * CASCADE_PARENT_SCALE);
    float beta = near.w > LIGHTING_CACHE_HAS_W ? 0.0 : 1.0;
    vec3 near_rgb = near.w > LIGHTING_CACHE_HAS_W ? max(near.rgb, vec3(0.0)) : vec3(0.0);
    vec3 far_rgb = far.w > LIGHTING_CACHE_HAS_W ? max(far.rgb, vec3(0.0)) : vec3(0.0);
    vec3 rgb = near_rgb + beta * far_rgb;
    float w = 0.0;
    if (near.w > LIGHTING_CACHE_HAS_W || far.w > LIGHTING_CACHE_HAS_W) {
        w = 1.0;
    }
    return vec4(rgb, w);
}

void lighting_cascade_store(uint word, uint packed, uint weight) {
    if (word == ~0u) {
        return;
    }
    ckeys.key[word + 10u] = packed;
    ckeys.key[word + 11u] = weight;
}

// Finest cell keeps the whole trace. A coarser cell (size above the 25 cm
// bounce cell) owns one shell, t from 2^level to 2^(level+1). The last shell
// runs out to the far trace. Contact stays at the ray epsilon until the merge
// adds those shells: clipping the 25 cm cell would drop the hall.
const float CASCADE_TRACE_NEAR = 1.0e-4;
const float CASCADE_TRACE_FAR = 200.0;
const float CASCADE_INTERVAL_MAX = 6.0;
const float CASCADE_INTERVAL_ROUND = 0.5;

void lighting_cascade_interval(float size, out float t0, out float t1) {
    t0 = CASCADE_TRACE_NEAR;
    t1 = CASCADE_TRACE_FAR;
    // Only a cell at least twice the 25 cm bounce cell owns a shell. A
    // smaller cell is still what the picture's gather reads, and clipping
    // it drops the hall.
    if (!(size >= LIGHTING_SC_HIT + LIGHTING_SC_HIT)) {
        return;
    }
    float ratio = max(size, LIGHTING_SC_HIT) / LIGHTING_SC_HIT;
    float level = floor(log2(max(ratio, 1.0)) + CASCADE_INTERVAL_ROUND);
    if (level < 1.0) {
        return;
    }
    level = min(level, CASCADE_INTERVAL_MAX);
    t0 = exp2(level);
    t1 = level >= CASCADE_INTERVAL_MAX ? CASCADE_TRACE_FAR : exp2(level + 1.0);
}

uint lighting_cascade_ray_slot(uint ray) {
    if (ray < lighting_probe_rays()) {
        return ~0u;
    }
    uint r = ray - lighting_probe_rays();
    if (lighting_ray_fill_on()) {
        if (r >= lighting_fill_traced()) {
            return ~0u;
        }
        uint slot = ckeys.key[lighting_fill_pick_at() + r];
        return slot < LIGHTING_CACHE_SLOTS ? slot : ~0u;
    }
    uint entry = r / LIGHTING_CACHE_RAYS;
    if (lighting_world_mode() == 3u
        && entry >= lighting_grad_first()
        && entry < LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES) {
        uint ci = r - lighting_grad_first() * LIGHTING_CACHE_RAYS;
        if (ci >= lighting_grad_count()) {
            return ~0u;
        }
        return ckeys.key[LIGHTING_GRAD_LIST_AT + ci];
    }
    return lighting_cache_batch_slot(entry);
}

void lighting_cascade_ray_range(uint ray, out float t0, out float t1) {
    t0 = CASCADE_TRACE_NEAR;
    t1 = CASCADE_TRACE_FAR;
    uint slot = lighting_cascade_ray_slot(ray);
    if (slot == ~0u || slot >= LIGHTING_CACHE_SLOTS) {
        return;
    }
    lighting_cascade_interval(cache.v[3u * slot + 1u].w, t0, t1);
}

uint lighting_cascade_find(ivec3 cell, uint ring) {
    uint key = hash_cell3(cell, CASCADE_LEVEL0_SALT ^ ring);
    if (key == 0u) {
        key = 1u;
    }
    uint frame = lighting_frame() + 1u;
    uint mask = CASCADE_LEVEL0_CAP - 1u;
    uint home = key & mask;
    uint base = lighting_cascade_base() + 1u;
    [[dont_unroll]] for (uint i = 0u; i < CASCADE_LEVEL0_PROBE; i++) {
        uint word = base + ((home + i) & mask) * CASCADE_LEVEL0_STRIDE;
        if (ckeys.key[word] != key || ckeys.key[word + 1u] != frame) {
            continue;
        }
        if (ckeys.key[word + 2u] != uint(cell.x) || ckeys.key[word + 3u] != uint(cell.y)) {
            continue;
        }
        if (ckeys.key[word + 4u] != uint(cell.z) || ckeys.key[word + 5u] != ring) {
            continue;
        }
        return word;
    }
    return ~0u;
}

vec4 lighting_cascade_record(uint word, vec3 pos, vec3 n) {
    if (word == ~0u) {
        return vec4(0.0);
    }
    vec3 stored = vec3(
        uintBitsToFloat(ckeys.key[word + 6u]),
        uintBitsToFloat(ckeys.key[word + 7u]),
        uintBitsToFloat(ckeys.key[word + 8u]));
    vec3 sn = lighting_unpack_normal(ckeys.key[word + 9u]);
    float size = lighting_cascade_cell_size(ckeys.key[word + 5u]);
    if (dot(sn, n) <= 0.0) {
        return vec4(0.0);
    }
    if (abs(dot(stored - pos, n)) > LIGHTING_CELL_HALF * size) {
        return vec4(0.0);
    }
    float weight = uintBitsToFloat(ckeys.key[word + 11u]);
    if (!(weight > LIGHTING_CACHE_HAS_W)) {
        return vec4(0.0);
    }
    return vec4(max(lighting_unpack_rgb9e5(ckeys.key[word + 10u]), vec3(0.0)), 1.0);
}

vec4 lighting_cascade_at(vec3 pos, vec3 n, uint ring) {
    float size = lighting_cascade_cell_size(ring);
    return lighting_cascade_record(lighting_cascade_find(ivec3(floor(pos / size)), ring), pos, n);
}

// This ring, blended with the next ring across the outer tenth. A refused
// sample is skipped. The next ring out shows through, so one empty cell is
// not black while a coarser ring has data.
vec4 lighting_cascade_picture(vec3 pos, vec3 n) {
    float cheb = lighting_cascade_cheb(pos);
    uint ring = lighting_cascade_ring(cheb);
    float blend = lighting_cascade_blend(cheb);
    vec4 near = lighting_cascade_at(pos, n, ring);
    vec4 far = vec4(0.0);
    if (ring < CASCADE_RING_MAX) {
        far = lighting_cascade_at(pos, n, ring + 1u);
    }
    bool near_ok = near.w > LIGHTING_CACHE_HAS_W;
    bool far_ok = far.w > LIGHTING_CACHE_HAS_W;
    if (near_ok && far_ok && blend > 0.0) {
        float near_w = 1.0 - blend;
        float w = near_w + blend;
        return vec4((near.rgb * near_w + far.rgb * blend) / w, 1.0);
    }
    if (near_ok) {
        return near;
    }
    if (far_ok) {
        return far;
    }
    [[dont_unroll]] for (uint up = ring + 2u; up <= CASCADE_RING_MAX; up++) {
        vec4 coarser = lighting_cascade_at(pos, n, up);
        if (coarser.w > LIGHTING_CACHE_HAS_W) {
            return coarser;
        }
    }
    return vec4(0.0);
}

void lighting_cascade_publish(uint pixel, vec3 pos, vec3 n) {
    // This pixel's merge, after the lattice fill. Writing the shared cell
    // from every pixel raced and painted it black on a camera turn. The
    // picture loads this pixel's own word.
    vec4 shown = lighting_cascade_merge(pos, n);
    uint at = lighting_cascade_pixel_at(pixel);
    ckeys.key[at] = lighting_pack_rgb9e5(max(shown.rgb, vec3(0.0)));
    ckeys.key[at + 1u] = shown.w > LIGHTING_CACHE_HAS_W ? lighting_frame() + 1u : 0u;
}

void lighting_cascade_insert(vec3 pos, vec3 n) {
    float cheb = lighting_cascade_cheb(pos);
    uint ring = lighting_cascade_ring(cheb);
    uint word = lighting_cascade_insert_ring(pos, n, ring);
    uint word_next = ~0u;
    float blend = lighting_cascade_blend(cheb);
    if (blend > 0.0 && ring < CASCADE_RING_MAX) {
        word_next = lighting_cascade_insert_ring(pos, n, ring + 1u);
    }
    if (word == ~0u && word_next == ~0u) {
        return;
    }
    // Radiance is stored by the merge. The direct pass only claims the cell
    // here: evaluating the gather while claiming changed the settled picture.
    lighting_cascade_store(word, 0u, 0u);
    lighting_cascade_store(word_next, 0u, 0u);
}
#endif
