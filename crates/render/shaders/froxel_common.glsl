#extension GL_EXT_control_flow_attributes : require
// Shared froxel math. Slice edges are Hillaire, SIGGRAPH 2015:
// z(i) = 0.10 * (64 / 0.10) ^ (i / n). History is not read.
//
// Flag bits in fog.size.w:
//   1  the column step ignores per-slice extinction
//   2  shadow rays are skipped (visibility 1)
//   4  a local volume bound is ignored (sphere and box)
//   8  slice edges are linear in i
//   16 Henyey-Greenstein g is negated

struct FroxelVolume {
    // xyz centre. w is 0 for a box and 1 for a sphere.
    vec4 position;
    // xyz box half-extents, or the sphere radius in x. w is the extinction.
    vec4 extents;
    // x albedo, y phase g, z falloff metres, w unused.
    vec4 shade;
};

struct FroxelLamp {
    vec4 position;
    vec4 color;
};

layout(std430, set = 0, binding = 1) readonly buffer FroxelParams {
    // width, height, slices, flags
    uvec4 size;
    vec4 origin;
    // xyz forward, w tan of half the horizontal field
    vec4 forward;
    // xyz right, w tan of half the vertical field
    vec4 right;
    vec4 up;
    vec4 sun_toward;
    // xyz sun colour, w height-fog albedo
    vec4 sun_color;
    // density, base height, thickness, phase g
    vec4 medium;
    // volume count, lamp count, slice stop (0 = every slice), unused
    uvec4 counts;
    FroxelVolume volumes[8];
    FroxelLamp lamps[8];
} fog;

const float FROXEL_Z_NEAR = 0.10;
const float FROXEL_Z_FAR = 64.0;
const float FROXEL_HALF = 0.5;
const float FROXEL_NDC = 2.0;
const float FROXEL_DISTANCE_EPS = 1.0e-8;
const float FROXEL_TAP_COUNT = 8.0;
const float FROXEL_SUN_REACH = 48.0;
const float FROXEL_LAMP_MIN_DIST2 = 0.25;
const float FROXEL_TAIL_COS = 1.0e-4;
const float FROXEL_TAIL_INF = 1.0e8;
const float FROXEL_TAU_SAT = 1.0e6;

float froxel_slice_edge(uint index, uint slices) {
    if (slices == 0u) {
        return FROXEL_Z_FAR;
    }
    float t = float(index) / float(slices);
    if ((fog.size.w & 8u) != 0u) {
        return mix(FROXEL_Z_NEAR, FROXEL_Z_FAR, t);
    }
    return FROXEL_Z_NEAR * pow(FROXEL_Z_FAR / FROXEL_Z_NEAR, t);
}

float froxel_phase_g(float g) {
    if ((fog.size.w & 16u) != 0u) {
        return -g;
    }
    return g;
}

// Unnormalised view ray. `point_at` scales this by view-Z. `ray_dir` normalises it.
vec3 froxel_offset(uvec2 c) {
    float width = float(max(fog.size.x, 1u));
    float height = float(max(fog.size.y, 1u));
    float u = (float(c.x) + FROXEL_HALF) / width;
    float v = (float(c.y) + FROXEL_HALF) / height;
    float nx = (u * FROXEL_NDC - 1.0) * fog.forward.w;
    float ny = (1.0 - v * FROXEL_NDC) * fog.right.w;
    return fog.forward.xyz + fog.right.xyz * nx + fog.up.xyz * ny;
}

vec3 froxel_point(uvec2 c, float view_z) {
    return fog.origin.xyz + froxel_offset(c) * view_z;
}

const float FROXEL_ALIGN_MIN_Z = 0.05;

// Move a local-volume tap onto the line parallel to the view through the
// volume's screen position. The extinction sample stays on the view ray.
vec3 froxel_align(vec3 volume_centre, vec3 point) {
    float depth = dot(volume_centre - fog.origin.xyz, fog.forward.xyz);
    float z = dot(point - fog.origin.xyz, fog.forward.xyz);
    if (depth <= FROXEL_ALIGN_MIN_Z || z <= FROXEL_ALIGN_MIN_Z) {
        return point;
    }
    vec3 lateral = point - fog.origin.xyz - fog.forward.xyz * z;
    return point + lateral * (depth / z - 1.0);
}

float froxel_falloff(FroxelVolume volume, vec3 point) {
    float face;
    if ((fog.size.w & 4u) != 0u) {
        return 1.0;
    }
    if (volume.position.w > FROXEL_HALF) {
        float radius = volume.extents.x;
        float dist = length(point - volume.position.xyz);
        if (dist > radius) {
            return 0.0;
        }
        face = radius - dist;
    } else {
        vec3 d = abs(point - volume.position.xyz);
        if (d.x > volume.extents.x || d.y > volume.extents.y || d.z > volume.extents.z) {
            return 0.0;
        }
        face = min(volume.extents.x - d.x, min(volume.extents.y - d.y, volume.extents.z - d.z));
    }
    if (volume.shade.z <= 0.0) {
        return 1.0;
    }
    return clamp(face / volume.shade.z, 0.0, 1.0);
}

