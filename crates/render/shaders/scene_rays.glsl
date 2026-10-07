// One description of the scene for every ray and every visibility test.
//
// Direct light, shadows, probe rays, bounce hits, world probes, the volume bake and
// the picture all ask the same three questions here: what does this ray hit first
// (scene_ray), is this segment clear (scene_occluded), and is this point inside
// something (scene_inside). Every surface type answers through the same path:
// the occluder list (walls, solids, stand-ins), the floor plane and the roof plane.
// A new primitive goes in scene_ray and scene_inside and nowhere else.
//
// Needs `scene` (floor_center, floor_data, ceiling, occs, occ_count) declared first.

struct SceneHit {
    float t;
    vec3 normal;
    vec3 albedo;
    // Diffuse reflectance and surface-color mix, already resolved from the defaults.
    float reflect;
    float color_mix;
};

const float SCENE_LAMBERT = 0.318309886;

bool floor_span(vec2 p) {
    vec2 half_e = vec2(max(scene.floor_center.w, 0.5), max(scene.floor_data.x, 0.5));
    vec2 d = p - vec2(scene.floor_center.x, scene.floor_center.z);
    return abs(d.x) <= half_e.x + 0.05 && abs(d.y) <= half_e.y + 0.05;
}

bool has_roof() {
    return scene.ceiling.x > 0.0;
}

// Vertical span of an occluder. Packing writes center.y at the middle of the span.
void occ_span(Occ occ, out float y0, out float y1) {
    y0 = occ.center_shape.y - occ.extent.y * 0.5;
    y1 = occ.center_shape.y + occ.extent.y * 0.5;
}

// Entry distance into a box, or -1. An origin inside reports -1: scene_inside covers it.
bool ray_box(vec3 origin, vec3 dir, vec3 min_p, vec3 max_p, out float t, out vec3 normal) {
    float t_enter = -1.0e20;
    float t_exit = 1.0e20;
    int axis_in = 0;
    for (int axis = 0; axis < 3; axis++) {
        if (abs(dir[axis]) < 1.0e-8) {
            if (origin[axis] < min_p[axis] || origin[axis] > max_p[axis]) {
                return false;
            }
            continue;
        }
        float inv = 1.0 / dir[axis];
        float t1 = (min_p[axis] - origin[axis]) * inv;
        float t2 = (max_p[axis] - origin[axis]) * inv;
        float lo = min(t1, t2);
        if (lo > t_enter) {
            t_enter = lo;
            axis_in = axis;
        }
        t_exit = min(t_exit, max(t1, t2));
    }
    if (t_exit < t_enter || t_enter <= 0.0) {
        return false;
    }
    t = t_enter;
    normal = vec3(0.0);
    normal[axis_in] = dir[axis_in] > 0.0 ? -1.0 : 1.0;
    return true;
}

// Entry distance into a closed cylinder: the side and both caps.
bool ray_cylinder(vec3 origin, vec3 dir, vec2 center, float radius, float y0, float y1, out float t, out vec3 normal) {
    float best = 1.0e20;
    vec2 o = origin.xz - center;
    float a = dot(dir.xz, dir.xz);
    if (a > 1.0e-8) {
        float b = dot(o, dir.xz);
        float c = dot(o, o) - radius * radius;
        float disc = b * b - a * c;
        if (disc >= 0.0) {
            float t0 = (-b - sqrt(disc)) / a;
            float y = origin.y + dir.y * t0;
            if (t0 > 0.0 && y >= y0 && y <= y1) {
                best = t0;
                vec2 side = (o + dir.xz * t0) / max(radius, 1.0e-4);
                normal = vec3(side.x, 0.0, side.y);
            }
        }
    }
    if (abs(dir.y) > 1.0e-8) {
        float cap = dir.y < 0.0 ? y1 : y0;
        float tc = (cap - origin.y) / dir.y;
        vec2 at = o + dir.xz * tc;
        if (tc > 0.0 && tc < best && dot(at, at) <= radius * radius) {
            best = tc;
            normal = vec3(0.0, dir.y < 0.0 ? 1.0 : -1.0, 0.0);
        }
    }
    if (best >= 1.0e19) {
        return false;
    }
    t = best;
    return true;
}

bool occ_ray(Occ occ, vec3 origin, vec3 dir, out float t, out vec3 normal) {
    float y0;
    float y1;
    occ_span(occ, y0, y1);
    if (occ.center_shape.w > 0.5) {
        return ray_cylinder(origin, dir, occ.center_shape.xz, occ.extent.w, y0, y1, t, normal);
    }
    vec3 lo = vec3(occ.center_shape.x - occ.extent.x, y0, occ.center_shape.z - occ.extent.z);
    vec3 hi = vec3(occ.center_shape.x + occ.extent.x, y1, occ.center_shape.z + occ.extent.z);
    return ray_box(origin, dir, lo, hi, t, normal);
}

const uint SCENE_FLOOR_BIT = 1u << 16u;
const uint SCENE_ROOF_BIT = 1u << 17u;
const uint SCENE_ALL = 0xFFFFFFFFu;

