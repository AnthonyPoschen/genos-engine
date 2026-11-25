// The cone-traced cascades (experimental, lighting key `cone_cascades`):
// the voxel field (crates/render/src/cone_field.rs), the cone march, the
// direction bins, the probe hash and the work buffer's layout. Every cone
// pass includes lighting_common.glsl first, then this.
//
// Push constant for the cone passes: params.x the cascade, params.y theta at
// cascade 0, params.z the cascade count, params.w the probe size in pixels;
// lamp.x flags (bit 0 visibility in the merge, bit 1 visibility in the shade,
// bit 2 seats on the pixels' mean: the gate knockout of the field seat),
// lamp.y the first reach t0 in probe spacings, as float bits; lamp.z the
// lamps' sphere radius in metres, as float bits.

// head: atlas texels xyz, coarse bricks. lv[2k]: brick origin xyz and the
// table offset of level k + 2; lv[2k + 1]: the table's dims in bricks.
layout(std430, set = 0, binding = 1) readonly buffer ConeTable {
    uvec4 head;
    ivec4 lv[16];
    uint table[];
} ctab;
// 33 words a fine brick: first record, 16 mask words, 16 counts before.
layout(std430, set = 0, binding = 2) readonly buffer ConeFine {
    uint fb[];
} cfine;
// 33 words a fine voxel: light and area for each way (2 words), its plane
// (3 words), then the lamp term (2 words), then its seat word.
layout(std430, set = 0, binding = 3) readonly buffer ConeRecords {
    uint fv[];
} crec;
// 16 words a coarse texel: light and area for each way and the lamps
// (2 words each, 7 of them), then the occupancy word, then the seat word.
layout(std430, set = 0, binding = 4) readonly buffer ConeCoarse {
    uint cw[];
} ccoarse;
layout(std430, set = 0, binding = 15) buffer ConeWork {
    uint w[];
} cwork;

const float CONE_BASE = 1.0 / 64.0;
const int CONE_FINEST = 2;
const int CONE_COARSEST = 9;
// The levels a crossing reads, and the refine level of the walk.
const int CONE_REFINE = 3;
const int CONE_BRICK = 8;
const int CONE_PADDED = 10;
const uint CONE_ATLAS_BRICKS = 32u;
const uint CONE_NO_BRICK = 0xffffffffu;
const uint CONE_HEADER = 33u;
const uint CONE_RECORD = 33u;
// A seat word (cone_field.rs seat_word_of): bit 31 set when the voxel has
// surface, 10 bits an axis over its cell from half a voxel below to half past.
const uint CONE_SEAT_WORD = 32u;
const uint CONE_TEXEL_SEAT = 15u;
// A level is chosen when its voxel is at least this share of the wanted
// size: the nearest level in log2, as the rounding before had it.
const float CONE_NEAREST = 0.70710678;
// A probe's cell is the voxel a quarter of a voxel (at most 6.25 cm) behind
// the surface, where the field filed that surface's points.
const float CONE_BEHIND = 0.25;
// The knockout's pixel sums: points relative to the eye in 1/256 m.
const float CONE_FIXED = 256.0;
const uint CONE_PIXEL_SEAT = 4u;
const uint CONE_TEXEL = 16u;
const float CONE_REACH = 200.0;
const float CONE_FULL = 0.98;
const float CONE_GROWTH = 4.0;
const float CONE_TOWARD_MIN = 0.05;
const float CONE_CROSS_FULL = 0.5;
const float CONE_BOX_MARGIN = 0.03125;
const float CONE_LIFT = 0.75;
const float CONE_START_GAP = 0.25;
const float CONE_HALF = 0.5;
const float CONE_OPEN_MIN = 0.02;
const float CONE_PI = 3.14159265;
const float CONE_TAU = 6.28318531;
// The whole sphere's solid angle.
const float CONE_SPHERE = 12.5663706;
// Smallest lengths and lumas worth dividing by.
const float CONE_TINY = 1.0e-6;
const float CONE_TINIER = 1.0e-9;
const float CONE_FAR = 1.0e30;
// The screen's height is two half-heights of the view's tangent.
const float CONE_HALVES = 2.0;
const float CONE_SEAT_HELD = 0.3;
const float CONE_SURFACE_LIFT = 0.02;
const int CONE_MAX_LOD = 12;
const int CONE_KEY_SPAN = 256;
const uint CONE_HASH_PROBES = 32u;
const vec3 CONE_LUMA = vec3(0.2126, 0.7152, 0.0722);
// Work buffer: counters, then for each cascade its hash, probes and cones.
const uint CONE_COUNTS = 0u;    // probes kept, per cascade
const uint CONE_DROPPED = 8u;   // probes past the cascade's cap in compact
const uint CONE_TRACED = 16u;   // cones traced, per cascade
const uint CONE_CLOSED = 24u;   // cones the chain closed, per cascade
const uint CONE_SEAT_FALLBACK = 40u; // probes the field had no seat for (seated at the cell centre)
const uint CONE_LAMP_DROPPED = 48u; // lamp points that found no lamp-hash slot
const uint CONE_LAMP_LAYOUT = 89u;  // the lamp hash's size and word offset
const uint CONE_LAYOUT = 64u;   // per cascade 5 words: hash size, cap, hash, probes, cones (cone_gpu.rs)
const uint CONE_CLEARED = 64u;  // header words cleared every frame
const uint CONE_INSERT_DROPPED = 32u; // pixels whose probe found no hash slot, per cascade
const uint CONE_HEADER_WORDS = 256u;
const uint CONE_HASH_WORDS = 9u;   // key, count, sum xyz, normal sum xyz, probe
const uint CONE_PROBE_WORDS = 32u; // see cone_probe_* below
const uint CONE_CONE_WORDS = 4u;   // half2 rg, half2 b through, half2 moment xy, half2 moment z filled
const uint CONE_EMPTY = 0u;

