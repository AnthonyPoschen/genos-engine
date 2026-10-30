// GI compute passes (lighting_gpu.rs): shared bindings, push constants and layout of
// the work buffer. Every pass that includes this has at most one trace site
// (trace_ray or trace_occluded, once): these functions are inlined wherever they
// are called, and many sites stalled the NVIDIA compiler and ran lavapipe out of
// memory (docs/systems/gi-design.md, Migration plan).
//
// Work buffer (vec4s), for P probes of R rays in S strata (LIGHTING_STRATA), a W x H
// picture and the light cache's B patches of C rays relit this frame
// (lighting_cache.glsl). Rays are numbered probe rays first (P R, this frame's
// stratum), then cache rays (B C):
//   probes      2 per probe: position (w 1 when placed), normal
//   cache rays  1 per cache ray. After the trace, a hit: x distance t (>= 0), y the
//               hit's normal (octahedral bits), z albedo times reflectance times pi
//               (unorm4x8 bits), w its emitted light (RGB9E5 bits); the hit point
//               is origin + dir t (lighting_ray). A miss or no ray already holds its
//               light, flagged by a negative x (lighting_ray_light). The light pass
//               turns each hit into the light leaving it back along the ray (rgb,
//               x >= 0) in place, so hits and radiance share the slot.
// A pixel's direct irradiance goes into its G-buffer record's w (RGB9E5).
//   filtered sh 7 per probe: irradiance SH (9 rgb coefficients), from the filtered
//               cells (lighting_filter.comp)
//   strata      2 per probe and stratum: the probe's position (w 1 while the
//               stratum's hits are kept) and normal when the stratum was traced
//   kept hits   1 per probe, stratum and ray: the hit record as traced (x distance
//               t; a miss is LIGHTING_MISS, a dropped or absent ray LIGHTING_NO_RAY). Hits
//               are visibility only: the light pass shades every kept hit again
//               each frame, so no light is ever carried over from a past frame.
//   kept light  1 uint (RGB9E5) per kept hit, 4 to a vec4: this frame's light
//               back along the ray
//   changes     LIGHTING_CHANGE_HEAD + LIGHTING_CHANGES (lighting_gpu.rs writes it each frame):
//               x the count of bounds below, y 1 to drop every kept stratum, z
//               which views block is last frame's (0 or 1); then the camera of
//               the last frame the probes ran (eye, w 1 when there was one; right,
//               w tan of half the fov; up; forward, w aspect); then one sphere
//               (centre, radius) per bound of an object that moved, came or went
//               since then
//   views       2 blocks (last frame's, this frame's) of 2 per probe: for each
//               stratum, 1 + the probe whose kept hits it reads (0: none). A
//               probe that moved on screen reads the strata of the probe that
//               stood where it stands now, so kept hits follow the camera
//               without being copied (lighting_gather.comp)
//   cells       2 tables of R per probe: each cell of the probe's own pattern (as
//               it stands now) holds the mean light its kept rays brought from
//               that direction (rgb) and their mean hit distance (w; -1 when the
//               cell has no ray). The gather fills table 0; the spatial filter
//               passes go 0 -> 1 -> 0 -> SH. Table 1 also holds the gather's
//               distance sums while it runs.
#extension GL_GOOGLE_include_directive : require
#extension GL_EXT_control_flow_attributes : require

#include "scene_data.glsl"
#include "scene_rays.glsl"
#include "mesh_field.glsl"
#include "lighting_pack.glsl"

layout(std430, set = 0, binding = 6) buffer GBuffer {
    uvec4 header;
    uvec4 px[];
} gbuf;

layout(std430, set = 0, binding = 7) buffer Work {
    vec4 v[];
} work;

layout(std430, set = 0, binding = 8) buffer Out {
    uint px[];
} outc;

