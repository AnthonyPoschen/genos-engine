// The persistent world probe tier (probe_tier.rs): probes on a fixed world lattice,
// in bricks of 4 x 4 x 4 that exist only near geometry. A window of bricks around
// the camera maps each brick to a slot, or to none. Each probe keeps three ambient
// cubes, six faces each of cosine-weighted mean radiance: light after every bounce
// (TIER_CUBE3, alpha = samples, 0 = no light yet; every pass feeds it back, so it
// converges to unlimited bounces), up to twice (TIER_CUBE2) and once (TIER_CUBE1),
// and where it sits (TIER_POSITION).
//
// Needs `field` and scene_rays.glsl declared first. Offsets mirror probe_tier.rs.

const uint TIER_INFO = 524288u;
const uint TIER_DIMS = TIER_INFO + 1u;
const uint TIER_INDIR = TIER_INFO + 2u;
const uint TIER_INDIR_CAP = 32768u;
const uint TIER_SLOTS = TIER_INDIR + TIER_INDIR_CAP;
const uint TIER_WORK = TIER_SLOTS + 2048u * 2u;
const uint TIER_WORK_TEXELS = 65u;
const uint TIER_PROBES = TIER_INFO + 106496u;
const uint TIER_PROBE_TEXELS = 19u;
const uint TIER_CUBE3 = 0u;
const uint TIER_CUBE2 = 6u;
const uint TIER_CUBE1 = 12u;
// xyz: where the probe sits. A probe moved out of a solid is off its lattice point.
const uint TIER_POSITION = 18u;

float tier_spacing() {
    return field.texels[TIER_INFO].w;
}

// Slot of a world brick, or -1 when it is outside the window, empty, or not lit yet.
int tier_slot(ivec3 brick) {
    vec4 info = field.texels[TIER_INFO];
    if (info.w <= 0.0) {
        return -1;
    }
    ivec3 dims = ivec3(field.texels[TIER_DIMS].xyz + 0.5);
    ivec3 rel = brick - ivec3(floor(info.xyz + 0.5));
    if (any(lessThan(rel, ivec3(0))) || any(greaterThanEqual(rel, dims))) {
        return -1;
    }
    float slot = field.texels[TIER_INDIR + uint((rel.y * dims.z + rel.z) * dims.x + rel.x)].x;
    return slot < 0.0 ? -1 : int(slot + 0.5);
}

// First texel of the probe at lattice cell `cell`, or -1.
int tier_probe(ivec3 cell) {
    int slot = tier_slot(cell >> 2);
    if (slot < 0) {
        return -1;
    }
    ivec3 local = cell & 3;
    return int(TIER_PROBES) + (slot * 64 + (local.y * 4 + local.z) * 4 + local.x) * int(TIER_PROBE_TEXELS);
}

// The probes a face at `world` with normal `face_n` reads: the eight around the point
// pushed half a spacing off the face, with trilinear weights that sum to 1. A probe
// that is dead (inside a solid it could not leave, far from surfaces), not lit yet,
// outside the window or hidden from the point drops out and the others take its
// weight. Returns how many are left; none inside a solid. `base` holds each one's
// first texel.
uint tier_taps(vec3 world, vec3 face_n, uint lit_face, out uint base[8], out float weight[8]) {
    for (uint k = 0u; k < 8u; k++) {
        base[k] = 0u;
        weight[k] = 0.0;
    }
    float spacing = tier_spacing();
    if (spacing <= 0.0) {
        return 0u;
    }
    vec3 q = (world + face_n * (0.5 * spacing)) / spacing - 0.5;
    ivec3 i0 = ivec3(floor(q));
    vec3 t = q - vec3(i0);
    vec3 from = world + face_n * 0.02;
    // The lit probes around the point, and the box they and the point span.
    uint count = 0u;
    vec3 lo = from;
    vec3 hi = from;
    for (uint corner = 0u; corner < 8u; corner++) {
        ivec3 c = i0 + ivec3(int(corner & 1u), int((corner >> 1u) & 1u), int((corner >> 2u) & 1u));
        float w = ((corner & 1u) == 0u ? 1.0 - t.x : t.x)
            * ((corner & 2u) == 0u ? 1.0 - t.y : t.y)
            * ((corner & 4u) == 0u ? 1.0 - t.z : t.z);
        if (w <= 1.0e-5) {
            continue;
        }
        int at = tier_probe(c);
        if (at < 0) {
            continue;
        }
        uint b = uint(at);
        if (field.texels[b + TIER_CUBE3 + lit_face].w <= 0.0) {
            continue;
        }
        vec3 p = field.texels[b + TIER_POSITION].xyz;
        lo = min(lo, p);
        hi = max(hi, p);
        base[count] = b;
        weight[count] = w;
        count++;
    }
    if (count == 0u) {
        return 0u;
    }
    // Only surfaces reaching into that box can hide a probe. Away from geometry that
    // is none, and the taps cost no ray tests.
    uint blockers = scene_candidates(lo, hi);
    // A point wedged in a corner can start inside the other solid. Rays from there see
    // no entry into it and would reach probes on its far side. Nothing reaches a point
    // inside a solid.
    if (blockers != 0u && scene_inside(from)) {
        return 0u;
    }
    float wsum = 0.0;
    uint kept = 0u;
    for (uint k = 0u; k < count; k++) {
        if (blockers != 0u) {
            vec3 delta = field.texels[base[k] + TIER_POSITION].xyz - from;
            float dist = length(delta);
            SceneHit hit;
            if (dist > 1.0e-3 && scene_ray_masked(from, delta / dist, 1.0e-4, dist - 1.0e-3, blockers, hit)) {
                continue;
            }
        }
        base[kept] = base[k];
        weight[kept] = weight[k];
        wsum += weight[k];
        kept++;
    }
    if (wsum <= 1.0e-4) {
        return 0u;
    }
    for (uint k = 0u; k < kept; k++) {
        weight[k] /= wsum;
    }
    return kept;
}

