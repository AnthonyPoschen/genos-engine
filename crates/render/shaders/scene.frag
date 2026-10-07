#version 450
layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

struct Lamp {
    vec4 pos;
    vec4 color;
};
struct Occ {
    vec4 center_shape;
    vec4 extent;
    vec4 albedo;
    vec4 bounce;
};
struct Obj {
    uvec4 range;
    vec4 color;
};
struct Puff {
    vec4 center_density;
    vec4 radius;
};
struct Cascade {
    float spacing;
    float t0;
    float t1;
    float origin_x;
    float origin_z;
    float count_x;
    float count_z;
    float dirs;
    float offset;
    float pad0;
    float pad1;
    float pad2;
};
layout(std430, set = 0, binding = 0) readonly buffer SceneData {
    uint lamp_count;
    uint occ_count;
    uint obj_count;
    uint count_x;
    uint count_z;
    float spacing;
    float origin_x;
    float origin_z;
    float near_end;
    float far_end;
    float world_end;
    uint pad2;
    Lamp lamps[32];
    Occ occs[16];
    Obj objects[32];
    vec4 eye;
    vec4 fire_pos;
    vec4 fire_color;
    Puff puffs[8];
    // xyz is the floor center. w is half extent on X.
    vec4 floor_center;
    // x is half extent on Z. yzw is the floor color.
    vec4 floor_data;
    Cascade cascades[3];
    // xyz is the camera basis. w is tan(half fov), 0, aspect, and unused.
    vec4 view_right;
    vec4 view_up;
    vec4 view_forward;
    // x is the screen-probe columns. y is the rows. z is the near-field rays plus one
    // (0: NEAR_RAYS).
    vec4 view_grid;
    // x is the roof underside height over the floor footprint (0 = no roof). yzw is its color.
    vec4 ceiling;
} scene;

#extension GL_GOOGLE_include_directive : require
#include "scene_rays.glsl"

layout(std430, set = 0, binding = 1) readonly buffer FieldData {
    vec4 texels[];
} field;

#include "tier.glsl"
#include "dither.glsl"

layout(std430, set = 0, binding = 2) readonly buffer ParticleImage {
    uint width;
    uint height;
    uint pad0;
    uint pad1;
    uint texels[];
} particle_image;

vec4 particle_texel_at(int x, int y) {
    int w = int(particle_image.width);
    int h = int(particle_image.height);
    x = clamp(x, 0, w - 1);
    y = clamp(y, 0, h - 1);
    uint packed = particle_image.texels[uint(y) * particle_image.width + uint(x)];
    float r = float(packed & 255u) / 255.0;
    float g = float((packed >> 8u) & 255u) / 255.0;
    float b = float((packed >> 16u) & 255u) / 255.0;
    float a = float((packed >> 24u) & 255u) / 255.0;
    return vec4(r, g, b, a);
}

vec4 particle_bilinear(vec2 uv) {
    float x = uv.x * float(particle_image.width) - 0.5;
    float y = uv.y * float(particle_image.height) - 0.5;
    int x0 = int(floor(x));
    int y0 = int(floor(y));
    float tx = x - float(x0);
    float ty = y - float(y0);
    vec4 c00 = particle_texel_at(x0, y0);
    vec4 c10 = particle_texel_at(x0 + 1, y0);
    vec4 c01 = particle_texel_at(x0, y0 + 1);
    vec4 c11 = particle_texel_at(x0 + 1, y0 + 1);
    return mix(mix(c00, c10, tx), mix(c01, c11, tx), ty);
}

vec4 particle_texel(vec2 uv) {
    if (uv.x < 0.0 || particle_image.width == 0u || particle_image.height == 0u) {
        return vec4(1.0);
    }
    // Spread the ember core so one screen pixel is not a bright ridge.
    vec2 step = 6.0 / vec2(float(particle_image.width), float(particle_image.height));
    vec4 sum = vec4(0.0);
    for (int dy = -1; dy <= 1; dy++) {
        for (int dx = -1; dx <= 1; dx++) {
            sum += particle_bilinear(uv + vec2(float(dx), float(dy)) * step);
        }
    }
    return sum / 9.0;
}