float cone_size(int level) {
    return CONE_BASE * exp2(float(level));
}
float cone_spacing(int lod, int c) {
    return CONE_BASE * exp2(float(lod + c));
}
uint cone_theta(int c) {
    return pc.params.y << uint(c);
}
uint cone_dirs(int c) {
    uint t = cone_theta(c);
    return 2u * t * t;
}
// Cascade c's table sizes and word offsets, sized on the CPU from the voxel
// layer's cap and written into the header every frame.
uint cone_hash_size(int c) {
    return cwork.w[CONE_LAYOUT + 5u * uint(c)];
}
uint cone_cap(int c) {
    return cwork.w[CONE_LAYOUT + 5u * uint(c) + 1u];
}
uint cone_hash_at(int c) {
    return cwork.w[CONE_LAYOUT + 5u * uint(c) + 2u];
}
uint cone_probes_at(int c) {
    return cwork.w[CONE_LAYOUT + 5u * uint(c) + 3u];
}
uint cone_cones_at(int c) {
    return cwork.w[CONE_LAYOUT + 5u * uint(c) + 4u];
}

// A probe's record: 0 key, 1 lod, 2 the chain child (the probe of cascade
// c - 1 its cones start from, or CONE_NO_BRICK), 3 the start's lod, 4..6 its
// cell, 7 its pixel count, 11 1 when the field seated it (CONE_PROBE_SEATED), 8..10 the seat (the field voxel's surface point,
// lifted along its surface's normal when that holds), 12..14 the cones' start point (on
// the voxel layer), 16..18 that point's normal; 20..27 the probes of the next
// cascade the merge reads and 28.. unused.
const uint CONE_PROBE_SEATED = 11u;
// No cell: a cone_weights `own` that matches none.
const ivec3 CONE_NO_CELL = ivec3(-2147483647);
uint cone_probe(int c, uint i) {
    return cone_probes_at(c) + i * CONE_PROBE_WORDS;
}
vec3 cone_word3(uint at) {
    return vec3(uintBitsToFloat(cwork.w[at]), uintBitsToFloat(cwork.w[at + 1u]), uintBitsToFloat(cwork.w[at + 2u]));
}
void cone_put3(uint at, vec3 v) {
    cwork.w[at] = floatBitsToUint(v.x);
    cwork.w[at + 1u] = floatBitsToUint(v.y);
    cwork.w[at + 2u] = floatBitsToUint(v.z);
}

