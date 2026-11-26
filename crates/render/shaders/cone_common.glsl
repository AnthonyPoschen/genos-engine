// The cone-traced cascades (experimental, lighting key `cone_cascades`):
// the voxel field (crates/render/src/cone_field.rs), the cone march, the
// direction bins, the probe hash and the work buffer's layout. Every cone
// pass includes lighting_common.glsl first, then this.
//
// Push constant for the cone passes: dims.z the finest probe level; params.x the cascade, params.y theta at
// cascade 0, params.z the cascade count, params.w the probe size in pixels;
// lamp.x flags (bit 0 visibility in the merge, bit 1 visibility in the shade,
// bit 2 seats on the pixels' mean: the gate knockout of the field seat),
// lamp.y the first reach t0 in probe spacings, as float bits; lamp.z the
// lamps' sphere radius in metres, as float bits; lamp.w, in the light pass
// only, the share of the way a voxel's light moves this frame (float bits).

// head: atlas texels xyz, coarse bricks. lv[2k]: brick origin xyz and the
// table offset of level k + 2; lv[2k + 1]: the table's dims in bricks.
// clip[k]: level k + 2's clip volume, corner xyz and side in its voxels
// (side 0: the whole scene, below 0: nothing; cone_field.rs FieldLevel::clip).
// info: where the fine bricks' origins start in table[] (the coarse bricks'
// follow at y), and the fine and coarse brick counts; an origin is the
// brick's coordinates and level (4 words).
layout(std430, set = 0, binding = 1) readonly buffer ConeTable {
    uvec4 head;
    ivec4 lv[16];
    ivec4 clip[8];
    uvec4 info;
    uint table[];
} ctab;
// 49 words a fine brick: first record, 16 mask words, 16 counts before,
// 16 block-mask words (CONE_BLOCKS).
layout(std430, set = 0, binding = 2) readonly buffer ConeFine {
    uint fb[];
} cfine;
// 37 words a fine voxel: light and area for each way (2 words), its plane
// (3 words), then the lamp term (2 words), its seat word, its albedo (2
// words), each way's sun share (2 words). The light and the sun share are
// written by the light pass (cone_light.comp).
layout(std430, set = 0, binding = 3) buffer ConeRecords {
    uint fv[];
} crec;
// 26 words a coarse texel (the last three each way's front along its axis): light and area for each way and the lamps
// (2 words each, 7 of them), then the occupancy word, the seat word, the
// albedo (2 words), each way's surface along its axis (3 words), each way's
// sun share (2 words). The light and the sun share are written by the light
// pass (cone_light.comp).
layout(std430, set = 0, binding = 4) buffer ConeCoarse {
    uint cw[];
} ccoarse;
layout(std430, set = 0, binding = 15) buffer ConeWork {
    uint w[];
} cwork;

const float CONE_BASE = 1.0 / 64.0;
const int CONE_FINEST = 2;
const int CONE_COARSEST = 9;
// The first level the field holds for the whole scene; finer ones are clip
// volumes about the eye (cone_field.rs WHOLE_FINEST).
const int CONE_WHOLE_FINEST = 5;
// Levels under a coarse step's own that its covered stretch is walked again at.
const int CONE_REFINE_DEPTH = 2;
// The levels a crossing reads, and the refine level of the walk.
const int CONE_REFINE = 3;
const int CONE_BRICK = 8;
const int CONE_PADDED = 10;
const uint CONE_ATLAS_BRICKS = 32u;
const uint CONE_NO_BRICK = 0xffffffffu;
const uint CONE_HEADER = 49u;
// A fine brick's block masks (cone_field.rs FINE_BLOCKS): bit x + 8y + 64z is
// set when any of the eight voxels from that one to the next on each axis
// holds a record.
const uint CONE_BLOCKS = 33u;
const uint CONE_RECORD = 37u;
const uint CONE_RECORD_SUN = 35u;  // unorm8 a way: the sun share each way sees (cone_light.comp)
const uint CONE_RECORD_ALBEDO = 33u;
// A seat word (cone_field.rs seat_word_of): bit 31 set when the voxel has
// surface, 10 bits an axis over its cell from half a voxel below to half past.
const uint CONE_SEAT_WORD = 32u;
const uint CONE_TEXEL_SEAT = 15u;
const uint CONE_TEXEL_ALBEDO = 16u;
const uint CONE_TEXEL_SUN = 21u;    // unorm8 a way: the sun share each way sees
const uint CONE_TEXEL_FRONTS = 23u; // half2 a pair of ways: each way's front along its axis, in voxels
const uint CONE_TEXEL_PLANES = 18u; // half2 a pair of ways: each way's surface along its axis, in voxels
// A level is chosen when its voxel is at least this share of the wanted
// size: the nearest level in log2, as the rounding before had it.
const float CONE_NEAREST = 0.70710678;
// A probe's cell is the voxel a quarter of a voxel (at most 6.25 cm) behind
// the surface, where the field filed that surface's points.
const float CONE_BEHIND = 0.25;
// The knockout's pixel sums: points relative to the eye in 1/256 m.
const float CONE_FIXED = 256.0;
const uint CONE_PIXEL_SEAT = 4u;
const uint CONE_TEXEL = 26u;
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
const int CONE_KEY_SPAN = 128;
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
const uint CONE_SUN_COUNT = 50u;    // pixels listed for the sun pass (cone_sun.comp)
const uint CONE_SUN_LAYOUT = 91u;   // the sun list's word offset: a pixel index a word
const uint CONE_FILL_COUNT = 51u;   // pixels listed for the fill pass (cone_fill.comp)
const uint CONE_KEPT = 56u;     // cones kept from last frame, per cascade (cone_trace.comp)
// Last frame's tables. The work buffer holds two frames' cascades, this
// frame's and last frame's, in turn; for each cascade, last frame's hash
// size, hash, probe cap, cones and target lists (word offsets). A probe
// that stands where it stood last frame is not traced again every frame:
// its cones are kept until its turn (cone_trace.comp).
const uint CONE_PREV_LAYOUT = 112u;
const uint CONE_TARGET_DROPPED = 52u; // lamps a probe's target list had no room for
// Lamp steering. A cone that meets a voxel holding one lamp's light does not
// take that light: it puts the lamp in its probe's target list. The lists
// pass down the cascades (cone_targets.comp), and each cascade-0 probe casts
// its own thin ray at each of its targets (cone_lamp.comp). A lamp is small
// at any distance, so its shadow does not widen as the cascades assume:
// gathered by a cascade whose probes are a metre apart, the patch of light a
// lamp threw through an arch was blurred over a metre or two and lost its
// edge (Anthony, 26 Nov 2026). The cones find the lamp; the finest probes
// aim at it.
// A probe's list: CONE_TARGETS lamp indices plus one (0: empty). The lists'
// word offsets, one a cascade, are in the header at CONE_TARGET_LAYOUT.
const uint CONE_TARGETS = 16u;
const uint CONE_TARGET_LAYOUT = 98u;
// What each cascade-0 probe's ray saw of each of its targets (a float a
// target, beside its list): the share of the lamp the probe sees. A pixel
// computes a lamp's light itself, from its own distance and angle to the
// lamp, and takes only this share from the probes about it, blended as
// their light is. Between a probe that sees the lamp and one that does not
// the blend runs from 1 to 0; the shadow's edge is drawn where it crosses a
// half (cone_lamp_share), a smooth line through the probes' grid with no ray
// from the pixel. Decided once a probe, the edge followed the grid in steps
// (Anthony, 26 Nov 2026).
const uint CONE_SEEN_LAYOUT = 103u;
// The lamps a gathered pixel carries: each a word, the lamp's index plus one
// in the low half and the share of it seen in the high half.
const uint CONE_PIXEL_LAMPS = 4u;
const uint CONE_GATHER_WORDS = 5u;
// Half the width of a lamp's shadow edge, as a share of the blend from a
// probe that sees the lamp to one that does not.
const float CONE_LAMP_EDGE = 0.5;
// The gather grid's and the fill list's word offsets (cone_gpu.rs writes
// them each frame): the grid holds a packed light for each gathered pixel
// (cone_shade.comp), the list a pixel index a word.
const uint CONE_GATHER_LAYOUT = 96u;
const uint CONE_GATHER_NONE = 0xffffffffu; // a gathered pixel with no surface
const uint CONE_LAYOUT = 64u;   // per cascade 5 words: hash size, cap, hash, probes, cones (cone_gpu.rs)
const uint CONE_CLEARED = 64u;  // header words cleared every frame
const uint CONE_INSERT_DROPPED = 32u; // pixels whose probe found no hash slot, per cascade
const uint CONE_HEADER_WORDS = 640u;
// The pixel pick's record (cone_shade.comp writes it for the pixel in push
// dims.w; cone_gpu.rs cone_pick reads it): 16 words of the pixel, then a
// CONE_PICK_LEVEL block for each level it reads (8 words, then
// CONE_PICK_PROBE words for each of the 8 probes about it).
const uint CONE_PICK = 256u;
const uint CONE_PICK_HEAD = 16u;
const uint CONE_PICK_LEVEL = 104u;
const uint CONE_PICK_PROBE = 12u;
// The pixel's own probe's first cones at each level: two words a cone
// (half2 luma and the share that passes, -1 luma for none; snorm8 way).
const uint CONE_PICK_DIRS_AT = 480u;
const uint CONE_PICK_DIRS = 8u;
// The pixel's own probe's next-cascade probes the merge reads, at each
// level: four words each (seat, weight).
const uint CONE_PICK_UP_AT = 512u;
// Push lamp.x bits 4 and 5: the debug view (cone_cascades.view): 1 the
// probe coverage, 2 the dark directions, 3 the specular light alone
// (cone_specular.glsl).
const uint CONE_VIEW_SHIFT = 4u;
const uint CONE_VIEW_COVERAGE = 1u;
const uint CONE_VIEW_DARK = 2u;
const uint CONE_VIEW_SPECULAR = 3u;
const uint CONE_PREV_EYE = 92u;   // last frame's eye (float bits xyz), then 1 when its probes are in the buffer
const uint CONE_HASH_WORDS = 10u;  // key, count, sum xyz, normal sum xyz, probe, screen references
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
// cell, 7 its pixel count, 11 its flags (CONE_PROBE_SEATED: bit 0 the field
// seated it, bit 1 a world probe no pixel or pixel-placed probe reads), 8..10 the seat (the field voxel's surface point,
// lifted along its surface's normal when that holds), 12..14 the cones' start point (on
// the voxel layer), 16..18 that point's normal; 20..27 the probes of the next
// cascade the merge reads and 28.. unused.
const uint CONE_PROBE_SEATED = 11u;
const uint CONE_FLAG_SEATED = 1u;
const uint CONE_FLAG_WORLD = 2u;
// The hash entry's count of screen references (pixels, and the cascade below's
// screen probes); a cascade-0 entry with none is a world probe's.
const uint CONE_HASH_SCREEN = 9u;
// The world layer (cone_world.comp): probes on the surfaces about the eye that
// no pixel covers, so the light pass has light for what the camera does not
// see. Off screen is far coarser than on it (Anthony, 25 Nov 2026: the world
// layer must cost a fraction of the screen's): 2 m voxels within 8 m of the
// eye, 4 m ones out to 16 m; traced on CONE_WORLD_CASCADES cascades (the far
// field taken from the sky), and a CONE_WORLD_SHARE of them each frame.
const int CONE_WORLD_NEAR = 7;
const float CONE_WORLD_NEAR_REACH = 8.0;
const int CONE_WORLD_FAR = 8;
const float CONE_WORLD_FAR_REACH = 16.0;
const int CONE_WORLD_CASCADES = 2;
const uint CONE_WORLD_SHARE = 4u;
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