const float WORLD_SPACING = 2.5;
const uint WORLD_CUBE_OFFSET = 458752u;
// The world probes sit 2.5 m apart. A per-corner occluder test keeps a probe behind
// a wall out of the blend.
const bool WORLD_TAP_VISIBILITY = true;
const uint WORLD_IRR_OFFSET = 176128u;
const float LAMBERT = 0.254647909;
// A unit white lamp 7 m above a white painted floor stays near 0.37.
const float LAMP_UNIT = 72.0;
// Radius of a lamp bulb. Outside it a lamp falls off with the inverse square.
const float LAMP_RADIUS = 0.1;

bool blocked(vec3 origin, vec3 target);
bool segment_blocked(vec3 origin, vec3 target);
bool inside_solid(vec3 p);

// Cosine-weighted mean radiance over the hemisphere around face_n, the same quantity
// the pinned cells store. The old sphere mean also counted the rays below a floor
// point, which see the lit floor itself, so the far field read several times brighter
// than the near cells and the hand-off showed as a band on the floor.
vec3 world_mean(vec3 pos, vec3 face_n) {
    vec2 half_e = vec2(max(scene.floor_center.w, 0.5), max(scene.floor_data.x, 0.5));
    vec3 origin = vec3(
        scene.floor_center.x - half_e.x - WORLD_SPACING * 2.0,
        0.45,
        scene.floor_center.z - half_e.y - WORLD_SPACING * 2.0
    );
    uvec3 count = uvec3(
        min(max(uint(ceil((half_e.x * 2.0 + WORLD_SPACING * 4.0) / WORLD_SPACING)), 1u), 48u),
        3u,
        min(max(uint(ceil((half_e.y * 2.0 + WORLD_SPACING * 4.0) / WORLD_SPACING)), 1u), 48u)
    );
    vec3 local = pos - origin;
    float fx = clamp(local.x / WORLD_SPACING - 0.5, 0.0, float(count.x - 1u));
    float fy = clamp((pos.y - origin.y) / 1.5, 0.0, float(count.y - 1u));
    float fz = clamp(local.z / WORLD_SPACING - 0.5, 0.0, float(count.z - 1u));
    uvec3 i0 = uvec3(uint(floor(fx)), uint(floor(fy)), uint(floor(fz)));
    uvec3 i1 = min(i0 + uvec3(1u), count - uvec3(1u));
    vec3 t = vec3(fx, fy, fz) - vec3(i0);
    // One inside test per pixel, not one per corner.
    bool pos_inside = WORLD_TAP_VISIBILITY && inside_solid(pos);
    vec3 sum = vec3(0.0);
    float weight = 0.0;
    for (uint corner = 0u; corner < 8u; corner++) {
        uvec3 ip = uvec3(
            (corner & 1u) == 0u ? i0.x : i1.x,
            (corner & 2u) == 0u ? i0.y : i1.y,
            (corner & 4u) == 0u ? i0.z : i1.z
        );
        float w = ((corner & 1u) == 0u ? 1.0 - t.x : t.x)
            * ((corner & 2u) == 0u ? 1.0 - t.y : t.y)
            * ((corner & 4u) == 0u ? 1.0 - t.z : t.z);
        if (w <= 1.0e-5) {
            continue;
        }
        uint index = (ip.y * count.z + ip.z) * count.x + ip.x;
        // A probe inside a solid holds nothing.
        if (field.texels[WORLD_IRR_OFFSET + index].a < 0.0) {
            continue;
        }
        if (WORLD_TAP_VISIBILITY) {
            vec3 probe = origin + vec3((float(ip.x) + 0.5) * WORLD_SPACING, float(ip.y) * 1.5, (float(ip.z) + 0.5) * WORLD_SPACING);
            if (pos_inside || segment_blocked(pos, probe)) {
                continue;
            }
        }
        uint cube = WORLD_CUBE_OFFSET + index * 6u;
        vec3 nn = face_n * face_n;
        vec3 hemi = nn.x * field.texels[cube + (face_n.x >= 0.0 ? 0u : 1u)].rgb
            + nn.y * field.texels[cube + (face_n.y >= 0.0 ? 2u : 3u)].rgb
            + nn.z * field.texels[cube + (face_n.z >= 0.0 ? 4u : 5u)].rgb;
        sum += hemi * w;
        weight += w;
    }
    if (weight <= 1.0e-4) {
        return vec3(0.0);
    }
    return sum / weight;
}

const float TAU = 6.2831853;
// Rays per pixel for the light from surfaces within a probe spacing.
const uint NEAR_RAYS = 16u;

vec3 direct_at(vec3 pos, vec3 normal, bool two_sided);

