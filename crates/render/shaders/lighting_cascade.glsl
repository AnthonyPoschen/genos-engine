// Level 0 of the sparse cascade. Rebuilt from the G-buffer each frame.
// Size is picked on the shared power-of-two world grid (1/16 m base) by
// screen footprint, not Chebyshev rings. A half-cell inset along the
// dominant axis stops grid-plane flips. Two tables (this frame and last)
// keep the previous leaves for the level-0 EMA and quarter-level hysteresis.
// Every live voxel traces 32 rays each frame. Publish blends only upward:
// this footprint level with the next coarser one across the outer tenth.
// There is no same-level neighbour gather. GI v1 stays removed.
//
// The record sits after the reach words. Word 0 is unused. Two tables of
// CASCADE_LEVEL0_CAP records, then four words a pixel: published radiance
// and stamp (compose), then the insert slot and the blend slot. After the
// pixels: inserted and traced counts, then one four-word hit record per
// voxel slot and ray. Each record is CASCADE_LEVEL0_STRIDE words: key,
// stamp, cell x y z, level packed with axis, position xyz, packed normal,
// radiance, weight. The host sizes the same tail.

const uint CASCADE_LEVEL0_CAP = 16384u;
const uint CASCADE_LEVEL0_STRIDE = 12u;
const uint CASCADE_LEVEL0_PROBE = 16u;
const uint CASCADE_LEVEL0_SALT = 0xC35Cu;
const float CASCADE_PROJ = 0.05;
const float CASCADE_RING_NEAR = 1.0;
const uint CASCADE_RING_MAX = 6u;
const float CASCADE_OVERLAP = 0.1;
const float CASCADE_PARENT_SCALE = 2.0;
// Voxel debug view (lighting_cascade_debug_voxels). Floor and mix keep
// hashed colours visible; ring dim and line mark distance rings.
const float CASCADE_VOXEL_VIEW_FLOOR = 0.12;
const float CASCADE_VOXEL_VIEW_MIX = 0.88;
const float CASCADE_VOXEL_VIEW_RING_DIM = 0.08;
const float CASCADE_VOXEL_VIEW_RING_LINE = 0.03;
const float CASCADE_VOXEL_VIEW_LINE_MIX = 0.65;
// Footprint size on the shared world grid. 2^n / 16 m. K is the angular
// size of a box (about 22 x 40 px at 1440p). The 1/16 graze floor caps
// the boost at two grid levels. Hysteresis is a quarter level either
// side of [1, 2). The outer tenth of a level, log scale, also inserts
// the coarser cell.
const float CASCADE_GRID_BASE = 0.0625;
const uint CASCADE_L0_MAX = 9u;
const float CASCADE_FOOTPRINT_K = 0.03125;
const float CASCADE_GRAZE_MIN = 0.0625;
const float CASCADE_HYST_LO = 0.84089641525;
const float CASCADE_HYST_HI = 2.37841423001;
const float CASCADE_FOOTPRINT_BLEND = 0.9;
const uint CASCADE_TABLES = 2u;
const uint CASCADE_PIXEL_WORDS = 4u;
const uint CASCADE_AXIS_SHIFT = 8u;
const uint CASCADE_LEVEL_MASK = 255u;
const float CASCADE_EPS = 1.0e-6;
// 32 stratified hemisphere rays per live voxel, every frame. Hits are
// stored after the count words so the shade pass can run without a
// second trace site in this compile.
const uint CASCADE_L0_RAYS = 32u;
const float CASCADE_GOLDEN = 2.399963229728653;
const uint CASCADE_L0_HIT_WORDS = 4u;
const vec3 CASCADE_LUMA = vec3(0.2126, 0.7152, 0.0722);
// History window of every cascade value, in frames (cascade.rs
// EMA_WINDOW_FRAMES). A level-0 voxel updates each frame and caps its stored
// count at this. An upper cell updates every refresh period and caps at the
// window over the period, so cells, voxels and bounce share one timescale.
const float CASCADE_EMA_WINDOW = 4.0;
const float CASCADE_EMA_N_MAX = CASCADE_EMA_WINDOW;

float lighting_cascade_ema_n_max(uint period) {
    return max(CASCADE_EMA_WINDOW / float(max(period, 1u)), 1.0);
}

