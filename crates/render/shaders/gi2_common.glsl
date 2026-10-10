// GI v2 compute passes (gi2_gpu.rs): shared bindings, push constants and layout of
// the work buffer. Every pass that includes this has at most one trace site
// (trace_ray or trace_occluded, once): these functions are inlined wherever they
// are called, and many sites stalled the NVIDIA compiler and ran lavapipe out of
// memory (docs/systems/gi-v2-design.md, Migration plan).
//
// Work buffer (vec4s), for P probes of R rays in S strata (GI2_STRATA), a W x H
// picture and the light cache's B patches of C rays relit this frame
// (gi2_cache.glsl). Rays are numbered probe rays first (P R, this frame's
// stratum), then cache rays (B C):
//   probes      2 per probe: position (w 1 when placed), normal
//   cache rays  1 per cache ray. After the trace, a hit: x distance t (>= 0), y the
//               hit's normal (octahedral bits), z albedo times reflectance times pi
//               (unorm4x8 bits), w its emitted light (RGB9E5 bits); the hit point
//               is origin + dir t (gi2_ray). A miss or no ray already holds its
//               light, flagged by a negative x (gi2_ray_light). The light pass
//               turns each hit into the light leaving it back along the ray (rgb,
//               x >= 0) in place, so hits and radiance share the slot.
// A pixel's direct irradiance goes into its G-buffer record's w (RGB9E5).
//   filtered sh 7 per probe: irradiance SH (9 rgb coefficients), from the filtered
//               cells (gi2_filter.comp)
//   strata      2 per probe and stratum: the probe's position (w 1 while the
//               stratum's hits are kept) and normal when the stratum was traced
//   kept hits   1 per probe, stratum and ray: the hit record as traced (x distance
//               t; a miss is GI2_MISS, a dropped or absent ray GI2_NO_RAY). Hits
//               are visibility only: the light pass shades every kept hit again
//               each frame, so no light is ever carried over from a past frame.
//   kept light  1 uint (RGB9E5) per kept hit, 4 to a vec4: this frame's light
//               back along the ray
//   changes     GI2_CHANGE_HEAD + GI2_CHANGES (gi2_gpu.rs writes it each frame):
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
//               without being copied (gi2_gather.comp)
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
#include "gi2_pack.glsl"

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
    // filter pass (bits 1-2, gi2_filter_round), which rays run (bits 8-9,
    // gi2_mode), young-only rounds (bit 10), lamp picks on cache rays (bit 11)
    // and the light cache round (bits 12-31, gi2_round), w: frame
    uvec4 params;
} pc;

#include "gi2_cache_slots.glsl"

uint gi2_probes() {
    return pc.dims.z * pc.dims.w;
}
uint gi2_rays() {
    return pc.params.x;
}
// Strata of the probe ray directions: frame f traces stratum f % GI2_STRATA, and
// a probe keeps the hits of the last GI2_STRATA frames (the visibility history
// cap in gi-v2-design.md), so it sees GI2_STRATA x R directions.
// How far (in probe spacings on the surface) a probe reaches for last frame's
// kept strata (gi2_gather.comp).
const float GI2_REACH = 2.5;
const uint GI2_STRATA = 8u;
const uint GI2_CHANGES = 64u;
const uint GI2_CHANGE_HEAD = 5u;

// Which rays this dispatch runs: 0 all (one queue), 1 the picture's (pixels and
// probe rays, in the frame), 2 the light cache's (gi2_compact, trace, light and
// cache on the async compute queue, gi2_gpu.rs).
uint gi2_mode() {
    return (pc.params.z >> 8u) & 3u;
}