// The 32-bit key of a probe: lod and the cell's place relative to the eye's
// cell at that spacing (9 bits an axis), or 0 when the cell is out of range.
uint cone_key(int lod, ivec3 cell, int c) {
    ivec3 eye = ivec3(floor(scene.eye.xyz / cone_spacing(lod, c)));
    ivec3 rel = cell - eye + ivec3(CONE_KEY_SPAN);
    if (any(lessThan(rel, ivec3(0))) || any(greaterThanEqual(rel, ivec3(2 * CONE_KEY_SPAN)))) {
        return CONE_EMPTY;
    }
    return 0x80000000u | (uint(lod) << 27u) | (uint(rel.x) << 18u) | (uint(rel.y) << 9u) | uint(rel.z);
}
uint cone_hash_of(uint key) {
    key ^= key >> 16u;
    key *= 0x7feb352du;
    key ^= key >> 15u;
    key *= 0x846ca68bu;
    key ^= key >> 16u;
    return key;
}
// The hash slot (word offset) of `key` in cascade c, claimed when `insert`.
uint cone_slot(int c, uint key, bool insert) {
    uint size = cone_hash_size(c);
    uint base = cone_hash_at(c);
    uint h = cone_hash_of(key) & (size - 1u);
    for (uint k = 0u; k < CONE_HASH_PROBES; k++) {
        uint at = base + ((h + k) & (size - 1u)) * CONE_HASH_WORDS;
        uint seen = insert ? atomicCompSwap(cwork.w[at], CONE_EMPTY, key) : cwork.w[at];
        if (seen == key || (insert && seen == CONE_EMPTY)) {
            return at;
        }
        if (seen == CONE_EMPTY) {
            return CONE_NO_BRICK;
        }
    }
    return CONE_NO_BRICK;
}
// The probe index of `key` in cascade c, or CONE_NO_BRICK.
uint cone_find(int c, uint key) {
    if (key == CONE_EMPTY) {
        return CONE_NO_BRICK;
    }
    uint at = cone_slot(c, key, false);
    return at == CONE_NO_BRICK ? CONE_NO_BRICK : cwork.w[at + 8u];
}

// The cell of spacing `size` a surface point at `pos` facing `n` stands in:
// the voxel the field filed it in.
ivec3 cone_cell(vec3 pos, vec3 n, float size) {
    return ivec3(floor((pos - n * (CONE_BEHIND * min(size, CONE_BEHIND))) / size));
}
// From this cascade up, a probe merges only its parent (which it put there
// itself); below it, the eight around it (cone_spread.comp,
// cone_weights.comp). The coarse cascades hold most of the cones (four times
// the directions each), and a rim probe there has no children to close its
// cones early: neighbours up to cascade 2 cost the hall 62 ms a frame at
// 1080p, up to cascade 1 50 ms (section 18, step 4b).
const int CONE_PARENT_ONLY = 2;
// A pixel's probe level: the finest level whose voxel is about `probe_px`
// pixels wide or more at the pixel's distance from the eye. Every frame from
// where the eye is, so a turn of the head changes nothing and a step moves a
// level boundary a little, never in a jump.
int cone_pixel_lod(vec3 pos) {
    float per_pixel = CONE_HALVES * scene.view_right.w / float(pc.dims.y);
    float px = float(pc.params.w);
    float reach = max(length(pos - scene.eye.xyz), CONE_TINY);
    float level = log2(CONE_NEAREST * px * per_pixel * reach / CONE_BASE);
    return clamp(int(ceil(level)), CONE_FINEST, CONE_MAX_LOD);
}

// Direction bins: 2 theta columns of azimuth by theta rows of z.
uint cone_dir_bin(vec3 d, uint theta) {
    float u = fract(atan(d.y, d.x) / CONE_TAU);
    float v = clamp(CONE_HALF * (d.z + 1.0), 0.0, 1.0);
    uint x = min(uint(u * float(2u * theta)), 2u * theta - 1u);
    uint y = min(uint(v * float(theta)), theta - 1u);
    return y * 2u * theta + x;
}
vec3 cone_bin_dir(uint bin, uint theta) {
    float u = (float(bin % (2u * theta)) + CONE_HALF) / float(2u * theta);
    float v = (float(bin / (2u * theta)) + CONE_HALF) / float(theta);
    float phi = u * CONE_TAU;
    float z = v * CONE_HALVES - 1.0;
    float r = sqrt(max(1.0 - z * z, 0.0));
    return vec3(r * cos(phi), r * sin(phi), z);
}

// ---- The lamps ----
// Every point lamp is written each frame into a hash keyed by field level and
// cell (cone_lamps.comp), so the march reads a lamp where it is now. An entry:
// key, then the area and the light its points add, in fixed point.

const uint CONE_LAMP_POINTS = 256u;
// The engine's lamp unit: a lamp's colour times this is its power (cone_field.rs).
const float CONE_LAMP_UNIT = 72.0;
const float CONE_LAMP_AREA_Q = 1.0e-8;
const float CONE_LAMP_LIGHT_Q = 1.0e-4;
const uint CONE_LAMP_WORDS = 5u;
// A lamp key holds the cell relative to the eye's cell at that level: 10 bits
// across, 9 up (about 32 m and 16 m at 6.25 cm, more at the coarser levels).
const int CONE_LAMP_SPAN_XZ = 512;
const int CONE_LAMP_SPAN_Y = 256;

