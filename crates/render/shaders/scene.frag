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
    Lamp lamps[4];
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
    // x is the screen-probe columns. y is the rows.
    vec4 view_grid;
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

const uint FIELD_COPY = 524288u;
const uint SCREEN_DIRS = 16u;
const float SCREEN_REACH = 4.0;
const float WORLD_SPACING = 2.5;
const float NEAR_SPACING = 0.5;
const uint WORLD_DIRS = 8u;
const uint WORLD_OFFSET = 184320u;
const uint SHOWN_COPY = 1u;
const float LAMBERT = 0.318309886;
// A unit white lamp 7 m above a white floor stays near 0.46. A lamp 1 m away stays under white.
const float LAMP_UNIT = 72.0;
const float TAU = 6.2831853;

bool probe_hidden(vec2 from, vec2 probe);

float ray_spin(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

vec4 probe_angle(uint copy, Cascade c, uint probe, vec2 probe_pos, float angle) {
    float n = max(c.dirs, 1.0);
    float spun = angle - ray_spin(probe_pos) * (TAU / n);
    float f = spun / TAU * n - 0.5;
    float i0 = floor(f);
    float t = clamp(f - i0, 0.0, 1.0);
    uint a = uint(mod(i0, n));
    uint b = uint(mod(i0 + 1.0, n));
    uint base = copy * FIELD_COPY + uint(c.offset);
    uint stride = uint(c.dirs);
    vec4 s0 = field.texels[base + probe * stride + a];
    vec4 s1 = field.texels[base + probe * stride + b];
    if (s0.a < 0.0) {
        return s1;
    }
    if (s1.a < 0.0) {
        return s0;
    }
    return mix(s0, s1, t);
}

vec4 sample_interval(uint copy, uint index, vec2 xz, float angle) {
    Cascade c = scene.cascades[index];
    if (c.count_x < 1.0 || c.count_z < 1.0 || c.spacing <= 0.0) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    float fx = clamp((xz.x - c.origin_x) / c.spacing - 0.5, 0.0, c.count_x - 1.0);
    float fz = clamp((xz.y - c.origin_z) / c.spacing - 0.5, 0.0, c.count_z - 1.0);
    uint x0 = uint(floor(fx));
    uint z0 = uint(floor(fz));
    uint x1 = min(x0 + 1u, uint(c.count_x) - 1u);
    uint z1 = min(z0 + 1u, uint(c.count_z) - 1u);
    float tx = fx - float(x0);
    float tz = fz - float(z0);
    uint stride = uint(c.count_x);
    vec4 acc = vec4(0.0);
    float weight = 0.0;
    for (int corner = 0; corner < 4; corner++) {
        uint ix = corner == 1 || corner == 3 ? x1 : x0;
        uint iz = corner >= 2 ? z1 : z0;
        float wx = corner == 1 || corner == 3 ? tx : 1.0 - tx;
        float wz = corner >= 2 ? tz : 1.0 - tz;
        float w = wx * wz;
        if (w <= 1e-6) {
            continue;
        }
        vec2 probe_pos = vec2(
            c.origin_x + (float(ix) + 0.5) * c.spacing,
            c.origin_z + (float(iz) + 0.5) * c.spacing
        );
        if (probe_hidden(xz, probe_pos)) {
            continue;
        }
        vec4 taken = probe_angle(copy, c, iz * stride + ix, probe_pos, angle);
        if (taken.a < 0.0) {
            continue;
        }
        acc += taken * w;
        weight += w;
    }
    if (weight <= 1e-4) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    return acc / weight;
}

void world_layout(out vec2 origin, out uint count_x, out uint count_z) {
    vec2 half_e = vec2(max(scene.floor_center.w, 0.5), max(scene.floor_data.x, 0.5));
    vec2 span = half_e * 2.0;
    float margin = WORLD_SPACING * 2.0;
    origin = vec2(scene.floor_center.x, scene.floor_center.z) - half_e - vec2(margin);
    count_x = max(uint(ceil((span.x + margin * 2.0) / WORLD_SPACING)), 1u);
    count_z = max(uint(ceil((span.y + margin * 2.0) / WORLD_SPACING)), 1u);
}

vec4 sample_open(uint base, vec2 origin, float spacing, uint count_x, uint count_z, uint dirs, vec2 xz, float angle) {
    if (count_x < 1u || count_z < 1u || spacing <= 0.0) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    float fx = clamp((xz.x - origin.x) / spacing - 0.5, 0.0, float(count_x - 1u));
    float fz = clamp((xz.y - origin.y) / spacing - 0.5, 0.0, float(count_z - 1u));
    uint x0 = uint(floor(fx));
    uint z0 = uint(floor(fz));
    uint x1 = min(x0 + 1u, count_x - 1u);
    uint z1 = min(z0 + 1u, count_z - 1u);
    float tx = fx - float(x0);
    float tz = fz - float(z0);
    float n = float(max(dirs, 1u));
    float f = angle / TAU * n - 0.5;
    float i0 = floor(f);
    float ang_t = clamp(f - i0, 0.0, 1.0);
    uint a = uint(mod(i0, n));
    uint b = uint(mod(i0 + 1.0, n));
    vec4 acc = vec4(0.0);
    float weight = 0.0;
    for (int corner = 0; corner < 4; corner++) {
        uint ix = corner == 1 || corner == 3 ? x1 : x0;
        uint iz = corner >= 2 ? z1 : z0;
        float wx = corner == 1 || corner == 3 ? tx : 1.0 - tx;
        float wz = corner >= 2 ? tz : 1.0 - tz;
        float w = wx * wz;
        if (w <= 1e-6) {
            continue;
        }
        uint probe = iz * count_x + ix;
        vec4 s0 = field.texels[base + probe * dirs + a];
        vec4 s1 = field.texels[base + probe * dirs + b];
        vec4 taken = mix(s0, s1, ang_t);
        if (s0.a < 0.0) {
            taken = s1;
        } else if (s1.a < 0.0) {
            taken = s0;
        }
        if (taken.a < 0.0) {
            continue;
        }
        acc += taken * w;
        weight += w;
    }
    if (weight <= 1e-4) {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    return acc / weight;
}

// The direction from the probe toward the shaded point. A near hit there hides the point.
bool hit_hides_point(uint base, uint probe, uint dirs, vec2 probe_pos, vec2 point) {
    vec2 delta = point - probe_pos;
    float dist = length(delta);
    if (dist < 0.05) {
        return false;
    }
    float angle = atan(delta.y, delta.x);
    if (angle < 0.0) {
        angle += TAU;
    }
    float n = float(max(dirs, 1u));
    float f = angle / TAU * n - 0.5;
    uint a = uint(mod(floor(f), n));
    float hit_d = field.texels[base + probe * dirs + a].a;
    return hit_d > 0.0 && hit_d <= SCREEN_REACH && hit_d + 0.4 < dist;
}

vec3 merged_grid(uint base, vec2 origin, float spacing, uint count_x, uint count_z, uint dirs, vec2 xz, vec2 face_n, bool uniform_disk) {
    float fx = clamp((xz.x - origin.x) / spacing - 0.5, 0.0, float(count_x - 1u));
    float fz = clamp((xz.y - origin.y) / spacing - 0.5, 0.0, float(count_z - 1u));
    uint x0 = uint(floor(fx));
    uint z0 = uint(floor(fz));
    uint x1 = min(x0 + 1u, count_x - 1u);
    uint z1 = min(z0 + 1u, count_z - 1u);
    float tx = fx - float(x0);
    float tz = fz - float(z0);
    float corner_w[4];
    uint corner_i[4];
    float sum_w = 0.0;
    for (int corner = 0; corner < 4; corner++) {
        uint ix = corner == 1 || corner == 3 ? x1 : x0;
        uint iz = corner >= 2 ? z1 : z0;
        float wx = corner == 1 || corner == 3 ? tx : 1.0 - tx;
        float wz = corner >= 2 ? tz : 1.0 - tz;
        vec2 probe = origin + (vec2(ix, iz) + 0.5) * spacing;
        float w = wx * wz;
        if (!uniform_disk && dot(probe - xz, face_n) < -0.02) {
            w = 0.0;
        }
        if (w > 1e-6 && probe_hidden(xz, probe)) {
            w = 0.0;
        }
        if (w > 1e-6 && hit_hides_point(base, iz * count_x + ix, dirs, probe, xz)) {
            w = 0.0;
        }
        corner_w[corner] = w;
        corner_i[corner] = iz * count_x + ix;
        sum_w += w;
    }
    if (sum_w <= 1e-4) {
        return vec3(0.0);
    }
    float n = float(max(dirs, 1u));
    vec3 sum = vec3(0.0);
    float weight = 0.0;
    for (uint d = 0u; d < dirs; d++) {
        float angle = (float(d) + 0.5) * TAU / float(dirs);
        float facing = 1.0;
        if (!uniform_disk) {
            facing = dot(face_n, vec2(cos(angle), sin(angle)));
            if (facing <= 0.0) {
                continue;
            }
        }
        float f = angle / TAU * n - 0.5;
        float i0 = floor(f);
        float ang_t = clamp(f - i0, 0.0, 1.0);
        uint a = uint(mod(i0, n));
        uint b = uint(mod(i0 + 1.0, n));
        vec3 color = vec3(0.0);
        for (int corner = 0; corner < 4; corner++) {
            if (corner_w[corner] <= 1e-6) {
                continue;
            }
            uint probe = corner_i[corner];
            vec4 s0 = field.texels[base + probe * dirs + a];
            vec4 s1 = field.texels[base + probe * dirs + b];
            vec3 taken = mix(s0.rgb, s1.rgb, ang_t);
            if (s0.a < 0.0) {
                taken = s1.rgb;
            } else if (s1.a < 0.0) {
                taken = s0.rgb;
            }
            color += taken * corner_w[corner];
        }
        sum += color / sum_w * facing;
        weight += facing;
    }
    if (weight <= 1e-4) {
        return vec3(0.0);
    }
    return sum / weight;
}

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

// Near cylinder hit. Matches the gather. The lamp test above also keeps the exit.
float hit_cyl_near(vec3 origin, vec3 dir, vec3 center, float radius, float y0, float y1) {
    vec2 o = origin.xz - center.xz;
    float a = dot(dir.xz, dir.xz);
    if (a <= 1.0e-8) {
        return -1.0;
    }
    float b = dot(o, dir.xz);
    float c = dot(o, o) - radius * radius;
    float disc = b * b - a * c;
    if (disc < 0.0) {
        return -1.0;
    }
    float t0 = (-b - sqrt(disc)) / a;
    float y = origin.y + dir.y * t0;
    if (t0 > 0.0 && y >= y0 && y <= y1) {
        return t0;
    }
    return -1.0;
}

vec3 view_ray(vec2 uv) {
    float ndc_x = uv.x * 2.0 - 1.0;
    float ndc_y = uv.y * 2.0 - 1.0;
    float tan_half = scene.view_right.w;
    float aspect = scene.view_forward.w;
    return scene.view_forward.xyz
        + scene.view_right.xyz * (ndc_x * tan_half * aspect)
        + scene.view_up.xyz * (-ndc_y * tan_half);
}

bool project_uv(vec3 world, out vec2 uv) {
    uv = vec2(0.0);
    vec3 rel = world - scene.eye.xyz;
    float depth = dot(rel, scene.view_forward.xyz);
    if (depth <= 0.05) {
        return false;
    }
    float tan_half = max(scene.view_right.w, 1.0e-4);
    float aspect = max(scene.view_forward.w, 1.0e-4);
    float ndc_x = dot(rel, scene.view_right.xyz) / (depth * tan_half * aspect);
    float ndc_y = -dot(rel, scene.view_up.xyz) / (depth * tan_half);
    uv = vec2(ndc_x * 0.5 + 0.5, ndc_y * 0.5 + 0.5);
    return uv.x >= -0.02 && uv.x <= 1.02 && uv.y >= -0.02 && uv.y <= 1.02;
}

uvec2 screen_counts() {
    return uvec2(
        max(uint(scene.view_grid.x + 0.5), 1u),
        max(uint(scene.view_grid.y + 0.5), 1u)
    );
}

// The same ray and the same 4 cm offset the gather used to place the probe.
bool surface_hit(vec3 eye, vec3 dir, out vec3 pos, out vec3 normal) {
    float best = 1.0e20;
    bool found = false;
    pos = eye;
    normal = vec3(0.0, 1.0, 0.0);
    uint count = min(scene.occ_count, 16u);
    for (uint i = 0u; i < count; i++) {
        Occ occ = scene.occs[i];
        float t = -1.0;
        vec3 n = vec3(0.0, 1.0, 0.0);
        vec3 center = vec3(occ.center_shape.x, occ.extent.y * 0.5, occ.center_shape.z);
        if (occ.center_shape.w > 0.5) {
            t = hit_cyl_near(eye, dir, center, occ.extent.w, 0.0, occ.extent.y);
            if (t > 0.0) {
                vec3 p = eye + dir * t;
                vec2 d = p.xz - occ.center_shape.xz;
                float len = max(length(d), 1.0e-4);
                n = vec3(d.x / len, 0.0, d.y / len);
            }
        } else {
            vec3 half_e = vec3(occ.extent.x, occ.extent.y * 0.5, occ.extent.z);
            t = hit_box(eye, dir, center - half_e, center + half_e);
            if (t > 0.0) {
                vec3 q = (eye + dir * t - center) / max(half_e, vec3(1.0e-4));
                vec3 aq = abs(q);
                if (aq.x >= aq.y && aq.x >= aq.z) {
                    n = vec3(sign(q.x), 0.0, 0.0);
                } else if (aq.y >= aq.z) {
                    n = vec3(0.0, sign(q.y), 0.0);
                } else {
                    n = vec3(0.0, 0.0, sign(q.z));
                }
                if (dot(n, n) < 0.5) {
                    n = vec3(0.0, 1.0, 0.0);
                }
            }
        }
        if (t > 0.002 && t < best) {
            best = t;
            pos = eye + dir * t;
            normal = n;
            found = true;
        }
    }
    if (dir.y < -1.0e-6) {
        float t = (0.0 - eye.y) / dir.y;
        if (t > 0.002 && t < best) {
            vec3 p = eye + dir * t;
            vec2 half_e = vec2(max(scene.floor_center.w, 0.5), max(scene.floor_data.x, 0.5));
            vec2 d = p.xz - vec2(scene.floor_center.x, scene.floor_center.z);
            if (abs(d.x) <= half_e.x + 0.05 && abs(d.y) <= half_e.y + 0.05) {
                pos = p;
                normal = vec3(0.0, 1.0, 0.0);
                found = true;
            }
        }
    }
    return found;
}

// World lattice around the eye. A turn does not move it. The gather uses the same snap.
void near_layout(out vec2 origin, out uint count_x, out uint count_z) {
    uvec2 counts = screen_counts();
    count_x = counts.x;
    count_z = counts.y;
    vec2 center = floor(vec2(scene.eye.x, scene.eye.z) / NEAR_SPACING + 0.5) * NEAR_SPACING;
    origin = center - vec2(float(count_x), float(count_z)) * NEAR_SPACING * 0.5;
}

vec3 merged_at(uint copy, vec3 world, vec2 face_n, bool uniform_disk) {
    // Cascade 0 already stores the merged interval. A miss there included the farther ranges.
    Cascade near = scene.cascades[0];
    uint dirs = uint(max(near.dirs, 1.0));
    float step = TAU / float(dirs);
    float spin = ray_spin(world.xz);
    vec3 sum = vec3(0.0);
    float weight = 0.0;
    for (uint d = 0u; d < dirs; d++) {
        float angle = (float(d) + spin + 0.5) * step;
        float w = 1.0;
        if (!uniform_disk) {
            w = dot(face_n, vec2(cos(angle), sin(angle)));
            if (w <= 0.0) {
                continue;
            }
        }
        vec4 near_i = sample_interval(copy, 0u, world.xz, angle);
        sum += near_i.rgb * w;
        weight += w;
    }
    if (weight <= 1.0e-4) {
        return vec3(0.0);
    }
    return sum / weight;
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
    return color * nd * LAMP_UNIT / (1.0 + dist2);
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

vec3 sample_field(vec2 xz) {
    return merged_at(SHOWN_COPY, vec3(xz.x, 0.0, xz.y), vec2(0.0), true);
}

void consider_exit(vec2 dest, float dist, inout vec2 best_free, inout float free_d, inout vec2 best_any, inout float any_d, inout bool has_free) {
    if (dist < any_d) {
        any_d = dist;
        best_any = dest;
    }
    if (dist < free_d && !probe_inside(dest)) {
        free_d = dist;
        best_free = dest;
        has_free = true;
    }
}

void exits_of(Occ occ, vec2 p, inout vec2 best_free, inout float free_d, inout vec2 best_any, inout float any_d, inout bool has_free, inout bool found) {
    if (!inside_footprint(p, occ)) {
        return;
    }
    found = true;
    vec2 d = p - occ.center_shape.xz;
    if (occ.center_shape.w > 0.5) {
        float reach = occ.extent.w + 0.06;
        vec2 n = dot(d, d) < 1.0e-8 ? vec2(1.0, 0.0) : normalize(d);
        consider_exit(occ.center_shape.xz + n * reach, reach - length(d), best_free, free_d, best_any, any_d, has_free);
        return;
    }
    consider_exit(vec2(occ.center_shape.x + occ.extent.x + 0.06, p.y), occ.extent.x - d.x + 0.06, best_free, free_d, best_any, any_d, has_free);
    consider_exit(vec2(occ.center_shape.x - occ.extent.x - 0.06, p.y), occ.extent.x + d.x + 0.06, best_free, free_d, best_any, any_d, has_free);
    consider_exit(vec2(p.x, occ.center_shape.z + occ.extent.z + 0.06), occ.extent.z - d.y + 0.06, best_free, free_d, best_any, any_d, has_free);
    consider_exit(vec2(p.x, occ.center_shape.z - occ.extent.z - 0.06), occ.extent.z + d.y + 0.06, best_free, free_d, best_any, any_d, has_free);
}

vec2 push_outside(vec2 xz) {
    vec2 p = xz;
    for (int step = 0; step < 4; step++) {
        vec2 best_free = p;
        vec2 best_any = p;
        float free_d = 1.0e9;
        float any_d = 1.0e9;
        bool has_free = false;
        bool found = false;
        uint count = min(scene.occ_count, 16u);
        for (uint i = 0u; i < count; i++) {
            exits_of(scene.occs[i], p, best_free, free_d, best_any, any_d, has_free, found);
        }
        if (!found) {
            return p;
        }
        p = has_free ? best_free : best_any;
        if (has_free) {
            return p;
        }
    }
    return p;
}

bool lamp_sees_upward(vec3 pos, vec3 normal) {
    vec3 origin = pos + normal * 0.02;
    uint lamps = min(scene.lamp_count, 4u);
    for (uint i = 0u; i < lamps; i++) {
        vec4 lamp = scene.lamps[i].pos;
        vec3 toward = lamp.w > 0.5 ? -normalize(lamp.xyz) : lamp.xyz - origin;
        vec3 target = lamp.w > 0.5 ? origin + toward * 80.0 : lamp.xyz;
        if (dot(toward, normal) <= 0.0 || blocked(origin, target)) {
            continue;
        }
        return true;
    }
    if (scene.fire_pos.w > 0.0 && dot(scene.fire_pos.xyz - origin, normal) > 0.0 && !blocked(origin, scene.fire_pos.xyz)) {
        return true;
    }
    return false;
}

vec2 poll_xz(vec3 pos, vec3 normal) {
    if (normal.y <= 0.5) {
        return pos.xz + normal.xz * 0.04;
    }
    return push_outside(pos.xz);
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
        float fall = LAMP_UNIT / (1.0 + dot(delta, delta));
        sum += scene.lamps[i].color.rgb * fall;
    }
    if (scene.fire_pos.w > 0.0) {
        vec3 fire = scene.fire_pos.xyz;
        if (!blocked(p, fire)) {
            vec3 delta = fire - p;
            float fall = scene.fire_pos.w * LAMP_UNIT / (1.0 + dot(delta, delta));
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
    vec2 polled = poll_xz(v_pos, normal);
    bool floor_face = abs(normal.y) > 0.5;
    vec3 bounce = merged_at(SHOWN_COPY, vec3(polled.x, v_pos.y, polled.y), normal.xz, floor_face);
    if (normal.y > 0.5 && probe_inside(v_pos.xz) && !lamp_sees_upward(v_pos, normal)) {
        bounce = vec3(0.0);
    }
    vec3 color = tone(albedo * LAMBERT * (direct + bounce));
    out_color = vec4(color * texel.a, texel.a);
}