float froxel_local_density(vec3 point) {
    float sum = 0.0;
    uint n = min(fog.counts.x, 8u);
    [[dont_unroll]] for (uint i = 0u; i < n; i++) {
        FroxelVolume volume = fog.volumes[i];
        sum += max(volume.extents.w, 0.0) * froxel_falloff(volume, point);
    }
    return sum;
}

float froxel_sigma(vec3 p0, vec3 delta, float distance, vec3 centre) {
    float sigma = 0.0;
    if (distance > FROXEL_DISTANCE_EPS && fog.medium.x > 0.0) {
        float cos_theta = delta.y / distance;
        float optical = media_height_optical_depth(
            fog.medium.x,
            fog.medium.z,
            fog.medium.y,
            p0.y,
            delta.y,
            cos_theta,
            distance
        );
        sigma = optical / distance;
    }
    return sigma + froxel_local_density(centre);
}

// ADR 0026 F2. Bit 32 selects the fog-ref column: closed-form optical depth,
// stratified shadow rays, 4-neighbour reuse, no history and no tail.
const uint FROXEL_F2 = 32u;

bool froxel_f2_on() {
    return (fog.size.w & FROXEL_F2) != 0u;
}

float f2_jitter(uint col, uint row, uint slice, uint sample_i) {
    const uint bayer[16] = uint[16](0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);
    const float phi = 0.6180339887498948;
    float stratum = float(bayer[(row % 4u) * 4u + col % 4u]);
    float v = (stratum + 0.5) / 16.0 + float(slice) * phi + float(sample_i) * phi * phi;
    return fract(v);
}

float f2_boundary(uint k) {
    float far_z = fog.origin.w > 0.0 ? fog.origin.w : FROXEL_Z_FAR;
    const float near_z = 0.10;
    uint d = max(fog.size.z, 1u);
    if (k == 0u) {
        return 0.0;
    }
    if (d <= 1u) {
        return far_z;
    }
    float f = float(k - 1u) / float(d - 1u);
    return near_z * pow(far_z / near_z, f);
}

float f2_tau(vec3 o, vec3 d, float t) {
    t = max(t, 0.0);
    if (fog.medium.z <= 0.0) {
        float inside;
        if (abs(d.y) < 1.0e-12) {
            inside = o.y < fog.medium.y ? t : 0.0;
        } else {
            float cross_y = clamp((fog.medium.y - o.y) / d.y, 0.0, t);
            inside = d.y > 0.0 ? cross_y : (t - cross_y);
        }
        return max(fog.medium.x, 0.0) * inside;
    }
    float h = fog.medium.z;
    if (abs(d.y) < 1.0e-12) {
        return fog.medium.x * exp(-o.y / h) * t;
    }
    return fog.medium.x * h / d.y * (exp(-o.y / h) - exp(-(o.y + t * d.y) / h));
}

float f2_sigma_at(vec3 p) {
    if (fog.medium.z <= 0.0) {
        return p.y < fog.medium.y ? max(fog.medium.x, 0.0) : 0.0;
    }
    return fog.medium.x * exp(-p.y / fog.medium.z);
}

float f2_leg(vec3 p, vec3 to_light) {
    float top_y;
    bool finite_top = true;
    if (fog.medium.z <= 0.0) {
        top_y = fog.medium.y;
        finite_top = top_y < 1.0e19;
    } else {
        top_y = p.y + 40.0 * fog.medium.z;
    }
    if (to_light.y > 1.0e-6 && finite_top) {
        return max((top_y - p.y) / to_light.y, 0.0);
    }
    return 1.0e4;
}

bool f2_blocks(vec3 bmin, vec3 bmax, vec3 o, vec3 d, float dist) {
    float near_t = 0.0;
    float far_t = dist;
    [[dont_unroll]] for (int axis = 0; axis < 3; axis++) {
        if (abs(d[axis]) < 1.0e-15) {
            if (o[axis] < bmin[axis] || o[axis] > bmax[axis]) {
                return false;
            }
        } else {
            float a = (bmin[axis] - o[axis]) / d[axis];
            float b = (bmax[axis] - o[axis]) / d[axis];
            near_t = max(near_t, min(a, b));
            far_t = min(far_t, max(a, b));
            if (near_t > far_t) {
                return false;
            }
        }
    }
    return true;
}