uint cone_lamp_size() {
    return cwork.w[CONE_LAMP_LAYOUT];
}
uint cone_lamp_at() {
    return cwork.w[CONE_LAMP_LAYOUT + 1u];
}
// The plastic sequence's point `i` on the unit sphere (cone_field.rs r2 and decode_dir).
vec3 cone_lamp_dir(uint i) {
    const float g = 1.324718;
    float u = fract(0.5 + float(i) / g);
    float v = fract(0.5 + float(i) / (g * g));
    float phi = u * CONE_TAU;
    float z = 2.0 * v - 1.0;
    float r = sqrt(max(1.0 - z * z, 0.0));
    return vec3(r * cos(phi), r * sin(phi), z);
}
uint cone_lamp_key(int level, ivec3 cell) {
    ivec3 rel = cell - ivec3(floor(scene.eye.xyz / cone_size(level)));
    ivec3 span = ivec3(CONE_LAMP_SPAN_XZ, CONE_LAMP_SPAN_Y, CONE_LAMP_SPAN_XZ);
    if (any(lessThanEqual(rel, -span)) || any(greaterThanEqual(rel, span))) {
        return CONE_EMPTY;
    }
    uvec3 q = uvec3(rel + span);
    return (uint(level - CONE_FINEST) << 29u) | (q.x << 19u) | (q.y << 10u) | q.z;
}
// The lamp-hash slot (word offset) of `key`, claimed when `insert`.
uint cone_lamp_slot(uint key, bool insert) {
    uint size = cone_lamp_size();
    if (size == 0u) {
        return CONE_NO_BRICK;
    }
    uint h = cone_hash_of(key) & (size - 1u);
    for (uint k = 0u; k < CONE_HASH_PROBES; k++) {
        uint at = cone_lamp_at() + ((h + k) & (size - 1u)) * CONE_LAMP_WORDS;
        uint seen = insert ? atomicCompSwap(cwork.w[at], CONE_EMPTY, key) : cwork.w[at];
        if (seen == key || (insert && seen == CONE_EMPTY)) {
            return at;
        }
        if (seen == CONE_EMPTY) {
            return CONE_NO_BRICK;
        }
    }
    return CONE_NO_BRICK;
}
// Lamp presence: each lamp also claims the cells of the coarsest level within
// CONE_LAMP_NEAR of the one its centre is in (area 0, so a read there is
// unchanged), so a coarse step far from every lamp skips its eight lamp reads
// after one lookup. Two cells: a level-9 step's corners reach one cell past
// its own, and a sphere of at most 1 m one past its centre's.
const int CONE_LAMP_NEAR_LEVEL = CONE_COARSEST;
const int CONE_LAMP_NEAR = 2;
uint cone_lamp_near_key(ivec3 cell) {
    return cone_lamp_key(CONE_LAMP_NEAR_LEVEL, cell);
}
bool cone_lamp_near(vec3 at) {
    uint key = cone_lamp_near_key(ivec3(floor(at / cone_size(CONE_LAMP_NEAR_LEVEL))));
    return key != CONE_EMPTY && cone_lamp_slot(key, false) != CONE_NO_BRICK;
}
// The lamps' area and light in the voxel at `level` and `cell`.
bool cone_lamp_get(int level, ivec3 cell, out float area, out vec3 light) {
    area = 0.0;
    light = vec3(0.0);
    uint key = cone_lamp_key(level, cell);
    if (key == CONE_EMPTY) {
        return false;
    }
    uint at = cone_lamp_slot(key, false);
    if (at == CONE_NO_BRICK) {
        return false;
    }
    area = float(cwork.w[at + 1u]) * CONE_LAMP_AREA_Q;
    light = vec3(cwork.w[at + 2u], cwork.w[at + 3u], cwork.w[at + 4u]) * CONE_LAMP_LIGHT_Q;
    return area > 0.0;
}

// ---- The field ----

