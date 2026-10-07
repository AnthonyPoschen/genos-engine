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

const float SCENE_LAMBERT = 0.254647909;

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

// The span t_in..t_out of the ray inside a box, with the normal of the entry face.
// t_in is negative when the origin is inside. A ray that starts on a face, or a hair
// in front of it, still enters: the caller clamps the entry to its own t0 instead of
// dropping a solid whose entry falls before t0, which let a point beside a wall see
// through it.
bool ray_box(vec3 origin, vec3 dir, vec3 min_p, vec3 max_p, out float t_in, out float t_out, out vec3 normal) {
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
    if (t_exit < t_enter || t_exit <= 0.0) {
        return false;
    }
    t_in = t_enter;
    t_out = t_exit;
    normal = vec3(0.0);
    normal[axis_in] = dir[axis_in] > 0.0 ? -1.0 : 1.0;
    return true;
}

// The span of the ray inside a closed cylinder (side and both caps), as ray_box.
bool ray_cylinder(vec3 origin, vec3 dir, vec2 center, float radius, float y0, float y1, out float t_in, out float t_out, out vec3 normal) {
    vec2 o = origin.xz - center;
    float a = dot(dir.xz, dir.xz);
    float side_in = -1.0e20;
    float side_out = 1.0e20;
    float c = dot(o, o) - radius * radius;
    if (a > 1.0e-8) {
        float b = dot(o, dir.xz);
        float disc = b * b - a * c;
        if (disc < 0.0) {
            return false;
        }
        float root = sqrt(disc);
        side_in = (-b - root) / a;
        side_out = (-b + root) / a;
    } else if (c > 0.0) {
        return false;
    }
    float cap_in = -1.0e20;
    float cap_out = 1.0e20;
    if (abs(dir.y) > 1.0e-8) {
        float t1 = (y0 - origin.y) / dir.y;
        float t2 = (y1 - origin.y) / dir.y;
        cap_in = min(t1, t2);
        cap_out = max(t1, t2);
    } else if (origin.y < y0 || origin.y > y1) {
        return false;
    }
    t_in = max(side_in, cap_in);
    t_out = min(side_out, cap_out);
    if (t_out < t_in || t_out <= 0.0) {
        return false;
    }
    if (side_in >= cap_in) {
        vec2 at = (o + dir.xz * t_in) / max(radius, 1.0e-4);
        normal = vec3(at.x, 0.0, at.y);
    } else {
        normal = vec3(0.0, dir.y < 0.0 ? 1.0 : -1.0, 0.0);
    }
    return true;
}

bool occ_ray(Occ occ, vec3 origin, vec3 dir, out float t_in, out float t_out, out vec3 normal) {
    float y0;
    float y1;
    occ_span(occ, y0, y1);
    if (occ.center_shape.w > 0.5) {
        return ray_cylinder(origin, dir, occ.center_shape.xz, occ.extent.w, y0, y1, t_in, t_out, normal);
    }
    vec3 lo = vec3(occ.center_shape.x - occ.extent.x, y0, occ.center_shape.z - occ.extent.z);
    vec3 hi = vec3(occ.center_shape.x + occ.extent.x, y1, occ.center_shape.z + occ.extent.z);
    return ray_box(origin, dir, lo, hi, t_in, t_out, normal);
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
        float t_in;
        float t_out;
        vec3 n;
        // Any part of the solid inside t0..t1 stops the ray, at t0 at the earliest. A
        // ray leaving the face it starts on has t_out at or before t0 and passes.
        if (occ_ray(occ, origin, dir, t_in, t_out, n) && t_out > t0 && max(t_in, t0) < hit.t) {
            hit.t = max(t_in, t0);
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