// Bounce light at a face from the probes `faces` (`far`: their cosine-weighted mean
// for this face), corrected for the surfaces within a probe spacing. The probes sit up
// to a spacing away and see past a solid the face cannot: on the floor at a solid's
// foot the solid hides the lit roof above it, and its own side is all the face sees
// in that direction. Rays from the face up to one spacing find those surfaces. A ray
// that meets one takes the light leaving it: the lamps there, plus all the bounced
// light from the same probes (their cube turned to the face it met; that face is
// within a spacing of the point, the probes' own resolution). Each ray that meets a
// surface swaps what the probes see along it (their six-face cube in that direction)
// for that surface's light: the cosine-weighted estimate of the difference, added to
// the probes' answer. A ratio of the cube along the hit rays to the cube over the
// whole hemisphere took too little away beside a wall (the cube's side faces see the
// lit floor too), so corners and wall feet came out brighter than a path trace. With
// nothing near, the answer is the probes' alone.
vec3 near_field(vec3 pos, vec3 n, uint count, uint base[8], float weight[8], vec3 far) {
    uint rays = scene.view_grid.z > 0.5 ? uint(scene.view_grid.z + 0.5) - 1u : NEAR_RAYS;
    float reach = TIER_NEAR_REACH * tier_spacing();
    uint mask = rays == 0u ? 0u : near_candidates(pos, n, reach);
    if (mask == 0u) {
        return far;
    }
    vec3 faces[6];
    tier_cube6(count, base, weight, TIER_CUBE3, faces);
    vec3 origin = pos + n * 0.02;
    vec3 helper = abs(n.y) > 0.9 ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 tx = normalize(cross(helper, n));
    vec3 ty = cross(n, tx);
    // Interleaved: each pixel of a 4 x 4 block turns the spiral by its own sixteenth,
    // so neighbouring pixels cover 16 times as many directions between them. An even
    // fine dither instead of random grain.
    const uint BAYER[16] = uint[16](0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);
    uvec2 cell = uvec2(gl_FragCoord.xy) & 3u;
    float spin = (float(BAYER[cell.y * 4u + cell.x]) + 0.5) / (16.0 * float(rays));
    vec3 hit_light = vec3(0.0);
    vec3 share_hit = vec3(0.0);
    for (uint k = 0u; k < rays; k++) {
        // Cosine-weighted spiral over the hemisphere.
        float u = (float(k) + 0.5) / float(rays);
        float r = sqrt(u);
        float phi = TAU * (float(k) * 0.61803399 + spin);
        vec3 dir = tx * (r * cos(phi)) + ty * (r * sin(phi)) + n * sqrt(max(1.0 - u, 0.0));
        vec3 seen = tier_cube_dir(faces, dir);
        SceneHit hit;
        if (!scene_ray_masked(origin, dir, 1.0e-4, reach, mask, hit)) {
            continue;
        }
        share_hit += seen;
        vec3 at = origin + dir * hit.t;
        vec3 tint = mix(vec3(1.0), hit.albedo, hit.color_mix);
        vec3 incoming = direct_at(at, hit.normal, false) + 3.14159265 * tier_cube_face(faces, hit.normal);
        hit_light += tint * hit.reflect * incoming;
    }
    if (all(lessThanEqual(share_hit, vec3(0.0))) && all(lessThanEqual(hit_light, vec3(0.0)))) {
        return far;
    }
    return max(far + (hit_light - share_hit) / float(rays), vec3(0.0));
}

// Bounce light at a face: the persistent tier inside its window, the coarse world
// probes beyond it. One tier at one spacing covers the whole window, so walking never
// hands a wall from one probe level to another. Where no probe around a point holds
// light yet (a brick still waiting for its first pass) the world probes answer.
vec3 screen_bounce(vec3 world, vec3 face_n) {
    float cover = tier_cover(world, 4.0);
    uint base[8];
    float weight[8];
    uint count = cover > 0.0 ? tier_taps(world, face_n, tier_faces_of(face_n).x, base, weight) : 0u;
    if (count > 0u) {
        vec3 far = vec3(0.0);
        for (uint k = 0u; k < count; k++) {
            far += weight[k] * tier_cube_at(base[k], TIER_CUBE3, face_n);
        }
        vec3 near = near_field(world, face_n, count, base, weight, far);
        if (cover >= 1.0) {
            return near;
        }
        // Lift off the face. A point on the face can test as inside its own solid.
        return mix(world_mean(world + face_n * 0.05, face_n), near, cover);
    }
    return world_mean(world + face_n * 0.05, face_n);
}