// The table entry of a voxel's brick, or CONE_NO_BRICK.
uint cone_entry(int level, ivec3 cell) {
    int k = level - CONE_FINEST;
    ivec4 o = ctab.lv[2 * k];
    ivec3 dims = ctab.lv[2 * k + 1].xyz;
    ivec3 rel = (cell >> 3) - o.xyz;
    if (any(lessThan(rel, ivec3(0))) || any(greaterThanEqual(rel, dims))) {
        return CONE_NO_BRICK;
    }
    return ctab.table[uint(o.w) + uint((rel.z * dims.y + rel.y) * dims.x + rel.x)];
}
// A fine voxel's record (first word), or CONE_NO_BRICK.
uint cone_record(int level, ivec3 cell) {
    uint brick = cone_entry(level, cell);
    if (brick == CONE_NO_BRICK) {
        return CONE_NO_BRICK;
    }
    ivec3 l = cell & ivec3(CONE_BRICK - 1);
    uint bit = uint(l.x + CONE_BRICK * l.y + CONE_BRICK * CONE_BRICK * l.z);
    uint h = brick * CONE_HEADER;
    uint word = bit >> 5u;
    uint mask = 1u << (bit & 31u);
    uint bits = cfine.fb[h + 1u + word];
    if ((bits & mask) == 0u) {
        return CONE_NO_BRICK;
    }
    return (cfine.fb[h] + cfine.fb[h + 17u + word] + bitCount(bits & (mask - 1u))) * CONE_RECORD;
}
// A coarse texel (first word) of the padded brick that holds `cell`, at the
// brick-relative padded index `local` (0..9), or CONE_NO_BRICK.
uint cone_coarse_texel(uint slot, ivec3 local) {
    uvec3 place = uvec3(slot % CONE_ATLAS_BRICKS, (slot / CONE_ATLAS_BRICKS) % CONE_ATLAS_BRICKS, slot / (CONE_ATLAS_BRICKS * CONE_ATLAS_BRICKS));
    uvec3 t = place * uint(CONE_PADDED) + uvec3(local);
    return ((t.z * ctab.head.y + t.y) * ctab.head.x + t.x) * CONE_TEXEL;
}

vec3 cone_axis(int d) {
    float s = (d % 2 == 0) ? 1.0 : -1.0;
    int a = d / 2;
    return a == 0 ? vec3(s, 0.0, 0.0) : (a == 1 ? vec3(0.0, s, 0.0) : vec3(0.0, 0.0, s));
}

// The field voxel's seat at `level` and `cell`: its surface's mean point
// and the area-weighted normal of the ways it faces; `held` is how much those
// ways agree (1 for a flat patch). False where the field has no surface.
bool cone_seat(int level, ivec3 cell, out vec3 mean, out vec3 normal, out float held) {
    mean = vec3(0.0);
    normal = vec3(0.0, 1.0, 0.0);
    held = 0.0;
    if (level < CONE_FINEST || level > CONE_COARSEST) {
        return false;
    }
    uint word;
    uint area_at;
    uint stride;
    if (level <= CONE_REFINE) {
        uint rec = cone_record(level, cell);
        if (rec == CONE_NO_BRICK) {
            return false;
        }
        word = crec.fv[rec + CONE_SEAT_WORD];
        area_at = rec;
        stride = 0u;
    } else {
        uint slot = cone_entry(level, cell);
        if (slot == CONE_NO_BRICK) {
            return false;
        }
        uint t = cone_coarse_texel(slot, (cell & ivec3(CONE_BRICK - 1)) + ivec3(1));
        word = ccoarse.cw[t + CONE_TEXEL_SEAT];
        area_at = t;
        stride = 1u;
    }
    if ((word & 0x80000000u) == 0u) {
        return false;
    }
    float size = cone_size(level);
    uvec3 q = uvec3(word, word >> 10u, word >> 20u) & uvec3(1023u);
    mean = (vec3(cell) + vec3(q) / 1023.0 * 2.0 - CONE_HALF) * size;
    vec3 sum = vec3(0.0);
    float total = 0.0;
    for (int d = 0; d < 6; d++) {
        uint w = stride == 0u ? crec.fv[area_at + uint(2 * d + 1)] : ccoarse.cw[area_at + uint(2 * d + 1)];
        float area = unpackHalf2x16(w).y;
        sum += cone_axis(d) * area;
        total += area;
    }
    if (total > 0.0 && dot(sum, sum) > 0.0) {
        normal = normalize(sum);
        held = length(sum) / total;
    }
    return true;
}

// Whether cascade c's probe at `lod` and `cell` is wanted as a neighbour:
// its field voxel has a surface, or the level is past the field's coarsest
// (no surface is known there, so every cell is).
bool cone_probe_wanted(int lod, int c, ivec3 cell) {
    int level = lod + c;
    if (level > CONE_COARSEST) {
        return true;
    }
    vec3 mean;
    vec3 normal;
    float held;
    return cone_seat(level, cell, mean, normal, held);
}

// Put cascade c's probe at `lod` and `cell` in its hash; the slot, or
// CONE_NO_BRICK (counted) when the hash had no room or the cell is out of range.
uint cone_insert_probe(int c, int lod, ivec3 cell) {
    uint key = cone_key(lod, cell, c);
    uint slot = key == CONE_EMPTY ? CONE_NO_BRICK : cone_slot(c, key, true);
    if (slot == CONE_NO_BRICK) {
        atomicAdd(cwork.w[CONE_INSERT_DROPPED + uint(c)], 1u);
    }
    return slot;
}

