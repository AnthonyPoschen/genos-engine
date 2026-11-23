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
// Footprint size on the shared world grid. 2^n / 8 m. K is the angular
// size of a box (about 44 x 80 px at 1440p). The 1/16 graze floor caps
// the boost at two grid levels.
const float CASCADE_GRID_BASE = 0.125;
const uint CASCADE_L0_MAX = 8u;
const float CASCADE_FOOTPRINT_K = 0.0625;
const float CASCADE_GRAZE_MIN = 0.0625;
// Voxel seat (lighting_cascade_seat). The owner rank spans this many cell
// sizes from the centre. The shown surface counts as the voxel's own within
// this share of a cell (plus a floor) of its plane and this facing.
const float CASCADE_OWNER_SPAN = 1.25;
const float CASCADE_SEAT_NEAR = 1.0e-3;
const float CASCADE_SEAT_PROBE = 0.02;
const float CASCADE_SEAT_FACING = 0.9;
// A kept seat stays while its normal is within 60 degrees of the owner pixel's
// and of the surface under it (lighting_cascade_seat).
const float CASCADE_SEAT_KEEP = 0.5;
const float CASCADE_SEAT_PLANE = 0.1;
const float CASCADE_SEAT_PLANE_MIN = 0.01;
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

// Inserted and traced counters, the level-0 indirect dispatch command
// (groups, 1, 1, pad), the list of seated voxel slots it runs over, then one
// hit record per (slot, ray). The trace and shade passes used to dispatch
// one group per table slot, 16384 of them, for the 1300 to 5000 that hold a
// voxel.
const uint CASCADE_L0_COUNT_WORDS = 2u + 4u + CASCADE_LEVEL0_CAP;
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

// Lamp cone. Most bounce indoors comes from the surfaces right around a
// lamp (the ceiling 0.4 m over it is a small patch a few hundred times
// brighter than the floor). A probe's uniform rays hit that patch by luck,
// so each cell came out bright or dark by chance: spots that popped in up
// close. Each probe now aims extra rays into a cone around its strongest
// nearby lamp (CASCADE_CONE_REACH metres about it). Uniform rays inside the
// cone are left out and the cone rays stand for that solid angle, so the sum
// is the same integral with the bright part sampled densely.
const uint CASCADE_CONE_L0 = 4u;
const uint CASCADE_CONE_UPPER = 8u;
const uint CASCADE_L0_TRACED = CASCADE_L0_RAYS + CASCADE_CONE_L0;
const float CASCADE_CONE_RANGE = 16.0;
const float CASCADE_CONE_REACH = 1.5;
const float CASCADE_CONE_MAX = 1.0;

// The cone of the probe at pos: the lamp with the most light per squared
// distance within CASCADE_CONE_RANGE whose cone reaches above the surface.
bool lighting_cascade_cone(vec3 pos, vec3 n, out vec3 axis, out float cos_a) {
    axis = n;
    cos_a = 1.0;
    LampList list = lamps_at(pos);
    uint count = lamp_list_size(list);
    float best = 0.0;
    [[dont_unroll]] for (uint k = 0u; k < count; k++) {
        Lamp lamp = lamp_list_get(list, k);
        if (lamp.pos.w > 0.5) {
            continue;
        }
        vec3 d = lamp.pos.xyz - pos;
        float dist = length(d);
        if (dist > CASCADE_CONE_RANGE || dist < 1.0e-3) {
            continue;
        }
        float a = min(atan(CASCADE_CONE_REACH, dist), CASCADE_CONE_MAX);
        vec3 dir = d / dist;
        if (dot(dir, n) < -sin(a)) {
            continue;
        }
        float score = dot(lamp.color.rgb, vec3(0.2126, 0.7152, 0.0722)) / max(dist * dist, 0.25);
        if (score > best) {
            best = score;
            axis = dir;
            cos_a = cos(a);
        }
    }
    return best > 0.0;
}