// Shared path (scene_rays.glsl): every surface type occludes the same way.
bool inside_solid(vec3 p) {
    return scene_inside(p);
}

bool blocked(vec3 origin, vec3 target) {
    return scene_occluded(origin, target);
}

// blocked() without the inside test, for callers that test the origin once.
bool segment_blocked(vec3 origin, vec3 target) {
    vec3 delta = target - origin;
    float dist = length(delta);
    if (dist < 1e-3) {
        return false;
    }
    SceneHit hit;
    return scene_ray(origin, delta / dist, 1.0e-4, dist - 1.0e-3, hit);
}

vec3 shade_lamp(vec3 origin, vec3 normal, vec4 lamp, vec3 color, bool two_sided) {
    bool sun = lamp.w > 0.5;
    vec3 toward;
    vec3 target;
    float dist2;
    if (sun) {
        toward = -normalize(lamp.xyz);
        target = origin + toward * 80.0;
        dist2 = 49.0;
    } else {
        toward = lamp.xyz - origin;
        float dist = max(length(toward), 1.0e-4);
        toward /= dist;
        target = lamp.xyz;
        dist2 = dist * dist;
    }
    float nd = dot(toward, normal);
    if (two_sided) {
        nd = abs(nd);
    }
    if (nd <= 0.0 || blocked(origin, target)) {
        return vec3(0.0);
    }
    return color * nd * LAMP_UNIT / max(dist2, LAMP_RADIUS * LAMP_RADIUS);
}

vec3 direct_at(vec3 pos, vec3 normal, bool two_sided) {
    // A point inside a solid (floor under a footprint) starts occluded in blocked().
    vec3 origin = pos + normal * 0.02;
    vec3 incoming = vec3(0.0);
    uint lamps = min(scene.lamp_count, 32u);
    for (uint i = 0u; i < lamps; i++) {
        incoming += shade_lamp(origin, normal, scene.lamps[i].pos, scene.lamps[i].color.rgb, two_sided);
    }
    if (scene.fire_pos.w > 0.0) {
        incoming += shade_lamp(
            origin,
            normal,
            vec4(scene.fire_pos.xyz, 0.0),
            scene.fire_color.rgb * scene.fire_pos.w,
            two_sided
        );
    }
    return incoming;
}

vec3 sample_field(vec2 xz) {
    return screen_bounce(vec3(xz.x, 0.0, xz.y), vec3(0.0, 1.0, 0.0));
}

float sphere_chord(vec3 origin, vec3 dir, vec3 center, float radius) {
    vec3 oc = origin - center;
    float b = dot(oc, dir);
    float c = dot(oc, oc) - radius * radius;
    float disc = b * b - c;
    if (disc < 0.0) {
        return 0.0;
    }
    float s = sqrt(disc);
    float t0 = -b - s;
    float t1 = -b + s;
    if (t1 < 0.0) {
        return 0.0;
    }
    return max(t1 - max(t0, 0.0), 0.0);
}

vec2 fog_field_xz(vec2 xz) {
    uint count = min(scene.occ_count, 16u);
    for (uint i = 0u; i < count; i++) {
        Occ occ = scene.occs[i];
        vec2 d = xz - occ.center_shape.xz;
        if (occ.center_shape.w > 0.5) {
            if (dot(d, d) < occ.extent.w * occ.extent.w) {
                vec2 n = dot(d, d) < 1.0e-6 ? vec2(1.0, 0.0) : normalize(d);
                return occ.center_shape.xz + n * (occ.extent.w + 0.1);
            }
        } else if (abs(d.x) < occ.extent.x - 0.001 && abs(d.y) < occ.extent.z - 0.001) {
            float ax = abs(d.x) / max(occ.extent.x, 1.0e-4);
            float az = abs(d.y) / max(occ.extent.z, 1.0e-4);
            if (ax >= az) {
                float s = d.x < 0.0 ? -1.0 : 1.0;
                return vec2(occ.center_shape.x + s * (occ.extent.x + 0.1), xz.y);
            }
            float s = d.y < 0.0 ? -1.0 : 1.0;
            return vec2(xz.x, occ.center_shape.y + s * (occ.extent.z + 0.1));
        }
    }
    return xz;
}