// The 8 cells of spacing `s` around `at` that a trilinear read takes.
ivec3 cone_trilinear_base(vec3 at, float s) {
    return ivec3(floor(at / s - CONE_HALF));
}

float cone_reads = 0.0;

struct ConeMet {
    float depth;
    float cover;
    vec3 radiance;
};
const int CONE_MAX_MET = 8;
ConeMet cone_met[CONE_MAX_MET];

// The planes the axis crosses between `from` and `to` at a fine level, each
// once, nearest first, in the voxel that holds the crossing point and inside
// its points' box.
int cone_crossed(int level, vec3 from, vec3 to, vec3 dir) {
    float size = cone_size(level);
    ivec3 first = ivec3(floor(from / size - CONE_HALF));
    int n = 0;
    for (int e = 0; e < 2; e++) {
        vec3 p = e == 0 ? from : to;
        ivec3 base = ivec3(floor(p / size - CONE_HALF));
        for (int k = 0; k < 8; k++) {
            ivec3 cell = base + ivec3(k & 1, (k >> 1) & 1, (k >> 2) & 1);
            if (e == 1 && all(greaterThanEqual(cell, first)) && all(lessThanEqual(cell, first + ivec3(1)))) {
                continue;
            }
            uint r = cone_record(level, cell);
            cone_reads += 1.0;
            if (r == CONE_NO_BRICK) {
                continue;
            }
            for (int d = 0; d < 6; d++) {
                vec3 axis = cone_axis(d);
                if (-dot(dir, axis) <= CONE_TOWARD_MIN) {
                    continue;
                }
                vec2 b_area = unpackHalf2x16(crec.fv[r + uint(2 * d + 1)]);
                if (b_area.y <= 0.0) {
                    continue;
                }
                uint pw = r + 12u + uint(3 * d);
                vec2 p0 = unpackHalf2x16(crec.fv[pw]);
                int ax = d / 2;
                vec3 low = vec3(cell) * size;
                float centre = low[ax] + p0.x * size;
                float sa = (from[ax] - centre) * axis[ax];
                float sb = (to[ax] - centre) * axis[ax];
                if (!(sa > 0.0 && sb <= 0.0)) {
                    continue;
                }
                int u = (ax + 1) % 3;
                int v = (ax + 2) % 3;
                vec3 hit = from + (to - from) * (sa / (sa - sb));
                if (int(floor(hit[u] / size)) != cell[u] || int(floor(hit[v] / size)) != cell[v]) {
                    continue;
                }
                vec2 p1 = unpackHalf2x16(crec.fv[pw + 1u]);
                vec2 p2 = unpackHalf2x16(crec.fv[pw + 2u]);
                float lo_u = low[u] + p0.y * size;
                float hi_u = low[u] + p1.x * size;
                float lo_v = low[v] + p1.y * size;
                float hi_v = low[v] + p2.x * size;
                if (hit[u] < lo_u - CONE_BOX_MARGIN || hit[u] > hi_u + CONE_BOX_MARGIN
                    || hit[v] < lo_v - CONE_BOX_MARGIN || hit[v] > hi_v + CONE_BOX_MARGIN) {
                    continue;
                }
                if (n < CONE_MAX_MET) {
                    vec2 rg = unpackHalf2x16(crec.fv[r + uint(2 * d)]);
                    float fill = b_area.y / (size * size);
                    cone_met[n].depth = dot(hit - from, dir);
                    cone_met[n].cover = fill >= CONE_CROSS_FULL ? 1.0 : fill;
                    cone_met[n].radiance = vec3(rg, b_area.x) / b_area.y;
                    n++;
                }
            }
        }
    }
    for (int i = 1; i < n; i++) {
        ConeMet m = cone_met[i];
        int j = i - 1;
        while (j >= 0 && cone_met[j].depth > m.depth) {
            cone_met[j + 1] = cone_met[j];
            j--;
        }
        cone_met[j + 1] = m;
    }
    return n;
}

// The lamp term at a fine level: the record of the voxel that holds `at`.
void cone_fine_lamp(int level, vec3 at, out float cover, out vec3 light) {
    cover = 0.0;
    light = vec3(0.0);
    float size = cone_size(level);
    float area;
    vec3 sum;
    if (cone_lamp_get(level, ivec3(floor(at / size)), area, sum)) {
        cover = area / (size * size);
        light = sum / area;
    }
}

