#version 450
#extension GL_GOOGLE_include_directive : require
#include "reflections.glsl"

layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

struct Plane {
    vec4 origin_hu;
    vec4 normal_hv;
    vec4 color;
    vec4 tangent;
    vec4 bitangent;
};

layout(std430, set = 0, binding = 0) readonly buffer WaterData {
    vec4 eye;
    vec4 forward;
    vec4 right;
    vec4 up;
    vec4 params;
    vec4 sky;
    Plane planes[8];
} water;

layout(set = 0, binding = 1) uniform sampler2D scene_depth;

const float HIT_DENOM_EPS = 1.0e-6;
const float HIT_MIN_T = 0.01;
const float MISS_T = 1.0e9;
const float COUNT_ROUND = 0.5;
const float NDC_SCALE = 2.0;
const float DEPTH_ALONG_EPS = 1.0e-4;
const float NORMAL_EPS = 1.0e-8;
const float CLEAR_DEPTH = 0.9995;
const float COS_T_FLOOR = 0.05;

bool hit_plane(vec3 origin, vec3 dir, Plane p, out float travel, out vec3 color) {
    vec3 normal = p.normal_hv.xyz;
    float denom = dot(normal, dir);
    if (abs(denom) < HIT_DENOM_EPS) {
        return false;
    }
    travel = dot(normal, p.origin_hu.xyz - origin) / denom;
    if (travel < HIT_MIN_T) {
        return false;
    }
    vec3 at = origin + dir * travel;
    vec3 rel = at - p.origin_hu.xyz;
    float along_u = dot(rel, p.tangent.xyz);
    float along_v = dot(rel, p.bitangent.xyz);
    if (abs(along_u) > p.origin_hu.w || abs(along_v) > p.normal_hv.w) {
        return false;
    }
    color = p.color.rgb;
    return true;
}

vec3 nearest_plane(vec3 origin, vec3 dir, vec3 miss) {
    float best = MISS_T;
    vec3 color = miss;
    int count = int(water.sky.w + COUNT_ROUND);
    for (int i = 0; i < 8; i++) {
        if (i >= count) {
            break;
        }
        float travel;
        vec3 hit_color;
        if (hit_plane(origin, dir, water.planes[i], travel, hit_color) && travel < best) {
            best = travel;
            color = hit_color;
        }
    }
    return color;
}

vec3 reconstruct(float depth, vec2 frag) {
    vec2 size = max(water.params.zw, vec2(1.0));
    vec2 uv = frag / size;
    vec2 ndc = uv * NDC_SCALE - 1.0;
    float hx = ndc.x * water.up.w * water.params.y;
    float hy = -ndc.y * water.params.y;
    vec3 dir = normalize(water.forward.xyz + water.right.xyz * hx + water.up.xyz * hy);
    const float near_z = 0.05;
    const float far_z = 200.0;
    float row_z = far_z / (near_z - far_z);
    float row_w = (far_z * near_z) / (near_z - far_z);
    float z_cam = -row_w / (depth + row_z);
    float dist_fwd = -z_cam;
    float along = max(dot(water.forward.xyz, dir), DEPTH_ALONG_EPS);
    return water.eye.xyz + dir * (dist_fwd / along);
}

void main() {
    vec3 eye = water.eye.xyz;
    vec3 incident = normalize(v_pos - eye);
    vec3 geom = dot(v_normal, v_normal) > NORMAL_EPS ? normalize(v_normal) : vec3(0.0, 1.0, 0.0);
    bool below = dot(geom, eye - v_pos) < 0.0;
    vec3 facing = below ? -geom : geom;
    // eta_t / eta_i. From under the surface the ratio is the reciprocal.
    float eta_ratio = below ? (1.0 / WATER_IOR) : WATER_IOR;
    float cos_i = clamp(dot(facing, -incident), 0.0, 1.0);
    float fresnel = reflection_fresnel(eta_ratio, cos_i);
    vec3 reflected_dir = reflection_mirror_dir(incident, facing);
    vec3 transmitted_dir = incident;
    bool bent = reflection_refract(incident, facing, 1.0 / eta_ratio, transmitted_dir);
    vec3 reflected = nearest_plane(v_pos, reflected_dir, water.sky.xyz);
    vec3 transmitted = nearest_plane(v_pos, incident, v_albedo);
    float beer_dist = 0.0;
    vec2 frag = gl_FragCoord.xy;
    vec2 uv = frag / max(water.params.zw, vec2(1.0));
    float depth = texture(scene_depth, uv).r;
    if (depth < CLEAR_DEPTH) {
        vec3 hit = reconstruct(depth, frag);
        float thickness = abs(dot(hit - v_pos, facing));
        float cos_t = bent ? max(abs(dot(normalize(transmitted_dir), facing)), COS_T_FLOOR) : 1.0;
        beer_dist = thickness / cos_t;
    }
    vec3 shaded = reflection_water_shade(fresnel, reflected, transmitted, water.params.x, beer_dist);
    out_color = vec4(shaded, 1.0);
}