// Mix weight and the stored sample count after this update.
// A gap inside two standard errors of this frame's estimate is noise:
// N grows and the blend is 1/N. A larger gap down-weights history by
// the relative luma change and shrinks N by the leftover history so
// later frames keep catching up instead of stalling inside the
// new-sample standard error. Non-finite values take weight 1 and N = 1.
float lighting_cascade_ema_step(vec3 cur, vec3 prior, float se, float prior_n, float n_max, out float new_n) {
    if (isnan(cur.x) || isinf(cur.x) || isnan(cur.y) || isinf(cur.y)
        || isnan(cur.z) || isinf(cur.z) || isnan(prior.x) || isinf(prior.x)
        || isnan(prior.y) || isinf(prior.y) || isnan(prior.z) || isinf(prior.z)) {
        new_n = 1.0;
        return 1.0;
    }
    float lc = dot(cur, CASCADE_LUMA);
    float lp = dot(prior, CASCADE_LUMA);
    float gap = abs(lc - lp);
    float noise = max(se, CASCADE_EPS);
    float cap = max(n_max, 1.0);
    float n0 = min(max(prior_n, 1.0), cap);
    float agree = min(n0 + 1.0, cap);
    if (gap <= 2.0 * noise) {
        new_n = agree;
        return 1.0 / new_n;
    }
    // A real change never takes less of the new sample than agreement
    // would: at a cap of 1 (an upper cell refreshed every 4 frames) the
    // value is replaced either way.
    float den = max(max(abs(lc), abs(lp)), CASCADE_EPS);
    float nw = max(clamp(gap / den, 0.0, 1.0), 1.0 / agree);
    new_n = min(max(n0 * (1.0 - nw) + 1.0, 1.0), cap);
    return nw;
}

uint lighting_cascade_base() {
    return lighting_reach_base()
        + LIGHTING_REACH_MASK_WORDS
        + LIGHTING_REACH_CHANGE_WORDS
        + (LIGHTING_CACHE_SLOTS >> 5u);
}

// Picture compose loads the first two words. The hash walk stays out of
// that compile: running it there faults lavapipe. Insert writes the slot
// words. Publish reads those slots and then writes the radiance stamp.
uint lighting_cascade_pixel_at(uint pixel) {
    return lighting_cascade_base() + 1u
        + CASCADE_TABLES * CASCADE_LEVEL0_CAP * CASCADE_LEVEL0_STRIDE
        + pixel * CASCADE_PIXEL_WORDS;
}

uint lighting_cascade_table_base(uint parity) {
    return lighting_cascade_base() + 1u
        + (parity & 1u) * CASCADE_LEVEL0_CAP * CASCADE_LEVEL0_STRIDE;
}

// Inserted and traced counters, then one hit record per (slot, ray).
uint lighting_cascade_counts_at() {
    return lighting_cascade_base() + 1u
        + CASCADE_TABLES * CASCADE_LEVEL0_CAP * CASCADE_LEVEL0_STRIDE
        + pc.dims.x * pc.dims.y * CASCADE_PIXEL_WORDS;
}

// Level-0 ray k of CASCADE_L0_RAYS about n: one stratum of cos(theta) each,
// so every equal-measure band of the hemisphere gets the same share of rays
// (a sqrt(N) x sqrt(N) grid with N = 32 leaves 6x6 cells with 2 rays short in
// the top band and oversamples grazing rays, biasing 2pi/N sum L cos low by
// ~4%). The turn steps by the golden angle per ray and per frame.
vec3 lighting_cascade_l0_dir(uint key, vec3 n, uint k) {
    float j1 = lighting_unit(key ^ (k * 0x9e3779b9u + 1u));
    float j2 = lighting_unit(key ^ (k * 0x85ebca6bu + 2u));
    float u = max((float(k) + j1) / float(CASCADE_L0_RAYS), CASCADE_EPS);
    float turn = fract(float(k) * 0.6180339887 + j2) * LIGHTING_TAU
        + float(lighting_frame()) * CASCADE_GOLDEN;
    return lighting_hemi_dir(key, n, u, turn / LIGHTING_TAU);
}

uint lighting_cascade_hit_at(uint slot, uint k) {
    return lighting_cascade_counts_at() + 2u
        + (slot * CASCADE_L0_RAYS + k) * CASCADE_L0_HIT_WORDS;
}

// Upper refresh list, after the level-0 hit records. A claimed bit per
// world-cache slot, then a header (period, flags, per-band counts) and
// three due lists (fine / mid / coarse), then one hit record per due
// slot and ray in the band being shaded.
const uint CASCADE_CLAIMED_WORDS = 32768u;
const uint CASCADE_UPPER_CAP = 16384u;
const uint CASCADE_UPPER_RAYS_MAX = 64u;
const float CASCADE_EXTENT_FACTOR = 16.0;

uint lighting_cascade_claimed_at() {
    return lighting_cascade_counts_at() + 2u
        + CASCADE_LEVEL0_CAP * CASCADE_L0_RAYS * CASCADE_L0_HIT_WORDS;
}

uint lighting_cascade_upper_head_at() {
    return lighting_cascade_claimed_at() + CASCADE_CLAIMED_WORDS;
}

// Word 0: period (low 8 bits), current dispatch band (bits 16-17).
// Words 1..3: due counts for size bands 0 (S<=0.5), 1 (S<=4), 2 (S>4),
// used only to merge coarse to fine this frame. Every cell shares one P.
uint lighting_cascade_upper_list_at(uint band) {
    return lighting_cascade_upper_head_at() + 4u + band * CASCADE_UPPER_CAP;
}