// The 32-bit key of a probe: lod (4 bits), face (3 bits: the axis way its
// pixels face, cone_face) and the cell's place relative to the eye's cell at
// that spacing (8 bits an axis), or 0 when the cell is out of range. A voxel
// whose surfaces face several ways (both sides of a curtain) has a probe for
// each side the pixels see.
uint cone_key_from(int lod, ivec3 cell, int c, int face, vec3 from_eye);
uint cone_key(int lod, ivec3 cell, int c, int face) {
    return cone_key_from(lod, cell, c, face, scene.eye.xyz);
}
// As cone_key, relative to `from_eye`: last frame's keys, for the light pass.
uint cone_key_from(int lod, ivec3 cell, int c, int face, vec3 from_eye) {
    ivec3 eye = ivec3(floor(from_eye / cone_spacing(lod, c)));
    ivec3 rel = cell - eye + ivec3(CONE_KEY_SPAN);
    if (any(lessThan(rel, ivec3(0))) || any(greaterThanEqual(rel, ivec3(2 * CONE_KEY_SPAN)))) {
        return CONE_EMPTY;
    }
    return 0x80000000u | (uint(lod) << 27u) | (uint(face) << 24u) | (uint(rel.x) << 16u) | (uint(rel.y) << 8u) | uint(rel.z);
}
int cone_key_lod(uint key) {
    return int((key >> 27u) & 15u);
}
int cone_key_face(uint key) {
    return int((key >> 24u) & 7u);
}
ivec3 cone_key_rel(uint key) {
    return ivec3(int((key >> 16u) & 255u), int((key >> 8u) & 255u), int(key & 255u)) - ivec3(CONE_KEY_SPAN);
}
// The axis way (cone_axis: +x, -x, +y, -y, +z, -z) a normal faces most.
int cone_face(vec3 n) {
    vec3 a = abs(n);
    int axis = a.x >= a.y && a.x >= a.z ? 0 : (a.y >= a.z ? 1 : 2);
    return 2 * axis + (n[axis] < 0.0 ? 1 : 0);
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
        // Read first: most inserts find their key there already (the pixels
        // of a probe's square all put the same nine), and a read costs far
        // less than a compare-and-swap.
        uint seen = cwork.w[at];
        if (insert && seen == CONE_EMPTY) {
            seen = atomicCompSwap(cwork.w[at], CONE_EMPTY, key);
        }
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
// cones early. With the fine levels in clip volumes, neighbours up to
// cascade 2 cost the hall 41.6 ms a frame at 1080p against 38 ms up to
// cascade 1, and pop 1.8 times the old path while moving against 2.6
// (section 18, step 5 part 2b).
const int CONE_PARENT_ONLY = 3;
// A pixel's probe level, continuous: log2 of the voxel that is about
// `probe_px` pixels wide at the pixel's distance from the eye, raised near a
// clip volume's edge so that it reaches the next level at the edge (over the
// CONE_CLIP_FADE voxels inside it) and is at least that past it. From where
// the eye is each frame, so a turn of the head changes nothing.
const float CONE_CLIP_FADE = 16.0;
float cone_level_from(vec3 pos, vec3 eye) {
    float per_pixel = CONE_HALVES * scene.view_right.w / float(pc.dims.y);
    float px = float(pc.params.w);
    float reach = max(length(pos - eye), CONE_TINY);
    float level = log2(CONE_NEAREST * px * per_pixel * reach / CONE_BASE);
    for (int k = CONE_FINEST; k < CONE_WHOLE_FINEST; k++) {
        ivec4 c = ctab.clip[k - CONE_FINEST];
        if (c.w == 0) {
            continue;
        }
        float inside = -1.0;
        if (c.w > 0) {
            vec3 g = pos / cone_size(k) - vec3(c.xyz);
            vec3 m = min(g, vec3(float(c.w)) - g);
            inside = min(m.x, min(m.y, m.z));
        }
        level = max(level, float(k) + 1.0 - clamp(inside / CONE_CLIP_FADE, 0.0, 1.0));
    }
    return level;
}
float cone_level_at(vec3 pos) {
    return cone_level_from(pos, scene.eye.xyz);
}
// The two probe levels a pixel reads and the upper one's weight: level `hi`
// = ceil of the continuous level and, in the first CONE_BLEND of that octave,
// also hi - 1, fading to it where the continuous level reaches hi - 1. So no
// level boundary is a seam (Anthony saw bands at them, 25 Nov 2026).
const float CONE_BLEND = 0.5;
// The finest probe level (cone_cascades.finest_probe; push dims.z).
int cone_finest_probe() {
    return clamp(int(pc.dims.z), CONE_FINEST, CONE_WHOLE_FINEST + 1);
}
void cone_pixel_levels(vec3 pos, out int lo, out int hi, out float w_hi) {
    int finest = cone_finest_probe();
    float level = max(cone_level_at(pos), float(finest));
    hi = clamp(int(ceil(level)), finest, CONE_MAX_LOD);
    w_hi = clamp((level - float(hi - 1)) / CONE_BLEND, 0.0, 1.0);
    lo = hi - 1;
    if (hi == finest || level >= float(CONE_MAX_LOD)) {
        lo = hi;
        w_hi = 1.0;
    }
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
const uint CONE_LAMP_WORDS = 9u;
// A lamp entry's lamps: the index in the scene's lamp list, plus one, of
// each of the first CONE_LAMP_IDS lamps with points in the voxel, and
// whether there are more (CONE_LAMP_MORE: those are steered only where a
// finer voxel holds them apart). No lamp's light travels in the cones: when
// a coarse voxel of two lamps kept its light in them, a probe that also had
// those lamps steered from a finer voxel counted them twice, and its patch
// of floor was brighter than its neighbours' (Anthony, 26 Nov 2026).
const uint CONE_LAMP_ID = 5u;
const uint CONE_LAMP_IDS = 3u;
const uint CONE_LAMP_MORE = 8u;
// A lamp key holds the cell relative to the eye's cell at that level: 10 bits
// across, 9 up (about 32 m and 16 m at 6.25 cm, more at the coarser levels).
const int CONE_LAMP_SPAN_XZ = 512;
const int CONE_LAMP_SPAN_Y = 256;

// A seat's position in its voxel: 10 bits an axis over two voxels' span.
const float CONE_SEAT_STEP = 2.0 / 1023.0;
// A lamp record's pos.w over this marks a sun (scene_rays.glsl).
const float CONE_SUN_MARK = 0.5;

uint cone_lamp_size() {
    return cwork.w[CONE_LAMP_LAYOUT];
}
// Whether the view draws a probe colour in place of the light (coverage,
// dark); the specular view draws light.
bool cone_probe_view() {
    uint view = (pc.lamp.x >> CONE_VIEW_SHIFT) & 3u;
    return view == CONE_VIEW_COVERAGE || view == CONE_VIEW_DARK;
}
uint cone_gather_at() {
    return cwork.w[CONE_GATHER_LAYOUT];
}
uint cone_fill_at() {
    return cwork.w[CONE_GATHER_LAYOUT + 1u];
}
// The gather grid: one gathered pixel for each square of `scale` pixels.
uvec2 cone_gather_cells(uint scale) {
    return (pc.dims.xy + scale - 1u) / scale;
}
// The pixel a grid cell gathers at: the middle of its square.
uvec2 cone_gather_pixel(uvec2 cell, uint scale) {
    return min(cell * scale + scale / 2u, pc.dims.xy - 1u);
}
// The sun list (cone_resolve.comp fills it, cone_sun.comp reads it).
uint cone_sun_at() {
    return cwork.w[CONE_SUN_LAYOUT];
}
uint cone_lamp_at() {
    return cwork.w[CONE_LAMP_LAYOUT + 1u];
}
// The plastic sequence's point `i` on the unit sphere (cone_field.rs r2 and decode_dir).
const float CONE_PLASTIC = 1.324718;
const float CONE_SEQ_START = 0.5;
vec3 cone_lamp_dir(uint i) {
    float u = fract(CONE_SEQ_START + float(i) / CONE_PLASTIC);
    float v = fract(CONE_SEQ_START + float(i) / (CONE_PLASTIC * CONE_PLASTIC));
    float phi = u * CONE_TAU;
    float z = v + v - 1.0;
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
        uint seen = cwork.w[at];
        if (insert && seen == CONE_EMPTY) {
            seen = atomicCompSwap(cwork.w[at], CONE_EMPTY, key);
        }
        if (seen == key || (insert && seen == CONE_EMPTY)) {
            return at;
        }
        if (seen == CONE_EMPTY) {
            return CONE_NO_BRICK;
        }
    }
    return CONE_NO_BRICK;
}
// Lamp presence: at each coarse level a lamp point also claims, with no
// area, the seven voxels before its own on each axis (cone_lamps.comp). A
// trilinear read takes the eight voxels from a base voxel on, so the base has
// an entry whenever any of the eight holds lamp light: one lookup says the
// eight hold none. (Before, a lamp claimed the 16 m voxels about it, and
// every coarse step within tens of metres of a lamp paid eight lookups.)
// A base past the key's span has no key: then the eight are read.
bool cone_lamp_about(int level, ivec3 base) {
    uint key = cone_lamp_key(level, base);
    return key == CONE_EMPTY || cone_lamp_slot(key, false) != CONE_NO_BRICK;
}
// Where cascade 0's probe keeps what its rays saw of its targets.
uint cone_seen_of(uint probe) {
    return cwork.w[CONE_SEEN_LAYOUT] + probe * CONE_TARGETS;
}
// The share of a lamp a pixel is lit by, from the blended share its probes
// see: the edge where the blend crosses a half, CONE_LAMP_EDGE either side.
float cone_lamp_share(float blended) {
    return smoothstep(CONE_HALF - CONE_LAMP_EDGE, CONE_HALF + CONE_LAMP_EDGE, blended);
}
// A gathered pixel's lamp word, and its parts.
const uint CONE_LAMP_WORD_ID = 0xffffu;
const uint CONE_LAMP_WORD_SHIFT = 16u;
const float CONE_LAMP_WORD_STEPS = 65535.0;
uint cone_lamp_word(uint id, float seen) {
    return (id & CONE_LAMP_WORD_ID) | (uint(clamp(seen, 0.0, 1.0) * CONE_LAMP_WORD_STEPS + CONE_HALF) << CONE_LAMP_WORD_SHIFT);
}
uint cone_lamp_word_id(uint word) {
    return word & CONE_LAMP_WORD_ID;
}
float cone_lamp_word_seen(uint word) {
    return float(word >> CONE_LAMP_WORD_SHIFT) / CONE_LAMP_WORD_STEPS;
}
// Last frame's probe of cascade c at `lod`, `cell` and `face` (its index in
// last frame's tables), or CONE_NO_BRICK: keyed from last frame's eye.
uint cone_prev_find(int c, int lod, ivec3 cell, int face) {
    if (cwork.w[CONE_PREV_EYE + 3u] == 0u) {
        return CONE_NO_BRICK;
    }
    vec3 eye = vec3(uintBitsToFloat(cwork.w[CONE_PREV_EYE]), uintBitsToFloat(cwork.w[CONE_PREV_EYE + 1u]), uintBitsToFloat(cwork.w[CONE_PREV_EYE + 2u]));
    uint key = cone_key_from(lod, cell, c, face, eye);
    if (key == CONE_EMPTY) {
        return CONE_NO_BRICK;
    }
    uint layout_ = CONE_PREV_LAYOUT + 5u * uint(c);
    uint size = cwork.w[layout_];
    uint base = cwork.w[layout_ + 1u];
    if (size == 0u) {
        return CONE_NO_BRICK;
    }
    uint h = cone_hash_of(key) & (size - 1u);
    for (uint k = 0u; k < CONE_HASH_PROBES; k++) {
        uint at = base + ((h + k) & (size - 1u)) * CONE_HASH_WORDS;
        uint seen = cwork.w[at];
        if (seen == key) {
            uint index = cwork.w[at + 8u];
            return index < cwork.w[layout_ + 2u] ? index : CONE_NO_BRICK;
        }
        if (seen == CONE_EMPTY) {
            return CONE_NO_BRICK;
        }
    }
    return CONE_NO_BRICK;
}
// The key's bits above the cell: its level and face.
const uint CONE_TURN_KIND = 24u;
// A number for a probe that is the same every frame it stands in the same
// world cell (its key holds its cell relative to the eye's, which moves):
// which frame of the period is its turn.
uint cone_turn_of(uint r) {
    uint h = cone_hash_of(cwork.w[r] >> CONE_TURN_KIND);
    h = cone_hash_of(h + cwork.w[r + 4u]);
    h = cone_hash_of(h + cwork.w[r + 5u]);
    return cone_hash_of(h + cwork.w[r + 6u]);
}
// Whether the probe at `r` keeps what it traced last frame, this frame: not
// its turn (one frame in the period, push lamp.w, counted by push dims.w).
// By the probe's world cell, so every probe has a turn each period whatever
// its number does. (Turns by blocks of probe numbers and by a kept age were
// tried, to have the threads that trace side by side all trace or all keep:
// faster, but the picture after a turn away and back then differed from run
// to run by up to 2% of its pixels, and the cause was not found.)
bool cone_keeps(uint r) {
    uint period = pc.lamp.w;
    return period > 1u && (cone_turn_of(r) + pc.dims.w) % period != 0u;
}
uint cone_prev_cones_at(int c) {
    return cwork.w[CONE_PREV_LAYOUT + 5u * uint(c) + 3u];
}
uint cone_prev_targets_of(int c, uint probe) {
    return cwork.w[CONE_PREV_LAYOUT + 5u * uint(c) + 4u] + probe * CONE_TARGETS;
}
// The target list of cascade c's probe (word offset).
uint cone_targets_of(int c, uint probe) {
    return cwork.w[CONE_TARGET_LAYOUT + uint(c)] + probe * CONE_TARGETS;
}
// Put lamp `id` (index plus one) in the list at `list`, once. Counted when
// the list is full: that lamp's light is then missing at that probe.
void cone_target_add(uint list, uint id) {
    for (uint k = 0u; k < CONE_TARGETS; k++) {
        uint seen = cwork.w[list + k];
        if (seen == 0u) {
            seen = atomicCompSwap(cwork.w[list + k], 0u, id);
        }
        if (seen == 0u || seen == id) {
            return;
        }
    }
    atomicAdd(cwork.w[CONE_TARGET_DROPPED], 1u);
}
// The target list the cone being marched steers lamps into (word offset;
// 0: none, the lamps' light is taken as it is met).
uint cone_steer_list = 0u;
// The lamps' area and light in the voxel at `level` and `cell`. A voxel that
// holds one lamp is steered when a cone is being marched for a probe
// (cone_steer_list): the lamp goes in the probe's targets and no light is
// returned here.
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
    if (area <= 0.0) {
        return false;
    }
    // The lamps are steered, never lit through the cone: each goes in the
    // marched cone's probe's targets (a march for no probe takes nothing).
    for (uint k = 0u; k < CONE_LAMP_IDS && cone_steer_list != 0u; k++) {
        uint id = cwork.w[at + CONE_LAMP_ID + k];
        if (id == 0u) {
            break;
        }
        cone_target_add(cone_steer_list, id);
    }
    area = 0.0;
    return false;
}