layout(push_constant) uniform Push {
    // width, height, probe columns, probe rows
    uvec4 dims;
    // rays per probe, tile size in pixels, z: bgra output (bit 0), the spatial
    // filter pass (bits 1-2, lighting_filter_round), which rays run (bits 8-9,
    // lighting_mode), young-only rounds (bit 10), lamp picks on cache rays (bit 11)
    // and the light cache round (bits 12-31, lighting_round), w: frame
    uvec4 params;
    // Lamp picking. Zero keeps today's shade loop (lighting_direct.comp).
    // x: mode (0 off, 1 hits, 2 pixels, 3 both, 4 reuse)
    // y: candidates in bits 0-7, shadow rays in bits 8-15, unbiased in bit 16,
    //    shadow-ray counters in bit 17
    // z: spatial radius in pixels (0 skips the neighbour merge)
    // w: M cap for the biased merge
    uvec4 lamp;
} pc;

#include "lighting_cache_slots.glsl"

uint lighting_probes() {
    return pc.dims.z * pc.dims.w;
}
uint lighting_rays() {
    return pc.params.x;
}
// Strata of the probe ray directions: frame f traces stratum f % LIGHTING_STRATA, and
// a probe keeps the hits of the last LIGHTING_STRATA frames (the visibility history
// cap in gi-design.md), so it sees LIGHTING_STRATA x R directions.
// How far (in probe spacings on the surface) a probe reaches for last frame's
// kept strata (lighting_gather.comp).
const float LIGHTING_REACH = 2.5;
const uint LIGHTING_STRATA = 8u;
const uint LIGHTING_CHANGES = 64u;
const uint LIGHTING_CHANGE_HEAD = 5u;

// Which rays this dispatch runs: 0 all (one queue), 1 the picture's (pixels and
// probe rays, in the frame), 2 the light cache's (lighting_compact, trace, light and
// cache on the async compute queue, lighting_gpu.rs).
uint lighting_mode() {
    return (pc.params.z >> 8u) & 3u;
}