uint lighting_cascade_upper_hit_at(uint due, uint k) {
    return lighting_cascade_upper_head_at() + 4u + 3u * CASCADE_UPPER_CAP
        + (due * CASCADE_UPPER_RAYS_MAX + k) * CASCADE_L0_HIT_WORDS;
}

uint lighting_cascade_refresh_period(uint setting) {
    if (setting == 1u || setting == 2u || setting == 4u || setting == 10u) {
        return setting;
    }
    return 4u;
}

uint lighting_cascade_refresh_rays(float size) {
    if (size <= 0.5) {
        return 16u;
    }
    if (size <= 4.0) {
        return 32u;
    }
    return 64u;
}

uint lighting_cascade_upper_band(float size) {
    if (size <= 0.5) {
        return 0u;
    }
    if (size <= 4.0) {
        return 1u;
    }
    return 2u;
}

bool lighting_cascade_in_extent(float cheb, float size) {
    return cheb <= CASCADE_EXTENT_FACTOR * max(size, 0.0);
}

bool lighting_cascade_refresh_due(uint key, uint frame, uint period) {
    if (period <= 1u) {
        return true;
    }
    return (key + frame) % period == 0u;
}

// Upper-cell interval radiance: 16 direction bins over the hemisphere of
// the cell's axis, 4 rows of equal cos(theta) by 4 columns of turn, so
// every bin is the same solid angle (pi/8 sr). Ray k of a refresh lies in
// bin k % 16, jittered inside it, so every bin gets a sample each refresh.
//
// A bin holds the mean radiance of the rays in it whose path stayed clear
// up to the cell's interval start 4S: an interval hit, or the parent's
// bins on a miss. A ray blocked before 4S is left out of the bins. A child
// reads a bin only along a direction its own rays found clear to 8s = 4S,
// so a bin that averaged in the blocked directions would hand the child
// light that is not there. Blocked rays still count in the cell's full
// irradiance (cache.v[3s+2]), which ray hits read for the next bounce.
//
// Bins are rgb9e5, four per vec4, in regions 27..30 of the cache.
const uint CASCADE_BINS = 16u;
const float CASCADE_BIN_SIDE = 4.0;
const uint CASCADE_BINS_AT = 27u * LIGHTING_CACHE_SLOTS;

