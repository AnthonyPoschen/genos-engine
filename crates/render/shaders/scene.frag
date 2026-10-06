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
    Lamp lamps[4];
    Occ occs[16];
    Obj objects[32];
    vec4 eye;
    vec4 fire_pos;
    vec4 fire_color;
    Puff puffs[8];
} scene;

layout(std430, set = 0, binding = 1) readonly buffer FieldData {
    vec4 texels[];
} field;

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

const uint GRID = 128u;

float hit_box(vec3 origin, vec3 dir, vec3 min_p, vec3 max_p) {
    float t_enter = 0.0;
    float t_exit = 1e20;
    for (int axis = 0; axis < 3; axis++) {
        if (abs(dir[axis]) < 1e-8) {
            if (origin[axis] < min_p[axis] || origin[axis] > max_p[axis]) {
                return -1.0;
            }
            continue;
        }
        float inv = 1.0 / dir[axis];
        float t1 = (min_p[axis] - origin[axis]) * inv;
        float t2 = (max_p[axis] - origin[axis]) * inv;
        float lo = min(t1, t2);
        float hi = max(t1, t2);
        t_enter = max(t_enter, lo);
        t_exit = min(t_exit, hi);
        if (t_exit < t_enter) {
            return -1.0;
        }
    }
    if (t_exit < 0.0) {
        return -1.0;
    }
    return t_enter >= 0.0 ? t_enter : -1.0;
}

float hit_cyl(vec3 origin, vec3 dir, vec3 center, float radius, float y0, float y1) {
    float best = -1.0;
    vec2 o = origin.xz - center.xz;
    float a = dot(dir.xz, dir.xz);
    if (a > 1e-8) {
        float b = dot(o, dir.xz);
        float c = dot(o, o) - radius * radius;
        float disc = b * b - a * c;
        if (disc >= 0.0) {
            float root = sqrt(disc);
            float t0 = (-b - root) / a;
            float t1 = (-b + root) / a;
            if (t0 >= 0.0) {
                float y = origin.y + dir.y * t0;
                if (y >= y0 && y <= y1) {
                    best = t0;
                }
            }
            if (t1 >= 0.0) {
                float y = origin.y + dir.y * t1;
                if (y >= y0 && y <= y1 && (best < 0.0 || t1 < best)) {
                    best = t1;
                }
            }
        }
    }
    return best;
}

bool inside_footprint(vec2 xz, Occ occ) {
    vec2 d = xz - occ.center_shape.xz;
    if (occ.center_shape.w > 0.5) {
        return dot(d, d) <= occ.extent.w * occ.extent.w;
    }
    return abs(d.x) <= occ.extent.x && abs(d.y) <= occ.extent.z;
}

bool inside_solid(vec3 p) {
    uint count = min(scene.occ_count, 16u);
    for (uint i = 0u; i < count; i++) {
        Occ occ = scene.occs[i];
        if (p.y <= 0.001 || p.y >= occ.extent.y) {
            continue;
        }
        if (inside_footprint(p.xz, occ)) {
            return true;
        }
    }
    return false;
}

bool blocked(vec3 origin, vec3 target) {
    if (inside_solid(origin)) {
        return true;
    }
    vec3 delta = target - origin;
    float dist = length(delta);
    if (dist < 1e-3) {
        return false;
    }
    vec3 dir = delta / dist;
    uint count = min(scene.occ_count, 16u);
    for (uint i = 0u; i < count; i++) {
        Occ occ = scene.occs[i];
        float t;
        if (occ.center_shape.w > 0.5) {
            t = hit_cyl(origin, dir, occ.center_shape.xyz, occ.extent.w, 0.0, occ.extent.y);
        } else {
            vec3 half_e = vec3(occ.extent.x, occ.extent.y * 0.5, occ.extent.z);
            vec3 center = vec3(occ.center_shape.x, occ.extent.y * 0.5, occ.center_shape.z);
            t = hit_box(origin, dir, center - half_e, center + half_e);
        }
        if (t > 1e-4 && t < dist - 1e-4) {
            return true;
        }
    }
    return false;
}

vec3 shade_lamp(vec3 origin, vec3 normal, vec3 lamp, vec3 color, bool two_sided) {
    vec3 delta = lamp - origin;
    float dist = max(length(delta), 1.0e-4);
    float nd = dot(delta, normal) / dist;
    if (two_sided) {
        nd = abs(nd);
    }
    if (nd <= 0.0 || blocked(origin, lamp)) {
        return vec3(0.0);
    }
    float shade = nd / (nd + 0.08);
    float dist2 = dist * dist;
    float peak = 4.5 / (1.0 + dist2 * 12.0);
    float tail = 0.90 / (1.0 + dist2 * 0.017);
    return color * shade * (peak + tail);
}

