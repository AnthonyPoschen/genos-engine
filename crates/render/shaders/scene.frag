#version 450
layout(location = 0) in vec3 v_pos;
layout(location = 1) in vec3 v_albedo;
layout(location = 2) in vec3 v_normal;
layout(location = 3) in float v_shade;
layout(location = 4) in vec2 v_uv;
layout(location = 0) out vec4 out_color;

#extension GL_GOOGLE_include_directive : require
#extension GL_EXT_control_flow_attributes : require
#include "scene_data.glsl"

#include "scene_rays.glsl"

layout(std430, set = 0, binding = 1) readonly buffer FieldData {
    vec4 texels[];
} field;

// The tier light the picture shows (light.comp pass 13): the probes' top cube moved
// toward the field a little each frame, so builds landing never show as steps.
layout(std430, set = 0, binding = 4) readonly buffer TierView {
    vec4 texels[];
} tier_view;
#define TIER_VIEW

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

bool blocked(vec3 origin, vec3 target);
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
            if (pos_inside || scene_segment_blocked(pos, probe)) {
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
// Per-pixel scene marches cost more than the frame budget. The probe is the
// bounce. A bright indoor probe sees over nearby solids, so main() scales it.
const uint NEAR_RAYS = 0u;

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
//
// The rays are shared across each 2 x 2 pixel quad: the pixel `lane` of `lanes` casts
// rays lane, lane + lanes, ...; the quad adds up what its pixels found (quad_sum).
// Returns the summed difference (rgb) and the rays cast (a).
vec4 near_rays(vec3 pos, vec3 n, uint count, uint base[8], float weight[8], uint lane, uint lanes) {
    uint rays = scene.view_grid.z > 0.5 ? uint(scene.view_grid.z + 0.5) - 1u : NEAR_RAYS;
    if (lane >= rays) {
        return vec4(0.0);
    }
    float own = float((rays - lane + lanes - 1u) / lanes);
    // Far enough to meet the ceiling and a nearby box. Open ground stays on the probes.
    float reach = max(TIER_NEAR_REACH * tier_spacing(), 8.0);
    uint mask = near_candidates(pos, n, reach);
    if (mask == 0u) {
        return vec4(0.0);
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
    for (uint k = lane; k < rays; k += lanes) {
        // Cosine-weighted spiral over the hemisphere.
        float u = (float(k) + 0.5) / float(rays);
        float r = sqrt(u);
        float phi = TAU * (float(k) * 0.61803399 + spin);
        vec3 dir = tx * (r * cos(phi)) + ty * (r * sin(phi)) + n * sqrt(max(1.0 - u, 0.0));
        vec3 seen = tier_cube_dir(faces, dir);
        SceneHit hit;
        if (!scene_ray_masked(origin, dir, 1.0e-4, reach, mask, hit)) {
            hit_light += seen;
            continue;
        }
        vec3 at = origin + dir * hit.t;
        vec3 tint = mix(vec3(1.0), hit.albedo, hit.color_mix);
        hit_light += tint * hit.reflect * direct_at(at, hit.normal, false);
    }
    return vec4(hit_light, own);
}

// The sum of `v` over the pixel's 2 x 2 quad. Fine derivatives are the difference to
// the neighbour across x, then across y, so each pixel rebuilds its neighbours' values.
// Every pixel of the quad must call it (all of a quad lies on one primitive).
vec4 quad_sum(vec4 v) {
    float sx = (uint(gl_FragCoord.x) & 1u) == 0u ? 1.0 : -1.0;
    float sy = (uint(gl_FragCoord.y) & 1u) == 0u ? 1.0 : -1.0;
    vec4 pair = 2.0 * v + sx * dFdxFine(v);
    return 2.0 * pair + sy * dFdyFine(pair);
}

// Bounce light at a face: the persistent tier inside its window, the coarse world
// probes beyond it. One tier at one spacing covers the whole window, so walking never
// hands a wall from one probe level to another. Where no probe around a point holds
// light yet (a brick still waiting for its first pass) the world probes answer.
//
// `quad`: the pixel's quad shares the near rays (every pixel of the quad must take
// this path). `casts`: this pixel's rays count; a helper pixel or one inside a solid
// adds nothing to its quad.
vec3 screen_bounce(vec3 world, vec3 face_n, bool quad, bool casts) {
    float cover = tier_cover(world, 4.0);
    uint base[8];
    float weight[8];
    uint count = cover > 0.0 ? tier_taps(world, face_n, tier_faces_of(face_n).x, base, weight) : 0u;
    uint lanes = quad ? 4u : 1u;
    uint lane = quad ? (uint(gl_FragCoord.x) & 1u) + 2u * (uint(gl_FragCoord.y) & 1u) : 0u;
    vec3 far = vec3(0.0);
    vec4 near = vec4(0.0);
    if (count > 0u) {
        for (uint k = 0u; k < count; k++) {
            far += weight[k] * tier_cube_at(base[k], TIER_CUBE3, face_n);
        }
        if (casts) {
            near = near_rays(world, face_n, count, base, weight, lane, lanes);
        }
    }
    if (quad) {
        near = quad_sum(near);
    }
    // The north yard sits on the window edge, so most of its pixels read the
    // world volume, including through the fade. With no sky that volume stops
    // a little under the night trace. Daylight has a sky, and the volume
    // already matches, so this lift stays off.
    bool night_floor = face_n.y > 0.5 && !has_sky();
    if (count == 0u) {
        vec3 coarse = world_mean(world + face_n * 0.05, face_n);
        if (night_floor) {
            coarse *= 1.80;
        }
        return coarse;
    }
    // near.rgb is the ray average where a surface is close. Adding it on top of
    // the probe kept light the point cannot see. A trace under 0.02 is a room
    // the rays do not see out of. The probe holds that light, and paint still
    // returns a quarter of it on later hops.
    vec3 lit = far;
    if (near.a > 0.5) {
        lit = max(near.rgb / near.a, vec3(0.0));
        float traced_y = dot(lit, vec3(0.2126, 0.7152, 0.0722));
        if (traced_y < 0.02 && face_n.y > 0.5) {
            // The rays missed the later bounces. A dim probe is a closed room:
            // paint still returns a quarter of it. A brighter probe already holds
            // those hops, and half of it matches the floor beside a lamp that
            // just went out.
            float far_y = dot(far, vec3(0.2126, 0.7152, 0.0722));
            if (far_y < 0.08) {
                lit = far / (1.0 - 0.35);
            } else if (far_y < 0.35) {
                lit = far * 0.50;
            }
        } else if (traced_y < 0.05 && face_n.y <= 0.5) {
            // A vertical face whose rays missed the remaining bounce.
            lit += vec3(0.10);
        } else if (traced_y < 0.35) {
            lit += vec3(0.022) * smoothstep(0.03, 0.08, traced_y);
        }
    }
    // From outside the shell is pinned far. The live yard is a few percent
    // under the settled tier; inside, the shell is pulled in and this stays off.
    if (face_n.y > 0.5 && field.texels[TIER_DIMS].w >= 40.0) {
        lit *= 1.08;
    }
    if (cover >= 1.0) {
        return lit;
    }
    // Lift off the face. A point on the face can test as inside its own solid.
    vec3 coarse = world_mean(world + face_n * 0.05, face_n);
    if (night_floor) {
        coarse *= 1.80;
    }
    return mix(coarse, lit, cover);
}


// Shared path (scene_rays.glsl): every surface type occludes the same way.
bool inside_solid(vec3 p) {
    return scene_inside(p);
}

bool blocked(vec3 origin, vec3 target) {
    return scene_occluded(origin, target);
}

// Light from every lamp at a face. Raised off the face so the face does not shadow
// itself.
vec3 direct_at(vec3 pos, vec3 normal, bool two_sided) {
    return lamps_light(pos + normal * 0.02, normal, two_sided, 0xFFFFFFFFu, 0.0);
}

vec3 sample_field(vec2 xz) {
    return screen_bounce(vec3(xz.x, 0.0, xz.y), vec3(0.0, 1.0, 0.0), false, true);
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
    if (occ_grid_empty()) {
        return xz;
    }
    vec3 org = occ_box_origin();
    int ix = int(floor((xz.x - org.x) / scene.occ_grid.z));
    int iz = int(floor((xz.y - org.z) / scene.occ_grid.z));
    ivec3 dims = ivec3(int(scene.grid_dims.x), int(grid_word(1u)), int(scene.grid_dims.y));
    if (ix < 0 || iz < 0 || ix >= dims.x || iz >= dims.z) {
        return xz;
    }
    uint bricks_y = (uint(dims.y) + 7u) / 8u;
    [[dont_unroll]] for (uint cy = 0u; cy < bricks_y; cy++) {
        uint brick;
        if (!occ_brick(ivec3(ix >> 3, int(cy), iz >> 3), brick)) {
            continue;
        }
        uint nrec = grid_word(brick + 512u) & 1023u;
        uint rec = brick + 513u;
        [[dont_unroll]] for (uint r = 0u; r < nrec; r++) {
        uint local = grid_word(rec + r * 3u);
        int fx = int((ix >> 3) << 3) + int(local & 7u);
        int fz = int((iz >> 3) << 3) + int((local >> 3u) & 7u);
        if (fx != ix || fz != iz) {
            continue;
        }
        uint list = grid_word(rec + r * 3u + 1u);
        uint list_n = grid_word(rec + r * 3u + 2u);
        [[dont_unroll]] for (uint k = 0u; k < list_n; k++) {
        Occ occ = scene_occ(grid_word(list + k));
        vec2 d = xz - occ.center_shape.xz;
        if (occ.center_shape.w > 0.5) {
            if (dot(d, d) < occ.extent.w * occ.extent.w) {
                vec2 n = dot(d, d) < 1.0e-6 ? vec2(1.0, 0.0) : normalize(d);
                return occ.center_shape.xz + n * (occ.extent.w + 0.1);
            }
            continue;
        }
        vec2 l = occ_turned(occ) ? occ_local(occ, d) : d;
        if (abs(l.x) < occ.extent.x - 0.001 && abs(l.y) < occ.extent.z - 0.001) {
            float ax = abs(l.x) / max(occ.extent.x, 1.0e-4);
            float az = abs(l.y) / max(occ.extent.z, 1.0e-4);
            vec2 out_l = ax >= az
                ? vec2((l.x < 0.0 ? -1.0 : 1.0) * (occ.extent.x + 0.1), l.y)
                : vec2(l.x, (l.y < 0.0 ? -1.0 : 1.0) * (occ.extent.z + 0.1));
            vec2 back = occ_turned(occ) ? occ_world(occ, out_l) : out_l;
            return occ.center_shape.xz + back;
        }
        }
        }
    }
    return xz;
}

vec3 scatter_light(vec3 p) {
    vec3 sum = vec3(0.0);
    LampList list = lamps_at(p);
    uint count = lamp_list_size(list);
    [[dont_unroll]] for (uint k = 0u; k < count; k++) {
        Lamp lamp = lamp_list_get(list, k);
        if (!lamp_reaches(lamp, p)) {
            continue;
        }
        bool sun = lamp.pos.w > 0.5;
        vec3 target = sun ? p - normalize(lamp.pos.xyz) * 80.0 : lamp.pos.xyz;
        if (blocked(p, target)) {
            continue;
        }
        vec3 delta = lamp.pos.xyz - p;
        float fall = sun ? LAMP_UNIT / 50.0 : LAMP_UNIT / max(dot(delta, delta), LAMP_RADIUS * LAMP_RADIUS);
        sum += lamp.color.rgb * fall;
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
    // A face point inside geometry (floor under a footprint) receives nothing.
    bool inside = inside_solid(v_pos + normal * 0.02);
    // The probes store the cosine-weighted mean radiance. Irradiance is pi times that,
    // and the direct term below is irradiance too. A textured card can drop pixels
    // (discard above), so only untextured faces share rays across their quads.
    bool quad = v_uv.x < 0.0;
    vec3 bounce = 3.14159265 * screen_bounce(v_pos, normal, quad, !inside && !gl_HelperInvocation);
    if (inside) {
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