uint gi2_probe_at(uint i) {
    return 2u * i;
}
uint gi2_probe_rays() {
    return gi2_probes() * gi2_rays();
}
uint gi2_all_rays() {
    return gi2_probe_rays() + GI2_CACHE_BATCH * GI2_CACHE_RAYS;
}
// This frame's stratum (the frame is pc.params.w, gi2_frame in gi2_cache.glsl).
uint gi2_stratum() {
    return pc.params.w % GI2_STRATA;
}
uint gi2_shf_at(uint i) {
    return 2u * gi2_probes() + GI2_CACHE_BATCH * GI2_CACHE_RAYS + 7u * i;
}
uint gi2_stratum_at(uint probe, uint s) {
    return gi2_shf_at(gi2_probes()) + 2u * (probe * GI2_STRATA + s);
}
// Kept hits: index (probe S + s) R + k.
uint gi2_kept_hits() {
    return gi2_probe_rays() * GI2_STRATA;
}
uint gi2_kept_at(uint h) {
    return gi2_stratum_at(gi2_probes(), 0u) + h;
}
uint gi2_kept_light_at() {
    return gi2_kept_at(gi2_kept_hits());
}
uint gi2_change_at() {
    return gi2_kept_light_at() + gi2_kept_hits() / 4u;
}
uint gi2_view_at(uint block, uint probe) {
    return gi2_change_at() + GI2_CHANGE_HEAD + GI2_CHANGES + block * 2u * gi2_probes() + 2u * probe;
}
uint gi2_cell_at(uint table, uint probe, uint k) {
    return gi2_view_at(2u, 0u) + (table * gi2_probes() + probe) * gi2_rays() + k;
}
// Which spatial filter pass this dispatch is (gi2_filter.comp): 0, 1 or 2.
uint gi2_filter_round() {
    return (pc.params.z >> 1u) & 3u;
}
// A ray's hit record: probe rays go straight into this frame's stratum of the
// kept hits, cache rays into their own slots.
uint gi2_hit_at(uint ray) {
    if (ray < gi2_probe_rays()) {
        uint rays = gi2_rays();
        return gi2_kept_at((ray / rays * GI2_STRATA + gi2_stratum()) * rays + ray % rays);
    }
    return 2u * gi2_probes() + (ray - gi2_probe_rays());
}
uint gi2_rad_at(uint ray) {
    return gi2_hit_at(ray);
}
void gi2_put_kept_light(uint h, vec3 light) {
    work.v[gi2_kept_light_at() + h / 4u][h % 4u] = uintBitsToFloat(gi2_pack_rgb9e5(light));
}
vec3 gi2_kept_light(uint h) {
    return gi2_unpack_rgb9e5(floatBitsToUint(work.v[gi2_kept_light_at() + h / 4u][h % 4u]));
}

// A ray slot that already holds its final light: a miss takes the sky (x is -1 - r)
// and a ray with no source is dark (x -0.5).
vec4 gi2_miss_slot(vec3 light) {
    return vec4(-1.0 - light.r, light.g, light.b, 0.0);
}
const vec4 GI2_NO_RAY = vec4(-0.5, 0.0, 0.0, 0.0);
bool gi2_ray_none(vec4 v) {
    return v.x > -0.75 && v.x < -0.25;
}
// Light a ray brought back, after the light pass.
vec3 gi2_ray_light(vec4 v) {
    return v.x < -0.75 ? vec3(-1.0 - v.x, v.y, v.z) : (v.x < 0.0 ? vec3(0.0) : v.xyz);
}
// A kept probe ray that missed: it brings the sky as it is this frame.
const vec4 GI2_MISS = vec4(-1.0, 0.0, 0.0, 0.0);

// The eye ray through pixel p (picture y down, clip space Y flipped).
vec3 gi2_eye_dir(uvec2 p) {
    vec2 ndc = vec2((float(p.x) + 0.5) / float(pc.dims.x) * 2.0 - 1.0,
                    1.0 - (float(p.y) + 0.5) / float(pc.dims.y) * 2.0);
    float t = scene.view_right.w;
    float aspect = scene.view_forward.w;
    return normalize(scene.view_forward.xyz + ndc.x * t * aspect * scene.view_right.xyz
                     + ndc.y * t * scene.view_up.xyz);
}