void lighting_cascade_bin_frame(vec3 n, out vec3 tx, out vec3 ty) {
    vec3 helper = abs(n.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    tx = normalize(cross(helper, n));
    ty = cross(n, tx);
}

// Direction of ray k of an upper cell's refresh: bin k % 16, jittered per
// key, ray and frame.
vec3 lighting_cascade_bin_dir(uint key, vec3 n, uint k, uint frame) {
    uint b = k % CASCADE_BINS;
    uint h = lighting_hash(key ^ (k * 0x9e3779b9u) ^ (frame * 0x85ebca6bu));
    float j1 = lighting_unit(h);
    float j2 = lighting_unit(lighting_hash(h ^ 0x27d4eb2fu));
    float u = (float(b / 4u) + j1) / CASCADE_BIN_SIDE;
    float phi = LIGHTING_TAU * (float(b % 4u) + j2) / CASCADE_BIN_SIDE;
    float r = sqrt(max(1.0 - u * u, 0.0));
    vec3 tx;
    vec3 ty;
    lighting_cascade_bin_frame(n, tx, ty);
    return normalize(tx * (r * cos(phi)) + ty * (r * sin(phi)) + n * u);
}

// Four bins around `dir` and their bilinear weights (rows clamped, the
// turn wraps).
void lighting_cascade_bin_taps(vec3 dir, vec3 n, out uvec4 idx, out vec4 w) {
    vec3 tx;
    vec3 ty;
    lighting_cascade_bin_frame(n, tx, ty);
    float u = clamp(dot(dir, n), 0.0, 1.0);
    float phi = atan(dot(dir, ty), dot(dir, tx));
    if (phi < 0.0) {
        phi += LIGHTING_TAU;
    }
    float fu = clamp(u * CASCADE_BIN_SIDE - 0.5, 0.0, CASCADE_BIN_SIDE - 1.0);
    float fp = phi / LIGHTING_TAU * CASCADE_BIN_SIDE - 0.5;
    float r0 = min(floor(fu), CASCADE_BIN_SIDE - 2.0);
    float wu = fu - r0;
    float c0 = floor(fp);
    float wp = fp - c0;
    uint ra = uint(r0);
    uint ca = uint(mod(c0, CASCADE_BIN_SIDE));
    uint cb = uint(mod(c0 + 1.0, CASCADE_BIN_SIDE));
    idx = uvec4(ra * 4u + ca, ra * 4u + cb, (ra + 1u) * 4u + ca, (ra + 1u) * 4u + cb);
    w = vec4((1.0 - wu) * (1.0 - wp), (1.0 - wu) * wp, wu * (1.0 - wp), wu * wp);
}

vec3 lighting_cascade_bin_get(uint slot, uint b) {
    float f = cache.v[CASCADE_BINS_AT + (b >> 2u) * LIGHTING_CACHE_SLOTS + slot][b & 3u];
    return max(lighting_unpack_rgb9e5(floatBitsToUint(f)), vec3(0.0));
}

void lighting_cascade_bin_put(uint slot, uint b, vec3 rgb) {
    if ((lighting_toggles() & LIGHTING_TOGGLE_FREEZE_CACHE) != 0u) {
        return;
    }
    uint at = CASCADE_BINS_AT + (b >> 2u) * LIGHTING_CACHE_SLOTS + slot;
    cache.v[at][b & 3u] = uintBitsToFloat(lighting_pack_rgb9e5(max(rgb, vec3(0.0))));
}

void lighting_cascade_mark_claimed(uint slot) {
    if (slot >= LIGHTING_CACHE_SLOTS) {
        return;
    }
    uint word = lighting_cascade_claimed_at() + (slot >> 5u);
    atomicOr(ckeys.key[word], 1u << (slot & 31u));
}

// Read and clear this slot's claim bit. The bits are not cleared each
// frame: a claim made by a ray hit after this frame's pick must survive
// to the next pick.
bool lighting_cascade_take_claimed(uint slot) {
    if (slot >= LIGHTING_CACHE_SLOTS) {
        return false;
    }
    uint word = lighting_cascade_claimed_at() + (slot >> 5u);
    uint bit = 1u << (slot & 31u);
    return (atomicAnd(ckeys.key[word], ~bit) & bit) != 0u;
}

float lighting_cascade_l0_size(uint level) {
    return CASCADE_GRID_BASE * exp2(float(min(level, CASCADE_L0_MAX)));
}

uint lighting_cascade_axis(vec3 n) {
    vec3 a = abs(n);
    if (a.x >= a.y && a.x >= a.z) {
        return n.x >= 0.0 ? 0u : 1u;
    }
    if (a.y >= a.z) {
        return n.y >= 0.0 ? 2u : 3u;
    }
    return n.z >= 0.0 ? 4u : 5u;
}

vec3 lighting_cascade_axis_vec(uint a) {
    vec3 v = vec3(0.0);
    v[a >> 1u] = (a & 1u) == 0u ? 1.0 : -1.0;
    return v;
}

uint lighting_cascade_pack_level(uint level, uint axis) {
    return (level & CASCADE_LEVEL_MASK) | (axis << CASCADE_AXIS_SHIFT);
}

uint lighting_cascade_unpack_level(uint packed) {
    return packed & CASCADE_LEVEL_MASK;
}

uint lighting_cascade_unpack_axis(uint packed) {
    return packed >> CASCADE_AXIS_SHIFT;
}

// Wanted cube size at `centre`, using the axis normal. The graze floor
// caps the boost at 4x, two grid levels.
float lighting_cascade_want(vec3 centre, vec3 n_axis) {
    vec3 rel = centre - scene.eye.xyz;
    float dist = length(rel);
    vec3 view = rel / max(dist, CASCADE_EPS);
    float cosang = max(abs(dot(n_axis, view)), CASCADE_GRAZE_MIN);
    return CASCADE_FOOTPRINT_K * dist / sqrt(cosang);
}

ivec3 lighting_cascade_l0_cell(vec3 pos, vec3 n_axis, float size) {
    return ivec3(floor(pos / size - n_axis * LIGHTING_CELL_HALF));
}

vec3 lighting_cascade_l0_centre(ivec3 cell, float size) {
    return (vec3(cell) + vec3(LIGHTING_CELL_HALF)) * size;
}

// Largest grid level S with S <= want at the S-cell centre. Math only, so
// compose can call it. The previous-table hysteresis is in the walk compile.
uint lighting_cascade_l0_level_plain(vec3 pos, vec3 n) {
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    for (int i = int(CASCADE_L0_MAX); i >= 0; i--) {
        float size = lighting_cascade_l0_size(uint(i));
        ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
        vec3 centre = lighting_cascade_l0_centre(cell, size);
        if (size <= lighting_cascade_want(centre, n_axis)) {
            return uint(i);
        }
    }
    return 0u;
}

float lighting_cascade_footprint_blend(float want, float size) {
    float ratio = log2(max(want / max(size, CASCADE_EPS), CASCADE_EPS));
    if (ratio < CASCADE_FOOTPRINT_BLEND) {
        return 0.0;
    }
    if (ratio >= 1.0) {
        return 1.0;
    }
    return (ratio - CASCADE_FOOTPRINT_BLEND) / (1.0 - CASCADE_FOOTPRINT_BLEND);
}

vec4 lighting_cascade_published(uint pixel) {
    uint at = lighting_cascade_pixel_at(pixel);
    if (ckeys.key[at + 1u] != lighting_frame() + 1u) {
        return vec4(0.0);
    }
    return vec4(max(lighting_unpack_rgb9e5(ckeys.key[at]), vec3(0.0)), 1.0);
}

// Level-0 surface voxel projection. Same size and cell as insert's plain
// rule. Math only: compose can call this. A hash walk here faults lavapipe.
vec3 lighting_cascade_debug_voxels(vec3 pos, vec3 n) {
    uint level = lighting_cascade_l0_level_plain(pos, n);
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
    uint h = hash_cell3(cell, CASCADE_LEVEL0_SALT ^ lighting_cascade_pack_level(level, axis));
    vec3 col = vec3(
        lighting_unit(h),
        lighting_unit(lighting_hash(h ^ 0x9E3779B9u)),
        lighting_unit(lighting_hash(h ^ 0x85EBCA6Bu))
    );
    col = mix(vec3(CASCADE_VOXEL_VIEW_FLOOR), col, CASCADE_VOXEL_VIEW_MIX);
    col *= 1.0 - CASCADE_VOXEL_VIEW_RING_DIM * float(level);
    vec3 centre = lighting_cascade_l0_centre(cell, size);
    float want = lighting_cascade_want(centre, n_axis);
    float frac = log2(max(want / max(size, CASCADE_EPS), CASCADE_EPS));
    if (level > 0u && frac < CASCADE_VOXEL_VIEW_RING_LINE) {
        col = mix(col, vec3(1.0), CASCADE_VOXEL_VIEW_LINE_MIX);
    }
    return col;
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

uint lighting_cascade_l0_key(ivec3 cell, uint level, uint axis) {
    uint key = hash_cell3(cell, CASCADE_LEVEL0_SALT ^ lighting_cascade_pack_level(level, axis));
    if (key == 0u) {
        key = 1u;
    }
    return key;
}

uint lighting_cascade_find_in(uint parity, uint stamp, ivec3 cell, uint level, uint axis) {
    uint key = lighting_cascade_l0_key(cell, level, axis);
    uint packed = lighting_cascade_pack_level(level, axis);
    uint mask = CASCADE_LEVEL0_CAP - 1u;
    uint home = key & mask;
    uint base = lighting_cascade_table_base(parity);
    [[dont_unroll]] for (uint i = 0u; i < CASCADE_LEVEL0_PROBE; i++) {
        uint word = base + ((home + i) & mask) * CASCADE_LEVEL0_STRIDE;
        if (ckeys.key[word] != key || ckeys.key[word + 1u] != stamp) {
            continue;
        }
        if (ckeys.key[word + 2u] != uint(cell.x) || ckeys.key[word + 3u] != uint(cell.y)) {
            continue;
        }
        if (ckeys.key[word + 4u] != uint(cell.z) || ckeys.key[word + 5u] != packed) {
            continue;
        }
        return word;
    }
    return ~0u;
}

bool lighting_cascade_was_leaf(vec3 pos, vec3 n, uint level) {
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
    uint prev = (lighting_frame() + 1u) & 1u;
    uint stamp = lighting_frame();
    return lighting_cascade_find_in(prev, stamp, cell, level, axis) != ~0u;
}

uint lighting_cascade_l0_level(vec3 pos, vec3 n) {
    uint plain = lighting_cascade_l0_level_plain(pos, n);
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    uint kept = ~0u;
    for (int d = -1; d <= 1; d++) {
        int cand = int(plain) + d;
        if (cand < 0 || cand > int(CASCADE_L0_MAX)) {
            continue;
        }
        uint level = uint(cand);
        if (!lighting_cascade_was_leaf(pos, n, level)) {
            continue;
        }
        float size = lighting_cascade_l0_size(level);
        ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
        vec3 centre = lighting_cascade_l0_centre(cell, size);
        float ratio = lighting_cascade_want(centre, n_axis) / size;
        if (ratio >= CASCADE_HYST_LO && ratio <= CASCADE_HYST_HI) {
            kept = level;
        }
    }
    if (kept != ~0u) {
        return kept;
    }
    return plain;
}

uint lighting_cascade_insert_level(vec3 pos, vec3 n, uint level) {
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
    uint key = lighting_cascade_l0_key(cell, level, axis);
    uint packed = lighting_cascade_pack_level(level, axis);
    uint frame = lighting_frame() + 1u;
    uint mask = CASCADE_LEVEL0_CAP - 1u;
    uint home = key & mask;
    uint base = lighting_cascade_table_base(lighting_frame() & 1u);
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
            && ckeys.key[word + 5u] == packed) {
            return word;
        }
        if (live && cur != 0u) {
            continue;
        }
        uint prev = atomicCompSwap(ckeys.key[word], cur, key);
        if (prev != cur) {
            // Another thread already claimed this key. Do not count it again.
            if (prev == key) {
                return word;
            }
            continue;
        }
        ckeys.key[word + 1u] = frame;
        ckeys.key[word + 2u] = uint(cell.x);
        ckeys.key[word + 3u] = uint(cell.y);
        ckeys.key[word + 4u] = uint(cell.z);
        ckeys.key[word + 5u] = packed;
        // The cell centre moved along the axis onto this pixel's surface.
        // The half-cell inset puts the cell behind a surface that sits in
        // the outer half of its cell, so the +axis face can be inside a
        // wall that is not on a grid plane; rays from there see nothing.
        vec3 centre = lighting_cascade_l0_centre(cell, size);
        vec3 stored = centre + n_axis * dot(pos - centre, n_axis);
        ckeys.key[word + 6u] = floatBitsToUint(stored.x);
        ckeys.key[word + 7u] = floatBitsToUint(stored.y);
        ckeys.key[word + 8u] = floatBitsToUint(stored.z);
        ckeys.key[word + 9u] = lighting_pack_normal(n);
        ckeys.key[word + 10u] = 0u;
        ckeys.key[word + 11u] = 0u;
        return word;
    }
    return ~0u;
}