// How far the light from surfaces near a face is gathered with rays, in spacings.
const float TIER_NEAR_REACH = 1.0;

// The surfaces a ray leaving the face at `pos` along `n` can meet within `reach`.
// The face's own shape is convex, so its rays never come back to it, and a ray
// leaving the floor or the roof upward or downward never meets that plane: on open
// floor or an open wall this is none and the gather costs nothing.
uint near_candidates(vec3 pos, vec3 n, float reach) {
    uint mask = scene_candidates_skip(pos - vec3(reach), pos + vec3(reach), true, pos - n * 0.01);
    if (abs(n.y) > 0.999) {
        mask &= ~(n.y > 0.0 ? SCENE_FLOOR_BIT : SCENE_ROOF_BIT);
    }
    return mask;
}

// Six-face cube weighting of a face normal: the faces on the side of each axis.
uvec3 tier_faces_of(vec3 n) {
    return uvec3(n.x >= 0.0 ? 0u : 1u, n.y >= 0.0 ? 2u : 3u, n.z >= 0.0 ? 4u : 5u);
}

vec3 tier_cube_at(uint b, uint cube, vec3 n) {
    uvec3 f = tier_faces_of(n);
    vec3 nn = n * n;
    return nn.x * field.texels[b + cube + f.x].rgb
        + nn.y * field.texels[b + cube + f.y].rgb
        + nn.z * field.texels[b + cube + f.z].rgb;
}

// Cosine-weighted mean radiance arriving at a face at `world` with normal `face_n`,
// from cubes `cube_a` and `cube_b` of the probes tier_taps picks. True with no light
// for a point inside a solid; false when no probe answers.
bool tier_sample2(vec3 world, vec3 face_n, uint cube_a, uint cube_b, out vec3 color_a, out vec3 color_b) {
    color_a = vec3(0.0);
    color_b = vec3(0.0);
    uint base[8];
    float weight[8];
    uint count = tier_taps(world, face_n, tier_faces_of(face_n).x, base, weight);
    if (count == 0u) {
        // Inside a solid: no light. Otherwise no probe answers.
        return tier_spacing() > 0.0 && scene_inside(world + face_n * 0.02);
    }
    for (uint k = 0u; k < count; k++) {
        color_a += weight[k] * tier_cube_at(base[k], cube_a, face_n);
        if (cube_b != cube_a) {
            color_b += weight[k] * tier_cube_at(base[k], cube_b, face_n);
        }
    }
    if (cube_b == cube_a) {
        color_b = color_a;
    }
    return true;
}

// The light the picture shows: every bounce.
bool tier_sample(vec3 world, vec3 face_n, out vec3 color) {
    vec3 unused;
    return tier_sample2(world, face_n, TIER_CUBE3, TIER_CUBE3, color, unused);
}

// All six faces of cube `cube` over taps from tier_taps.
void tier_cube6(uint count, uint base[8], float weight[8], uint cube, out vec3 faces[6]) {
    for (uint f = 0u; f < 6u; f++) {
        faces[f] = vec3(0.0);
    }
    for (uint k = 0u; k < count; k++) {
        for (uint f = 0u; f < 6u; f++) {
            faces[f] += weight[k] * field.texels[base[k] + cube + f].rgb;
        }
    }
}

// Cosine-weighted mean the six faces give a face with normal `n`.
vec3 tier_cube_face(vec3 faces[6], vec3 n) {
    uvec3 f = tier_faces_of(n);
    vec3 nn = n * n;
    return nn.x * faces[f.x] + nn.y * faces[f.y] + nn.z * faces[f.z];
}

// Radiance the six faces give along `dir` (each face weighted by the squared cosine).
vec3 tier_cube_dir(vec3 faces[6], vec3 dir) {
    vec3 dd = dir * dir;
    return dd.x * faces[dir.x >= 0.0 ? 0 : 1] + dd.y * faces[dir.y >= 0.0 ? 2 : 3] + dd.z * faces[dir.z >= 0.0 ? 4 : 5];
}

// How much the tier covers `world`: 1 inside the window, fading to 0 over the last
// `fade` metres before its edge.
float tier_cover(vec3 world, float fade) {
    vec4 info = field.texels[TIER_INFO];
    if (info.w <= 0.0) {
        return 0.0;
    }
    vec3 lo = info.xyz * 4.0 * info.w;
    vec3 hi = lo + field.texels[TIER_DIMS].xyz * 4.0 * info.w;
    vec3 inside = min(world - lo, hi - world);
    float edge = min(inside.x, min(inside.y, inside.z));
    return clamp(edge / fade, 0.0, 1.0);
}