// The surfaces that reach into the box lo..hi: bit i for occluder i, plus the floor
// and roof bits. A segment with both ends in the box can only meet these, so a caller
// testing many short segments in one neighbourhood tests only them (often none).
uint scene_candidates(vec3 lo, vec3 hi) {
    uint mask = 0u;
    uint count = min(scene.occ_count, 16u);
    for (uint i = 0u; i < count; i++) {
        Occ occ = scene.occs[i];
        float y0;
        float y1;
        occ_span(occ, y0, y1);
        vec2 half_e = occ.center_shape.w > 0.5 ? vec2(occ.extent.w) : vec2(occ.extent.x, occ.extent.z);
        vec3 o_lo = vec3(occ.center_shape.x - half_e.x, y0, occ.center_shape.z - half_e.y);
        vec3 o_hi = vec3(occ.center_shape.x + half_e.x, y1, occ.center_shape.z + half_e.y);
        if (all(lessThanEqual(o_lo, hi)) && all(lessThanEqual(lo, o_hi))) {
            mask |= 1u << i;
        }
    }
    if (lo.y <= 0.0 && hi.y >= 0.0) {
        mask |= SCENE_FLOOR_BIT;
    }
    if (has_roof() && lo.y <= scene.ceiling.x && hi.y >= scene.ceiling.x) {
        mask |= SCENE_ROOF_BIT;
    }
    return mask;
}

// Nearest surface along the ray with t0 <= t < t1.
bool scene_ray(vec3 origin, vec3 dir, float t0, float t1, out SceneHit hit);

// scene_ray over the surfaces in mask (scene_candidates) only.
bool scene_ray_masked(vec3 origin, vec3 dir, float t0, float t1, uint mask, out SceneHit hit) {
    hit.t = t1;
    hit.normal = vec3(0.0, 1.0, 0.0);
    hit.albedo = vec3(1.0);
    hit.reflect = SCENE_LAMBERT;
    hit.color_mix = 1.0;
    bool found = false;
    uint count = min(scene.occ_count, 16u);
    for (uint i = 0u; i < count; i++) {
        if ((mask & (1u << i)) == 0u) {
            continue;
        }
        Occ occ = scene.occs[i];
        float t;
        vec3 n;
        if (occ_ray(occ, origin, dir, t, n) && t >= t0 && t < hit.t) {
            hit.t = t;
            hit.normal = n;
            hit.albedo = occ.albedo.rgb;
            hit.reflect = occ.bounce.x < 0.0 ? SCENE_LAMBERT : clamp(occ.bounce.x, 0.0, 1.0);
            hit.color_mix = occ.bounce.y < 0.0 ? 1.0 : clamp(occ.bounce.y, 0.0, 1.0);
            found = true;
        }
    }
    // The floor and the roof are opaque planes over the floor footprint, seen from
    // either side.
    for (int plane = 0; plane < 2; plane++) {
        if (plane == 1 && !has_roof()) {
            continue;
        }
        if ((mask & (plane == 0 ? SCENE_FLOOR_BIT : SCENE_ROOF_BIT)) == 0u) {
            continue;
        }
        float height = plane == 0 ? 0.0 : scene.ceiling.x;
        if (abs(dir.y) < 1.0e-8) {
            continue;
        }
        float t = (height - origin.y) / dir.y;
        if (t >= t0 && t < hit.t && t > 0.0 && floor_span((origin + dir * t).xz)) {
            hit.t = t;
            hit.normal = vec3(0.0, origin.y >= height ? 1.0 : -1.0, 0.0);
            hit.albedo = plane == 0 ? scene.floor_data.yzw : scene.ceiling.yzw;
            hit.reflect = SCENE_LAMBERT;
            hit.color_mix = 1.0;
            found = true;
        }
    }
    return found;
}

bool scene_ray(vec3 origin, vec3 dir, float t0, float t1, out SceneHit hit) {
    return scene_ray_masked(origin, dir, t0, t1, SCENE_ALL, hit);
}

// Strictly inside a solid or wall, or above the roof over the floor footprint. A
// point on a face is outside.
bool scene_inside(vec3 p) {
    if (has_roof() && p.y > scene.ceiling.x && floor_span(p.xz)) {
        return true;
    }
    uint count = min(scene.occ_count, 16u);
    for (uint i = 0u; i < count; i++) {
        Occ occ = scene.occs[i];
        float y0;
        float y1;
        occ_span(occ, y0, y1);
        if (p.y <= y0 || p.y >= y1) {
            continue;
        }
        vec2 d = p.xz - occ.center_shape.xz;
        if (occ.center_shape.w > 0.5) {
            if (dot(d, d) < occ.extent.w * occ.extent.w) {
                return true;
            }
        } else if (abs(d.x) < occ.extent.x && abs(d.y) < occ.extent.z) {
            return true;
        }
    }
    return false;
}

// True when anything opaque lies between a and b, or a starts inside something.
bool scene_occluded(vec3 a, vec3 b) {
    vec3 delta = b - a;
    float dist = length(delta);
    if (dist < 1.0e-3) {
        return false;
    }
    if (scene_inside(a)) {
        return true;
    }
    SceneHit hit;
    return scene_ray(a, delta / dist, 1.0e-4, dist - 1.0e-3, hit);
}