uint lighting_cascade_find(ivec3 cell, uint level, uint axis);
vec4 lighting_cascade_record(uint word, vec3 pos, vec3 n);
vec4 lighting_cascade_at(vec3 pos, vec3 n, uint level);

// A parent corner the leak gates refuse contributes nothing. Facing, a
// stale cell, and an empty mean are the same refusals as before. The
// plane window is `plane`, not the enlarged cube: a coarser parent must
// not join a parallel surface farther than K·dist.
vec4 lighting_cascade_gated(uint s, vec3 q, vec3 n, float plane) {
    if (s == ~0u) {
        return vec4(0.0);
    }
    if (lighting_sc_stale(s)) {
        return vec4(0.0);
    }
    if (dot(cache.v[3u * s + 1u].xyz, n) <= 0.0) {
        return vec4(0.0);
    }
    if (abs(dot(cache.v[3u * s].xyz - q, n)) > LIGHTING_CELL_HALF * plane) {
        return vec4(0.0);
    }
    vec4 v = cache.v[3u * s + 2u];
    if (!(v.w > LIGHTING_CACHE_HAS_W)) {
        return vec4(0.0);
    }
    return vec4(max(v.rgb, vec3(0.0)), 1.0);
}

// Parent bins for a probe at `pos` facing `n` whose own rays end at the
// parent's interval start: the trilinear mix of the eight parent cells
// around `pos`, each through the leak gates. When no parent corner passes,
// the nearest coarser ancestor that does is used instead, never zero.
// `ok` is false only when no ancestor up to 32 m exists yet.
void lighting_cascade_parent_bins(vec3 pos, vec3 n, float child_size, out vec3 bins[16], out bool ok) {
    for (uint b = 0u; b < CASCADE_BINS; b++) {
        bins[b] = vec3(0.0);
    }
    ok = false;
    float size = child_size * CASCADE_PARENT_SCALE;
    [[dont_unroll]] for (uint level = 0u; level < 10u; level++) {
        if (size > 32.0 + CASCADE_EPS) {
            return;
        }
        // The parent cell's own thickness: a metre-scale parent must pass
        // its trilinear corners. Facing and stale still reject the far wall.
        vec3 base_p = pos / size - vec3(LIGHTING_CELL_HALF);
        ivec3 base = ivec3(floor(base_p));
        vec3 f = fract(base_p);
        float wsum = 0.0;
        [[dont_unroll]] for (uint i = 0u; i < 8u; i++) {
            ivec3 o = ivec3(int(i & 1u), int((i >> 1u) & 1u), int((i >> 2u) & 1u));
            vec3 wt = mix(1.0 - f, f, vec3(o));
            float w = wt.x * wt.y * wt.z;
            if (!(w > 0.0)) {
                continue;
            }
            vec3 q = (vec3(base + o) + vec3(LIGHTING_CELL_HALF)) * size;
            uint s = lighting_cache_find_sized(q, n, false, size, q);
            vec4 g = lighting_cascade_gated(s, pos, n, size);
            if (!(g.w > LIGHTING_CACHE_HAS_W)) {
                continue;
            }
            lighting_cascade_mark_claimed(s);
            for (uint b = 0u; b < CASCADE_BINS; b++) {
                bins[b] += w * lighting_cascade_bin_get(s, b);
            }
            wsum += w;
        }
        if (wsum > 0.0) {
            for (uint b = 0u; b < CASCADE_BINS; b++) {
                bins[b] /= wsum;
            }
            ok = true;
            return;
        }
        size *= CASCADE_PARENT_SCALE;
    }
}