// ---- The field ----

// Whether the field holds `level` at `cell`: inside the level's clip volume.
// Past a fine level's volume the next coarser level stands in.
bool cone_held(int level, ivec3 cell) {
    if (level > CONE_COARSEST) {
        return true;
    }
    ivec4 c = ctab.clip[level - CONE_FINEST];
    return c.w == 0 || (all(greaterThanEqual(cell, c.xyz)) && all(lessThan(cell, c.xyz + ivec3(c.w))));
}
// The finest level from `level` up that holds the point `at`.
int cone_held_level(int level, vec3 at) {
    while (level < CONE_COARSEST && !cone_held(level, ivec3(floor(at / cone_size(level))))) {
        level++;
    }
    return level;
}

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
// The last fine brick looked up (cone_record) and its table entry: a march's
// crossing test reads sixteen voxels a step, nearly all in one or two bricks,
// and each lookup read the level's box and the table again.
ivec4 cone_brick_last = ivec4(0x7fffffff);
uint cone_brick_last_entry = CONE_NO_BRICK;
uint cone_fine_brick(int level, ivec3 cell) {
    ivec4 which = ivec4(cell >> 3, level);
    if (which != cone_brick_last) {
        cone_brick_last = which;
        cone_brick_last_entry = cone_entry(level, cell);
    }
    return cone_brick_last_entry;
}
// Whether none of the eight fine voxels from `base` to the next on each axis
// holds a record (the brick's block mask; a block with a record has a brick
// at its base).
bool cone_block_empty(int level, ivec3 base) {
    uint brick = cone_fine_brick(level, base);
    if (brick == CONE_NO_BRICK) {
        return true;
    }
    ivec3 l = base & ivec3(CONE_BRICK - 1);
    uint bit = uint(l.x + CONE_BRICK * l.y + CONE_BRICK * CONE_BRICK * l.z);
    return (cfine.fb[brick * CONE_HEADER + CONE_BLOCKS + (bit >> 5u)] & (1u << (bit & 31u))) == 0u;
}
// A fine voxel's record (first word), or CONE_NO_BRICK.
uint cone_record(int level, ivec3 cell) {
    uint brick = cone_fine_brick(level, cell);
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
    mean = (vec3(cell) + vec3(q) * CONE_SEAT_STEP - CONE_HALF) * size;
    vec3 sum = vec3(0.0);
    float total = 0.0;
    for (int d = 0; d < 6; d++) {
        uint w = stride == 0u ? crec.fv[area_at + uint(2 * d + 1)] : ccoarse.cw[area_at + uint(2 * d + 1)];
        // A back (a negative area, cone_field.rs written_area) is no surface.
        float area = max(unpackHalf2x16(w).y, 0.0);
        sum += cone_axis(d) * area;
        total += area;
    }
    if (total > 0.0 && dot(sum, sum) > 0.0) {
        normal = normalize(sum);
        held = length(sum) / total;
    }
    return true;
}

// The surface area of `face` (an axis way) in the field voxel at `level`
// and `cell`; 0 where the field has none.
float cone_face_area(int level, ivec3 cell, int face) {
    if (level < CONE_FINEST || level > CONE_COARSEST) {
        return 0.0;
    }
    if (level <= CONE_REFINE) {
        uint rec = cone_record(level, cell);
        return rec == CONE_NO_BRICK ? 0.0 : unpackHalf2x16(crec.fv[rec + uint(2 * face + 1)]).y;
    }
    uint slot = cone_entry(level, cell);
    if (slot == CONE_NO_BRICK) {
        return 0.0;
    }
    uint t = cone_coarse_texel(slot, (cell & ivec3(CONE_BRICK - 1)) + ivec3(1));
    return unpackHalf2x16(ccoarse.cw[t + uint(2 * face + 1)]).y;
}

// The word and byte of `face`'s sun share in a fine record or coarse texel
// that starts at `at` (CONE_RECORD_SUN or CONE_TEXEL_SUN past it).
float cone_sun_share_at(uint word, int face) {
    return unpackUnorm4x8(word)[face & 3];
}
// The sun share the field keeps for `face` of the voxel at `level` and `cell`;
// -1 where the voxel has no surface that way.
float cone_face_sun(int level, ivec3 cell, int face) {
    if (level < CONE_FINEST || level > CONE_COARSEST || cone_face_area(level, cell, face) <= 0.0) {
        return -1.0;
    }
    uint word;
    if (level <= CONE_REFINE) {
        word = crec.fv[cone_record(level, cell) + CONE_RECORD_SUN + uint(face / 4)];
    } else {
        uint slot = cone_entry(level, cell);
        word = ccoarse.cw[cone_coarse_texel(slot, (cell & ivec3(CONE_BRICK - 1)) + ivec3(1)) + CONE_TEXEL_SUN + uint(face / 4)];
    }
    return cone_sun_share_at(word, face);
}

// Where the probe of `face` on the field voxel at `level` and `cell` sits, on
// that side's own surface, at its front: along the axis the point of that
// way's surface furthest the way it faces (on the mean plane of a pleated
// curtain the probe sat inside the pleats, its cones met the cloth at once
// and the curtain was black; cone_pick showed seated probes with a hundredth
// of their neighbours' light); across it, at a fine level the middle of its
// points' box, at a coarse level the voxel's seat.
// The caller lifts it along the face's axis.
// False where the voxel has no surface facing that way.
bool cone_seat_face(int level, ivec3 cell, int face, out vec3 at) {
    at = vec3(0.0);
    if (cone_face_area(level, cell, face) <= 0.0) {
        return false;
    }
    float size = cone_size(level);
    if (level <= CONE_REFINE) {
        uint rec = cone_record(level, cell);
        uint pw = rec + 12u + uint(3 * face);
        vec2 p0 = unpackHalf2x16(crec.fv[pw]);
        vec2 p1 = unpackHalf2x16(crec.fv[pw + 1u]);
        vec2 p2 = unpackHalf2x16(crec.fv[pw + 2u]);
        int ax = face / 2;
        int u = (ax + 1) % 3;
        int v = (ax + 2) % 3;
        vec3 low = vec3(cell) * size;
        at[ax] = low[ax] + p2.y * size;
        at[u] = low[u] + clamp(CONE_HALF * (p0.y + p1.x), 0.0, 1.0) * size;
        at[v] = low[v] + clamp(CONE_HALF * (p1.y + p2.x), 0.0, 1.0) * size;
        return true;
    }
    vec3 normal;
    float held;
    if (!cone_seat(level, cell, at, normal, held)) {
        return false;
    }
    // The voxel's mean point, moved along the face's axis onto that way's own
    // surface: the underside of a slab, not its middle.
    uint slot = cone_entry(level, cell);
    uint t = cone_coarse_texel(slot, (cell & ivec3(CONE_BRICK - 1)) + ivec3(1));
    vec2 pair = unpackHalf2x16(ccoarse.cw[t + CONE_TEXEL_FRONTS + uint(face / 2)]);
    int ax = face / 2;
    at[ax] = (float(cell[ax]) + ((face & 1) == 0 ? pair.x : pair.y)) * size;
    return true;
}

// Mark a hash entry as put there from the screen (a pixel, or a probe the
// pixels placed): compact reads only whether the word is 0.
void cone_mark_screen(uint slot) {
    if (cwork.w[slot + CONE_HASH_SCREEN] == 0u) {
        atomicOr(cwork.w[slot + CONE_HASH_SCREEN], 1u);
    }
}

// Whether cascade c's probe at `lod`, `cell` and `face` is wanted as a
// neighbour: its field voxel has a surface facing that way, or the level is
// past the field's coarsest (no surface is known there, so every cell is).
bool cone_probe_wanted(int lod, int c, ivec3 cell, int face) {
    int level = lod + c;
    return level > CONE_COARSEST || cone_face_area(level, cell, face) > 0.0;
}

