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

// One world cell. The lattice table is 16384 records with a 16-probe window.
// A room evicts the farther surfaces, so a lattice miss is not an empty cell.
// This read is the cell the hash still holds. Claim stays off: publish does
// not allocate a coarser cell the fill never asked for.
vec4 lighting_cascade_voxel(vec3 pos, vec3 n, float size) {
    uint s = lighting_cache_find_sized(pos, n, false, size, pos);
    return lighting_cascade_gated(s, pos, n, size);
}

// Fine cells already traced on this surface, one step out. A new voxel has no
// stored parent, and the coarser slot is often empty too. The step stays on
// the tangent plane, so a probe does not leave the wall. Each coarser size
// takes a longer step. The plane and facing gates stay, so the far side of a
// wall does not join the sum.
vec4 lighting_cascade_children(vec3 pos, vec3 n, float size) {
    uint ax = lighting_axis(n) >> 1u;
    uint a1 = (ax + 1u) % 3u;
    uint a2 = (ax + 2u) % 3u;
    float step = size / CASCADE_PARENT_SCALE;
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    [[dont_unroll]] for (uint i = 0u; i < 9u; i++) {
        int s1 = int(i % 3u) - 1;
        int s2 = int(i / 3u) - 1;
        if (s1 == 0 && s2 == 0) {
            continue;
        }
        vec3 q = pos;
        q[a1] += float(s1) * step;
        q[a2] += float(s2) * step;
        vec4 tap = lighting_cascade_voxel(q, n, LIGHTING_SC_HIT);
        if (!(tap.w > LIGHTING_CACHE_HAS_W)) {
            continue;
        }
        sum += tap.rgb;
        wsum += 1.0;
    }
    if (!(wsum > 0.0)) {
        return vec4(0.0);
    }
    return vec4(sum / wsum, 1.0);
}

// A cell read that keeps the facing gate and drops the plane gate. The plane
// gate is right for a blend. On a miss it also rejects the lit part of this
// surface when the new pixel's position sits more than half a cell off the
// stored hit. The facing gate still refuses the other side of a wall.
vec4 lighting_cascade_facing(vec3 pos, vec3 n, float size) {
    uint s = lighting_cache_find_sized(pos, n, false, size, pos);
    if (s == ~0u || lighting_sc_stale(s)) {
        return vec4(0.0);
    }
    if (dot(cache.v[3u * s + 1u].xyz, n) <= 0.0) {
        return vec4(0.0);
    }
    vec4 v = cache.v[3u * s + 2u];
    if (!(v.w > LIGHTING_CACHE_HAS_W)) {
        return vec4(0.0);
    }
    return vec4(max(v.rgb, vec3(0.0)), 1.0);
}

vec4 lighting_cascade_ancestor(vec3 pos, vec3 n);

// Pixels toward the image centre. A camera turn inserts a strip of voxels
// with no ray yet, on the edge, wider than the lattice reach. The first
// inward pixel whose cell, lattice, or coarser sample already has data is
// that ancestor. Past eight metres it is a different surface.
vec4 lighting_cascade_inward(uvec2 p, vec3 pos, vec3 n) {
    ivec2 delta = ivec2(pc.dims.xy) / 2 - ivec2(p);
    int sx = delta.x > 0 ? 1 : (delta.x < 0 ? -1 : 0);
    int sy = delta.y > 0 ? 1 : (delta.y < 0 ? -1 : 0);
    if (sx == 0 && sy == 0) {
        return vec4(0.0);
    }
    float limit = LIGHTING_SC_HIT
        * CASCADE_PARENT_SCALE * CASCADE_PARENT_SCALE
        * CASCADE_PARENT_SCALE * CASCADE_PARENT_SCALE
        * CASCADE_PARENT_SCALE;
    [[dont_unroll]] for (uint k = 1u; k <= 16u; k++) {
        ivec2 q = ivec2(p) + ivec2(sx, sy) * int(k);
        if (q.x < 0 || q.y < 0 || uint(q.x) >= pc.dims.x || uint(q.y) >= pc.dims.y) {
            break;
        }
        vec3 qpos;
        vec3 qn;
        vec3 albedo;
        bool two;
        if (!lighting_surface(uvec2(q), qpos, qn, albedo, two)) {
            continue;
        }
        if (two && dot(qn, scene.eye.xyz - qpos) < 0.0) {
            qn = -qn;
        }
        // The new strip's normal is not the wall it just covered. Keep the
        // sample the inward pixel already resolved. The distance limit is
        // what refuses a surface in another room.
        if (distance(qpos, pos) > limit) {
            continue;
        }
        vec4 tap = lighting_sh_compose_shown(qpos, qn);
        if (!(tap.w > LIGHTING_CACHE_HAS_W)) {
            tap = lighting_cascade_voxel(qpos, qn, LIGHTING_SC_HIT);
        }
        if (!(tap.w > LIGHTING_CACHE_HAS_W)) {
            tap = lighting_cascade_facing(qpos, qn, LIGHTING_SC_HIT);
        }
        if (!(tap.w > LIGHTING_CACHE_HAS_W)) {
            tap = lighting_cascade_ancestor(qpos, qn);
        }
        if (tap.w > LIGHTING_CACHE_HAS_W) {
            return tap;
        }
    }
    return vec4(0.0);
}