bool f2_occluded(vec3 o, vec3 d, float dist) {
    uint n = min(fog.counts.x, 8u);
    [[dont_unroll]] for (uint i = 0u; i < n; i++) {
        if (f2_blocks(fog.volumes[i].position.xyz, fog.volumes[i].extents.xyz, o, d, dist)) {
            return true;
        }
    }
    return false;
}

float f2_direct(vec3 p, vec3 view, float u) {
    vec3 dirs[9];
    float occlude[9];
    float weight[9];
    uint n = 0u;
    float phase_g = froxel_phase_g(fog.medium.w);
    if (fog.sun_color.r > 0.0) {
        vec3 dir = fog.sun_toward.xyz;
        float reach = f2_leg(p, dir);
        float tr = exp(-f2_tau(p, dir, reach));
        float mu = clamp(dot(dir, view), -1.0, 1.0);
        dirs[0] = dir;
        occlude[0] = 1.0e20;
        weight[0] = media_phase_hg(phase_g, mu) * fog.sun_color.r * tr;
        n = 1u;
    }
    uint lamps = min(fog.counts.y, 8u);
    [[dont_unroll]] for (uint i = 0u; i < lamps; i++) {
        if (n >= 9u || fog.lamps[i].color.r <= 0.0) {
            continue;
        }
        vec3 offset = fog.lamps[i].position.xyz - p;
        float r = length(offset);
        if (r < 1.0e-4) {
            continue;
        }
        vec3 dir = offset / r;
        float radiance = fog.lamps[i].color.r / (r * r);
        float tr = exp(-f2_tau(p, dir, r));
        float mu = clamp(dot(dir, view), -1.0, 1.0);
        dirs[n] = dir;
        occlude[n] = r;
        weight[n] = media_phase_hg(phase_g, mu) * radiance * tr;
        n++;
    }
    float total = 0.0;
    [[dont_unroll]] for (uint i = 0u; i < n; i++) {
        total += weight[i];
    }
    if (total <= 0.0 || n == 0u) {
        return 0.0;
    }
    float pick = u * total;
    uint chosen = n - 1u;
    [[dont_unroll]] for (uint i = 0u; i < n; i++) {
        if (pick < weight[i] || i + 1u == n) {
            chosen = i;
            break;
        }
        pick -= weight[i];
    }
    if (f2_occluded(p, dirs[chosen], occlude[chosen])) {
        return 0.0;
    }
    return total;
}

struct F2Span {
    vec3 origin;
    vec3 dir;
    float len;
};

F2Span f2_span(uvec2 c, uint slice) {
    vec3 dir = normalize(froxel_offset(c));
    float cos_v = max(dot(dir, fog.forward.xyz), 1.0e-6);
    float t0 = f2_boundary(slice) / cos_v;
    float t1 = f2_boundary(slice + 1u) / cos_v;
    F2Span span;
    span.origin = fog.origin.xyz + dir * t0;
    span.dir = dir;
    span.len = max(t1 - t0, 0.0);
    return span;
}

float f2_lit(uvec2 c, uint slice) {
    if (c.x >= fog.size.x || c.y >= fog.size.y) {
        return 0.0;
    }
    F2Span span = f2_span(c, slice);
    uint rays = max(fog.counts.w, 1u);
    float rays_f = float(rays);
    float sum = 0.0;
    float albedo = clamp(fog.sun_color.w, 0.0, 1.0);
    [[dont_unroll]] for (uint r = 0u; r < rays; r++) {
        float along = (float(r) + f2_jitter(c.x, c.y, slice, 2u * r)) / rays_f;
        vec3 p = span.origin + span.dir * (span.len * along);
        float u = f2_jitter(c.x, c.y, slice, 2u * r + 1u);
        sum += albedo * f2_sigma_at(p) * f2_direct(p, span.dir, u);
    }
    return sum / rays_f;
}

float f2_scatter(uvec2 c, uint slice) {
    float acc = f2_lit(c, slice);
    float n = 1.0;
    const ivec2 taps[4] = ivec2[4](ivec2(-1, 0), ivec2(1, 0), ivec2(0, -1), ivec2(0, 1));
    [[dont_unroll]] for (uint i = 0u; i < 4u; i++) {
        int nx = int(c.x) + taps[i].x;
        int ny = int(c.y) + taps[i].y;
        if (nx >= 0 && ny >= 0 && uint(nx) < fog.size.x && uint(ny) < fog.size.y) {
            acc += f2_lit(uvec2(uint(nx), uint(ny)), slice);
            n += 1.0;
        }
    }
    return acc / n;
}

float f2_mean_sigma(uvec2 c, uint slice) {
    F2Span span = f2_span(c, slice);
    if (span.len <= FROXEL_DISTANCE_EPS) {
        return 0.0;
    }
    return f2_tau(span.origin, span.dir, span.len) / span.len;
}