vec3 direct_at(vec3 pos, vec3 normal, bool two_sided) {
    if (pos.y < 0.02) {
        uint count = min(scene.occ_count, 16u);
        for (uint i = 0u; i < count; i++) {
            if (inside_footprint(pos.xz, scene.occs[i])) {
                return vec3(0.0);
            }
        }
    }
    vec3 origin = pos + normal * 0.02;
    vec3 incoming = vec3(0.0);
    uint lamps = min(scene.lamp_count, 4u);
    for (uint i = 0u; i < lamps; i++) {
        incoming += shade_lamp(origin, normal, scene.lamps[i].pos.xyz, scene.lamps[i].color.rgb, two_sided);
    }
    if (scene.fire_pos.w > 0.0) {
        incoming += shade_lamp(
            origin,
            normal,
            scene.fire_pos.xyz,
            scene.fire_color.rgb * scene.fire_pos.w,
            two_sided
        );
    }
    return incoming;
}

float outside_dist(vec2 p, Occ occ) {
    vec2 d = p - occ.center_shape.xz;
    if (occ.center_shape.w > 0.5) {
        return length(d) - occ.extent.w;
    }
    vec2 q = abs(d) - vec2(occ.extent.x, occ.extent.z);
    if (q.x <= 0.0 && q.y <= 0.0) {
        return -1.0;
    }
    return length(max(q, vec2(0.0)));
}

// An overhead lamp colors the floor around an object. The object's back face
// does not receive that lamp.
vec3 overhead_tint(vec3 pos) {
    vec3 tint = vec3(0.0);
    uint occs = min(scene.occ_count, 16u);
    uint lamps = min(scene.lamp_count, 4u);
    for (uint i = 0u; i < occs; i++) {
        Occ occ = scene.occs[i];
        float dist = outside_dist(pos.xz, occ);
        if (dist < 0.0 || dist > 2.5) {
            continue;
        }
        vec3 top = vec3(occ.center_shape.x, occ.extent.y + 0.05, occ.center_shape.z);
        vec3 lit = vec3(0.0);
        for (uint lamp_i = 0u; lamp_i < lamps; lamp_i++) {
            vec3 lamp = scene.lamps[lamp_i].pos.xyz;
            vec2 to_lamp = lamp.xz - occ.center_shape.xz;
            vec2 to_point = pos.xz - occ.center_shape.xz;
            // A lamp over the object tints every side. A side lamp stops at the object.
            bool above = outside_dist(lamp.xz, occ) < 0.0;
            if (!above && dot(to_lamp, to_point) <= 0.0) {
                continue;
            }
            lit += shade_lamp(top, vec3(0.0, 1.0, 0.0), lamp, scene.lamps[lamp_i].color.rgb, false);
        }
        float fall = 1.0 / (1.0 + dist * dist * 1.6);
        tint += occ.albedo.rgb * lit * fall * 0.45;
    }
    return tint;
}

bool segment_hits_box(vec2 a, vec2 b, vec2 min_p, vec2 max_p) {
    vec2 delta = b - a;
    float dist = length(delta);
    if (dist < 1e-4) {
        return false;
    }
    vec2 dir = delta / dist;
    float t_enter = 0.0;
    float t_exit = dist;
    for (int axis = 0; axis < 2; axis++) {
        if (abs(dir[axis]) < 1e-8) {
            if (a[axis] < min_p[axis] || a[axis] > max_p[axis]) {
                return false;
            }
            continue;
        }
        float inv = 1.0 / dir[axis];
        float t1 = (min_p[axis] - a[axis]) * inv;
        float t2 = (max_p[axis] - a[axis]) * inv;
        t_enter = max(t_enter, min(t1, t2));
        t_exit = min(t_exit, max(t1, t2));
        if (t_exit < t_enter) {
            return false;
        }
    }
    // A shorter overlap is the surface itself. It is not a probe past the wall.
    float lo = max(t_enter, 1e-3);
    float hi = min(t_exit, dist - 1e-3);
    return hi > lo + 0.05;
}

bool segment_hits_circle(vec2 a, vec2 b, vec2 center, float radius) {
    vec2 delta = b - a;
    float dist = length(delta);
    if (dist < 1e-4) {
        return false;
    }
    vec2 dir = delta / dist;
    vec2 o = a - center;
    float along = dot(o, dir);
    float disc = along * along - (dot(o, o) - radius * radius);
    if (disc < 0.0) {
        return false;
    }
    float root = sqrt(disc);
    float lo = max(-along - root, 1e-3);
    float hi = min(-along + root, dist - 1e-3);
    return hi > lo + 0.05;
}

bool probe_hidden(vec2 from, vec2 probe) {
    vec2 delta = probe - from;
    float dist = length(delta);
    if (dist < 1e-3) {
        return false;
    }
    uint count = min(scene.occ_count, 16u);
    for (uint i = 0u; i < count; i++) {
        Occ occ = scene.occs[i];
        bool hit;
        if (occ.center_shape.w > 0.5) {
            hit = segment_hits_circle(from, probe, occ.center_shape.xz, occ.extent.w);
        } else {
            vec2 half_e = vec2(occ.extent.x, occ.extent.z);
            hit = segment_hits_box(from, probe, occ.center_shape.xz - half_e, occ.center_shape.xz + half_e);
        }
        if (hit) {
            return true;
        }
    }
    return false;
}

