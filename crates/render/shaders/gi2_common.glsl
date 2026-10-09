// GI v2 compute passes (gi2_gpu.rs): shared bindings, push constants and layout of
// the work buffer. Every pass that includes this has at most one trace site
// (trace_ray or trace_occluded, once): these functions are inlined wherever they
// are called, and many sites stalled the NVIDIA compiler and ran lavapipe out of
// memory (docs/systems/gi-v2-design.md, Migration plan).
//
// Work buffer (vec4s), for P probes of R rays, a W x H picture and the light
// cache's B patches of C rays relit this frame (gi2_cache.glsl). Rays are numbered
// probe rays first (P R), then cache rays (B C):
//   probes      2 per probe: position (w 1 when placed), normal
//   rays        1 per ray. After the trace, a hit: x distance t (>= 0), y the hit's
//               normal (octahedral bits), z albedo times reflectance times pi
//               (unorm4x8 bits), w its emitted light (RGB9E5 bits); the hit point
//               is origin + dir t (gi2_ray). A miss or no ray already holds its
//               light, flagged by a negative x (gi2_ray_light). The light pass
//               turns each hit into the light leaving it back along the ray (rgb,
//               x >= 0) in place, so hits and radiance share the slot.
// A pixel's direct irradiance goes into its G-buffer record's w (RGB9E5).
//   sh          7 per probe: irradiance SH, 9 rgb coefficients
//   filtered sh 7 per probe: the same after the spatial filter (gi2_filter.comp)
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
    // rays per probe, tile size in pixels, bgra output (1), flags
    uvec4 params;
} pc;

#include "gi2_cache_slots.glsl"

uint gi2_probes() {
    return pc.dims.z * pc.dims.w;
}
uint gi2_rays() {
    return pc.params.x;
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
uint gi2_hit_at(uint ray) {
    return 2u * gi2_probes() + ray;
}
uint gi2_rad_at(uint ray) {
    return gi2_hit_at(ray);
}
uint gi2_sh_at(uint i) {
    return 2u * gi2_probes() + gi2_all_rays() + 7u * i;
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
uint gi2_shf_at(uint i) {
    return gi2_sh_at(gi2_probes()) + 7u * i;
}

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

// Ray k of a screen probe at pos facing n: uniform over the hemisphere, stratified
// on a sqrt(R) x sqrt(R) grid of (cos theta, phi). The rotation and jitter come from
// a hash of the probe's world cell (25 cm), never the frame, so a probe that stays
// put shoots the same directions every frame and any leftover noise does not crawl.
vec3 gi2_dir(uint h, vec3 n, uint k, uint count) {
    uint s = max(uint(sqrt(float(count)) + 0.5), 1u);
    float phi0 = gi2_unit(h) * GI2_TAU;
    float j1 = gi2_unit(h ^ (k * 0x9e3779b9u + 1u));
    float j2 = gi2_unit(h ^ (k * 0x85ebca6bu + 2u));
    float u = (float(k % s) + j1) / float(s);
    float v = (float(k / s) + j2) / float(s);
    float r = sqrt(max(1.0 - u * u, 0.0));
    float phi = GI2_TAU * v + phi0;
    vec3 helper = abs(n.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 tx = normalize(cross(helper, n));
    vec3 ty = cross(n, tx);
    return tx * (r * cos(phi)) + ty * (r * sin(phi)) + n * u;
}

vec3 gi2_ray_dir(vec3 pos, vec3 n, uint k) {
    ivec3 cell = ivec3(floor(pos / 0.25));
    uint h = gi2_hash(uint(cell.x) * 73856093u ^ uint(cell.y) * 19349663u ^ uint(cell.z) * 83492791u);
    return gi2_dir(h, n, k, gi2_rays());
}

#include "gi2_cache.glsl"

// Ray `ray`'s origin (just off its surface) and direction: probe rays first, then
// this frame's light cache rays. False when the ray has no live source.
bool gi2_ray(uint ray, out vec3 origin, out vec3 dir, out vec3 n) {
    origin = vec3(0.0);
    dir = vec3(0.0, 1.0, 0.0);
    n = vec3(0.0, 1.0, 0.0);
    if (ray < gi2_probe_rays()) {
        uint rays = gi2_rays();
        uint probe = ray / rays;
        vec4 pp = work.v[gi2_probe_at(probe)];
        if (pp.w < 0.5) {
            return false;
        }
        n = work.v[gi2_probe_at(probe) + 1u].xyz;
        origin = pp.xyz + n * 0.02;
        dir = gi2_ray_dir(pp.xyz, n, ray % rays);
        return true;
    }
    uint r = ray - gi2_probe_rays();
    uint slot = gi2_cache_batch_slot(r / GI2_CACHE_RAYS);
    uint key = slot == ~0u ? 0u : ckeys.key[slot];
    if (key == 0u) {
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
