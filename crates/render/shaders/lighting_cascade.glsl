// Level 0 of the sparse cascade. Rebuilt from the G-buffer each frame on
// the shared power-of-two world grid (1/16 m base). A pixel's continuous
// footprint level picks two grid levels, floor and floor + 1, and insert
// claims both. Two tables (this frame and last) keep the previous voxels
// for the four-slot direction ring. Every live voxel traces 32 rays each
// frame: frame f traces a fixed interleaved 32 of 128 hemisphere
// directions (slot f % 4) and replaces that slot. The voxel value is the
// average of the filled slots. There is no EMA. Publish shows
// mix(floor, floor + 1, fract(level)), each side a bilinear of cell
// centres on the surface, gated by plane and facing. GI v1 stays removed.
//
// The record sits after the reach words. Word 0 is unused. Two tables of
// CASCADE_LEVEL0_CAP records, then four words a pixel: published radiance
// and stamp (compose), then the insert slot and the blend slot. After the
// pixels: inserted and traced counts, then one four-word hit record per
// voxel slot and ray. Each record is CASCADE_LEVEL0_STRIDE words: key,
// stamp, cell x y z, level packed with axis, position xyz, packed normal,
// average radiance, valid-slot count, then four packed slot radiances.
// The host sizes the same tail.

const uint CASCADE_LEVEL0_CAP = 16384u;
const uint CASCADE_LEVEL0_STRIDE = 16u;
const uint CASCADE_LEVEL0_PROBE = 16u;
const uint CASCADE_LEVEL0_SALT = 0xC35Cu;
const float CASCADE_PROJ = 0.05;
const float CASCADE_RING_NEAR = 1.0;
const uint CASCADE_RING_MAX = 6u;
const float CASCADE_OVERLAP = 0.1;
const float CASCADE_PARENT_SCALE = 2.0;
const float CASCADE_RESEAT = 0.002;
// Voxel debug view (lighting_cascade_debug_voxels). Floor and mix keep
// hashed colours visible; ring dim and line mark distance rings.
const float CASCADE_VOXEL_VIEW_FLOOR = 0.12;
const float CASCADE_VOXEL_VIEW_MIX = 0.88;
const float CASCADE_VOXEL_VIEW_RING_DIM = 0.08;
const float CASCADE_VOXEL_VIEW_RING_LINE = 0.03;
const float CASCADE_VOXEL_VIEW_LINE_MIX = 0.65;
// Footprint size on the shared world grid. 2^n / 16 m. K is the angular
// size of a box (about 22 x 40 px at 1440p). The 1/16 graze floor caps
// the boost at two grid levels.
const float CASCADE_GRID_BASE = 0.0625;
const uint CASCADE_L0_MAX = 9u;
const float CASCADE_FOOTPRINT_K = 0.03125;
const float CASCADE_GRAZE_MIN = 0.0625;
const uint CASCADE_TABLES = 2u;
const uint CASCADE_PIXEL_WORDS = 4u;
const uint CASCADE_AXIS_SHIFT = 8u;
const uint CASCADE_LEVEL_MASK = 255u;
const float CASCADE_EPS = 1.0e-6;
// 32 stratified hemisphere rays per live voxel, every frame. Hits are
// stored after the count words so the shade pass can run without a
// second trace site in this compile. 128 fixed directions are split
// into 4 interleaved sets of 32; frame f traces and replaces slot f % 4.
const uint CASCADE_L0_RAYS = 32u;
const uint CASCADE_L0_SLOTS = 4u;
const uint CASCADE_L0_DIRS = CASCADE_L0_RAYS * CASCADE_L0_SLOTS;
const uint CASCADE_L0_HIT_WORDS = 4u;
const vec3 CASCADE_LUMA = vec3(0.2126, 0.7152, 0.0722);

bool lighting_cascade_finite3(vec3 c) {
    return !(isnan(c.x) || isinf(c.x) || isnan(c.y) || isinf(c.y)
        || isnan(c.z) || isinf(c.z));
}