// G-buffer surface at pixel p: false where the pixel shows nothing.
bool gi2_surface(uvec2 p, out vec3 pos, out vec3 n, out vec3 albedo, out bool two_sided) {
    uvec4 g = gbuf.px[p.y * pc.dims.x + p.x];
    float dist = uintBitsToFloat(g.x);
    pos = vec3(0.0);
    n = vec3(0.0, 1.0, 0.0);
    albedo = vec3(0.0);
    two_sided = false;
    if (!(dist > 0.0)) {
        return false;
    }
    pos = scene.eye.xyz + gi2_eye_dir(p) * dist;
    n = gi2_unpack_normal(g.y);
    vec4 a = unpackUnorm4x8(g.z);
    albedo = a.rgb;
    two_sided = (uint(a.a * 255.0 + 0.5) & 1u) != 0u;
    return true;
}

// ---- Tracer interface (gi-v2-design.md "Tracer interface") ---------------------
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

uint gi2_hash(uint v) {
    v ^= v >> 16u;
    v *= 0x7feb352du;
    v ^= v >> 15u;
    v *= 0x846ca68bu;
    v ^= v >> 16u;
    return v;
}

float gi2_unit(uint v) {
    return float(gi2_hash(v) >> 8u) * (1.0 / 16777216.0);
}

const float GI2_TAU = 6.2831853;
const float GI2_PI = 3.14159265;