// The next coarser size that still has a sample the gates accept. A missing
// stored cell uses the fine cells already inside that footprint. Each step
// doubles. Black only when every size missed.
vec4 lighting_cascade_ancestor(vec3 pos, vec3 n) {
    float size = LIGHTING_SC_HIT;
    [[dont_unroll]] for (uint level = 1u; level <= CASCADE_RING_MAX; level++) {
        size *= CASCADE_PARENT_SCALE;
        vec4 tap = lighting_cascade_voxel(pos, n, size);
        if (!(tap.w > LIGHTING_CACHE_HAS_W)) {
            tap = lighting_cascade_children(pos, n, size);
        }
        if (tap.w > LIGHTING_CACHE_HAS_W) {
            return tap;
        }
    }
    return vec4(0.0);
}

// Visible surfaces whose published sample is empty. Stats word 35 (diag[11]).
// diag[46] is the adopt reset and must not be reused.
const uint LIGHTING_DIAG_CASCADE_GAP = 11u;

// L(a,c) = L(a,b) + beta(a,b) * L(b,c). Beta is 0 when the nearer sample
// exists, so that sample occludes the parent. The lattice sample is that
// nearer sample when it has data. A lattice miss reads the 25 cm cell, then
// each coarser size. Black only when none of those has data.
vec4 lighting_cascade_merge(uvec2 p, vec3 pos, vec3 n) {
    vec4 near = lighting_sh_compose_shown(pos, n);
    vec4 voxel = vec4(0.0);
    vec4 far = vec4(0.0);
    if (!(near.w > LIGHTING_CACHE_HAS_W)) {
        voxel = lighting_cascade_voxel(pos, n, LIGHTING_SC_HIT);
        near = voxel;
    }
    if (!(near.w > LIGHTING_CACHE_HAS_W)) {
        far = lighting_cascade_ancestor(pos, n);
    }
    if (!(near.w > LIGHTING_CACHE_HAS_W) && !(far.w > LIGHTING_CACHE_HAS_W)) {
        far = lighting_cascade_inward(p, pos, n);
    }
    float beta = near.w > LIGHTING_CACHE_HAS_W ? 0.0 : 1.0;
    vec3 near_rgb = near.w > LIGHTING_CACHE_HAS_W ? max(near.rgb, vec3(0.0)) : vec3(0.0);
    vec3 far_rgb = far.w > LIGHTING_CACHE_HAS_W ? max(far.rgb, vec3(0.0)) : vec3(0.0);
    vec3 rgb = near_rgb + beta * far_rgb;
    float w = 0.0;
    if (near.w > LIGHTING_CACHE_HAS_W || far.w > LIGHTING_CACHE_HAS_W) {
        w = 1.0;
    }
    // A visible surface with no published sample. A real zero has weight and
    // is not counted. The moving-camera test requires this to stay 0: a new
    // voxel reads its ancestor in the same frame.
    if (!(w > LIGHTING_CACHE_HAS_W)) {
        atomicAdd(gstats.diag[LIGHTING_DIAG_CASCADE_GAP], 1u);
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

void lighting_cascade_publish(uint pixel, uvec2 p, vec3 pos, vec3 n) {
    // This pixel's merge, after the lattice fill. Writing the shared cell
    // from every pixel raced and painted it black on a camera turn. The
    // picture loads this pixel's own word.
    vec4 shown = lighting_cascade_merge(p, pos, n);
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