uint lighting_probe_at(uint i) {
    return 2u * i;
}
uint lighting_probe_rays() {
    return lighting_probes() * lighting_rays();
}
uint lighting_all_rays() {
    return lighting_probe_rays() + LIGHTING_CACHE_BATCH * LIGHTING_CACHE_RAYS;
}
// This frame's stratum (the frame is pc.params.w, lighting_frame in lighting_cache.glsl).
uint lighting_stratum() {
    return pc.params.w % LIGHTING_STRATA;
}
uint lighting_shf_at(uint i) {
    return 2u * lighting_probes() + LIGHTING_CACHE_BATCH * LIGHTING_CACHE_RAYS + 7u * i;
}
uint lighting_stratum_at(uint probe, uint s) {
    return lighting_shf_at(lighting_probes()) + 2u * (probe * LIGHTING_STRATA + s);
}
// Kept hits: index (probe S + s) R + k.
uint lighting_kept_hits() {
    return lighting_probe_rays() * LIGHTING_STRATA;
}
uint lighting_kept_at(uint h) {
    return lighting_stratum_at(lighting_probes(), 0u) + h;
}
uint lighting_kept_light_at() {
    return lighting_kept_at(lighting_kept_hits());
}
uint lighting_change_at() {
    return lighting_kept_light_at() + lighting_kept_hits() / 4u;
}
uint lighting_view_at(uint block, uint probe) {
    return lighting_change_at() + LIGHTING_CHANGE_HEAD + LIGHTING_CHANGES + block * 2u * lighting_probes() + 2u * probe;
}
uint lighting_cell_at(uint table, uint probe, uint k) {
    return lighting_view_at(2u, 0u) + (table * lighting_probes() + probe) * lighting_rays() + k;
}
// Fill: the strata a probe holds from nowhere (no probe near its place traced
// them: a surface the camera just turned or walked onto) are traced from the probe's
// place in this frame (params.z bit 6), into the fill block, which mirrors the kept
// hits (index (probe S + s) R + k) and is not kept: a filled stratum lives one frame.
// The gather's first round (params.z bit 1) picks each probe's kept strata, so the
// trace knows which to fill before the light pass shades them.
bool lighting_fill_on() {
    return ((pc.params.z >> 6u) & 1u) != 0u;
}
uint lighting_fill_at(uint h) {
    return lighting_cell_at(2u, 0u, 0u) + h;
}
uint lighting_fill_light_at() {
    return lighting_fill_at(lighting_kept_hits());
}
void lighting_put_fill_light(uint h, vec3 light) {
    work.v[lighting_fill_light_at() + h / 4u][h % 4u] = uintBitsToFloat(lighting_pack_rgb9e5(light));
}
vec3 lighting_fill_light(uint h) {
    return lighting_unpack_rgb9e5(floatBitsToUint(work.v[lighting_fill_light_at() + h / 4u][h % 4u]));
}
// The views block the gather writes this frame (its picks), 0 or 1.
uint lighting_view_next() {
    return 1u - (uint(work.v[lighting_change_at()].z) & 1u);
}
// This frame's pick for probe's stratum s: last frame's probe + 1, or 0 for none.
uint lighting_pick(uint probe, uint s) {
    vec4 v = work.v[lighting_view_at(lighting_view_next(), probe) + s / 4u];
    return floatBitsToUint(v[s % 4u]);
}
// Which spatial filter pass this dispatch is (lighting_filter.comp): 0, 1 or 2.
uint lighting_filter_round() {
    return (pc.params.z >> 1u) & 3u;
}
// A ray's hit record: probe rays go straight into this frame's stratum of the
// kept hits, cache rays into their own slots.
uint lighting_hit_at(uint ray) {
    if (ray < lighting_probe_rays()) {
        uint rays = lighting_rays();
        return lighting_kept_at((ray / rays * LIGHTING_STRATA + lighting_stratum()) * rays + ray % rays);
    }
    return 2u * lighting_probes() + (ray - lighting_probe_rays());
}
uint lighting_rad_at(uint ray) {
    return lighting_hit_at(ray);
}
void lighting_put_kept_light(uint h, vec3 light) {
    work.v[lighting_kept_light_at() + h / 4u][h % 4u] = uintBitsToFloat(lighting_pack_rgb9e5(light));
}
vec3 lighting_kept_light(uint h) {
    return lighting_unpack_rgb9e5(floatBitsToUint(work.v[lighting_kept_light_at() + h / 4u][h % 4u]));
}

// A ray slot that already holds its final light: a miss takes the sky (x is -1 - r)
// and a ray with no source is dark (x -0.5).
// w: the share of the light from the suns and the sky, as luminance (S63A; a
// miss is all sky).
vec4 lighting_miss_slot(vec3 light) {
    return vec4(-1.0 - light.r, light.g, light.b, dot(light, vec3(0.2126, 0.7152, 0.0722)));
}
const vec4 LIGHTING_NO_RAY = vec4(-0.5, 0.0, 0.0, 0.0);
bool lighting_ray_none(vec4 v) {
    return v.x > -0.75 && v.x < -0.25;
}
// Light a ray brought back, after the light pass.
vec3 lighting_ray_light(vec4 v) {
    return v.x < -0.75 ? vec3(-1.0 - v.x, v.y, v.z) : (v.x < 0.0 ? vec3(0.0) : v.xyz);
}
// A kept probe ray that missed: it brings the sky as it is this frame.
const vec4 LIGHTING_MISS = vec4(-1.0, 0.0, 0.0, 0.0);

// The eye ray through pixel p (picture y down, clip space Y flipped).
vec3 lighting_eye_dir(uvec2 p) {
    vec2 ndc = vec2((float(p.x) + 0.5) / float(pc.dims.x) * 2.0 - 1.0,
                    1.0 - (float(p.y) + 0.5) / float(pc.dims.y) * 2.0);
    float t = scene.view_right.w;
    float aspect = scene.view_forward.w;
    return normalize(scene.view_forward.xyz + ndc.x * t * aspect * scene.view_right.xyz
                     + ndc.y * t * scene.view_up.xyz);
}