// The hemisphere about n at (u, v) in [0, 1)^2: u is the cosine to n (uniform
// over the hemisphere), v the turn, offset by a turn from hash h.
vec3 gi2_hemi_dir(uint h, vec3 n, float u, float v) {
    float r = sqrt(max(1.0 - u * u, 0.0));
    float phi = GI2_TAU * v + gi2_unit(h) * GI2_TAU;
    vec3 helper = abs(n.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 tx = normalize(cross(helper, n));
    vec3 ty = cross(n, tx);
    return tx * (r * cos(phi)) + ty * (r * sin(phi)) + n * u;
}

// Ray k of R over the hemisphere about n, stratified on a sqrt(R) x sqrt(R) grid
// of (cos theta, phi) with a jitter and turn from hash h (gi2_cache.comp's rays).
vec3 gi2_dir(uint h, vec3 n, uint k, uint count) {
    uint s = max(uint(sqrt(float(count)) + 0.5), 1u);
    float j1 = gi2_unit(h ^ (k * 0x9e3779b9u + 1u));
    float j2 = gi2_unit(h ^ (k * 0x85ebca6bu + 2u));
    return gi2_hemi_dir(h, n, (float(k % s) + j1) / float(s), (float(k / s) + j2) / float(s));
}

// A screen probe's pattern: R cells on a sqrt(R) x sqrt(R) grid over the
// hemisphere about its normal, aimed by the surface's response alone (the cosine):
// rows are equal steps of cos^2 theta, columns equal steps of the turn. Each cell
// is cut 2 x 4 and stratum s takes part s of every cell, so one frame's R rays
// cover the hemisphere evenly on their own and the GI2_STRATA strata together
// cover it 16 x 32. The turn and the jitter come from the probe's world cell
// (25 cm), so a probe that stays put traces the same rays each time a stratum
// comes round (noise never crawls), and so does a kept stratum shaded again.
uint gi2_cell_hash(vec3 pos) {
    ivec3 cell = ivec3(floor(pos / 0.25));
    return gi2_hash(uint(cell.x) * 73856093u ^ uint(cell.y) * 19349663u ^ uint(cell.z) * 83492791u);
}
uint gi2_grid() {
    return max(uint(sqrt(float(gi2_rays())) + 0.5), 1u);
}
vec3 gi2_stratum_dir(vec3 pos, vec3 n, uint k, uint stratum) {
    uint h = gi2_cell_hash(pos);
    uint s = gi2_grid();
    float j1 = gi2_unit(h ^ (k * 0x9e3779b9u + 1u));
    float j2 = gi2_unit(h ^ (k * 0x85ebca6bu + 2u));
    float u2 = (float(k % s) + (float(stratum % 2u) + j1) * 0.5) / float(s);
    float v = (float(k / s) + (float(stratum / 2u) + j2) * 0.25) / float(s);
    return gi2_hemi_dir(h, n, sqrt(u2), v);
}
// The centre of cell k of the pattern of a probe at pos facing n.
vec3 gi2_cell_dir(vec3 pos, vec3 n, uint k) {
    uint s = gi2_grid();
    return gi2_hemi_dir(gi2_cell_hash(pos), n, sqrt((float(k % s) + 0.5) / float(s)),
                        (float(k / s) + 0.5) / float(s));
}
// Solid angle of a cell in row k % sqrt(R).
float gi2_cell_solid_angle(uint k) {
    uint s = gi2_grid();
    float r = float(k % s);
    return GI2_TAU / float(s) * (sqrt((r + 1.0) / float(s)) - sqrt(r / float(s)));
}
// Which cell of the pattern of a probe at pos facing n holds direction d, or ~0u
// below its horizon.
uint gi2_cell_of(vec3 pos, vec3 n, vec3 d) {
    float u = dot(d, n);
    if (u <= 0.0) {
        return ~0u;
    }
    uint s = gi2_grid();
    vec3 helper = abs(n.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 tx = normalize(cross(helper, n));
    vec3 ty = cross(n, tx);
    float phi = atan(dot(d, ty), dot(d, tx)) - gi2_unit(gi2_cell_hash(pos)) * GI2_TAU;
    float v = fract(phi / GI2_TAU);
    uint row = min(uint(u * u * float(s)), s - 1u);
    uint col = min(uint(v * float(s)), s - 1u);
    return row + s * col;
}

#include "gi2_cache.glsl"

// Ray k of a probe's stratum, from where the probe stood when the stratum was
// traced (the strata block, written by gi2_place.comp). False when not kept.
bool gi2_stratum_ray(uint probe, uint s, uint k, out vec3 origin, out vec3 dir, out vec3 n) {
    vec4 sp = work.v[gi2_stratum_at(probe, s)];
    n = work.v[gi2_stratum_at(probe, s) + 1u].xyz;
    origin = sp.xyz + n * 0.02;
    dir = gi2_stratum_dir(sp.xyz, n, k, s);
    return sp.w > 0.5;
}

// Ray `ray`'s origin (just off its surface) and direction: probe rays first, then
// this frame's light cache rays. False when the ray has no live source.
bool gi2_ray(uint ray, out vec3 origin, out vec3 dir, out vec3 n) {
    origin = vec3(0.0);
    dir = vec3(0.0, 1.0, 0.0);
    n = vec3(0.0, 1.0, 0.0);
    if (ray < gi2_probe_rays()) {
        uint rays = gi2_rays();
        return gi2_stratum_ray(ray / rays, gi2_stratum(), ray % rays, origin, dir, n);
    }
    uint r = ray - gi2_probe_rays();
    uint slot = gi2_cache_batch_slot(r / GI2_CACHE_RAYS);
    uint key = slot == ~0u ? 0u : ckeys.key[slot];
    // A patch claimed while the list was built may not have its surface yet.
    if (key == 0u || !(cache.v[3u * slot + 1u].w > 0.0)) {
        return false;
    }
    n = cache.v[3u * slot + 1u].xyz;
    origin = cache.v[3u * slot].xyz + n * 0.02;
    dir = gi2_dir(key, n, r % GI2_CACHE_RAYS, GI2_CACHE_RAYS);
    return true;
}

// Real SH, bands 0-2, at unit direction d.
void gi2_sh9(vec3 d, out float y[9]) {
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

float gi2_sh_coef(uint probe, uint j, uint c) {
    uint f = 3u * j + c;
    return work.v[gi2_shf_at(probe) + f / 4u][f % 4u];
}

// Irradiance at normal n from probe's filtered (already cosine-convolved) SH.
vec3 gi2_probe_irradiance(uint probe, vec3 n) {
    float y[9];
    gi2_sh9(n, y);
    vec3 e = vec3(0.0);
    [[dont_unroll]] for (uint j = 0u; j < 9u; j++) {
        e += y[j] * vec3(gi2_sh_coef(probe, j, 0u), gi2_sh_coef(probe, j, 1u), gi2_sh_coef(probe, j, 2u));
    }
    return max(e, vec3(0.0));
}