// Visible surfaces whose published sample is empty. Stats word 35 (diag[11]).
const uint LIGHTING_DIAG_CASCADE_GAP = 11u;

// This footprint level, mixed with the next coarser level across the
// outer tenth. An empty nearer cell may still take the coarser level.
// There is no same-level neighbour gather and no claimed-slot bypass
// of the plane and facing gates. An empty published sample is a gap.
vec4 lighting_cascade_merge(uvec2 p, vec3 pos, vec3 n) {
    uint pixel = p.y * pc.dims.x + p.x;
    uint at = lighting_cascade_pixel_at(pixel);
    uint word = ckeys.key[at + 2u];
    uint level;
    if (word != ~0u) {
        level = lighting_cascade_unpack_level(ckeys.key[word + 5u]);
    } else {
        level = lighting_cascade_l0_level(pos, n);
    }
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
    vec3 centre = lighting_cascade_l0_centre(cell, size);
    float blend = lighting_cascade_footprint_blend(lighting_cascade_want(centre, n_axis), size);
    vec4 near = lighting_cascade_at(pos, n, level);
    vec4 far = vec4(0.0);
    if (level < CASCADE_L0_MAX) {
        far = lighting_cascade_at(pos, n, level + 1u);
    }
    bool near_ok = near.w > LIGHTING_CACHE_HAS_W;
    bool far_ok = far.w > LIGHTING_CACHE_HAS_W;
    vec4 shown = vec4(0.0);
    if (near_ok && far_ok && blend > 0.0) {
        shown = vec4(mix(near.rgb, far.rgb, blend), 1.0);
    } else if (near_ok) {
        shown = near;
    } else if (far_ok) {
        shown = far;
    }
    if (!(shown.w > LIGHTING_CACHE_HAS_W)) {
        atomicAdd(gstats.diag[LIGHTING_DIAG_CASCADE_GAP], 1u);
    }
    return shown;
}

