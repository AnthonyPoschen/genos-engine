// The persistent world probe tier (probe_tier.rs): probes on a fixed world lattice,
// in bricks of 4 x 4 x 4 that exist only near geometry. A window of bricks around
// the camera maps each brick to a slot, or to none. Each probe keeps three ambient
// cubes, six faces each of cosine-weighted mean radiance: light that bounced up to
// three times (TIER_CUBE3, alpha = samples, 0 = no light yet), up to twice
// (TIER_CUBE2) and once (TIER_CUBE1).
//
// Needs `field` and scene_rays.glsl declared first. Offsets mirror probe_tier.rs.

const uint TIER_INFO = 524288u;
const uint TIER_DIMS = TIER_INFO + 1u;
const uint TIER_INDIR = TIER_INFO + 2u;
const uint TIER_INDIR_CAP = 32768u;
const uint TIER_SLOTS = TIER_INDIR + TIER_INDIR_CAP;
const uint TIER_WORK = TIER_SLOTS + 2048u * 2u;
const uint TIER_PROBES = TIER_INFO + 40960u;
const uint TIER_PROBE_TEXELS = 18u;
const uint TIER_CUBE3 = 0u;
const uint TIER_CUBE2 = 6u;
const uint TIER_CUBE1 = 12u;

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

// Cosine-weighted mean radiance arriving at a face at `world` with normal `face_n`,
// from cubes `cube_a` and `cube_b` of the eight probes around the point pushed half a
// spacing off the face. A probe that is dead (inside a solid, far from surfaces), not
// lit yet, outside the window or hidden from the point drops out and the others take
// its weight. False when none is left.
bool tier_sample2(vec3 world, vec3 face_n, uint cube_a, uint cube_b, out vec3 color_a, out vec3 color_b) {
    color_a = vec3(0.0);
    color_b = vec3(0.0);
    float spacing = tier_spacing();
    if (spacing <= 0.0) {
        return false;
    }
    vec3 q = (world + face_n * (0.5 * spacing)) / spacing - 0.5;
    ivec3 i0 = ivec3(floor(q));
    vec3 t = q - vec3(i0);
    vec3 from = world + face_n * 0.02;
    vec3 nn = face_n * face_n;
    uint fx = face_n.x >= 0.0 ? 0u : 1u;
    uint fy = face_n.y >= 0.0 ? 2u : 3u;
    uint fz = face_n.z >= 0.0 ? 4u : 5u;
    // Only surfaces reaching into the box around the point and its eight probes can
    // hide a probe. Away from geometry that is none, and the taps cost no ray tests.
    vec3 near_lo = (vec3(i0) + 0.5) * spacing;
    vec3 near_hi = near_lo + vec3(spacing);
    uint blockers = scene_candidates(min(near_lo, from), max(near_hi, from));
    // A point wedged in a corner can start inside the other solid. Rays from there see
    // no entry into it and would reach probes on its far side. Nothing reaches a point
    // inside a solid.
    if (blockers != 0u && scene_inside(from)) {
        return true;
    }
    vec3 sum_a = vec3(0.0);
    vec3 sum_b = vec3(0.0);
    float wsum = 0.0;
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
        uint base = uint(at);
        if (field.texels[base + TIER_CUBE3 + fx].w <= 0.0) {
            continue;
        }
        if (blockers != 0u) {
            vec3 delta = (vec3(c) + 0.5) * spacing - from;
            float dist = length(delta);
            SceneHit hit;
            if (dist > 1.0e-3 && scene_ray_masked(from, delta / dist, 1.0e-4, dist - 1.0e-3, blockers, hit)) {
                continue;
            }
        }
        sum_a += w * (nn.x * field.texels[base + cube_a + fx].rgb
            + nn.y * field.texels[base + cube_a + fy].rgb
            + nn.z * field.texels[base + cube_a + fz].rgb);
        if (cube_b != cube_a) {
            sum_b += w * (nn.x * field.texels[base + cube_b + fx].rgb
                + nn.y * field.texels[base + cube_b + fy].rgb
                + nn.z * field.texels[base + cube_b + fz].rgb);
        }
        wsum += w;
    }
    if (wsum <= 1.0e-4) {
        return false;
    }
    color_a = sum_a / wsum;
    color_b = cube_b != cube_a ? sum_b / wsum : color_a;
    return true;
}

// The light the picture shows: up to three bounces.
bool tier_sample(vec3 world, vec3 face_n, out vec3 color) {
    vec3 unused;
    return tier_sample2(world, face_n, TIER_CUBE3, TIER_CUBE3, color, unused);
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