// G-buffer surface at pixel p: false where the pixel shows nothing.
bool lighting_surface(uvec2 p, out vec3 pos, out vec3 n, out vec3 albedo, out bool two_sided) {
    uvec4 g = gbuf.px[p.y * pc.dims.x + p.x];
    float dist = uintBitsToFloat(g.x);
    pos = vec3(0.0);
    n = vec3(0.0, 1.0, 0.0);
    albedo = vec3(0.0);
    two_sided = false;
    if (!(dist > 0.0)) {
        return false;
    }
    pos = scene.eye.xyz + lighting_eye_dir(p) * dist;
    n = lighting_unpack_normal(g.y);
    vec4 a = unpackUnorm4x8(g.z);
    albedo = a.rgb;
    two_sided = (uint(a.a * 255.0 + 0.5) & 1u) != 0u;
    return true;
}

// ---- Tracer interface (gi-design.md "Tracer interface") ---------------------
// Software tracer: the analytic shapes (exact, scene_rays.glsl) and the mesh SDF
// instances (mesh_field.glsl). Call one of these at most once per shader.

bool trace_ray(vec3 origin, vec3 dir, float t0, float t1, out SceneHit hit) {
    bool found = scene_ray(origin, dir, t0, t1, hit);
    if (mf_trace(origin, dir, t0, hit)) {
        found = true;
    }
    return found;
}

bool trace_occluded(vec3 a, vec3 b) {
    vec3 delta = b - a;
    float dist = length(delta);
    if (dist < 1.0e-3) {
        return false;
    }
    SceneHit hit;
    return trace_ray(a, delta / dist, 1.0e-4, dist - 1.0e-3, hit);
}

uint lighting_hash(uint v) {
    v ^= v >> 16u;
    v *= 0x7feb352du;
    v ^= v >> 15u;
    v *= 0x846ca68bu;
    v ^= v >> 16u;
    return v;
}

// The one key for an integer cell (S86): each coordinate goes through the hash
// in turn, then the salt. Never XOR per-axis products (x * p1 ^ y * p2 ^ ...):
// sign-mirrored cells share that key exactly (scripts/lint-shader-hashes.py).
uint hash_cell3(ivec3 c, uint salt) {
    return lighting_hash(uint(c.x) ^ lighting_hash(uint(c.y) ^ lighting_hash(uint(c.z) ^ lighting_hash(salt))));
}

float lighting_unit(uint v) {
    return float(lighting_hash(v) >> 8u) * (1.0 / 16777216.0);
}

const float LIGHTING_TAU = 6.2831853;
const float LIGHTING_PI = 3.14159265;