void lighting_cascade_store(uint word, uint packed, uint weight) {
    if (word == ~0u) {
        return;
    }
    if ((lighting_toggles() & LIGHTING_TOGGLE_FREEZE_CACHE) != 0u) {
        return;
    }
    ckeys.key[word + 10u] = packed;
    ckeys.key[word + 11u] = weight;
}

// Level-0 rays cover [ε, 8s]. An upper cell of size S traces [4S, 8S],
// with the far end clipped at 200 m. The 25 cm full-trace exception is gone:
// contact belongs to level 0.
const float CASCADE_TRACE_NEAR = 1.0e-4;
const float CASCADE_TRACE_FAR = 200.0;

void lighting_cascade_l0_interval(float size, out float t0, out float t1) {
    t0 = CASCADE_TRACE_NEAR;
    t1 = min(8.0 * max(size, CASCADE_EPS), CASCADE_TRACE_FAR);
}

void lighting_cascade_interval(float size, out float t0, out float t1) {
    t0 = 4.0 * max(size, CASCADE_EPS);
    t1 = min(8.0 * max(size, CASCADE_EPS), CASCADE_TRACE_FAR);
    if (t0 >= CASCADE_TRACE_FAR) {
        t0 = CASCADE_TRACE_FAR;
        t1 = CASCADE_TRACE_FAR;
    }
    t0 = max(t0, CASCADE_TRACE_NEAR);
    t1 = max(t1, t0);
}