// The filtered read of a coarse level at `at`: the cross-section's cover by
// the areas that face the cone and by the lamps, and their light. Trilinear
// over the eight voxels around the point, all in the padded brick that holds
// the point's voxel.
void cone_coarse(int level, vec3 at, vec3 dir, out float cover, out vec3 light, out float lamp_cover, out vec3 lamp_light) {
    cover = 0.0;
    light = vec3(0.0);
    lamp_cover = 0.0;
    lamp_light = vec3(0.0);
    float size = cone_size(level);
    ivec3 home = ivec3(floor(at / size));
    vec3 g = at / size - CONE_HALF;
    ivec3 base = ivec3(floor(g));
    vec3 f = g - vec3(base);
    float cross_section = size * size;
    // The lamps, trilinear about `at`, wherever the field has a brick or not.
    float lamp_a = 0.0;
    vec3 lamp_s = vec3(0.0);
    bool near = cone_lamp_near(at);
    for (int k = 0; k < 8 && near; k++) {
        ivec3 o = ivec3(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        vec3 wv = mix(vec3(1.0) - f, f, vec3(o));
        float wt = wv.x * wv.y * wv.z;
        float area;
        vec3 sum;
        if (wt > 0.0 && cone_lamp_get(level, base + o, area, sum)) {
            lamp_a += wt * area;
            lamp_s += wt * sum;
        }
    }
    if (lamp_a > 0.0) {
        lamp_cover = lamp_a / cross_section;
        lamp_light = lamp_s / lamp_a;
    }
    uint slot = cone_entry(level, home);
    if (slot == CONE_NO_BRICK) {
        return;
    }
    ivec3 origin = (home >> 3) * CONE_BRICK;
    float facing = 0.0;
    vec3 sent = vec3(0.0);
    for (int k = 0; k < 8; k++) {
        ivec3 o = ivec3(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        vec3 wv = mix(vec3(1.0) - f, f, vec3(o));
        float wt = wv.x * wv.y * wv.z;
        if (wt <= 0.0) {
            continue;
        }
        uint t = cone_coarse_texel(slot, base + o - origin + ivec3(1));
        cone_reads += 1.0;
        for (int d = 0; d < 6; d++) {
            float toward = -dot(dir, cone_axis(d));
            if (toward <= 0.0) {
                continue;
            }
            vec2 ba = unpackHalf2x16(ccoarse.cw[t + uint(2 * d + 1)]);
            if (ba.y <= 0.0) {
                continue;
            }
            facing += wt * ba.y * toward;
            sent += wt * vec3(unpackHalf2x16(ccoarse.cw[t + uint(2 * d)]), ba.x) * toward;
        }
    }
    if (facing > 0.0) {
        cover = min(facing / cross_section, 1.0);
        light = sent / facing;
    }
}

// Whether the axis from `a` to `b` at a coarse level can meet a surface: its
// start, middle or end is in a dilated occupied cell.
bool cone_occupied(int level, vec3 start, vec3 dir, float a, float b) {
    float size = cone_size(level);
    float ts[3] = float[3](a, CONE_HALF * (a + b), b);
    for (int i = 0; i < 3; i++) {
        ivec3 cell = ivec3(floor((start + dir * ts[i]) / size));
        uint slot = cone_entry(level, cell);
        cone_reads += 1.0;
        if (slot == CONE_NO_BRICK) {
            continue;
        }
        ivec3 local = cell - (cell >> 3) * CONE_BRICK + ivec3(1);
        if (ccoarse.cw[cone_coarse_texel(slot, local) + 14u] != 0u) {
            return true;
        }
    }
    return false;
}

void cone_take(inout float covered, inout vec3 light, ConeMet m) {
    float seen = min(m.cover, 1.0 - covered);
    if (seen > 0.0) {
        light += m.radiance * seen;
        covered += seen;
    }
}

float cone_steps = 0.0;

// One interval of a cone: from `done` to `end` along `dir` from `start`, at
// `width` a metre out. Adds the light met to `light` and the cover to
// `covered`; with the crossing test at the fine levels and, at a coarse
// level, a covered step walked front to back from its coarsest half down.
void cone_march(vec3 start, vec3 dir, float width, float done, float end, inout float covered, inout vec3 light) {
    float finest = cone_size(CONE_FINEST);
    while (done < end && covered < CONE_FULL) {
        float wide = max(width * done, finest);
        int level = clamp(int(ceil(log2(wide / CONE_BASE))), CONE_FINEST, CONE_COARSEST);
        float step = cone_size(level);
        float from = done;
        float to = done + step;
        cone_steps += 1.0;
        if (level <= CONE_REFINE) {
            int n = cone_crossed(level, start + dir * from, start + dir * to, dir);
            float lc;
            vec3 ll;
            cone_fine_lamp(level, start + dir * (from + CONE_HALF * step), lc, ll);
            if (lc > 0.0) {
                light += ll * min(lc, 1.0 - covered);
            }
            for (int i = 0; i < n && covered < CONE_FULL; i++) {
                cone_take(covered, light, cone_met[i]);
            }
            done = to;
            continue;
        }
        float cov;
        vec3 li;
        float lc;
        vec3 ll;
        cone_coarse(level, start + dir * (from + CONE_HALF * step), dir, cov, li, lc, ll);
        if (lc > 0.0) {
            light += ll * min(lc, 1.0 - covered);
        }
        if (cov > 0.0) {
            float u = from;
            while (u < to && covered < CONE_FULL) {
                int l = level - 1;
                while (l > CONE_REFINE && cone_occupied(l, start, dir, u, min(u + cone_size(l), to))) {
                    l--;
                }
                float next = min(u + cone_size(l), to);
                if (l <= CONE_REFINE) {
                    int n = cone_crossed(CONE_REFINE, start + dir * u, start + dir * next, dir);
                    for (int i = 0; i < n && covered < CONE_FULL; i++) {
                        cone_take(covered, light, cone_met[i]);
                    }
                }
                u = next;
                cone_steps += 1.0;
            }
        }
        done = to;
    }
}

// The share of light that passes from `from` to `to` at the refine level,
// stopping one step short of `to`.
float cone_passing(vec3 from, vec3 to) {
    vec3 way = to - from;
    float length_ = length(way);
    if (length_ <= CONE_TINY) {
        return 1.0;
    }
    vec3 dir = way / length_;
    float step = cone_size(CONE_REFINE);
    float end = max(length_ - step, 0.0);
    float blocked = 0.0;
    float u = 0.0;
    while (u < end && blocked < 1.0) {
        float next = min(u + step, end);
        int n = cone_crossed(CONE_REFINE, from + dir * u, from + dir * next, dir);
        for (int i = 0; i < n; i++) {
            blocked += cone_met[i].cover;
        }
        u = next;
    }
    return max(1.0 - blocked, 0.0);
}

// A cone record: light, the share passed, the luma-weighted direction.
void cone_put(uint at, vec3 light, float through, vec3 moment) {
    cwork.w[at] = packHalf2x16(light.rg);
    cwork.w[at + 1u] = packHalf2x16(vec2(light.b, through));
    cwork.w[at + 2u] = packHalf2x16(moment.xy);
    cwork.w[at + 3u] = packHalf2x16(vec2(moment.z, 1.0));
}
bool cone_get(uint at, out vec3 light, out float through, out vec3 moment) {
    vec2 a = unpackHalf2x16(cwork.w[at]);
    vec2 b = unpackHalf2x16(cwork.w[at + 1u]);
    vec2 m = unpackHalf2x16(cwork.w[at + 2u]);
    vec2 z = unpackHalf2x16(cwork.w[at + 3u]);
    light = vec3(a, b.x);
    through = b.y;
    moment = vec3(m, z.x);
    return z.y > CONE_HALF;
}
uint cone_record_at(int c, uint probe, uint bin) {
    return cone_cones_at(c) + (probe * cone_dirs(c) + bin) * CONE_CONE_WORDS;
}

// The probes of cascade c at level `lod` around `at`, trilinear, at most 8,
// each weighted by the share that passes from `from` to its seat when `see`.
// A probe the field had no seat for (seated at its cell's centre: a pixel on
// something the field does not hold) is read only as the cell `own`: whether
// it exists follows what is on screen, so no other probe or pixel leans on it.
int cone_weights(int c, int lod, vec3 at, bool see, vec3 from, ivec3 own, out uint probes[8], out float weights[8]) {
    float s = cone_spacing(lod, c);
    vec3 g = at / s - CONE_HALF;
    vec3 base = floor(g);
    vec3 f = g - base;
    int n = 0;
    for (int k = 0; k < 8; k++) {
        probes[k] = CONE_NO_BRICK;
        weights[k] = 0.0;
    }
    for (int k = 0; k < 8; k++) {
        ivec3 o = ivec3(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        vec3 wv = mix(vec3(1.0) - f, f, vec3(o));
        float wt = wv.x * wv.y * wv.z;
        if (wt <= 0.0) {
            continue;
        }
        ivec3 cell = ivec3(base) + o;
        uint probe = cone_find(c, cone_key(lod, cell, c));
        if (probe == CONE_NO_BRICK) {
            continue;
        }
        if (cwork.w[cone_probe(c, probe) + CONE_PROBE_SEATED] == 0u && cell != own) {
            continue;
        }
        if (see) {
            wt *= cone_passing(from, cone_word3(cone_probe(c, probe) + 8u));
        }
        if (wt > 0.0) {
            probes[n] = probe;
            weights[n] = wt;
            n++;
        }
    }
    return n;
}
