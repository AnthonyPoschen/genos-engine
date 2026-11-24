// The cone-traced cascades (experimental, lighting key `cone_cascades`):
// the voxel field (crates/render/src/cone_field.rs), the cone march, the
// direction bins, the probe hash and the work buffer's layout. Every cone
// pass includes lighting_common.glsl first, then this.
//
// Push constant for the cone passes: params.x the cascade, params.y theta at
// cascade 0, params.z the cascade count, params.w the probe size in pixels;
// lamp.x flags (bit 0 visibility in the merge, bit 1 visibility in the shade),
// lamp.y the first reach t0 in probe spacings, as float bits.

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
// 32 words a fine voxel: light and area for each way (2 words), its plane
// (3 words), then the lamp term (2 words).
layout(std430, set = 0, binding = 3) readonly buffer ConeRecords {
    uint fv[];
} crec;
// 16 words a coarse texel: light and area for each way and the lamps
// (2 words each, 7 of them), then the occupancy word.
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
const uint CONE_RECORD = 32u;
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
const float CONE_FIXED = 256.0;
const vec3 CONE_LUMA = vec3(0.2126, 0.7152, 0.0722);
// Work buffer: counters, then for each cascade its hash, probes and cones.
const uint CONE_COUNTS = 0u;    // probes kept, per cascade
const uint CONE_DROPPED = 8u;   // probes lost (out of the key's range or the hash full)
const uint CONE_TRACED = 16u;   // cones traced, per cascade
const uint CONE_CLOSED = 24u;   // cones the chain closed, per cascade
const uint CONE_HEADER_WORDS = 256u;
const uint CONE_HASH_WORDS = 9u;   // key, count, sum xyz, normal sum xyz, probe
const uint CONE_PROBE_WORDS = 32u; // see cone_probe_* below
const uint CONE_CONE_WORDS = 4u;   // half2 rg, half2 b through, half2 moment xy, half2 moment z filled
const uint CONE_CONES_CAP = 2097152u;
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
uint cone_hash_size(int c) {
    return 65536u >> uint(c);
}
uint cone_cap(int c) {
    return min(cone_hash_size(c) / 2u, CONE_CONES_CAP / cone_dirs(c));
}
// Word offsets of cascade c's hash, probes and cones.
uint cone_hash_at(int c) {
    uint at = CONE_HEADER_WORDS;
    for (int k = 0; k < c; k++) {
        at += cone_hash_size(k) * CONE_HASH_WORDS + cone_cap(k) * CONE_PROBE_WORDS + cone_cap(k) * cone_dirs(k) * CONE_CONE_WORDS;
    }
    return at;
}
uint cone_probes_at(int c) {
    return cone_hash_at(c) + cone_hash_size(c) * CONE_HASH_WORDS;
}
uint cone_cones_at(int c) {
    return cone_probes_at(c) + cone_cap(c) * CONE_PROBE_WORDS;
}

// A probe's record: 0 key, 1 lod, 2 the chain child (the probe of cascade
// c - 1 its cones start from, or CONE_NO_BRICK), 3 the start's lod, 4..6 its
// cell, 7 its pixel count, 8..10 the seat (the pixels' mean point, lifted
// along their mean normal when it holds), 12..14 the cones' start point (on
// the voxel layer), 16..18 that point's normal; 20..27 the probes of the next
// cascade the merge reads and 28.. unused.
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

// A pixel's probe level: a probe is about `probe_px` pixels wide where it is.
int cone_pixel_lod(vec3 pos) {
    vec3 away = pos - scene.eye.xyz;
    float reach = max(abs(away.x), max(abs(away.y), abs(away.z)));
    float per_pixel = CONE_HALVES * scene.view_right.w / float(pc.dims.y);
    float wanted = float(pc.params.w) * per_pixel * reach;
    return clamp(int(round(log2(max(wanted, CONE_TINY) / CONE_BASE))), 0, CONE_MAX_LOD);
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
    uint r = cone_record(level, ivec3(floor(at / size)));
    if (r == CONE_NO_BRICK) {
        return;
    }
    vec2 rg = unpackHalf2x16(crec.fv[r + 30u]);
    vec2 ba = unpackHalf2x16(crec.fv[r + 31u]);
    if (ba.y > 0.0) {
        cover = ba.y / (size * size);
        light = vec3(rg, ba.x) / ba.y;
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
    uint slot = cone_entry(level, home);
    if (slot == CONE_NO_BRICK) {
        return;
    }
    vec3 g = at / size - CONE_HALF;
    ivec3 base = ivec3(floor(g));
    vec3 f = g - vec3(base);
    ivec3 origin = (home >> 3) * CONE_BRICK;
    float facing = 0.0;
    vec3 sent = vec3(0.0);
    float lamp_a = 0.0;
    vec3 lamp_s = vec3(0.0);
    for (int k = 0; k < 8; k++) {
        ivec3 o = ivec3(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        vec3 wv = mix(vec3(1.0) - f, f, vec3(o));
        float wt = wv.x * wv.y * wv.z;
        if (wt <= 0.0) {
            continue;
        }
        uint t = cone_coarse_texel(slot, base + o - origin + ivec3(1));
        cone_reads += 1.0;
        vec2 lb = unpackHalf2x16(ccoarse.cw[t + 13u]);
        if (lb.y > 0.0) {
            lamp_a += wt * lb.y;
            lamp_s += wt * vec3(unpackHalf2x16(ccoarse.cw[t + 12u]), lb.x);
        }
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
    float cross_section = size * size;
    if (facing > 0.0) {
        cover = min(facing / cross_section, 1.0);
        light = sent / facing;
    }
    if (lamp_a > 0.0) {
        lamp_cover = lamp_a / cross_section;
        lamp_light = lamp_s / lamp_a;
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
int cone_weights(int c, int lod, vec3 at, bool see, vec3 from, out uint probes[8], out float weights[8]) {
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
        uint probe = cone_find(c, cone_key(lod, ivec3(base) + o, c));
        if (probe == CONE_NO_BRICK) {
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
