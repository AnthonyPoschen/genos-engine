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

float froxel_slice_edge(uint index, uint slices) {
    if (slices == 0u) {
        return 64.0;
    }
    float t = float(index) / float(slices);
    if ((fog.size.w & 8u) != 0u) {
        return mix(0.10, 64.0, t);
    }
    return 0.10 * pow(64.0 / 0.10, t);
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
    float u = (float(c.x) + 0.5) / width;
    float v = (float(c.y) + 0.5) / height;
    float nx = (u * 2.0 - 1.0) * fog.forward.w;
    float ny = (1.0 - v * 2.0) * fog.right.w;
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
    if (volume.position.w > 0.5) {
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
    if (distance > 1.0e-8 && fog.medium.x > 0.0) {
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