uint lighting_cascade_ray_slot(uint ray) {
    if (ray < lighting_probe_rays()) {
        return ~0u;
    }
    uint r = ray - lighting_probe_rays();
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

uint lighting_cascade_find(ivec3 cell, uint level, uint axis) {
    return lighting_cascade_find_in(
        lighting_frame() & 1u, lighting_frame() + 1u, cell, level, axis);
}

vec4 lighting_cascade_at_in(uint parity, uint stamp, vec3 pos, vec3 n, uint level) {
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
    return lighting_cascade_record(
        lighting_cascade_find_in(parity, stamp, cell, level, axis), pos, n);
}

vec4 lighting_cascade_blend_pair(vec4 near, vec4 far, float blend) {
    bool near_ok = near.w > LIGHTING_CACHE_HAS_W;
    bool far_ok = far.w > LIGHTING_CACHE_HAS_W;
    if (near_ok && far_ok && blend > 0.0) {
        return vec4(mix(near.rgb, far.rgb, blend), 1.0);
    }
    if (near_ok) {
        return near;
    }
    if (far_ok) {
        return far;
    }
    return vec4(0.0);
}

// Bounce at a hit: last frame's level-0 record at the hit, blended with
// the next coarser level. A miss takes an upper cell the rolling refresh
// keeps at the hit (claiming the parent chain so later frames can shade
// it). The current table is not read: this frame's shade has not finished.
vec4 lighting_cascade_bounce_level(uint parity, uint stamp, vec3 pos, vec3 n, uint level) {
    if (level > CASCADE_L0_MAX) {
        return vec4(0.0);
    }
    return lighting_cascade_at_in(parity, stamp, pos, n, level);
}

vec3 lighting_cascade_bounce(vec3 pos, vec3 n) {
    if (!lighting_bounce_ok()) {
        return vec3(0.0);
    }
    uint level = lighting_cascade_l0_level(pos, n);
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
    vec3 centre = lighting_cascade_l0_centre(cell, size);
    float blend = lighting_cascade_footprint_blend(lighting_cascade_want(centre, n_axis), size);
    uint prev = (lighting_frame() + 1u) & 1u;
    uint stamp = lighting_frame();
    vec4 near = lighting_cascade_bounce_level(prev, stamp, pos, n, level);
    if (!(near.w > LIGHTING_CACHE_HAS_W) && level > 0u) {
        near = lighting_cascade_bounce_level(prev, stamp, pos, n, level - 1u);
    }
    vec4 far = vec4(0.0);
    if (level < CASCADE_L0_MAX) {
        far = lighting_cascade_bounce_level(prev, stamp, pos, n, level + 1u);
    }
    vec4 shown = lighting_cascade_blend_pair(near, far, blend);
    if (shown.w > LIGHTING_CACHE_HAS_W) {
        return shown.rgb;
    }
    float parent = size * CASCADE_PARENT_SCALE;
    vec3 e = vec3(0.0);
    [[dont_unroll]] for (uint i = 0u; i < 9u; i++) {
        if (parent > 32.0 + CASCADE_EPS) {
            break;
        }
        uint s = lighting_cache_find_sized(pos, n, true, parent, pos);
        if (s != ~0u) {
            lighting_cascade_mark_claimed(s);
            vec4 g = lighting_cascade_gated(s, pos, n, parent);
            if (g.w > LIGHTING_CACHE_HAS_W) {
                e = g.rgb;
                break;
            }
        }
        parent *= CASCADE_PARENT_SCALE;
    }
    return e;
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
    float size = lighting_cascade_l0_size(lighting_cascade_unpack_level(ckeys.key[word + 5u]));
    if (dot(sn, n) <= 0.0) {
        return vec4(0.0);
    }
    if (abs(dot(stored - pos, n)) > size) {
        return vec4(0.0);
    }
    float weight = uintBitsToFloat(ckeys.key[word + 11u]);
    if (!(weight > LIGHTING_CACHE_HAS_W)) {
        return vec4(0.0);
    }
    return vec4(max(lighting_unpack_rgb9e5(ckeys.key[word + 10u]), vec3(0.0)), 1.0);
}

vec4 lighting_cascade_at(vec3 pos, vec3 n, uint level) {
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
    return lighting_cascade_record(lighting_cascade_find(cell, level, axis), pos, n);
}

// This level, blended with the next coarser level across the outer tenth.
// Each side is that level's own cell. There is no same-level gather.
vec4 lighting_cascade_picture(vec3 pos, vec3 n) {
    uint level = lighting_cascade_l0_level(pos, n);
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
    vec3 centre = lighting_cascade_l0_centre(cell, size);
    float blend = lighting_cascade_footprint_blend(lighting_cascade_want(centre, n_axis), size);
    vec4 near = lighting_cascade_at(pos, n, level);
    vec4 far = vec4(0.0);
    if (level < CASCADE_L0_MAX) {
        far = lighting_cascade_at(pos, n, level + 1u);
    }
    bool near_ok = near.w > LIGHTING_CACHE_HAS_W;
    bool far_ok = far.w > LIGHTING_CACHE_HAS_W;
    if (near_ok && far_ok && blend > 0.0) {
        return vec4(mix(near.rgb, far.rgb, blend), 1.0);
    }
    if (near_ok) {
        return near;
    }
    if (far_ok) {
        return far;
    }
    return vec4(0.0);
}

void lighting_cascade_publish(uint pixel, uvec2 p, vec3 pos, vec3 n) {
    // This pixel's merge. The picture loads this pixel's own word. Insert
    // claims the slots; publish blends this footprint level with the next
    // coarser one and writes the radiance compose loads.
    if ((lighting_toggles() & LIGHTING_TOGGLE_FREEZE_CACHE) != 0u) {
        return;
    }
    uint at = lighting_cascade_pixel_at(pixel);
    uint word = ckeys.key[at + 2u];
    uint word_next = ckeys.key[at + 3u];
    vec4 shown = lighting_cascade_merge(p, pos, n);
    ckeys.key[at] = lighting_pack_rgb9e5(max(shown.rgb, vec3(0.0)));
    ckeys.key[at + 1u] = shown.w > LIGHTING_CACHE_HAS_W ? lighting_frame() + 1u : 0u;
    ckeys.key[at + 2u] = word;
    ckeys.key[at + 3u] = word_next;
}

void lighting_cascade_insert(uint pixel, vec3 pos, vec3 n) {
    uint level = lighting_cascade_l0_level(pos, n);
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    ivec3 cell = lighting_cascade_l0_cell(pos, n_axis, size);
    vec3 centre = lighting_cascade_l0_centre(cell, size);
    float want = lighting_cascade_want(centre, n_axis);
    uint word = lighting_cascade_insert_level(pos, n, level);
    uint word_next = ~0u;
    float blend = lighting_cascade_footprint_blend(want, size);
    if (blend > 0.0 && level < CASCADE_L0_MAX) {
        word_next = lighting_cascade_insert_level(pos, n, level + 1u);
    }
    uint at = lighting_cascade_pixel_at(pixel);
    ckeys.key[at + 2u] = word;
    ckeys.key[at + 3u] = word_next;
    if (word == ~0u && word_next == ~0u) {
        return;
    }
    // Radiance is stored by the merge. The direct pass only claims the cell
    // here: evaluating the gather while claiming changed the settled picture.
    lighting_cascade_store(word, 0u, 0u);
    lighting_cascade_store(word_next, 0u, 0u);
}
#endif