vec3 scatter_light(vec3 p) {
    vec3 sum = vec3(0.0);
    uint lamps = min(scene.lamp_count, 32u);
    for (uint i = 0u; i < lamps; i++) {
        vec4 lamp = scene.lamps[i].pos;
        if (lamp.w > 0.5) {
            vec3 toward = -normalize(lamp.xyz);
            if (blocked(p, p + toward * 80.0)) {
                continue;
            }
            sum += scene.lamps[i].color.rgb * LAMP_UNIT / 50.0;
            continue;
        }
        if (blocked(p, lamp.xyz)) {
            continue;
        }
        vec3 delta = lamp.xyz - p;
        float fall = LAMP_UNIT / max(dot(delta, delta), LAMP_RADIUS * LAMP_RADIUS);
        sum += scene.lamps[i].color.rgb * fall;
    }
    if (scene.fire_pos.w > 0.0) {
        vec3 fire = scene.fire_pos.xyz;
        if (!blocked(p, fire)) {
            vec3 delta = fire - p;
            float fall = scene.fire_pos.w * LAMP_UNIT / max(dot(delta, delta), LAMP_RADIUS * LAMP_RADIUS);
            sum += scene.fire_color.rgb * fall;
        }
    }
    sum += sample_field(fog_field_xz(p.xz));
    return sum;
}

vec4 fog_fragment(vec3 pos) {
    vec3 eye = scene.eye.xyz;
    vec3 dir = pos - eye;
    float len = length(dir);
    if (len < 1.0e-4) {
        return vec4(0.0);
    }
    dir /= len;
    float trans = 1.0;
    vec3 scatter = vec3(0.0);
    uint count = min(uint(scene.eye.w + 0.5), 8u);
    for (uint i = 0u; i < count; i++) {
        vec3 center = scene.puffs[i].center_density.xyz;
        float density = scene.puffs[i].center_density.w;
        float radius = scene.puffs[i].radius.x;
        if (density <= 0.0 || radius <= 0.0) {
            continue;
        }
        float chord = sphere_chord(eye, dir, center, radius);
        if (chord <= 0.0) {
            continue;
        }
        float slice = density * chord;
        float absorb = 1.0 - exp(-slice);
        float t = max(dot(center - eye, dir), 0.0);
        vec3 mid = eye + dir * t;
        scatter += trans * absorb * scatter_light(mid) * 0.62;
        trans *= exp(-slice);
    }
    return vec4(scatter, 1.0 - trans);
}

float tone_channel(float x) {
    // A shadow stays linear. A hot corridor wall bends instead of clipping.
    if (x <= 0.64) {
        return max(x, 0.0);
    }
    float extra = x - 0.64;
    return 0.64 + 0.14 * (extra / (extra + 1.1));
}

vec3 tone(vec3 color) {
    return vec3(tone_channel(color.r), tone_channel(color.g), tone_channel(color.b));
}

void main() {
    if (v_shade > 1.5) {
        vec4 fog = fog_fragment(v_pos);
        // A miss must not write depth. The card sits in front of the flames.
        if (fog.a < 0.004) {
            discard;
        }
        out_color = fog;
        return;
    }
    vec4 texel = particle_texel(v_uv);
    if (v_uv.x >= 0.0 && texel.a < 0.04) {
        discard;
    }
    vec3 albedo = v_albedo * texel.rgb;
    if (v_shade < 0.5) {
        out_color = vec4(albedo * texel.a, texel.a);
        return;
    }
    vec3 normal = normalize(v_normal);
    // A camera card is thin. The lamp can light the visible side from either face.
    bool two_sided = v_shade > 1.15 && v_shade < 1.5;
    vec3 direct = direct_at(v_pos, normal, two_sided);
    // The probes store the cosine-weighted mean radiance. Irradiance is pi times that,
    // and the direct term below is irradiance too.
    vec3 bounce = 3.14159265 * screen_bounce(v_pos, normal);
    // A face point inside geometry (floor under a footprint) receives nothing.
    if (inside_solid(v_pos + normal * 0.02)) {
        bounce = vec3(0.0);
    }
    vec3 color = tone(albedo * LAMBERT * (direct + bounce));
    // An opaque face goes straight to the 8-bit target: dither its rounding. A
    // blended card stays exact so its layers do not stack noise.
    if (texel.a >= 1.0) {
        color = dither8(color, uvec2(gl_FragCoord.xy));
    }
    out_color = vec4(color * texel.a, texel.a);
}