// The hemisphere about n at (u, v) in [0, 1)^2: u is the cosine to n (uniform
// over the hemisphere), v the turn, offset by a turn from hash h.
vec3 lighting_hemi_dir(uint h, vec3 n, float u, float v) {
    float r = sqrt(max(1.0 - u * u, 0.0));
    float phi = LIGHTING_TAU * v + lighting_unit(h) * LIGHTING_TAU;
    vec3 helper = abs(n.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 tx = normalize(cross(helper, n));
    vec3 ty = cross(n, tx);
    return tx * (r * cos(phi)) + ty * (r * sin(phi)) + n * u;
}

// Ray k of R over the hemisphere about n, stratified on a sqrt(R) x sqrt(R) grid
// of (cos theta, phi) with a jitter and turn from hash h (lighting_cache.comp's rays).
// S72O: a texel's 64-ray set is four quarters of strata; the quarter a relight
// starts from is turned per texel and set (from its ray key), so the texels of a
// page do not take the same part of the hemisphere in the same round.
uint lighting_sc_stratum(uint rkey, uint idx, uint quarter_rays, uint set_rays) {
    return (idx + quarter_rays * ((rkey >> 28u) & 3u)) % set_rays;
}
vec3 lighting_dir(uint h, vec3 n, uint k, uint count) {
    uint s = max(uint(sqrt(float(count)) + 0.5), 1u);
    float j1 = lighting_unit(h ^ (k * 0x9e3779b9u + 1u));
    float j2 = lighting_unit(h ^ (k * 0x85ebca6bu + 2u));
    return lighting_hemi_dir(h, n, (float(k % s) + j1) / float(s), (float(k / s) + j2) / float(s));
}

// A screen probe's pattern: R cells on a sqrt(R) x sqrt(R) grid over the
// hemisphere about its normal, aimed by the surface's response alone (the cosine):
// rows are equal steps of cos^2 theta, columns equal steps of the turn. Each cell
// is cut 2 x 4 and stratum s takes part s of every cell, so one frame's R rays
// cover the hemisphere evenly on their own and the LIGHTING_STRATA strata together
// cover it 16 x 32. The turn and the jitter come from the probe's world cell
// (25 cm), so a probe that stays put traces the same rays each time a stratum
// comes round (noise never crawls), and so does a kept stratum shaded again.
uint lighting_cell_hash(vec3 pos, vec3 n) {
    // Half a cell in from the surface: a floor or wall on a cell boundary (the
    // stress scene's floor at 0 m, its roof at 3 m) would otherwise fall in one
    // cell or the next by rounding, so the pattern of a probe that moved a hair
    // (any camera motion) turned at random, and kept strata traced with one turn
    // landed unevenly in the cells of the other (blotches that crawl with a turn).
    ivec3 cell = ivec3(floor(pos / 0.25 - 0.5 * n));
    return hash_cell3(cell, 0u);
}
uint lighting_grid() {
    return max(uint(sqrt(float(lighting_rays())) + 0.5), 1u);
}
vec3 lighting_stratum_dir(vec3 pos, vec3 n, uint k, uint stratum) {
    uint h = lighting_cell_hash(pos, n);
    uint s = lighting_grid();
    float j1 = lighting_unit(h ^ (k * 0x9e3779b9u + 1u));
    float j2 = lighting_unit(h ^ (k * 0x85ebca6bu + 2u));
    float u2 = (float(k % s) + (float(stratum % 2u) + j1) * 0.5) / float(s);
    float v = (float(k / s) + (float(stratum / 2u) + j2) * 0.25) / float(s);
    return lighting_hemi_dir(h, n, sqrt(u2), v);
}
// The centre of cell k of the pattern of a probe at pos facing n: its turn's
// middle, and the middle of its cosine range (the cell's centroid over solid
// angle, where its mean light sits). The middle of the cos^2 range, the rays'
// centre, lies nearer the normal: projected there, uniform light reads 3.5 %
// too bright along the normal.
vec3 lighting_cell_dir(vec3 pos, vec3 n, uint k) {
    uint s = lighting_grid();
    float r = float(k % s);
    float u = 0.5 * (sqrt(r / float(s)) + sqrt((r + 1.0) / float(s)));
    return lighting_hemi_dir(lighting_cell_hash(pos, n), n, u, (float(k / s) + 0.5) / float(s));
}
// Solid angle of a cell in row k % sqrt(R).
float lighting_cell_solid_angle(uint k) {
    uint s = lighting_grid();
    float r = float(k % s);
    return LIGHTING_TAU / float(s) * (sqrt((r + 1.0) / float(s)) - sqrt(r / float(s)));
}
// Which cell of the pattern of a probe at pos facing n holds direction d, or ~0u
// below its horizon.
uint lighting_cell_of(vec3 pos, vec3 n, vec3 d) {
    float u = dot(d, n);
    if (u <= 0.0) {
        return ~0u;
    }
    uint s = lighting_grid();
    vec3 helper = abs(n.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 tx = normalize(cross(helper, n));
    vec3 ty = cross(n, tx);
    float phi = atan(dot(d, ty), dot(d, tx)) - lighting_unit(lighting_cell_hash(pos, n)) * LIGHTING_TAU;
    float v = fract(phi / LIGHTING_TAU);
    uint row = min(uint(u * u * float(s)), s - 1u);
    uint col = min(uint(v * float(s)), s - 1u);
    return row + s * col;
}

#include "lighting_cache.glsl"

// Ray k of a probe's stratum, from where the probe stood when the stratum was
// traced (the strata block, written by lighting_place.comp). False when not kept.
// S78K held strata (A-SVGF-style confirm, not assume): a probe standing where it
// stood when this stratum was last traced (place pass: stratum w = 2) keeps its
// hits; one ray of the stratum a frame (by hash) is re-traced as the check, and
// rays crossing a change are re-traced as before. A check whose hit moved
// (miss/hit, 2 % of its distance, its normal) marks the probe for full traces
// over the next LIGHTING_STRATA frames (probe normal w = until frame).
const bool LIGHTING_HOLD_STRATA = true;
bool lighting_stratum_held(uint probe) {
    return LIGHTING_HOLD_STRATA && work.v[lighting_stratum_at(probe, lighting_stratum())].w > 1.5;
}
uint lighting_hold_check_k(uint probe) {
    return lighting_hash(probe ^ lighting_hash(pc.params.w)) % lighting_rays();
}
float lighting_seg_dist2c(vec3 a, vec3 b, vec3 c) {
    vec3 ab = b - a;
    float t = clamp(dot(c - a, ab) / max(dot(ab, ab), 1.0e-12), 0.0, 1.0);
    vec3 d = a + ab * t - c;
    return dot(d, d);
}
// Ray k of a held probe's stratum is traced this frame: the check, or its path
// (with its kept hit distance t, < 0 a miss) crosses a change.
bool lighting_held_crosses(vec3 origin, vec3 dir, float t);
bool lighting_held_traced(uint probe, uint k, vec3 origin, vec3 dir, float t) {
    if (k == lighting_hold_check_k(probe)) {
        return true;
    }
    return lighting_held_crosses(origin, dir, t);
}
// The held ray's path (kept hit distance t, < 0 a miss) crosses a change.
bool lighting_held_crosses(vec3 origin, vec3 dir, float t) {
    vec4 head = work.v[lighting_change_at()];
    uint changes = min(uint(head.x), LIGHTING_CHANGES);
    if (head.y > 0.5) {
        return true;
    }
    vec3 end = origin + dir * (t >= 0.0 ? t : 200.0);
    [[dont_unroll]] for (uint j = 0u; j < changes; j++) {
        vec4 b = work.v[lighting_change_at() + LIGHTING_CHANGE_HEAD + j];
        if (lighting_seg_dist2c(origin, end, b.xyz) < b.w * b.w) {
            return true;
        }
    }
    return false;
}
bool lighting_stratum_ray(uint probe, uint s, uint k, out vec3 origin, out vec3 dir, out vec3 n) {
    vec4 sp = work.v[lighting_stratum_at(probe, s)];
    n = work.v[lighting_stratum_at(probe, s) + 1u].xyz;
    origin = sp.xyz + n * 0.02;
    dir = lighting_stratum_dir(sp.xyz, n, k, s);
    return sp.w > 0.5;
}

// Ray k of probe's stratum s traced from its place now (a fill), when that stratum
// is filled this frame.
bool lighting_fill_ray(uint probe, uint s, uint k, out vec3 origin, out vec3 dir, out vec3 n) {
    vec4 pp = work.v[lighting_probe_at(probe)];
    n = work.v[lighting_probe_at(probe) + 1u].xyz;
    origin = pp.xyz + n * 0.02;
    dir = vec3(0.0, 1.0, 0.0);
    if (!lighting_fill_on() || pp.w < 0.5 || s == lighting_stratum() || lighting_pick(probe, s) != 0u) {
        return false;
    }
    dir = lighting_stratum_dir(pp.xyz, n, k, s);
    return true;
}

// Ray `ray`'s origin (just off its surface) and direction: probe rays first, then
// this frame's light cache rays. False when the ray has no live source.
bool lighting_ray(uint ray, out vec3 origin, out vec3 dir, out vec3 n) {
    origin = vec3(0.0);
    dir = vec3(0.0, 1.0, 0.0);
    n = vec3(0.0, 1.0, 0.0);
    if (ray < lighting_probe_rays()) {
        uint rays = lighting_rays();
        return lighting_stratum_ray(ray / rays, lighting_stratum(), ray % rays, origin, dir, n);
    }
    uint r = ray - lighting_probe_rays();
    uint entry = r / LIGHTING_CACHE_RAYS;
    if (lighting_world_mode() == 3u && entry >= lighting_grad_first() && entry < LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES) {
        // S74A gradient check: the stored ray of the texel, same seed.
        uint ci = r - lighting_grad_first() * LIGHTING_CACHE_RAYS;
        if (ci >= lighting_grad_count()) {
            return false;
        }
        uint gs = ckeys.key[LIGHTING_GRAD_LIST_AT + ci];
        vec3 gp;
        if (ckeys.key[gs] == 0u || !lighting_sc_texel_world(gs, gp, n)) {
            return false;
        }
        vec4 gg = cache.v[LIGHTING_SC_GRAD_AT + gs];
        uint grk = floatBitsToUint(gg.z);
        origin = gp + n * 0.02;
        dir = lighting_dir(grk, n, lighting_sc_stratum(grk, floatBitsToUint(gg.w), LIGHTING_CACHE_RAYS, LIGHTING_SC_SET_RAYS), LIGHTING_SC_SET_RAYS);
        return true;
    }
    if (lighting_world_mode() == 3u && entry >= LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES) {
        // A WRC scout ray: its hit claims and reads the WRC cell there.
        if (!lighting_wrc_scout_due()) {
            return false;
        }
        lighting_wrc_scout(r - (LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES) * LIGHTING_CACHE_RAYS, origin, dir);
        n = dir;
        return true;
    }
    uint slot = lighting_cache_batch_slot(entry);
    uint key = slot == ~0u ? 0u : ckeys.key[slot];
    // A patch claimed while the list was built may not have its surface yet.
    if (key == 0u || !(cache.v[3u * slot + 1u].w > 0.0)) {
        return false;
    }
    vec3 p;
    if (!lighting_sc_texel_world(slot, p, n)) {
        return false;
    }
    origin = p + n * 0.02;
    uint q = lighting_cache_entry_quarter(r / LIGHTING_CACHE_RAYS, slot);
    uint rk = lighting_sc_ray_key(slot, key);
    dir = lighting_dir(rk, n, lighting_sc_stratum(rk, q * LIGHTING_CACHE_RAYS + r % LIGHTING_CACHE_RAYS, LIGHTING_CACHE_RAYS, LIGHTING_SC_SET_RAYS), LIGHTING_SC_SET_RAYS);
    return true;
}

// Real SH, bands 0-2, at unit direction d.
void lighting_sh9(vec3 d, out float y[9]) {
    y[0] = 0.282095;
    y[1] = 0.488603 * d.y;
    y[2] = 0.488603 * d.z;
    y[3] = 0.488603 * d.x;
    y[4] = 1.092548 * d.x * d.y;
    y[5] = 1.092548 * d.y * d.z;
    y[6] = 0.315392 * (3.0 * d.z * d.z - 1.0);
    y[7] = 1.092548 * d.x * d.z;
    y[8] = 0.546274 * (d.x * d.x - d.y * d.y);
}

float lighting_sh_coef(uint probe, uint j, uint c) {
    uint f = 3u * j + c;
    return work.v[lighting_shf_at(probe) + f / 4u][f % 4u];
}

// Irradiance at normal n from probe's filtered (already cosine-convolved) SH.
vec3 lighting_probe_irradiance(uint probe, vec3 n) {
    float y[9];
    lighting_sh9(n, y);
    vec3 e = vec3(0.0);
    [[dont_unroll]] for (uint j = 0u; j < 9u; j++) {
        e += y[j] * vec3(lighting_sh_coef(probe, j, 0u), lighting_sh_coef(probe, j, 1u), lighting_sh_coef(probe, j, 2u));
    }
    return max(e, vec3(0.0));
}