bool probe_inside(vec2 xz) {
    uint count = min(scene.occ_count, 16u);
    for (uint i = 0u; i < count; i++) {
        if (inside_footprint(xz, scene.occs[i])) {
            return true;
        }
    }
    return false;
}

bool probe_rejected(vec2 from, vec2 probe) {
    if (probe_inside(probe)) {
        return true;
    }
    // A neighbor one cell away is still this side of a wall. Rejecting it
    // left a black ring on the floor around a solid.
    if (length(probe - from) <= scene.spacing * 1.25) {
        return false;
    }
    return probe_hidden(from, probe);
}

vec2 probe_center(uint ix, uint iz) {
    return vec2(scene.origin_x, scene.origin_z) + (vec2(float(ix), float(iz)) + 0.5) * scene.spacing;
}

vec3 sample_raw(vec2 xz) {
    if (scene.count_x == 0u || scene.spacing <= 0.0) {
        return vec3(0.0);
    }
    float fx = clamp((xz.x - scene.origin_x) / scene.spacing - 0.5, 0.0, float(scene.count_x - 1u));
    float fz = clamp((xz.y - scene.origin_z) / scene.spacing - 0.5, 0.0, float(scene.count_z - 1u));
    uint x0 = uint(floor(fx));
    uint z0 = uint(floor(fz));
    uint x1 = min(x0 + 1u, scene.count_x - 1u);
    uint z1 = min(z0 + 1u, scene.count_z - 1u);
    float tx = fx - float(x0);
    float tz = fz - float(z0);
    uint base = GRID * GRID;
    vec3 c00 = field.texels[base + z0 * GRID + x0].rgb;
    vec3 c10 = field.texels[base + z0 * GRID + x1].rgb;
    vec3 c01 = field.texels[base + z1 * GRID + x0].rgb;
    vec3 c11 = field.texels[base + z1 * GRID + x1].rgb;
    float w00 = (1.0 - tx) * (1.0 - tz);
    float w10 = tx * (1.0 - tz);
    float w01 = (1.0 - tx) * tz;
    float w11 = tx * tz;
    if (probe_rejected(xz, probe_center(x0, z0))) {
        w00 = 0.0;
    }
    if (probe_rejected(xz, probe_center(x1, z0))) {
        w10 = 0.0;
    }
    if (probe_rejected(xz, probe_center(x0, z1))) {
        w01 = 0.0;
    }
    if (probe_rejected(xz, probe_center(x1, z1))) {
        w11 = 0.0;
    }
    float sum = w00 + w10 + w01 + w11;
    if (sum < 1e-4) {
        return vec3(0.0);
    }
    return (c00 * w00 + c10 * w10 + c01 * w01 + c11 * w11) / sum;
}

vec3 sample_field(vec2 xz) {
    return sample_raw(xz);
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
    uint lamps = min(scene.lamp_count, 4u);
    for (uint i = 0u; i < lamps; i++) {
        vec3 lamp = scene.lamps[i].pos.xyz;
        if (blocked(p, lamp)) {
            continue;
        }
        vec3 delta = lamp - p;
        float fall = 1.0 / (1.0 + dot(delta, delta) * 0.08);
        sum += scene.lamps[i].color.rgb * fall;
    }
    if (scene.fire_pos.w > 0.0) {
        vec3 fire = scene.fire_pos.xyz;
        if (!blocked(p, fire)) {
            vec3 delta = fire - p;
            float fall = scene.fire_pos.w / (1.0 + dot(delta, delta) * 0.65);
            sum += scene.fire_color.rgb * fall;
        }
    }
    sum += sample_field(fog_field_xz(p.xz)) * 0.35;
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
    vec3 bounce = sample_field(v_pos.xz + normal.xz * 0.02);
    float level = (direct.r + direct.g + direct.b) / 3.0;
    float gain = 1.0;
    if (abs(normal.y) > 0.5) {
        // The field already holds reflected radiance. A small lift keeps a floor shadow readable.
        gain = 1.0 + (1.0 - clamp(level, 0.0, 1.0));
        bounce += overhead_tint(v_pos);
    }
    // The field keeps a stronger lamp so a tint and a corner bounce survive.
    // The picture uses a smaller share, so a white wall does not clip.
    vec3 color = tone(albedo * (direct * 0.58 + bounce * gain * 0.75));
    // The packed object table is live. An empty texture id stays zero.
    if (scene.obj_count > 0u && scene.objects[0].range.z != 0u) {
        color *= 1.0;
    }
    out_color = vec4(color * texel.a, texel.a);
}