// Copy last frame's four slot results and replace slot `lighting_frame() % 4`
// with `cur` when it is finite. Writes the packed slots, the average, and
// the valid count onto `word`. A non-finite sample leaves that slot as it was.
void lighting_cascade_ring_store(uint word, uint prev, vec3 cur) {
    uint s0 = ~0u;
    uint s1 = ~0u;
    uint s2 = ~0u;
    uint s3 = ~0u;
    if (prev != ~0u) {
        s0 = ckeys.key[prev + 12u];
        s1 = ckeys.key[prev + 13u];
        s2 = ckeys.key[prev + 14u];
        s3 = ckeys.key[prev + 15u];
    }
    uint slot = lighting_frame() % CASCADE_L0_SLOTS;
    if (lighting_cascade_finite3(cur)) {
        uint packed = lighting_pack_rgb9e5(max(cur, vec3(0.0)));
        if (slot == 0u) {
            s0 = packed;
        } else if (slot == 1u) {
            s1 = packed;
        } else if (slot == 2u) {
            s2 = packed;
        } else {
            s3 = packed;
        }
    }
    ckeys.key[word + 12u] = s0;
    ckeys.key[word + 13u] = s1;
    ckeys.key[word + 14u] = s2;
    ckeys.key[word + 15u] = s3;
    vec3 acc = vec3(0.0);
    float n = 0.0;
    if (s0 != ~0u) {
        acc += max(lighting_unpack_rgb9e5(s0), vec3(0.0));
        n += 1.0;
    }
    if (s1 != ~0u) {
        acc += max(lighting_unpack_rgb9e5(s1), vec3(0.0));
        n += 1.0;
    }
    if (s2 != ~0u) {
        acc += max(lighting_unpack_rgb9e5(s2), vec3(0.0));
        n += 1.0;
    }
    if (s3 != ~0u) {
        acc += max(lighting_unpack_rgb9e5(s3), vec3(0.0));
        n += 1.0;
    }
    ckeys.key[word + 10u] = lighting_pack_rgb9e5(n > 0.0 ? acc / n : vec3(0.0));
    ckeys.key[word + 11u] = floatBitsToUint(n);
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

// Level-0 ray k of a fixed 4-frame cycle. 128 stratified hemisphere
// directions (one equal-measure cos(theta) stratum each) are split into
// 4 interleaved sets of 32. Frame f traces slot f % 4 (indices k*4+slot).
// The set is the same every cycle: no per-frame rotation. lighting_hemi_dir
// adds a per-voxel turn from the key, which stays fixed for the voxel.
vec3 lighting_cascade_l0_dir(uint key, vec3 n, uint k) {
    uint slot = lighting_frame() % CASCADE_L0_SLOTS;
    uint i = k * CASCADE_L0_SLOTS + slot;
    float u = max((float(i) + 0.5) / float(CASCADE_L0_DIRS), CASCADE_EPS);
    float turn = fract(float(i) * 0.6180339887);
    return lighting_hemi_dir(key, n, u, turn);
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
// A bin holds the mean radiance of its rays: a hit, or the parent's bins
// on a miss. A child reads only parent cells it can see, so a parent
// across a wall does not reach it.
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
// key and ray only, so a still refresh repeats the same directions.
vec3 lighting_cascade_bin_dir(uint key, vec3 n, uint k) {
    uint b = k % CASCADE_BINS;
    uint h = lighting_hash(key ^ (k * 0x9e3779b9u));
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

float lighting_cascade_lod(vec3 pos, vec3 n) {
    vec3 n_axis = lighting_cascade_axis_vec(lighting_cascade_axis(n));
    float want = lighting_cascade_want(pos, n_axis);
    return clamp(log2(max(want / CASCADE_GRID_BASE, CASCADE_EPS)), 0.0, float(CASCADE_L0_MAX));
}

uint lighting_cascade_lod_level(float lod) {
    return min(uint(floor(lod)), CASCADE_L0_MAX);
}

float lighting_cascade_lod_blend(float lod) {
    if (lighting_cascade_lod_level(lod) >= CASCADE_L0_MAX) {
        return 0.0;
    }
    return lod - floor(lod);
}

// Debug view "path" (compose mode 9): red 1 where the pixel has no
// published level-0 sample (a gap), green the blend towards the coarser
// level, blue the finer footprint level over the deepest level.
vec3 lighting_cascade_path_view(vec3 pos, vec3 n, float published_w) {
    float lod = lighting_cascade_lod(pos, n);
    float gap = published_w > LIGHTING_CACHE_HAS_W ? 0.0 : 1.0;
    return vec3(gap, lighting_cascade_lod_blend(lod), float(lighting_cascade_lod_level(lod)) / float(CASCADE_L0_MAX));
}

vec4 lighting_cascade_published(uint pixel) {
    uint at = lighting_cascade_pixel_at(pixel);
    if (ckeys.key[at + 1u] != lighting_frame() + 1u) {
        return vec4(0.0);
    }
    return vec4(max(lighting_unpack_rgb9e5(ckeys.key[at]), vec3(0.0)), 1.0);
}

// Level-0 surface voxel projection: the floor level insert claims.
// Math only: compose can call this. A hash walk here faults lavapipe.
vec3 lighting_cascade_debug_voxels(vec3 pos, vec3 n) {
    float lod = lighting_cascade_lod(pos, n);
    uint level = lighting_cascade_lod_level(lod);
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
    if (level > 0u && lod - floor(lod) < CASCADE_VOXEL_VIEW_RING_LINE) {
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
        if (ckeys.key[word + 1u] != stamp) {
            return ~0u;
        }
        if (ckeys.key[word] != key) {
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

uint lighting_cascade_find_layer(uint parity, uint stamp, ivec3 cell, uint ax, uint level, uint axis, uint found) {
    if (found != ~0u) {
        return found;
    }
    ivec3 below = cell;
    below[ax] -= 1;
    uint word = lighting_cascade_find_in(parity, stamp, below, level, axis);
    if (word != ~0u) {
        return word;
    }
    ivec3 above = cell;
    above[ax] += 1;
    return lighting_cascade_find_in(parity, stamp, above, level, axis);
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
        uint prev = atomicCompSwap(ckeys.key[word], 0u, key);
        if (prev == key) {
            return word;
        }
        if (prev != 0u) {
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
        float along = dot(pos - centre, n) / max(dot(n_axis, n), CASCADE_EPS);
        vec3 stored = centre + n_axis * clamp(along, -size, size);
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

// One of the eight parent-grid cells around `pos` at exactly `size`.
// `claim` allocates a missing corner so the rolling refresh can shade it;
// a newly claimed cell is empty until that refresh.
void lighting_cascade_parent_corner(vec3 pos, vec3 n, float size, uint i, bool claim, bool seat, out uint slot, out float weight) {
    // Same grid as lighting_cache_key_sized / lighting_sc_centre: half a
    // cell in along the axis, not a cube inset on every axis. Otherwise
    // the eight corners miss the cells the cache actually allocated.
    uint ax = lighting_axis(n) >> 1u;
    ivec3 o = ivec3(int(i & 1u), int((i >> 1u) & 1u), int((i >> 2u) & 1u));
    slot = ~0u;
    weight = 0.0;
    if (o[ax] != 0) {
        return;
    }
    vec3 uv = pos / size - vec3(0.5);
    ivec3 base = ivec3(floor(uv));
    vec3 f = fract(uv);
    vec3 wt = mix(1.0 - f, f, vec3(o));
    wt[ax] = 1.0;
    weight = wt.x * wt.y * wt.z;
    if (!(weight > 0.0)) {
        return;
    }
    vec3 q = (vec3(base + o) + vec3(0.5)) * size;
    uint a1 = (ax + 1u) % 3u;
    uint a2 = (ax + 2u) % 3u;
    float slope = ((q[a1] - pos[a1]) * n[a1] + (q[a2] - pos[a2]) * n[a2]) / n[ax];
    q[ax] = pos[ax] - clamp(slope, -size, size);
    slot = lighting_cache_find_sized(q, n, claim, size, q);
    if (seat && slot != ~0u) {
        float off = dot(cache.v[3u * slot].xyz - q, n);
        if (off < -CASCADE_RESEAT || off > LIGHTING_CELL_HALF * size) {
            cache.v[3u * slot].xyz = q;
        }
    }
}

// Claim the eight parent-grid corners at every exact parent size of a
// level-0 voxel, so interval chaining has the cell whose 4S starts
// where the child's rays end (8s). A coarser size is a different
// interval and is not a stand-in.
void lighting_cascade_claim_parents(vec3 pos, vec3 n, float child_size) {
    float size = child_size * CASCADE_PARENT_SCALE;
    [[dont_unroll]] for (uint level = 0u; level < 9u; level++) {
        if (size > 32.0 + CASCADE_EPS) {
            return;
        }
        [[dont_unroll]] for (uint i = 0u; i < 8u; i++) {
            uint s;
            float w;
            lighting_cascade_parent_corner(pos, n, size, i, true, true, s, w);
            if (s != ~0u) {
                lighting_cascade_mark_claimed(s);
            }
        }
        size *= CASCADE_PARENT_SCALE;
    }
}

// Parent bins for a probe at `pos` facing `n` whose own rays end at the
// parent's interval start: the trilinear mix of the eight parent cells
// at exactly 2 * child_size, each through the leak gates. A coarser
// ancestor's bins start farther out (its own 4S), so they are not used
// as this interval. `ok` is false until those exact parents exist and
// have been shaded.
void lighting_cascade_parent_bins(vec3 pos, vec3 n, float child_size, bool seat, out vec3 bins[16], out bool ok) {
    for (uint b = 0u; b < CASCADE_BINS; b++) {
        bins[b] = vec3(0.0);
    }
    ok = false;
    float size = child_size * CASCADE_PARENT_SCALE;
    if (size > 32.0 + CASCADE_EPS) {
        return;
    }
    float wsum = 0.0;
    [[dont_unroll]] for (uint i = 0u; i < 8u; i++) {
        uint s;
        float w;
        lighting_cascade_parent_corner(pos, n, size, i, true, seat, s, w);
        if (!(w > 0.0)) {
            continue;
        }
        vec4 g = lighting_cascade_gated(s, pos, n, size);
        if (!(g.w > LIGHTING_CACHE_HAS_W)) {
            continue;
        }
        lighting_cascade_mark_claimed(s);
        vec3 cpos = cache.v[3u * s].xyz;
        if (trace_occluded(pos + n * 0.02, cpos + n * 0.02)) {
            continue;
        }
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
    }
}

// Visible surfaces whose published sample is empty. Stats word 35 (diag[11]).
const uint LIGHTING_DIAG_CASCADE_GAP = 11u;

// mix(floor, floor + 1, fract(level)) of the pixel's footprint level. An
// empty nearer side takes the coarser one. An empty sample is a gap.
vec4 lighting_cascade_merge(uvec2 p, vec3 pos, vec3 n) {
    float lod = lighting_cascade_lod(pos, n);
    uint level = lighting_cascade_lod_level(lod);
    float blend = lighting_cascade_lod_blend(lod);
    vec4 near = lighting_cascade_at(pos, n, level);
    vec4 far = vec4(0.0);
    if (level < CASCADE_L0_MAX && (blend > 0.0 || !(near.w > LIGHTING_CACHE_HAS_W))) {
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

// Sparse trilinear of the eight same-level cells around `pos`, renormalised
// over the corners that pass the plane and facing gates. The stored
// values are not written. Off-surface corners drop out, so a wall stays
// a bilinear in its plane.
vec4 lighting_cascade_at_in(uint parity, uint stamp, vec3 pos, vec3 n, uint level) {
    uint axis = lighting_cascade_axis(n);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(level);
    uint ax = axis >> 1u;
    uint a1 = (ax + 1u) % 3u;
    uint a2 = (ax + 2u) % 3u;
    vec3 uv = pos / size - vec3(LIGHTING_CELL_HALF);
    ivec3 base = ivec3(floor(uv));
    base[ax] = lighting_cascade_l0_cell(pos, n_axis, size)[ax];
    vec3 f = fract(uv);
    vec3 acc = vec3(0.0);
    float wsum = 0.0;
    for (uint i = 0u; i < 4u; i++) {
        ivec3 o = ivec3(0);
        o[a1] = int(i & 1u);
        o[a2] = int((i >> 1u) & 1u);
        float w = (o[a1] != 0 ? f[a1] : 1.0 - f[a1]) * (o[a2] != 0 ? f[a2] : 1.0 - f[a2]);
        if (!(w > 0.0)) {
            continue;
        }
        ivec3 c = base + o;
        vec3 q = (vec3(c) + vec3(LIGHTING_CELL_HALF)) * size;
        float slope = ((q[a1] - pos[a1]) * n[a1] + (q[a2] - pos[a2]) * n[a2]) / n[ax];
        q[ax] = pos[ax] - clamp(slope, -size, size);
        c[ax] = lighting_cascade_l0_cell(q, n_axis, size)[ax];
        uint word = lighting_cascade_find_in(parity, stamp, c, level, axis);
        word = lighting_cascade_find_layer(parity, stamp, c, ax, level, axis, word);
        vec4 rec = lighting_cascade_record(word, pos, n);
        if (rec.w > LIGHTING_CACHE_HAS_W) {
            acc += w * rec.rgb;
            wsum += w;
        }
    }
    if (wsum > 0.0) {
        return vec4(acc / wsum, 1.0);
    }
    return vec4(0.0);
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

// Last frame's stored bounce only. Fresh direct is shaded at the hit by
// the caller. The old bounce-round gate (lighting_bounce_ok, cache reads
// from bounce 2) does not apply: multi-bounce is this history lookup.
vec3 lighting_cascade_bounce(vec3 pos, vec3 n) {
    float lod = lighting_cascade_lod(pos, n);
    uint level = lighting_cascade_lod_level(lod);
    float blend = lighting_cascade_lod_blend(lod);
    float size = lighting_cascade_l0_size(level);
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
    return lighting_cascade_at_in(
        lighting_frame() & 1u, lighting_frame() + 1u, pos, n, level);
}

// The same blend as lighting_cascade_merge, without the gap count.
vec4 lighting_cascade_picture(vec3 pos, vec3 n) {
    float lod = lighting_cascade_lod(pos, n);
    uint level = lighting_cascade_lod_level(lod);
    float blend = lighting_cascade_lod_blend(lod);
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
    uint level = lighting_cascade_lod_level(lighting_cascade_lod(pos, n));
    uint word = lighting_cascade_insert_level(pos, n, level);
    uint word_next = ~0u;
    if (level < CASCADE_L0_MAX) {
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