// Cone ray j of count, stratified on cos(theta) with a golden turn; h jitters.
vec3 lighting_cascade_cone_dir(vec3 axis, float cos_a, uint j, uint count, uint h) {
    float u = (float(j) + lighting_unit(h)) / float(count);
    float ct = 1.0 - u * (1.0 - cos_a);
    float st = sqrt(max(1.0 - ct * ct, 0.0));
    float phi = LIGHTING_TAU * fract(float(j) * 0.6180339887 + lighting_unit(lighting_hash(h ^ 0x68bc21ebu)));
    vec3 helper = abs(axis.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 tx = normalize(cross(helper, axis));
    vec3 ty = cross(axis, tx);
    return normalize(tx * (st * cos(phi)) + ty * (st * sin(phi)) + axis * ct);
}

// Solid-angle weight of a ray: a uniform ray of `uniform_count` stands for
// 2pi / uniform_count, or nothing inside the cone; a cone ray of
// `cone_count` stands for the cone's solid angle over cone_count.
float lighting_cascade_ray_weight(bool is_cone, bool has_cone, vec3 dir, vec3 axis, float cos_a,
                                  uint uniform_count, uint cone_count) {
    if (is_cone) {
        return has_cone ? LIGHTING_TAU * (1.0 - cos_a) / float(cone_count) : 0.0;
    }
    if (has_cone && dot(dir, axis) > cos_a) {
        return 0.0;
    }
    return LIGHTING_TAU / float(uniform_count);
}

// How far off its seat a level-0 voxel's rays start. Clear of a thin field's
// shell (mf_skin: half a voxel to each side, 3 cm in a level field). At 2 cm
// the start was inside the shell, and on relief finer than a voxel (Sponza's
// lion) a ray that left at a shallow angle ended on the surface beside it.
float lighting_cascade_l0_lift() {
    return max(0.02, 2.0 * mf_skin() + 0.01);
}

// Direction of level-0 ray k: k < CASCADE_L0_RAYS is the uniform set,
// the rest are cone rays, fixed per voxel and ring slot like the set.
vec3 lighting_cascade_l0_ray(uint key, vec3 n, uint k, bool has_cone, vec3 axis, float cos_a) {
    if (k < CASCADE_L0_RAYS) {
        return lighting_cascade_l0_dir(key, n, k);
    }
    uint j = k - CASCADE_L0_RAYS;
    uint h = lighting_hash(key ^ lighting_hash(j ^ lighting_hash(lighting_frame() % CASCADE_L0_SLOTS)));
    return lighting_cascade_cone_dir(axis, cos_a, j, CASCADE_CONE_L0, h);
}

uint lighting_cascade_l0_args_at() {
    return lighting_cascade_counts_at() + 2u;
}

uint lighting_cascade_l0_list_at() {
    return lighting_cascade_counts_at() + 6u;
}

// Add a seated voxel to this frame's dispatch list.
void lighting_cascade_l0_list_add(uint slot) {
    uint i = atomicAdd(ckeys.key[lighting_cascade_l0_args_at()], 1u);
    if (i < CASCADE_LEVEL0_CAP) {
        ckeys.key[lighting_cascade_l0_list_at() + i] = slot;
    }
}

// The table slot work group `group` of the trace and shade passes runs.
uint lighting_cascade_l0_list_slot(uint group) {
    if (group >= CASCADE_LEVEL0_CAP) {
        return ~0u;
    }
    return ckeys.key[lighting_cascade_l0_list_at() + group];
}

uint lighting_cascade_hit_at(uint slot, uint k) {
    return lighting_cascade_counts_at() + CASCADE_L0_COUNT_WORDS
        + (slot * CASCADE_L0_TRACED + k) * CASCADE_L0_HIT_WORDS;
}

// Upper refresh list, after the level-0 hit records. A claimed bit per
// world-cache slot, then a header (period, flags, per-band counts) and
// three due lists (fine / mid / coarse), then one hit record per due
// slot and ray in the band being shaded.
const uint CASCADE_CLAIMED_WORDS = 32768u;
const uint CASCADE_UPPER_CAP = 16384u;
const uint CASCADE_UPPER_RAYS_MAX = 64u;
const float CASCADE_EXTENT_FACTOR = 16.0;
// Share of a refresh an upper cell takes (lighting_cascade_upper_shade.comp).
// 0.05 keeps a still night scene from shimmering now that each refresh traces
// new directions.
const float CASCADE_UPPER_RATE = 0.05;
// A cell the scene has just changed under (a lamp that reaches it moved,
// changed colour or went out since its last refresh; a mover; a change in
// what lights the scene, lighting_lit_changed, which a camera move is not) takes half of each refresh, and all of one that differs by more
// than CASCADE_UPPER_JUMP of its irradiance. Until 23 Nov 2026 the jump was
// taken on the size of the difference alone. In a dim cell one refresh of a
// few rays differs by more than that from the next by chance, so a still
// cell took a whole new estimate every few refreshes: patches of bounce
// light that came and went on a still picture.
// An unchanged cell goes on averaging: after 1 / CASCADE_UPPER_RATE refreshes
// its share is one over its refresh count, down to one in CASCADE_UPPER_LONG.
// At a fixed 0.05 a dim cell's mean kept wandering by about a sixth of one
// refresh's noise, a slow drift of soft patches. The count is the w of the
// cell's irradiance word. A change puts it back to the warm count.
const float CASCADE_UPPER_LONG = 128.0;
const float CASCADE_UPPER_CHANGE_RATE = 0.5;
// Frames after a lamp change that a cell it reaches stays at that rate.
const uint CASCADE_CHANGE_FRAMES = 32u;
const float CASCADE_UPPER_JUMP = 0.9;
// Refreshes an upper cell takes whole, with fixed directions, before it is
// damped and its directions change each refresh.
const float CASCADE_UPPER_WARM = 8.0;

uint lighting_cascade_claimed_at() {
    return lighting_cascade_counts_at() + CASCADE_L0_COUNT_WORDS
        + CASCADE_LEVEL0_CAP * CASCADE_L0_TRACED * CASCADE_L0_HIT_WORDS;
}

uint lighting_cascade_upper_head_at() {
    return lighting_cascade_claimed_at() + CASCADE_CLAIMED_WORDS;
}

// Word 0: period (low 8 bits), current dispatch band (bits 16-17).
// Words 1..3: due counts for size bands 0 (S<=0.5), 1 (S<=4), 2 (S>4),
// used only to merge coarse to fine this frame. Every cell shares one P.
// Words 4..12: one indirect dispatch command (groups, 1, 1) per band, so a
// band dispatches its due cells and not 16384 groups.
const uint CASCADE_UPPER_HEAD_WORDS = 16u;
uint lighting_cascade_upper_args_at(uint band) {
    return lighting_cascade_upper_head_at() + 4u + band * 3u;
}

uint lighting_cascade_upper_list_at(uint band) {
    return lighting_cascade_upper_head_at() + CASCADE_UPPER_HEAD_WORDS + band * CASCADE_UPPER_CAP;
}

uint lighting_cascade_upper_hit_at(uint due, uint k) {
    return lighting_cascade_upper_head_at() + CASCADE_UPPER_HEAD_WORDS + 3u * CASCADE_UPPER_CAP
        + (due * CASCADE_UPPER_RAYS_MAX + k) * CASCADE_L0_HIT_WORDS;
}

uint lighting_cascade_refresh_period(uint setting) {
    if (setting == 1u || setting == 2u || setting == 4u || setting == 10u) {
        return setting;
    }
    return 4u;
}

// Uniform rays of an upper refresh. CASCADE_CONE_UPPER lamp-cone rays
// follow them (lighting_cascade_upper_ray); the coarse band gives up 8
// uniform rays for them to stay within CASCADE_UPPER_RAYS_MAX.
uint lighting_cascade_refresh_rays(float size) {
    if (size <= 0.5) {
        return 16u;
    }
    if (size <= 4.0) {
        return 32u;
    }
    return 56u;
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

// Jitter seed of an upper cell's refresh. A warm cell (CASCADE_UPPER_WARM
// refreshes) takes new directions each frame, which the damped refresh
// (CASCADE_UPPER_RATE) averages. A jitter fixed per cell kept the same one to
// four rays a bin, and a ray that hit a small bright patch (the ceiling
// 0.4 m over a lamp) kept its bin bright: a fixed bright patch on the floor
// under the cell. A cold cell keeps the fixed jitter: its running mean has few
// refreshes, and new directions made a still wall shimmer in its first frames.
uint lighting_cascade_bin_seed(uint slot) {
    bool warm = cache.v[3u * slot + 2u].w > CASCADE_UPPER_WARM - 0.5;
    return warm ? lighting_hash(lighting_frame()) : 0u;
}

// Direction of ray k of an upper cell's refresh: bin k % 16, jittered per
// key, ray and seed (lighting_cascade_bin_seed).
vec3 lighting_cascade_bin_dir(uint key, vec3 n, uint k, uint seed) {
    uint b = k % CASCADE_BINS;
    uint h = lighting_hash(key ^ (k * 0x9e3779b9u) ^ seed);
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

// Direction of upper ray k: k < uniform is bin k % 16, the rest are cone
// rays with the same seed as the bins (lighting_cascade_bin_seed).
vec3 lighting_cascade_upper_ray(uint key, vec3 n, uint k, uint uniform_count, uint seed,
                                vec3 axis, float cos_a) {
    if (k < uniform_count) {
        return lighting_cascade_bin_dir(key, n, k, seed);
    }
    uint j = k - uniform_count;
    uint h = lighting_hash(key ^ (j * 0x9e3779b9u) ^ seed ^ 0x51ed270bu);
    return lighting_cascade_cone_dir(axis, cos_a, j, CASCADE_CONE_UPPER, h);
}

// The bin that holds `dir`. A bin is the mean radiance of its own pi/8 sr,
// so a ray reads that bin alone. Blending the 90-degree neighbours smeared
// the sunlit ground below the horizon into the empty directions above it:
// a shadowed wall facing sunlit ground took twice the light it should.
uint lighting_cascade_bin_of(vec3 dir, vec3 n) {
    vec3 tx;
    vec3 ty;
    lighting_cascade_bin_frame(n, tx, ty);
    float u = clamp(dot(dir, n), 0.0, 1.0);
    float phi = atan(dot(dir, ty), dot(dir, tx));
    if (phi < 0.0) {
        phi += LIGHTING_TAU;
    }
    uint row = uint(clamp(floor(u * CASCADE_BIN_SIDE), 0.0, CASCADE_BIN_SIDE - 1.0));
    uint col = uint(clamp(floor(phi / LIGHTING_TAU * CASCADE_BIN_SIDE), 0.0, CASCADE_BIN_SIDE - 1.0));
    return row * 4u + col;
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

// Which pixel seats a voxel. Every pixel of a cell offers a rank: the
// distance from the cell centre in the high bits (nearer is larger), the
// pixel index in the low bits. atomicMax keeps the nearest pixel, and the
// lowest distance step with the highest index on a tie, so a still view
// seats a voxel from the same pixel every frame. The first pixel to claim
// the slot used to seat it, and that order is a race.
uint lighting_cascade_owner_bits() {
    return uint(findMSB(max(pc.dims.x * pc.dims.y, 2u) - 1u)) + 1u;
}

uint lighting_cascade_owner_rank(uint pixel, vec3 pos, vec3 centre, float size) {
    uint pb = lighting_cascade_owner_bits();
    uint levels = (1u << (32u - pb)) - 1u;
    float d = clamp(length(pos - centre) / (CASCADE_OWNER_SPAN * size), 0.0, 1.0);
    uint q = levels - min(uint(d * float(levels - 1u)), levels - 1u);
    return (q << pb) | pixel;
}

uint lighting_cascade_insert_level(uint pixel, vec3 pos, vec3 n, uint level) {
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
    uint rank = lighting_cascade_owner_rank(pixel, pos, lighting_cascade_l0_centre(cell, size), size);
    [[dont_unroll]] for (uint i = 0u; i < CASCADE_LEVEL0_PROBE; i++) {
        uint slot = (home + i) & mask;
        uint word = base + slot * CASCADE_LEVEL0_STRIDE;
        uint prev = atomicCompSwap(ckeys.key[word], 0u, key);
        if (prev != 0u && prev != key) {
            continue;
        }
        // Word 9 holds the owner rank until lighting_cascade_seat replaces it
        // with the normal. The frame's clear left it 0, below every rank.
        atomicMax(ckeys.key[word + 9u], rank);
        if (prev == key) {
            return word;
        }
        ckeys.key[word + 1u] = frame;
        ckeys.key[word + 2u] = uint(cell.x);
        ckeys.key[word + 3u] = uint(cell.y);
        ckeys.key[word + 4u] = uint(cell.z);
        ckeys.key[word + 5u] = packed;
        return word;
    }
    return ~0u;
}

// Seat a claimed voxel: write the point its rays start from and its normal.
// The point is the cell centre moved along the axis onto the owner pixel's
// surface, when the picture shows that surface there. The half-cell inset
// puts the cell behind a surface that sits in the outer half of its cell, so
// the +axis face can be inside a wall that is not on a grid plane; rays from
// there see nothing. A centre the picture shows as another surface, and
// that a short ray finds no surface under (under a floor the wall stands
// on, inside a box, past an edge), is not on the surface, so the voxel
// takes the owner pixel's own point. A side-face voxel
// seated 12 cm under the floor read 0.4 against 3 for one frame, and the
// pixels over it went dark. A centre off the picture takes the ray test too.
// True when `p` lies on a surface facing `n`: a short ray from just in
// front of it, back along the normal, meets one. A centre the picture does
// not show (behind a wall, behind a box) is still a real point of a floor
// that carries on there, and keeps the seat it always had.
bool lighting_cascade_seat_real(vec3 p, vec3 n, float facing) {
    vec3 origin = p + n * CASCADE_SEAT_PROBE;
    if (scene_inside(origin)) {
        return false;
    }
    SceneHit hit;
    if (!trace_ray(origin, -n, CASCADE_SEAT_NEAR, 2.0 * CASCADE_SEAT_PROBE, hit)) {
        return false;
    }
    return abs(dot(hit.normal, n)) > facing;
}

// True when the picture shows, at the pixel `p` projects to, a surface that
// faces `n` and passes through `p`.
bool lighting_cascade_seat_shown(vec3 p, vec3 n, float size, float facing) {
    vec3 rel = p - scene.eye.xyz;
    float z = dot(rel, scene.view_forward.xyz);
    float t = scene.view_right.w;
    float aspect = scene.view_forward.w;
    if (!(z > CASCADE_SEAT_NEAR && t > 0.0)) {
        return false;
    }
    // Half the picture's height at that depth; a parallel view has one height.
    float span = scene.view_up.w > 0.0 ? scene.view_up.w : z * t;
    vec2 ndc = vec2(dot(rel, scene.view_right.xyz) / (span * aspect), dot(rel, scene.view_up.xyz) / span);
    vec2 px = vec2((ndc.x + 1.0) * 0.5 * float(pc.dims.x), (1.0 - ndc.y) * 0.5 * float(pc.dims.y));
    if (!(px.x >= 0.0 && px.y >= 0.0 && px.x < float(pc.dims.x) && px.y < float(pc.dims.y))) {
        return false;
    }
    vec3 spos;
    vec3 sn;
    vec3 albedo;
    bool two_sided;
    bool shown = lighting_surface(uvec2(px), spos, sn, albedo, two_sided);
    return shown && dot(sn, n) > facing
        && abs(dot(spos - p, n)) <= CASCADE_SEAT_PLANE * size + CASCADE_SEAT_PLANE_MIN;
}

void lighting_cascade_seat(uint word) {
    uint pb = lighting_cascade_owner_bits();
    uint pixel = ckeys.key[word + 9u] & ((1u << pb) - 1u);
    vec3 pos;
    vec3 n;
    vec3 albedo;
    bool two_sided;
    if (pixel >= pc.dims.x * pc.dims.y
        || !lighting_surface(uvec2(pixel % pc.dims.x, pixel / pc.dims.x), pos, n, albedo, two_sided)) {
        // No owner surface: leave the voxel unlit rather than seat it at 0.
        ckeys.key[word] = 0u;
        return;
    }
    uint packed = ckeys.key[word + 5u];
    uint axis = lighting_cascade_unpack_axis(packed);
    vec3 n_axis = lighting_cascade_axis_vec(axis);
    float size = lighting_cascade_l0_size(lighting_cascade_unpack_level(packed));
    ivec3 cell = ivec3(int(ckeys.key[word + 2u]), int(ckeys.key[word + 3u]), int(ckeys.key[word + 4u]));
    vec3 centre = lighting_cascade_l0_centre(cell, size);
    float along = dot(pos - centre, n) / max(dot(n_axis, n), CASCADE_EPS);
    vec3 stored = centre + n_axis * clamp(along, -size, size);
    // A voxel that was live last frame keeps that seat and that normal while
    // the seat is still on a surface that faces the same way. Until 23 Nov
    // 2026 every frame took both from the owner pixel. The owner changes with
    // each small turn of the camera, and its point and its normal-mapped
    // normal with it: on a pan about one voxel in ten kept its seat from one
    // step to the next (the vault in Sponza, 0.006 rad a step), the fixed ray
    // set turned and moved with the seat, and 15% of the voxels changed their
    // light by more than a tenth each step. That is the patches of bounce
    // light that crawl while the camera pans and stop when it stops.
    uint was = lighting_cascade_find_in((lighting_frame() + 1u) & 1u, lighting_frame(), cell,
        lighting_cascade_unpack_level(packed), axis);
    if (was != ~0u) {
        vec3 was_p = vec3(
            uintBitsToFloat(ckeys.key[was + 6u]),
            uintBitsToFloat(ckeys.key[was + 7u]),
            uintBitsToFloat(ckeys.key[was + 8u]));
        vec3 was_n = lighting_unpack_normal(ckeys.key[was + 9u]);
        // The kept normal is one pixel's normal-mapped normal, so the test
        // that it still fits is looser than the test that seats a voxel.
        // On the owner pixel's own plane and within a cell of it is the
        // same patch of surface: an edge voxel, whose seat is an owner's
        // point, passes here when the picture and the ray tests do not.
        bool beside = abs(dot(was_p - pos, n)) <= CASCADE_SEAT_PLANE * size + CASCADE_SEAT_PLANE_MIN
            && distance(was_p, pos) <= size;
        if (dot(was_n, n) > CASCADE_SEAT_KEEP
            && (beside
                || lighting_cascade_seat_shown(was_p, was_n, size, CASCADE_SEAT_KEEP)
                || lighting_cascade_seat_real(was_p, was_n, CASCADE_SEAT_KEEP))) {
            ckeys.key[word + 6u] = floatBitsToUint(was_p.x);
            ckeys.key[word + 7u] = floatBitsToUint(was_p.y);
            ckeys.key[word + 8u] = floatBitsToUint(was_p.z);
            ckeys.key[word + 9u] = ckeys.key[was + 9u];
            return;
        }
    }
    bool same = lighting_cascade_seat_shown(stored, n, size, CASCADE_SEAT_FACING);
    if (!same && !lighting_cascade_seat_real(stored, n, CASCADE_SEAT_FACING)) {
        stored = pos;
    }
    ckeys.key[word + 6u] = floatBitsToUint(stored.x);
    ckeys.key[word + 7u] = floatBitsToUint(stored.y);
    ckeys.key[word + 8u] = floatBitsToUint(stored.z);
    ckeys.key[word + 9u] = lighting_pack_normal(n);
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

// How much closer to the cell's axis a normal has to be to take a seated
// parent cell (lighting_cascade_parent_corner).
const float CASCADE_SEAT_GAIN = 0.02;
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
        // Every level-0 voxel under a parent offers its own normal and its
        // own point on the cell. Until 23 Nov 2026 each one wrote them, and
        // the last writer of the frame won: a race. On a curved surface
        // (Sponza's curtains) the parent's seat changed at random from frame
        // to frame, its gates passed and failed with it, and the voxels that
        // read its bins pulsed: soft patches of light that came and went.
        // Now the seat is kept. It is taken while the cell has no light yet,
        // when it belongs to a surface that faces the other way, and by a
        // normal that is closer to the cell's axis. That order does not
        // depend on which thread ran last, so a still scene keeps one seat.
        vec3 had_n = cache.v[3u * slot + 1u].xyz;
        bool empty = !(cache.v[3u * slot + 2u].w > LIGHTING_CACHE_HAS_W);
        bool take = empty
            || !(dot(had_n, n) > 0.0)
            || abs(n[ax]) > abs(had_n[ax]) + CASCADE_SEAT_GAIN;
        if (take) {
            cache.v[3u * slot + 1u].xyz = n;
            float off = dot(cache.v[3u * slot].xyz - q, n);
            if (off < -CASCADE_RESEAT || off > LIGHTING_CELL_HALF * size) {
                cache.v[3u * slot].xyz = q;
            }
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
        // Both ends stand clear of a thin field's shell (mf_skin: half a
        // voxel to each side, 3 cm in a level field). At 2 cm they were
        // inside it, and the ray to a parent on the same flat surface ran
        // along inside the shell and counted as blocked. A floor voxel then
        // had no parent at any size and read the sky: in Sponza's upper
        // gallery 172 of 953 voxels in one view.
        float lift = max(0.02, 2.0 * mf_skin() + 0.01);
        if (trace_occluded(pos + n * lift, cpos + n * lift)) {
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

// lighting_cascade_parent_bins, or the nearest coarser ancestor's bins when
// the exact parents are missing, unshaded, or all behind the leak gates.
// Until 23 Nov 2026 a probe with no exact parent read the sky colour for
// every ray that left its interval, indoors too. A level-0 voxel on a wall
// in Sponza whose 25 cm parents were gated out read (0.33, 0.41, 0.54), the
// sky times pi, beside neighbours at 0.10: a cool patch one voxel across. A
// coarser ancestor's interval starts farther out, so the light between the
// two intervals is missed; that is less wrong than the sky. `ok` is false
// only when no ancestor up to 32 m has light, and then the caller's sky is
// the open air.
void lighting_cascade_coarser_bins(vec3 pos, vec3 n, float child_size, bool seat, out vec3 bins[16], out bool ok) {
    float size = child_size;
    ok = false;
    [[dont_unroll]] for (uint i = 0u; i < 9u; i++) {
        lighting_cascade_parent_bins(pos, n, size, seat, bins, ok);
        if (ok || size * CASCADE_PARENT_SCALE > 32.0 + CASCADE_EPS) {
            return;
        }
        size *= CASCADE_PARENT_SCALE;
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
    t1 = min(32.0 * max(size, CASCADE_EPS), CASCADE_TRACE_FAR);
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
vec4 lighting_cascade_at_axis(uint parity, uint stamp, vec3 pos, vec3 n, uint level, uint axis) {
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

// A voxel belongs to one of six families, the axis its surface faces most.
// A pixel used to read only the family of its own normal's largest
// component. Each family holds the light for its own side: on Sponza's lion,
// under a vault, the forward-facing voxels read 0.55 and the up-facing ones
// 0.14. A normal that tipped past 45 degrees changed family and the pixel
// stepped by that much, so the relief, and its normal map, showed darker
// patches with hard edges. The pixel now mixes the families its normal has a
// share of, by the squared component (they sum to one) less a small floor, so
// the light turns with the normal. A family with no voxel there drops out.
const float CASCADE_AXIS_FLOOR = 0.05;
vec4 lighting_cascade_at_in(uint parity, uint stamp, vec3 pos, vec3 n, uint level) {
    vec3 share = max(n * n - vec3(CASCADE_AXIS_FLOOR), vec3(0.0));
    vec3 acc = vec3(0.0);
    float wsum = 0.0;
    [[dont_unroll]] for (uint a = 0u; a < 3u; a++) {
        if (!(share[a] > 0.0)) {
            continue;
        }
        uint axis = 2u * a + (n[a] >= 0.0 ? 0u : 1u);
        vec4 v = lighting_cascade_at_axis(parity, stamp, pos, n, level, axis);
        if (v.w > LIGHTING_CACHE_HAS_W) {
            acc += share[a] * v.rgb;
            wsum += share[a];
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
    // The footprint level, or the finer one, and never the coarser one. A
    // floor cell twice the size, seated on the lit side of a 40 cm wall,
    // lit the hits at the wall's foot on the dark side: the base of the back
    // face read 68 against a limit of 45
    // (a_close_lamp_does_not_cross_the_wall, with the 12.5 cm base grid).
    // The picture still blends the two levels. A hit with neither takes the
    // upper cell below, through its leak gates.
    vec4 shown = near;
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
    uint word = lighting_cascade_insert_level(pixel, pos, n, level);
    uint word_next = ~0u;
    if (level < CASCADE_L0_MAX) {
        word_next = lighting_cascade_insert_level(pixel, pos, n, level + 1u);
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