// Put cascade c's probe at `lod`, `cell` and `face` in its hash; the slot, or
// CONE_NO_BRICK (counted) when the hash had no room or the cell is out of range.
uint cone_insert_probe(int c, int lod, ivec3 cell, int face) {
    uint key = cone_key(lod, cell, c, face);
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

// The horizon of the cone being marched: the point on its probe's own
// surface it starts from and that surface's way (cone_trace.comp sets them
// for a cone that leaves the surface; `on` false: no horizon). A coarse
// voxel is read by the area that faces the cone, wherever in the voxel that
// area is: on a pleated curtain or a thin two-sided wall the probe's own
// voxels hold area that faces its outward cones (pleat sides, the back
// face), and past the 12.5 cm clip volume, where no crossing test tells in
// front from behind, those cones closed on the cloth they started from and
// the curtain was black from 8 m off (Anthony, 25 Nov 2026; cone_pick showed
// seated probes whose own cones passed 2% to 15% on, against 50%). A voxel
// whose surface (its mean point) is not in front of the horizon, and is
// within CONE_HORIZON_NEAR voxels of it, cannot cover a cone that starts on
// it and leaves: the surface the cone starts from, not something far off
// that a grazing cone does reach. The same holds for the thin rays of the
// sun march and of what passes between a reader and a probe.
const float CONE_HORIZON_NEAR = 2.0;
bool cone_horizon_on = false;
vec3 cone_horizon_at = vec3(0.0);
vec3 cone_horizon_n = vec3(0.0);
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
        // Most steps are through air: one bit says the eight hold nothing.
        if (cone_block_empty(level, base)) {
            continue;
        }
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
            // Behind the horizon of what is being marched (cone_horizon_on):
            // the surface it starts from. A voxel of a curved or folded
            // surface holds planes of several ways, and a cone that left one
            // of them crossed another of the same voxel.
            if (cone_horizon_on) {
                uint word = crec.fv[r + CONE_SEAT_WORD];
                uvec3 q = uvec3(word, word >> 10u, word >> 20u) & uvec3(1023u);
                vec3 away = (vec3(cell) + vec3(q) * CONE_SEAT_STEP - CONE_HALF) * size - cone_horizon_at;
                if ((word & 0x80000000u) != 0u && dot(away, cone_horizon_n) <= 0.0 && dot(away, away) <= CONE_HORIZON_NEAR * CONE_HORIZON_NEAR * size * size) {
                    continue;
                }
            }
            for (int d = 0; d < 6; d++) {
                vec3 axis = cone_axis(d);
                if (-dot(dir, axis) <= CONE_TOWARD_MIN) {
                    continue;
                }
                // A negative area is a surface's back (cone_field.rs
                // written_area): it covers as the surface does and sends
                // no light.
                vec2 b_area = unpackHalf2x16(crec.fv[r + uint(2 * d + 1)]);
                if (b_area.y == 0.0) {
                    continue;
                }
                bool back = b_area.y < 0.0;
                b_area.y = abs(b_area.y);
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
                    cone_met[n].radiance = back ? vec3(0.0) : vec3(rg, b_area.x) / b_area.y;
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

bool cone_past_horizon(uint t, ivec3 cell, float size) {
    if (!cone_horizon_on) {
        return true;
    }
    uint word = ccoarse.cw[t + CONE_TEXEL_SEAT];
    if ((word & 0x80000000u) == 0u) {
        return true;
    }
    uvec3 q = uvec3(word, word >> 10u, word >> 20u) & uvec3(1023u);
    vec3 mean = (vec3(cell) + vec3(q) * CONE_SEAT_STEP - CONE_HALF) * size;
    vec3 away = mean - cone_horizon_at;
    return dot(away, cone_horizon_n) > 0.0 || dot(away, away) > CONE_HORIZON_NEAR * CONE_HORIZON_NEAR * size * size;
}

// A coarse face whose area fills this share of its voxel blocks a thin ray
// fully; less, in proportion. A face's area is an estimate, and a solid roof
// read 0.94 of a 25 cm voxel: 4% of the sun came through it, and the shade
// under the hall's roof showed bands of sun 8 m and more from the eye.
const float CONE_COARSE_SOLID = 0.75;
const float CONE_BACK_SIDES = 2.0;
// Set by cone_coarse: a back that faces the cone lies in the voxels about
// the point, on either side of it. The march walks such a step finely: a
// back before the step's middle gives the point no cover, and the step was
// passed whole, floor and all.
bool cone_back_near = false;
// The filtered read of a coarse level at `at`: the cross-section's cover by
// the areas that face the cone and by the lamps, and their light. Trilinear
// over the eight voxels around the point, all in the padded brick that holds
// the point's voxel. The lamps are read only when `lamps`: a caller that
// takes the cover alone does not pay for their eight lookups.
void cone_coarse(int level, vec3 at, vec3 dir, bool lamps, out float cover, out vec3 light, out float lamp_cover, out vec3 lamp_light) {
    cone_back_near = false;
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
    bool near = lamps && cone_lamp_about(level, base);
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
        if (!cone_past_horizon(t, base + o, size)) {
            continue;
        }
        for (int d = 0; d < 6; d++) {
            float toward = -dot(dir, cone_axis(d));
            if (toward <= 0.0) {
                continue;
            }
            vec2 ba = unpackHalf2x16(ccoarse.cw[t + uint(2 * d + 1)]);
            if (ba.y == 0.0) {
                continue;
            }
            if (ba.y < 0.0) {
                // A surface's back (cone_field.rs written_area) covers a
                // cone that comes at it from behind and sends no light. Only
                // from behind: this read has no crossing, and a cone that
                // leaves the surface's front goes the same way through the
                // same voxel.
                cone_back_near = true;
                vec2 pair = unpackHalf2x16(ccoarse.cw[t + CONE_TEXEL_PLANES + uint(d / 2)]);
                int ax = d / 2;
                float plane = (float((base + o)[ax]) + ((d & 1) == 0 ? pair.x : pair.y)) * size;
                if ((at[ax] - plane) * cone_axis(d)[ax] > 0.0) {
                    // Twice over: a surface is counted by the points on
                    // both sides of it (their weights add to one crossing),
                    // a back by those behind it alone. Counted once, a floor
                    // covered half of each cone that came up through it.
                    facing += CONE_BACK_SIDES * wt * -ba.y * toward;
                }
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

// Whether the field holds no surface in the voxel of whole level `level`
// (CONE_WHOLE_FINEST or coarser) that holds `at`, nor in the 26 beside it:
// the voxel has no brick, or its occupancy (a surface in it or beside it,
// cone_field.rs) is clear. A point is filed at every level within 6.25 cm of
// where it is, so no finer level holds a surface in that voxel either.
bool cone_clear(int level, vec3 at) {
    ivec3 cell = ivec3(floor(at / cone_size(level)));
    uint slot = cone_entry(level, cell);
    if (slot == CONE_NO_BRICK) {
        return true;
    }
    return ccoarse.cw[cone_coarse_texel(slot, (cell & ivec3(CONE_BRICK - 1)) + ivec3(1)) + 14u] == 0u;
}
// How far a ray from `at` along `dir` goes before it leaves the voxel of
// side `size` that holds `at`.
float cone_cell_exit(vec3 at, vec3 dir, float size) {
    vec3 low = floor(at / size) * size;
    vec3 edge = low + step(vec3(0.0), dir) * size;
    vec3 t = mix((edge - at) / dir, vec3(CONE_FAR), lessThan(abs(dir), vec3(CONE_TINY)));
    return max(min(t.x, min(t.y, t.z)), 0.0);
}
// The finest level a thin ray skips clear space at (25 cm, where its clip
// volume holds the point), and the margin kept from the far side of the
// clear voxels beside the one left: the fine crossing test reads the voxels
// a step and a half (under 19 cm) about its segment.
const int CONE_SKIP_NEAR = 4;
const float CONE_SKIP_MARGIN = 0.25;
const float CONE_SKIP_PAST = 0.001;
// How far a thin ray at `at`, stepping at `level`, can go on with no surface
// to cross: 0 when the space about it is not clear. The voxel it is in and
// those beside it are clear (cone_clear) at the coarsest level that is so,
// from the ray's own step level up: a coarse face's plane spans its whole
// voxel, so a finer level's clear voxel says nothing of it. The ray leaves
// its voxel and, at a whole level, crosses most of the next (clear too); at
// the 25 cm level it stops at the voxel's side, where the clip volume may end.
float cone_clear_run(int level, vec3 at, vec3 dir) {
    int x = max(level, CONE_SKIP_NEAR);
    if (x < CONE_WHOLE_FINEST && !cone_held(x, ivec3(floor(at / cone_size(x))))) {
        x = CONE_WHOLE_FINEST;
    }
    if (x > CONE_COARSEST || !cone_clear(x, at)) {
        return 0.0;
    }
    while (x < CONE_COARSEST && cone_clear(x + 1, at)) {
        x++;
    }
    float size = cone_size(x);
    return cone_cell_exit(at, dir, size) + (x >= CONE_WHOLE_FINEST ? size - CONE_SKIP_MARGIN : 0.0) + CONE_SKIP_PAST;
}
// How far a ray from `from` along `dir` goes before it leaves the box of the
// finest whole level's bricks: every surface of the scene is inside it.
float cone_field_exit(vec3 from, vec3 dir) {
    int k = CONE_WHOLE_FINEST - CONE_FINEST;
    float side = cone_size(CONE_WHOLE_FINEST) * float(CONE_BRICK);
    vec3 low = vec3(ctab.lv[2 * k].xyz) * side;
    vec3 high = low + vec3(ctab.lv[2 * k + 1].xyz) * side;
    vec3 edge = mix(low, high, step(vec3(0.0), dir));
    vec3 t = mix((edge - from) / dir, vec3(CONE_FAR), lessThan(abs(dir), vec3(CONE_TINY)));
    return max(min(t.x, min(t.y, t.z)), 0.0);
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
// The coarsest level the cone being marched reads (cone_trace.comp sets it:
// CONE_MOST_OVER levels over its cascade's probe spacing). A cone's level
// follows its width, and with eight directions cascade 0's cones are 72
// degrees wide: a 50 cm probe read 2 m and 4 m voxels a few probe spacings
// out, a voxel that size holds sunlit and shaded floor as one light, and the
// coarser a probe the brighter the same wall (at Anthony's pose B, 25 cm,
// 50 cm and 1 m probes: display 25, 38 and 50; capped: 27, 25 and 34). The
// light then changed wherever a pixel changed probe level, on every field
// rebuild of a walk. A cone wider than the voxels it reads samples its
// middle, as it does at the fine levels.
const int CONE_MOST_OVER = 1;
// The cover of the surfaces' backs (cone_field.rs written_area) the axis
// crosses from behind between `a` and `b` at a coarse level: each back's
// plane, crossed inside the voxel that holds it, by the share of the
// voxel's side its area fills (as coarse_crossed counts a thin ray). The
// filtered read (cone_coarse) counts a back by the points behind it alone,
// and how many of its eight weights those are turns on where the plane lies
// in its voxel: a floor near its voxel's low side covered a cone from below
// by a seventh, and light came through floors and walls in bands by
// distance, where one level or another did the reading (Anthony, 26 Nov
// 2026). A surface's front needs none of this: the points on both sides of
// it count it, and their weights add to one crossing wherever it lies.
float cone_back_crossed(int level, vec3 a, vec3 b, vec3 dir) {
    float size = cone_size(level);
    float cover = 0.0;
    ivec3 first = ivec3(floor(a / size));
    ivec3 last = ivec3(floor(b / size));
    for (int e = 0; e < 2; e++) {
        ivec3 cell = e == 0 ? first : last;
        if (e == 1 && last == first) {
            break;
        }
        uint slot = cone_entry(level, cell);
        if (slot == CONE_NO_BRICK) {
            continue;
        }
        uint t = cone_coarse_texel(slot, (cell & ivec3(CONE_BRICK - 1)) + ivec3(1));
        for (int d = 0; d < 6; d++) {
            vec3 axis = cone_axis(d);
            if (-dot(dir, axis) <= CONE_TOWARD_MIN) {
                continue;
            }
            float area = unpackHalf2x16(ccoarse.cw[t + uint(2 * d + 1)]).y;
            if (area >= 0.0) {
                continue;
            }
            vec2 pair = unpackHalf2x16(ccoarse.cw[t + CONE_TEXEL_PLANES + uint(d / 2)]);
            int ax = d / 2;
            float plane = (float(cell[ax]) + ((d & 1) == 0 ? pair.x : pair.y)) * size;
            float sa = (a[ax] - plane) * axis[ax];
            float sb = (b[ax] - plane) * axis[ax];
            if (!(sa > 0.0 && sb <= 0.0)) {
                continue;
            }
            cover += min(-area / (size * size) / CONE_COARSE_SOLID, 1.0);
        }
    }
    return min(cover, 1.0);
}
int cone_most_level = CONE_COARSEST;
void cone_march_least(vec3 start, vec3 dir, float width, float least, float done, float end, inout float covered, inout vec3 light);
void cone_march(vec3 start, vec3 dir, float width, float done, float end, inout float covered, inout vec3 light) {
    cone_march_least(start, dir, width, cone_size(CONE_FINEST), done, end, covered, light);
}
// As cone_march, never reading finer than a cone `least` metres across: a
// coarse voxel's sun march walks at its own size.
void cone_march_least(vec3 start, vec3 dir, float width, float least, float done, float end, inout float covered, inout vec3 light) {
    float finest = least;
    while (done < end && covered < CONE_FULL) {
        float wide = max(width * done, finest);
        int level = clamp(int(ceil(log2(wide / CONE_BASE))), CONE_FINEST, cone_most_level);
        if (level < CONE_WHOLE_FINEST) {
            level = cone_held_level(level, start + dir * done);
        }
        float step = cone_size(level);
        float from = done;
        float to = done + step;
        cone_steps += 1.0;
        if (level <= CONE_REFINE) {
            int n = cone_crossed(level, start + dir * from, start + dir * to, dir);
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
        // No lamps: the cascade-0 probes aim at them (cone_lamp.comp).
        cone_coarse(level, start + dir * (from + CONE_HALF * step), dir, false, cov, li, lc, ll);
        if (lc > 0.0) {
            light += ll * min(lc, 1.0 - covered);
        }
        if (cov > 0.0 || cone_back_near) {
            float u = from;
            while (u < to && covered < CONE_FULL) {
                // The finest level the field holds here: the refine level
                // inside its clip volume, else the finest whole level, whose
                // filtered cover is taken in place of the crossing test.
                // Not finer than CONE_REFINE_DEPTH levels under the cone's own:
                // a cone metres wide gains nothing from a walk at 12.5 cm, and
                // from far off every step re-walked to the finest level cost a
                // frame three times the old path's (Anthony, 25 Nov 2026).
                int floor_level = max(cone_held_level(CONE_REFINE, start + dir * u), level - CONE_REFINE_DEPTH);
                int l = max(level - 1, floor_level);
                while (l > floor_level && cone_occupied(l, start, dir, u, min(u + cone_size(l), to))) {
                    l--;
                }
                float next = min(u + cone_size(l), to);
                if (l <= CONE_REFINE) {
                    int n = cone_crossed(CONE_REFINE, start + dir * u, start + dir * next, dir);
                    for (int i = 0; i < n && covered < CONE_FULL; i++) {
                        cone_take(covered, light, cone_met[i]);
                    }
                } else if (l == floor_level) {
                    float fc;
                    vec3 fl;
                    float flc;
                    vec3 fll;
                    cone_coarse(l, start + dir * (CONE_HALF * (u + next)), dir, false, fc, fl, flc, fll);
                    float seen = min(fc, 1.0 - covered);
                    if (seen > 0.0) {
                        light += fl * seen;
                        covered += seen;
                    }
                    // What lies past a back the axis went through is not seen.
                    covered += min(cone_back_crossed(l, start + dir * u, start + dir * next, dir), 1.0 - covered);
                }
                u = next;
                cone_steps += 1.0;
            }
        }
        done = to;
    }
}

const float CONE_PASS_STEPS = 16.0;
// The share of light that passes from `from` to `to` at the refine level,
// stopping one step short of `to`.
float cone_passing(vec3 from, vec3 to) {
    vec3 way = to - from;
    float length_ = length(way);
    if (length_ <= CONE_TINY) {
        return 1.0;
    }
    vec3 dir = way / length_;
    float end = max(length_ - cone_size(CONE_REFINE), 0.0);
    float blocked = 0.0;
    float u = 0.0;
    // About CONE_PASS_STEPS steps at most: a test over many metres (a coarse
    // probe to a coarser one) reads a level coarse enough for that. At
    // 12.5 cm a step whatever the length, the few longest tests set how long
    // the whole pass ran.
    int least = clamp(int(ceil(log2(length_ / (CONE_PASS_STEPS * CONE_BASE)))), CONE_REFINE, CONE_COARSEST);
    while (u < end && blocked < 1.0) {
        // Past the refine level's clip volume, the finest whole level's cover.
        int level = cone_held_level(least, from + dir * u);
        float next = min(u + cone_size(level), end);
        if (level <= CONE_REFINE) {
            int n = cone_crossed(CONE_REFINE, from + dir * u, from + dir * next, dir);
            for (int i = 0; i < n; i++) {
                blocked += cone_met[i].cover;
            }
        } else {
            float cov;
            vec3 li;
            float lc;
            vec3 ll;
            cone_coarse(level, from + dir * (CONE_HALF * (u + next)), dir, false, cov, li, lc, ll);
            // And the backs the line goes through (cone_back_crossed).
            blocked += cov + cone_back_crossed(level, from + dir * u, from + dir * next, dir);
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
// A cone has two records. Its own (cone_record_at): what its own interval
// gathered, which the trace writes and a probe keeps from frame to frame
// until its turn. Its merged one (cone_merged_at): its own plus what the
// coarser cascades bring it, written by the merge for every cone every
// frame, and what the pixels, the lamp pass and the light pass read. The
// merged records lie in the other half of the work buffer, where last
// frame's own records were until the trace had kept what it keeps
// (CONE_MERGED_LAYOUT: their word offset, one a cascade). Kept with the
// merge in it, a cone's light reached the pixels a round of turns late at
// each cascade.
const uint CONE_MERGED_LAYOUT = 137u;
uint cone_merged_at(int c, uint probe, uint bin) {
    return cwork.w[CONE_MERGED_LAYOUT + uint(c)] + (probe * cone_dirs(c) + bin) * CONE_CONE_WORDS;
}
uint cone_record_at(int c, uint probe, uint bin) {
    return cone_cones_at(c) + (probe * cone_dirs(c) + bin) * CONE_CONE_WORDS;
}

// The probes of cascade c at level `lod` and `face` around `at`, trilinear, at most 8,
// each weighted by the share that passes from `from` to its seat when `see`.
// A probe the field had no seat for (seated at its cell's centre: a pixel on
// something the field does not hold) is read only as the cell `own`: whether
// it exists follows what is on screen, so no other probe or pixel leans on it.
// The least share a probe in the hash is weighed by, whatever passes to it.
// In a fold of cloth every probe about a pixel can lie past another layer:
// each passed 0, the pixel read no probe and was black, and it popped as the
// camera changed the level it read (Anthony, 25 Nov 2026: the notches down
// Sponza's curtains; cone_pick showed four lit, seated probes, each passed
// 0). With the floor such a pixel reads them by their trilinear weights
// alone; where any probe passes, a blocked one adds a thousandth of its light.
const float CONE_PASS_FLOOR = 0.001;
// Where cone_weights writes the pixel pick's level block (0: no pick).
uint cone_pick_at = 0u;
int cone_weights(int c, int lod, int face, vec3 at, bool see, vec3 from, ivec3 own, out uint probes[8], out float weights[8]) {
    float s = cone_spacing(lod, c);
    vec3 g = at / s - CONE_HALF;
    vec3 base = floor(g);
    vec3 f = g - base;
    int n = 0;
    for (int k = 0; k < 8; k++) {
        probes[k] = CONE_NO_BRICK;
        weights[k] = 0.0;
    }
    // What passes to a probe is not blocked by the surface the reader stands
    // on (cone_horizon_on): its way is the probes' own.
    cone_horizon_on = see;
    cone_horizon_at = from;
    cone_horizon_n = cone_axis(face);
    for (int k = 0; k < 8; k++) {
        ivec3 o = ivec3(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        vec3 wv = mix(vec3(1.0) - f, f, vec3(o));
        float wt = wv.x * wv.y * wv.z;
        if (wt <= 0.0) {
            continue;
        }
        ivec3 cell = ivec3(base) + o;
        uint probe = cone_find(c, cone_key(lod, cell, c, face));
        // The pick's record of this candidate: flags (1 in the hash, 2
        // seated, 4 the pixel's own cell, 8 the field has a surface that
        // way), the probe, its trilinear weight, the share that passed.
        uint pick = cone_pick_at == 0u ? 0u : cone_pick_at + 8u + uint(k) * CONE_PICK_PROBE;
        if (pick != 0u) {
            cwork.w[pick] = (probe != CONE_NO_BRICK ? 1u : 0u) | (cell == own ? 4u : 0u) | (cone_probe_wanted(lod, c, cell, face) ? 8u : 0u);
            cwork.w[pick + 1u] = probe;
            cwork.w[pick + 2u] = floatBitsToUint(wt);
            cwork.w[pick + 3u] = floatBitsToUint(-1.0);
        }
        if (probe == CONE_NO_BRICK) {
            continue;
        }
        bool seated_probe = (cwork.w[cone_probe(c, probe) + CONE_PROBE_SEATED] & CONE_FLAG_SEATED) != 0u;
        if (pick != 0u) {
            cwork.w[pick] |= seated_probe ? 2u : 0u;
            cone_put3(pick + 6u, cone_word3(cone_probe(c, probe) + 8u));
        }
        if (!seated_probe && cell != own) {
            continue;
        }
        if (see) {
            float passed = cone_passing(from, cone_word3(cone_probe(c, probe) + 8u));
            if (pick != 0u) {
                cwork.w[pick + 3u] = floatBitsToUint(passed);
            }
            wt *= max(passed, CONE_PASS_FLOOR);
        }
        if (wt > 0.0) {
            probes[n] = probe;
            weights[n] = wt;
            n++;
        }
    }
    cone_horizon_on = false;
    // The own probe unseated while seated ones stand about it: read those.
    // A fold of a curtain turns a pixel a way the field filed no surface for
    // in its own voxel; that probe sat at the voxel's centre, in or behind
    // the cloth, and the pixels on it went dark in streaks (Anthony,
    // 25 Nov 2026). A pixel with no seated probe about it (a mover) keeps it.
    int seated = 0;
    for (int k = 0; k < n; k++) {
        if ((cwork.w[cone_probe(c, probes[k]) + CONE_PROBE_SEATED] & CONE_FLAG_SEATED) != 0u) {
            seated++;
        }
    }
    if (seated > 0 && seated < n) {
        int kept = 0;
        for (int k = 0; k < n; k++) {
            if ((cwork.w[cone_probe(c, probes[k]) + CONE_PROBE_SEATED] & CONE_FLAG_SEATED) != 0u) {
                probes[kept] = probes[k];
                weights[kept] = weights[k];
                kept++;
            }
        }
        for (int k = kept; k < 8; k++) {
            probes[k] = CONE_NO_BRICK;
            weights[k] = 0.0;
        }
        n = kept;
    }
    return n;
}

// ---- The sun through the field ----

// A sun march's step grows to this share of the distance gone.
const float CONE_SUN_STEP = 0.1;

// How much of a thin ray from `from` along `dir` the field's surfaces block:
// the sun is half a degree wide, so it is a ray, not a cone. At a fine level
// the planes the ray crosses (cone_crossed); at a coarse one the faces whose
// plane (each way's surface along its axis, CONE_TEXEL_PLANES) the ray
// crosses inside the cell, each by the share of the cell's side its area
// fills. A coarse voxel's whole cross-section is a wide cone's cover, and
// counted a thin ray that passed a column's cell as blocked. Steps are a
// voxel of `level` (at the finest the field holds there) and, past the fine
// clip volumes, grow to a tenth of the distance gone.
float coarse_crossed(int level, vec3 a, vec3 b, vec3 dir);
float ray_blocked(vec3 from, vec3 dir, int level, float most);
float sun_blocked(vec3 from, vec3 dir, int level) {
    return ray_blocked(from, dir, level, CONE_REACH);
}
// How much of a thin ray from `from` to `to` the field blocks, stopping a
// finest voxel short of `to` (a lamp's own sphere is not in the field, but
// what it hangs from is beside it), reading no level finer than `level`.
float cone_ray_blocked(vec3 from, vec3 to, int level) {
    vec3 way = to - from;
    float length_ = length(way);
    float most = length_ - cone_size(CONE_FINEST);
    if (most <= 0.0) {
        return 0.0;
    }
    return ray_blocked(from, way / length_, level, most);
}
// As sun_blocked, for at most `most` metres. cone_ray_hit is then how far
// along the ray the field had blocked CONE_RAY_HIT of it (CONE_FAR: never),
// a lamp map's depth (cone_map_trace.comp).
const float CONE_RAY_HIT = 0.49;
float cone_ray_hit;
float coarse_near;
float ray_blocked(vec3 from, vec3 dir, int level, float most) {
    float covered = 0.0;
    float done = 0.0;
    cone_ray_hit = CONE_FAR;
    float least = cone_size(level);
    // Past the scene's box there is nothing to cross.
    float reach = min(most, cone_field_exit(from, dir));
    while (done < reach && covered < CONE_FULL) {
        // Inside the clip volumes the ray keeps to the finest level held
        // (planes with their points' box at 6.25 and 12.5 cm, each face's
        // plane at 25 cm): the step grows only past them, where only the
        // whole levels are held. Grown at the 25 cm volume, 2 m voxels at
        // roof edges blocked a quarter of the sunlit floor in Sponza's arcade.
        int l = cone_held_level(clamp(int(ceil(log2(least / CONE_BASE))), CONE_FINEST, CONE_COARSEST), from + dir * done);
        if (l >= CONE_WHOLE_FINEST) {
            int grown = clamp(int(ceil(log2(max(least, CONE_SUN_STEP * done) / CONE_BASE))), CONE_FINEST, CONE_COARSEST);
            l = max(l, grown);
        }
        float step_ = cone_size(l);
        vec3 a = from + dir * done;
        // Clear space is crossed in one stride: no plane is in it to cross,
        // so the cover is the same and a ray through open air costs a few
        // lookups a metre in place of sixteen records a 6.25 cm step.
        float clear = cone_clear_run(l, a, dir);
        if (clear > 0.0) {
            done += clear;
            continue;
        }
        // Not past the ray's end: what is behind a lamp does not block it.
        vec3 b = from + dir * min(done + step_, reach);
        if (l <= CONE_REFINE) {
            int n = cone_crossed(l, a, b, dir);
            for (int i = 0; i < n; i++) {
                covered += min(cone_met[i].cover, 1.0 - covered);
                if (covered >= CONE_RAY_HIT && cone_ray_hit == CONE_FAR) {
                    cone_ray_hit = done + cone_met[i].depth;
                }
            }
        } else {
            covered += min(coarse_crossed(l, a, b, dir), 1.0 - covered);
            if (covered >= CONE_RAY_HIT && cone_ray_hit == CONE_FAR) {
                cone_ray_hit = done + coarse_near;
            }
        }
        done += step_;
    }
    return clamp(covered / CONE_FULL, 0.0, 1.0);
}

// The cover the faces of the coarse voxels in the box from `a` to `b` give a
// thin ray from `a` to `b`: each face turned to the ray whose plane it crosses
// inside the voxel, by its area over the voxel's side squared. The box, not
// only the voxels holding `a` and `b`: a step across a corner crosses a wall
// in the voxel beside both, and at 25 cm the sun leaked through Sponza's
// outer walls onto the arcade floor there.

float coarse_crossed(int level, vec3 a, vec3 b, vec3 dir) {
    float size = cone_size(level);
    float cover = 0.0;
    coarse_near = length(b - a);
    ivec3 lo = min(ivec3(floor(a / size)), ivec3(floor(b / size)));
    ivec3 hi = max(ivec3(floor(a / size)), ivec3(floor(b / size)));
    for (int e = 0; e < 8; e++) {
        ivec3 cell = lo + ivec3(e & 1, (e >> 1) & 1, (e >> 2) & 1);
        if (any(greaterThan(cell, hi))) {
            continue;
        }
        uint slot = cone_entry(level, cell);
        if (slot == CONE_NO_BRICK) {
            continue;
        }
        uint t = cone_coarse_texel(slot, (cell & ivec3(CONE_BRICK - 1)) + ivec3(1));
        if (!cone_past_horizon(t, cell, size)) {
            continue;
        }
        for (int d = 0; d < 6; d++) {
            vec3 axis = cone_axis(d);
            if (-dot(dir, axis) <= CONE_TOWARD_MIN) {
                continue;
            }
            // A back (a negative area) stops the ray as its surface does.
            float area = abs(unpackHalf2x16(ccoarse.cw[t + uint(2 * d + 1)]).y);
            if (area <= 0.0) {
                continue;
            }
            vec2 pair = unpackHalf2x16(ccoarse.cw[t + CONE_TEXEL_PLANES + uint(d / 2)]);
            int ax = d / 2;
            float plane = (float(cell[ax]) + ((d & 1) == 0 ? pair.x : pair.y)) * size;
            float sa = (a[ax] - plane) * axis[ax];
            float sb = (b[ax] - plane) * axis[ax];
            if (!(sa > 0.0 && sb <= 0.0)) {
                continue;
            }
            vec3 hit = a + (b - a) * (sa / (sa - sb));
            int u = (ax + 1) % 3;
            int v = (ax + 2) % 3;
            if (int(floor(hit[u] / size)) != cell[u] || int(floor(hit[v] / size)) != cell[v]) {
                continue;
            }
            cover += min(area / (size * size) / CONE_COARSE_SOLID, 1.0);
            coarse_near = min(coarse_near, dot(hit - a, dir));
        }
    }
    return cover;
}

// Lamp maps. Each point lamp (the first CONE_MAP_HEAD + 1 of the scene's
// lights) keeps a map of how far its light goes in each direction before
// the field blocks it: an octahedral square at CONE_MAP_LEVELS sizes (32 to
// 512 texels a side), in tiles of CONE_MAP_TILE by CONE_MAP_TILE texels. A
// pixel is lit by a lamp where the map's depth toward it is not short of
// the pixel (cone_lamp_lit): a shadow's edge is where the lamp sees it, at
// the map's texel, not at the probes' spacing, and no ray is cast from a
// pixel. A tile is traced (cone_map_trace.comp) when a probe on screen asks
// for it (cone_lamp.comp) and it is not current: the lamp moved since, or
// the field changed. A still lamp's tiles are traced once.
//
// The header's map words. CONE_MAP_COUNT on are cleared each frame.
const uint CONE_MAP_COUNT = 160u;  // tiles listed for the trace this frame
const uint CONE_MAP_ASKED = 161u;  // a level: tiles asked for whose lamp moved
const uint CONE_MAP_LISTED = 166u; // a level: tiles listed
// The maps' word offset (0: none), the lamps mapped, the time in ms, the
// frame's number (from 1), the field's number, the most tiles a frame
// traces past the coarsest level's, the list's word offset and its room.
const uint CONE_MAP_HEAD = 176u;
const int CONE_MAP_LEVELS = 5;
const uint CONE_MAP_SIDE = 32u;
const uint CONE_MAP_TILE = 8u;
// A lamp's words: its version (raised when it moves; 0: never seen), its
// place and the frame it last moved; then its tiles, then its texels.
const uint CONE_MAP_LAMP_HEAD = 8u;
const uint CONE_MAP_LAMP_MOVED = 4u;
// A tile's words: the lamp's version it was traced at, the time (ms) since
// which it has been kept current, the frame a probe last asked for it, the
// field's number it was traced in, the frame it was traced.
const uint CONE_MAP_TILE_WORDS = 5u;
const uint CONE_MAP_TILES = 5456u;
// A texel is two words. The first's low half is a distance from the lamp;
// CONE_MAP_MODE says what the texel holds.
// One surface: the furthest its rays went (the low half) and the nearest as
// a share of that. A pixel no further than the furthest is lit.
// A plane: one surface. The low half is its distance at the texel's
// middle, the second word how its inverse distance changes across the
// texel (two halves; a plane's inverse distance runs straight across), and
// the high bits how far the texel's rays lie from that plane at most. A
// pixel is lit where it is no further than the plane there and that. (With one
// furthest distance for the texel, a column's side that runs back to the
// wall it stands on left the wall beside it lit: the wall is no further
// than where the column meets it. The column's short shadow on the wall
// was drawn in teeth a texel long, Anthony 26 Nov 2026.)
// An edge: two surfaces. The low half is the near one's furthest distance;
// a pixel within the slack of it is on the near surface, lit. A pixel past
// it is lit by which side it stands of the line where the near surface's
// silhouette runs in the texel (its normal's angle, toward the far side,
// and its distance from the texel's middle), found with rays
// (cone_map_trace.comp): a shadow's edge is as straight as the silhouette
// at any size on screen. A pixel past the far surface's furthest distance
// (the second word, with its nearest) is dark.
// Covered: as an edge, with the share of its rays the near surface stopped
// in place of a line, where no one line parts them (a corner, something
// thinner than the texel).
// All distances are straight from the lamp. (Brought to the texel's middle
// along each surface's plane, so one surface gave one depth, a wall the
// lamp lights at a low angle read as two surfaces or as none, and its
// shadows were drawn with streaks and teeth, Sponza 26 Nov 2026. Before
// that: one depth a texel moved an edge a whole texel at a time; a covered
// share a texel, read with its neighbours, still drew the texels.)
// Two words a texel: the first as below; the second, where the texel holds
// two surfaces, the far one's nearest and furthest distances (two halves). (That distance was once a
// step of four past the first word's: by the step, light fell on what
// stood behind the far surface.)
const uint CONE_MAP_WORDS = 2u;
const uint CONE_MAP_TEXEL_WORDS = 698368u;
const uint CONE_MAP_HALF_BITS = 0xffffu;
const uint CONE_MAP_MODE_SHIFT = 16u;
const uint CONE_MAP_MODE = 3u;
const uint CONE_MAP_ONE = 0u;
const uint CONE_MAP_EDGE = 1u;
const uint CONE_MAP_COVERED = 2u;
const uint CONE_MAP_PLANE = 3u;
// The rays of a plane texel lie within so many of these metres of its plane.
const float CONE_MAP_OFF_STEP = 0.001;
// One surface: its nearest, as the share of the furthest it falls short.
const uint CONE_MAP_SPAN_SHIFT = 18u;
const uint CONE_MAP_SPANS = 16383u;
// Two surfaces: the near one's furthest distance, the line's angle, and
// its distance or the cover.
const uint CONE_MAP_ANGLE_SHIFT = 20u;
const uint CONE_MAP_ANGLES = 64u;
const uint CONE_MAP_OFFSET_SHIFT = 26u;
const uint CONE_MAP_OFFSETS = 63u;
const float CONE_MAP_TURN = 6.2831853;
// Two surfaces in a texel are this far apart at least, and this share of
// their distance; a pixel half that before the far one is on the near one.
const float CONE_MAP_APART = 0.02;
const float CONE_MAP_APART_SHARE = 0.004;
float cone_map_apart(float depth) {
    return CONE_MAP_APART + CONE_MAP_APART_SHARE * depth;
}
vec2 cone_map_angle(uint index) {
    float a = float(index) * CONE_MAP_TURN / float(CONE_MAP_ANGLES);
    return vec2(cos(a), sin(a));
}
uint cone_map_half(float depth) {
    return packHalf2x16(vec2(depth, 0.0)) & CONE_MAP_HALF_BITS;
}
uint cone_map_pack_one(float nearest, float furthest) {
    uint span = uint(ceil(clamp(1.0 - nearest / max(furthest, CONE_TINY), 0.0, 1.0) * float(CONE_MAP_SPANS)));
    return cone_map_half(furthest) | (min(span, CONE_MAP_SPANS) << CONE_MAP_SPAN_SHIFT);
}
uint cone_map_pack_plane(float middle, float off) {
    uint steps = uint(ceil(max(off, 0.0) / CONE_MAP_OFF_STEP));
    return cone_map_half(middle) | (CONE_MAP_PLANE << CONE_MAP_MODE_SHIFT) | (min(steps, CONE_MAP_SPANS) << CONE_MAP_SPAN_SHIFT);
}
uint cone_map_pack_edge(float near, vec2 normal, vec2 through) {
    uint index = uint(round(atan(normal.y, normal.x) / CONE_MAP_TURN * float(CONE_MAP_ANGLES) + float(CONE_MAP_ANGLES))) % CONE_MAP_ANGLES;
    float offset = clamp(dot(cone_map_angle(index), through - CONE_HALF), -1.0, 1.0);
    uint steps = uint(round((offset * CONE_HALF + CONE_HALF) * float(CONE_MAP_OFFSETS)));
    return cone_map_half(near) | (CONE_MAP_EDGE << CONE_MAP_MODE_SHIFT) | (index << CONE_MAP_ANGLE_SHIFT) | (steps << CONE_MAP_OFFSET_SHIFT);
}
uint cone_map_pack_covered(float near, float cover) {
    uint steps = uint(round(clamp(cover, 0.0, 1.0) * float(CONE_MAP_OFFSETS)));
    return cone_map_half(near) | (CONE_MAP_COVERED << CONE_MAP_MODE_SHIFT) | (steps << CONE_MAP_OFFSET_SHIFT);
}
const uint CONE_MAP_LAMP_WORDS = CONE_MAP_LAMP_HEAD + CONE_MAP_TILES * CONE_MAP_TILE_WORDS + CONE_MAP_TEXEL_WORDS;
// The first tiles a side and texel words of the coarsest level.
const uint CONE_MAP_TILES_SIDE = 4u;
const uint CONE_MAP_TILES_FIRST = 16u;
const uint CONE_MAP_WORDS_FIRST = 2048u;
// A shadow's edge is blended over this many metres, or a screen pixel.
const float CONE_MAP_EDGE_WIDE = 0.02;
// A depth past this is open air (a half float's reach).
const float CONE_MAP_OPEN = 60000.0;
// A finer level's tile is blended in over this long once it is kept
// current, so a shadow sharpens and never jumps.
const float CONE_MAP_FADE_MS = 150.0;
// The sphere's square root of area over a side's texels: a texel's angle.
const float CONE_MAP_SPAN = 3.5449;
// A map texel this many screen pixels wide at the pixel is fine enough, and
// one CONE_MAP_SOFT metres wide: nine texels are read, so a shadow's edge
// is blended over twice that, the 12.5 cm the old path's shadow filter
// has. Finer, the edge showed the mesh distance field's own steps as teeth
// a texel long. (While the maps were traced through the cone field the
// least width was the field's voxel at the pixel; far from the eye that
// was 25 cm and more, and the edges there were drawn in steps.)
const float CONE_MAP_PIXELS = 1.0;
const float CONE_MAP_SOFT = 0.0625;
// The map's depth may fall this far short of a pixel turned to the lamp,
// past the shell a thin mesh has in the tracer (mf_skin): the tracer's
// surface is not the pixel's. A pixel turned from the lamp (the back of a
// two-sided surface, a curtain) is lit only where the map's depth is this
// far past it: where the tracer holds its surface, the surface shades its
// own back. (By one slack scaled with the field's voxel, a curtain's back
// was lit from 10 m away, where the voxel is larger than the shell.)
const float CONE_MAP_SLACK = 0.03;
const float CONE_MAP_DONE = 0.004;

uint cone_map_lamps() {
    return cwork.w[CONE_MAP_HEAD] == 0u ? 0u : cwork.w[CONE_MAP_HEAD + 1u];
}
uint cone_map_lamp(uint lamp) {
    return cwork.w[CONE_MAP_HEAD] + lamp * CONE_MAP_LAMP_WORDS;
}
// The words of level `level`'s first tile and first texel: the levels
// before it hold 16, 64, 256 .. tiles and four times the texels each.
uint cone_map_before(int level, uint first) {
    return first * ((1u << uint(2 * level)) - 1u) / 3u;
}
uint cone_map_tile(uint lamp, int level, ivec2 tile) {
    uint side = CONE_MAP_TILES_SIDE << uint(level);
    return cone_map_lamp(lamp) + CONE_MAP_LAMP_HEAD + (cone_map_before(level, CONE_MAP_TILES_FIRST) + uint(tile.y) * side + uint(tile.x)) * CONE_MAP_TILE_WORDS;
}
uint cone_map_texels(uint lamp, int level) {
    return cone_map_lamp(lamp) + CONE_MAP_LAMP_HEAD + CONE_MAP_TILES * CONE_MAP_TILE_WORDS + cone_map_before(level, CONE_MAP_WORDS_FIRST);
}
// A direction's place on the octahedral square (0 to 1), and back.
vec2 cone_oct(vec3 d) {
    vec3 n = d / (abs(d.x) + abs(d.y) + abs(d.z));
    if (n.z < 0.0) {
        n.xy = (1.0 - abs(n.yx)) * vec2(n.x >= 0.0 ? 1.0 : -1.0, n.y >= 0.0 ? 1.0 : -1.0);
    }
    return n.xy * CONE_HALF + CONE_HALF;
}
vec3 cone_oct_dir(vec2 uv) {
    vec2 f = (uv - CONE_HALF) / CONE_HALF;
    vec3 n = vec3(f, 1.0 - abs(f.x) - abs(f.y));
    float t = max(-n.z, 0.0);
    n.xy -= vec2(n.x >= 0.0 ? t : -t, n.y >= 0.0 ? t : -t);
    return normalize(n);
}
// A texel past the square's edge is the one mirrored along that edge.
ivec2 cone_map_wrap(ivec2 t, int side) {
    if (t.x < 0 || t.x >= side) {
        t = ivec2(clamp(t.x, 0, side - 1), side - 1 - t.y);
    }
    if (t.y < 0 || t.y >= side) {
        t = ivec2(side - 1 - t.x, clamp(t.y, 0, side - 1));
    }
    return clamp(t, ivec2(0), ivec2(side - 1));
}
// The level whose texel at `far` from the lamp is CONE_MAP_PIXELS pixels
// wide where the eye sees `pos`.
int cone_map_level(float far, vec3 pos) {
    float per_pixel = CONE_HALVES * scene.view_right.w / float(pc.dims.y);
    float want = max(CONE_MAP_PIXELS * per_pixel * length(pos - scene.eye.xyz), CONE_MAP_SOFT);
    return clamp(int(ceil(log2(far * CONE_MAP_SPAN / (float(CONE_MAP_SIDE) * want)))), 0, CONE_MAP_LEVELS - 1);
}
// The share of the lamp a texel shows a pixel `there` metres from the lamp
// that stands at `at` in the texel (0 to 1 across it; outside it, the
// texel's line is carried on). `slack` is how far short of the pixel the
// map may fall; `back` a pixel turned from the lamp, lit only where the
// lamp's rays go past it (a surface the tracer holds shades its own back);
// `soft` the edge's width in texels.
// The tracer's surface stands its skin (mf_skin) off the pixel's, and along
// the lamp's ray that is the skin over the cosine of the ray's angle to the
// pixel's normal, the cosine no less than CONE_MAP_LEAN_LEAST. cone_map_lean
// is that factor for the pixel being lit; 0 where the asker has no normal (a
// probe), and then a plane texel shows what its furthest corner shows.
const float CONE_MAP_LEAN_LEAST = 0.1;
float cone_map_lean = 0.0;
float cone_map_shows(uint word, uint more, vec2 at, float there, float slack, bool back, float soft) {
    float held = unpackHalf2x16(word & CONE_MAP_HALF_BITS).x;
    uint mode = (word >> CONE_MAP_MODE_SHIFT) & CONE_MAP_MODE;
    if (mode == CONE_MAP_PLANE) {
        vec2 slope = unpackHalf2x16(more);
        float middle = 1.0 / held;
        float off = float(word >> CONE_MAP_SPAN_SHIFT) * CONE_MAP_OFF_STEP;
        if (cone_map_lean <= 0.0) {
            float turn = CONE_HALF * (abs(slope.x) + abs(slope.y));
            return (back ? there <= 1.0 / (middle + turn) - off - slack : there <= 1.0 / max(middle - turn, 1.0 / CONE_MAP_OPEN) + off + slack) ? 1.0 : 0.0;
        }
        float plane = 1.0 / max(middle + dot(slope, at - CONE_HALF), 1.0 / CONE_MAP_OPEN);
        return (back ? there <= plane - off - slack : there <= plane + off + mf_skin() * cone_map_lean + CONE_MAP_SLACK) ? 1.0 : 0.0;
    }
    if (mode == CONE_MAP_ONE) {
        float nearest = held * (1.0 - float(word >> CONE_MAP_SPAN_SHIFT) / float(CONE_MAP_SPANS));
        return (back ? there <= nearest - slack : there <= held + slack) ? 1.0 : 0.0;
    }
    // Two surfaces: the texel holds the near one's furthest distance
    // (cone_map_trace.comp). Within the slack of it is the near surface,
    // lit; further is the far one, lit on its side of the line.
    // Where the far surface is nearer than twice the slack, half way to it
    // parts the two: something that stands a few centimetres off a wall
    // still shades it.
    vec2 fars = unpackHalf2x16(more);
    if (there <= held + min(slack, CONE_HALF * (fars.x - held))) {
        return back ? 0.0 : 1.0;
    }
    if (back || there > fars.y + slack) {
        return 0.0;
    }
    float share = float(word >> CONE_MAP_OFFSET_SHIFT) / float(CONE_MAP_OFFSETS);
    if (mode == CONE_MAP_COVERED) {
        return 1.0 - share;
    }
    float side = dot(cone_map_angle((word >> CONE_MAP_ANGLE_SHIFT) & (CONE_MAP_ANGLES - 1u)), at - CONE_HALF) - (share - CONE_HALF) / CONE_HALF;
    return smoothstep(-soft, soft, side);
}
// The share of the lamp at `from` that level `level` of its map shows
// reaching `pos`: what the texel it stands in shows at its distance. Where
// that texel is all one surface and the pixel is within the edge's width of
// a texel beside it that holds an edge, that texel's line carried on: the
// edge's blend does not stop at the texel's side. (Carried further, a line
// that ends at a corner cut dark shapes out of the lit floor past it.)
// A lamp has a width (cone_cascades.lamp_radius, the map head's ninth word),
// so a shadow's edge has a penumbra: half as wide, at the pixel, as the
// lamp's radius by how far the pixel is past the near surface over how far
// that is from the lamp. An edge is blended over that, or a pixel if wider.
// A level reads its own texels and those beside them, so it blends no wider
// than CONE_MAP_WIDE_MOST of a texel. (Read from a coarser level, a wider
// penumbra was drawn in that level's texels: they see too much to hold one
// clean line.) (With every edge a pixel wide, the steps of the tracer's
// distance field along an arch showed in its shadow on a wall metres past
// it, Anthony 26 Nov 2026.)
const float CONE_MAP_WIDE_MOST = 0.5;
float cone_map_wide = 0.0;
float cone_map_penumbra(uint word, float far, int side) {
    float near = unpackHalf2x16(word & CONE_MAP_HALF_BITS).x;
    float radius = uintBitsToFloat(cwork.w[CONE_MAP_HEAD + 8u]);
    return radius * max(far - near, 0.0) / max(near, CONE_TINY) * float(side) / (far * CONE_MAP_SPAN);
}
float cone_map_taps(uint lamp, int level, vec3 from, vec3 pos, bool back, float slack) {
    vec3 toward = pos - from;
    float far = length(toward);
    int side = int(CONE_MAP_SIDE << uint(level));
    vec2 at = cone_oct(toward / far) * float(side);
    ivec2 base = min(ivec2(floor(at)), ivec2(side - 1));
    vec2 f = at - vec2(base);
    // The edge's least half width in texels: a pixel's, or CONE_MAP_EDGE_WIDE.
    float per_pixel = CONE_HALVES * scene.view_right.w / float(pc.dims.y);
    float wide = max(per_pixel * length(pos - scene.eye.xyz), CONE_MAP_EDGE_WIDE);
    float soft = CONE_HALF * wide * float(side) / (far * CONE_MAP_SPAN);
    uint texels = cone_map_texels(lamp, level);
    uint first = texels + CONE_MAP_WORDS * uint(base.y * side + base.x);
    uint word = cwork.w[first];
    uint kind = (word >> CONE_MAP_MODE_SHIFT) & CONE_MAP_MODE;
    cone_map_wide = 0.0;
    if (kind == CONE_MAP_ONE || kind == CONE_MAP_PLANE) {
        // The texel is one surface: an edge in a texel beside it, where the
        // pixel is within that edge's width of the side between them.
        vec2 to_side = min(f, vec2(1.0) - f);
        ivec2 step_ = ivec2(f.x < CONE_MAP_WIDE_MOST ? -1 : 1, f.y < CONE_MAP_WIDE_MOST ? -1 : 1);
        for (int e = 0; e < 3; e++) {
            ivec2 o = e == 0 ? ivec2(step_.x, 0) : (e == 1 ? ivec2(0, step_.y) : step_);
            float away = e == 0 ? to_side.x : (e == 1 ? to_side.y : max(to_side.x, to_side.y));
            ivec2 t = base + o;
            if (away >= CONE_MAP_WIDE_MOST || any(lessThan(t, ivec2(0))) || any(greaterThanEqual(t, ivec2(side)))) {
                continue;
            }
            uint other = texels + CONE_MAP_WORDS * uint(t.y * side + t.x);
            uint beside = cwork.w[other];
            if (((beside >> CONE_MAP_MODE_SHIFT) & CONE_MAP_MODE) == CONE_MAP_EDGE) {
                float penumbra = cone_map_penumbra(beside, far, side);
                float reach = max(soft, min(penumbra, CONE_MAP_WIDE_MOST));
                if (away < reach) {
                    // The edge's line where it meets the side between the
                    // two texels (the pixel's nearest point on that side),
                    // fading into this texel's own answer over the width.
                    // (The line carried on into this texel drew a dark
                    // triangle in it wherever the line was not along the
                    // side: this texel's rays all met one lit surface. With
                    // the penumbra's width, half a texel, the triangles
                    // stood along a lamp's shadow on a wall, Anthony
                    // 27 Nov 2026.)
                    cone_map_wide = penumbra;
                    vec2 edge_at = vec2(o.x < 0 ? 0.0 : (o.x > 0 ? 1.0 : f.x), o.y < 0 ? 0.0 : (o.y > 0 ? 1.0 : f.y));
                    float there = cone_map_shows(beside, cwork.w[other + 1u], edge_at - vec2(o), far, slack, back, reach);
                    float own = cone_map_shows(word, cwork.w[first + 1u], f, far, slack, back, soft);
                    return mix(there, own, away / reach);
                }
            }
        }
    } else if (kind == CONE_MAP_EDGE) {
        cone_map_wide = cone_map_penumbra(word, far, side);
        soft = max(soft, min(cone_map_wide, CONE_MAP_WIDE_MOST));
    }
    return cone_map_shows(word, cwork.w[first + 1u], f, far, slack, back, soft);
}
// The share of lamp `lamp` (at `from`) that reaches `pos`: the finest level
// of its map, from `wanted` down, whose tile there is current, blended with
// the next coarser one while it is new; `other` where no level is.
float cone_lamp_lit(uint lamp, vec3 from, vec3 pos, bool back, int wanted, float other) {
    vec3 toward = pos - from;
    float far = length(toward);
    if (far <= CONE_TINY) {
        return 1.0;
    }
    vec2 uv = cone_oct(toward / far);
    uint version = cwork.w[cone_map_lamp(lamp)];
    uint now = cwork.w[CONE_MAP_HEAD + 2u];
    float bias = back ? CONE_MAP_SLACK : mf_skin() + CONE_MAP_SLACK;
    float got = 0.0;
    float left = 1.0;
    for (int k = wanted; k >= 0 && left > CONE_MAP_DONE; k--) {
        int side = int(CONE_MAP_SIDE << uint(k));
        uint tile = cone_map_tile(lamp, k, min(ivec2(uv * float(side)), ivec2(side - 1)) / int(CONE_MAP_TILE));
        if (version == 0u || cwork.w[tile] != version) {
            // The level the pixel wants, not traced: asked for here, for the
            // next frame's list (cone_map_list.comp). Asked only by the
            // probes about their own places, a tile between them that a
            // pixel wanted waited for a probe of another view to ask for
            // it, and the picture after turning away and back was sharper
            // than before (gate D).
            if (k == wanted && version != 0u) {
                // For the next frame: this frame's list has run.
                cwork.w[tile + 2u] = cwork.w[CONE_MAP_HEAD + 3u] + 1u;
            }
            continue;
        }
        float w = k == 0 ? 1.0 : clamp(float(now - cwork.w[tile + 1u]) / CONE_MAP_FADE_MS, 0.0, 1.0);
        if (w <= 0.0) {
            continue;
        }
        got += left * w * cone_map_taps(lamp, k, from, pos, back, bias);
        left *= 1.0 - w;
    }
    return got + left * other;
}

// Lamp `lamp`'s light on a pixel at `from` (lifted off its surface) of
// normal `n`: the pixel's own distance and angle to the lamp, by the share
// its map shows reaching the pixel; `blended` (the probes' shares) for a
// lamp with no map.
vec3 cone_lamp_on(uint lamp, vec3 from, vec3 n, bool two_sided, float blended) {
    Lamp light = scene_lamp(lamp);
    vec3 target;
    vec3 arrives = lamp_light(light, from, n, two_sided, target);
    if (arrives.r + arrives.g + arrives.b <= 0.0) {
        return vec3(0.0);
    }
    if (lamp >= cone_map_lamps()) {
        return arrives * blended;
    }
    vec3 toward = light.pos.xyz - from;
    bool back = dot(n, toward) < 0.0;
    cone_map_lean = 1.0 / max(dot(n, normalize(toward)), CONE_MAP_LEAN_LEAST);
    return arrives * cone_lamp_lit(lamp, light.pos.xyz, from, back, cone_map_level(length(toward), from), blended);
}
// The share of the sun the field holds for the pixel's side at `pos`:
// the sun shares the light pass marched for the voxel faces around it
// (cone_light.comp), trilinear over the finest level the field holds there,
// the faces turned the way the pixel's normal is. So the sun on a pixel is the
// sun the field's voxels see and send on as bounce (Anthony, 25 Nov 2026: the
// sun has to be in the field). Up a level where none of the eight has a face.
float field_sun(vec3 pos, vec3 n) {
    int face = cone_face(n);
    for (int level = cone_held_level(CONE_FINEST, pos); level <= CONE_COARSEST; level++) {
        float s = cone_size(level);
        vec3 g = (pos - n * (CONE_BEHIND * min(s, CONE_BEHIND))) / s - CONE_HALF;
        vec3 base = floor(g);
        vec3 f = g - base;
        float sum = 0.0;
        float weight = 0.0;
        for (int k = 0; k < 8; k++) {
            ivec3 o = ivec3(k & 1, (k >> 1) & 1, (k >> 2) & 1);
            vec3 wv = mix(vec3(1.0) - f, f, vec3(o));
            float wt = wv.x * wv.y * wv.z;
            float share = wt > 0.0 ? cone_face_sun(level, ivec3(base) + o, face) : -1.0;
            if (share >= 0.0) {
                sum += wt * share;
                weight += wt;
            }
        }
        if (weight > 0.0) {
            return sum / weight;
        }
    }
    return 1.0;
}

// A sun share this close to 0 or 1 is taken as it is; between, the pixel
// marches its own ray.
const float CONE_SUN_EDGE = 0.02;

// The sun's light on the pixel: its share from the field (field_sun), lit
// as the old direct pass lights a pixel (lamp_light, albedo over pi).
// `edge` is set when a shadow edge crosses the voxels about the pixel (they
// see part of the sun); then, when `march`, the pixel's own ray gives the
// share. The shade pass (march false) lists such a pixel and the sun pass
// (cone_sun.comp, march true) marches it: a few pixels in a hundred march,
// and scattered among the others they held every thread beside them for the
// length of their march (13 of the shade's 29 ms in Sponza).
vec3 cone_sun(vec3 pos, vec3 n, bool two_sided, bool march, out bool edge) {
    edge = false;
    vec3 from = pos + n * CONE_SURFACE_LIFT;
    LampList list = lamps_at(from);
    vec3 sum = vec3(0.0);
    float share = -1.0;
    for (uint k = 0u; k < list.suns.y; k++) {
        Lamp lamp = lamp_list_get(list, k);
        if (lamp.pos.w <= CONE_SUN_MARK) {
            continue;
        }
        vec3 target;
        vec3 light = lamp_light(lamp, from, n, two_sided, target);
        if (light.r + light.g + light.b > 0.0) {
            if (share < 0.0) {
                share = field_sun(pos, n);
                // The voxels around the pixel see part of the sun: a shadow
                // edge crosses them. The pixel marches its own thin ray
                // through the field's finest levels for a crisp edge
                // (Anthony, 25 Nov 2026); fully lit and fully shadowed pixels
                // keep the voxels' share, and the bounce always does.
                if (share > CONE_SUN_EDGE && share < 1.0 - CONE_SUN_EDGE) {
                    edge = true;
                    if (march) {
                        // Not shadowed by the surface the pixel is on.
                        cone_horizon_on = true;
                        cone_horizon_at = pos;
                        cone_horizon_n = n;
                        share = 1.0 - sun_blocked(from, -normalize(lamp.pos.xyz), CONE_FINEST);
                        cone_horizon_on = false;
                    }
                }
            }
            sum += light * share;
        }
    }
    return sum;
}
