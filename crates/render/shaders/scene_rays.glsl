// One description of the scene for every ray and every visibility test.
//
// Direct light, shadows, probe rays, bounce hits, world probes, the volume bake and
// the picture all ask the same three questions here: what does this ray hit first
// (scene_ray), is this segment clear (scene_occluded), and is this point inside
// something (scene_inside). Every surface type answers through the same path:
// the occluder list (walls, solids, stand-ins), the floor plane and the roof plane.
// A new primitive goes in scene_ray and scene_inside and nowhere else.
//
// Needs the scene block (scene_data.glsl) declared first.
//
// The occluders sit in a grid of square cells on the ground plane. A ray walks the
// cells it crosses and tests only the occluders listed there, so its cost follows
// the shapes along its path, not the size of the scene.

struct SceneHit {
    float t;
    vec3 normal;
    vec3 albedo;
    // Diffuse reflectance and surface-color mix, already resolved from the defaults.
    float reflect;
    float color_mix;
};

// Lambertian BRDF of the default reflectance, white paint: 0.8 / pi.
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

// Turn a ground-plane offset into an occluder's own frame (its yaw undone), and back.
vec2 occ_local(Occ occ, vec2 d) {
    float c = occ.bounce.z;
    float s = occ.bounce.w;
    return vec2(c * d.x - s * d.y, s * d.x + c * d.y);
}

vec2 occ_world(Occ occ, vec2 d) {
    float c = occ.bounce.z;
    float s = occ.bounce.w;
    return vec2(c * d.x + s * d.y, -s * d.x + c * d.y);
}

bool occ_turned(Occ occ) {
    return occ.center_shape.w < 0.5 && occ.bounce.w != 0.0;
}

bool occ_ray(Occ occ, vec3 origin, vec3 dir, out float t_in, out float t_out, out vec3 normal) {
    float y0;
    float y1;
    occ_span(occ, y0, y1);
    if (occ.center_shape.w > 0.5) {
        return ray_cylinder(origin, dir, occ.center_shape.xz, occ.extent.w, y0, y1, t_in, t_out, normal);
    }
    vec3 half_e = vec3(occ.extent.x, 0.0, occ.extent.z);
    if (occ_turned(occ)) {
        // A turned box: test the ray in the box's own frame, then turn the normal back.
        vec2 o = occ_local(occ, origin.xz - occ.center_shape.xz);
        vec2 d = occ_local(occ, dir.xz);
        vec3 lo = vec3(-half_e.x, y0, -half_e.z);
        vec3 hi = vec3(half_e.x, y1, half_e.z);
        vec3 n;
        bool hit = ray_box(vec3(o.x, origin.y, o.y), vec3(d.x, dir.y, d.y), lo, hi, t_in, t_out, n);
        vec2 nw = occ_world(occ, n.xz);
        normal = vec3(nw.x, n.y, nw.y);
        return hit;
    }
    vec3 lo = vec3(occ.center_shape.x - half_e.x, y0, occ.center_shape.z - half_e.z);
    vec3 hi = vec3(occ.center_shape.x + half_e.x, y1, occ.center_shape.z + half_e.z);
    return ray_box(origin, dir, lo, hi, t_in, t_out, normal);
}

// Ground-plane half extents of the box around an occluder (a turned box's corners).
vec2 occ_reach(Occ occ) {
    if (occ.center_shape.w > 0.5) {
        return vec2(occ.extent.w);
    }
    if (occ_turned(occ)) {
        float c = abs(occ.bounce.z);
        float s = abs(occ.bounce.w);
        return vec2(c * occ.extent.x + s * occ.extent.z, s * occ.extent.x + c * occ.extent.z);
    }
    return vec2(occ.extent.x, occ.extent.z);
}

// Strictly inside one occluder.
bool occ_inside(Occ occ, vec3 p) {
    float y0;
    float y1;
    occ_span(occ, y0, y1);
    if (p.y <= y0 || p.y >= y1) {
        return false;
    }
    vec2 d = p.xz - occ.center_shape.xz;
    if (occ.center_shape.w > 0.5) {
        return dot(d, d) < occ.extent.w * occ.extent.w;
    }
    if (occ_turned(occ)) {
        d = occ_local(occ, d);
    }
    return abs(d.x) < occ.extent.x && abs(d.y) < occ.extent.z;
}

// Occluder grid cell of a ground point, clamped into the grid.
ivec2 occ_cell_of(vec2 p) {
    ivec2 dims = ivec2(scene.grid_dims.xy);
    ivec2 c = ivec2(floor((p - scene.occ_grid.xy) / scene.occ_grid.z));
    return clamp(c, ivec2(0), dims - 1);
}

bool occ_grid_empty() {
    return scene.occ_count == 0u || scene.grid_dims.x == 0u || scene.grid_dims.y == 0u;
}

// First list word and count of one occluder cell.
uvec2 occ_cell(ivec2 c) {
    uint at = scene.grid_at.x + 2u * (uint(c.y) * scene.grid_dims.x + uint(c.x));
    return uvec2(grid_word(at), grid_word(at + 1u));
}

// A candidate mask: whether any occluder, the floor, or the roof reaches a box.
const uint SCENE_OCC_BIT = 1u;
const uint SCENE_FLOOR_BIT = 1u << 16u;
const uint SCENE_ROOF_BIT = 1u << 17u;
const uint SCENE_ALL = 0xFFFFFFFFu;

// True when the ground box of occluder `occ` and the box lo..hi meet.
bool occ_meets(Occ occ, vec3 lo, vec3 hi) {
    float y0;
    float y1;
    occ_span(occ, y0, y1);
    vec2 half_e = occ_reach(occ);
    vec3 o_lo = vec3(occ.center_shape.x - half_e.x, y0, occ.center_shape.z - half_e.y);
    vec3 o_hi = vec3(occ.center_shape.x + half_e.x, y1, occ.center_shape.z + half_e.y);
    return all(lessThanEqual(o_lo, hi)) && all(lessThanEqual(lo, o_hi));
}

// The surfaces that reach into the box lo..hi: SCENE_OCC_BIT when any occluder does
// (other than one that holds `skip_in`, when `skip` is set), plus the floor and roof
// bits. A segment with both ends in the box can only meet these, so a caller testing
// many short segments in one neighbourhood can skip the tests when there are none.
uint scene_candidates_skip(vec3 lo, vec3 hi, bool skip, vec3 skip_in) {
    uint mask = 0u;
    if (!occ_grid_empty()) {
        ivec2 c0 = occ_cell_of(lo.xz);
        ivec2 c1 = occ_cell_of(hi.xz);
        for (int cz = c0.y; cz <= c1.y && mask == 0u; cz++) {
            for (int cx = c0.x; cx <= c1.x && mask == 0u; cx++) {
                uvec2 cell = occ_cell(ivec2(cx, cz));
                for (uint k = 0u; k < cell.y; k++) {
                    Occ occ = scene_occ(grid_word(cell.x + k));
                    if (occ_meets(occ, lo, hi) && !(skip && occ_inside(occ, skip_in))) {
                        mask |= SCENE_OCC_BIT;
                        break;
                    }
                }
            }
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

uint scene_candidates(vec3 lo, vec3 hi) {
    return scene_candidates_skip(lo, hi, false, vec3(0.0));
}

// Nearest surface along the ray with t0 <= t < t1.
bool scene_ray(vec3 origin, vec3 dir, float t0, float t1, out SceneHit hit);

// Test the occluders of one cell against the ray, keeping the nearest hit.
void occ_test_cell(ivec2 c, vec3 origin, vec3 dir, float t0, inout SceneHit hit, inout bool found) {
    uvec2 cell = occ_cell(c);
    for (uint k = 0u; k < cell.y; k++) {
        Occ occ = scene_occ(grid_word(cell.x + k));
        float t_in;
        float t_out;
        vec3 n;
        // Any part of the solid inside t0..t1 stops the ray, at t0 at the earliest. A
        // ray leaving the face it starts on has t_out at or before t0 and passes.
        if (occ_ray(occ, origin, dir, t_in, t_out, n) && t_out > t0 && max(t_in, t0) < hit.t) {
            hit.t = max(t_in, t0);
            hit.normal = n;
            hit.albedo = occ.albedo.rgb;
            // A set reflectance is an albedo, 0 to 1: over pi it is the Lambertian BRDF.
            hit.reflect = occ.bounce.x < 0.0 ? SCENE_LAMBERT : clamp(occ.bounce.x, 0.0, 1.0) * 0.318309886;
            hit.color_mix = occ.bounce.y < 0.0 ? 1.0 : clamp(occ.bounce.y, 0.0, 1.0);
            found = true;
        }
    }
}

// Walk the occluder cells the ray crosses from t0 to hit.t, nearest first, and stop
// once the nearest hit lies before the next cell. The span is first cut to the grid
// and to the height band the occluders fill (floor to the tallest top).
void occ_walk(vec3 origin, vec3 dir, float t0, inout SceneHit hit, inout bool found) {
    float lo_t = t0;
    float hi_t = hit.t;
    float top = scene.occ_grid.w;
    if (abs(dir.y) > 1.0e-8) {
        float ta = (0.0 - origin.y) / dir.y;
        float tb = (top - origin.y) / dir.y;
        lo_t = max(lo_t, min(ta, tb));
        hi_t = min(hi_t, max(ta, tb));
    } else if (origin.y < 0.0 || origin.y > top) {
        return;
    }
    vec2 g0 = scene.occ_grid.xy;
    float cell = scene.occ_grid.z;
    vec2 g1 = g0 + vec2(scene.grid_dims.xy) * cell;
    for (int axis = 0; axis < 2; axis++) {
        float o = origin[axis * 2];
        float d = dir[axis * 2];
        if (abs(d) < 1.0e-8) {
            if (o < g0[axis] || o > g1[axis]) {
                return;
            }
            continue;
        }
        float ta = (g0[axis] - o) / d;
        float tb = (g1[axis] - o) / d;
        lo_t = max(lo_t, min(ta, tb));
        hi_t = min(hi_t, max(ta, tb));
    }
    if (hi_t < lo_t) {
        return;
    }
    vec2 start = origin.xz + dir.xz * lo_t;
    ivec2 c = occ_cell_of(start);
    ivec2 dims = ivec2(scene.grid_dims.xy);
    ivec2 step = ivec2(dir.x > 0.0 ? 1 : -1, dir.z > 0.0 ? 1 : -1);
    vec2 inv = vec2(
        abs(dir.x) > 1.0e-8 ? 1.0 / dir.x : 1.0e30,
        abs(dir.z) > 1.0e-8 ? 1.0 / dir.z : 1.0e30
    );
    // Ray time at the next cell edge on each axis, and the time to cross one cell.
    vec2 edge = g0 + (vec2(c) + vec2(step.x > 0 ? 1.0 : 0.0, step.y > 0 ? 1.0 : 0.0)) * cell;
    vec2 t_next = vec2(
        abs(dir.x) > 1.0e-8 ? (edge.x - origin.x) * inv.x : 1.0e30,
        abs(dir.z) > 1.0e-8 ? (edge.y - origin.z) * inv.y : 1.0e30
    );
    vec2 t_cell = vec2(abs(cell * inv.x), abs(cell * inv.y));
    for (int i = 0; i < 4096; i++) {
        occ_test_cell(c, origin, dir, t0, hit, found);
        float leave = min(t_next.x, t_next.y);
        if (leave >= min(hit.t, hi_t)) {
            return;
        }
        if (t_next.x < t_next.y) {
            c.x += step.x;
            t_next.x += t_cell.x;
        } else {
            c.y += step.y;
            t_next.y += t_cell.y;
        }
        if (c.x < 0 || c.y < 0 || c.x >= dims.x || c.y >= dims.y) {
            return;
        }
    }
}

// scene_ray over the surfaces in mask (scene_candidates) only.
bool scene_ray_masked(vec3 origin, vec3 dir, float t0, float t1, uint mask, out SceneHit hit) {
    hit.t = t1;
    hit.normal = vec3(0.0, 1.0, 0.0);
    hit.albedo = vec3(1.0);
    hit.reflect = SCENE_LAMBERT;
    hit.color_mix = 1.0;
    bool found = false;
    if ((mask & SCENE_OCC_BIT) != 0u && !occ_grid_empty()) {
        occ_walk(origin, dir, t0, hit, found);
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
    if (occ_grid_empty()) {
        return false;
    }
    vec2 g0 = scene.occ_grid.xy;
    vec2 g1 = g0 + vec2(scene.grid_dims.xy) * scene.occ_grid.z;
    if (any(lessThan(p.xz, g0)) || any(greaterThan(p.xz, g1))) {
        return false;
    }
    uvec2 cell = occ_cell(occ_cell_of(p.xz));
    for (uint k = 0u; k < cell.y; k++) {
        if (occ_inside(scene_occ(grid_word(cell.x + k)), p)) {
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

// ---- Sky ---------------------------------------------------------------------
// A ray that leaves the scene meets the sky (genos_scene::Sky): one radiance in every
// direction. A probe ray that met nothing up to its end looks on from there; the
// occluder walk stops at the grid's edge and the occluders' top, so the extra span
// costs the cells the ray still crosses inside the scene and nothing past it.

bool has_sky() {
    return any(greaterThan(scene.sky.rgb, vec3(0.0)));
}

// The sky's radiance when a ray that met nothing before `t_end` meets nothing after
// it either; zero when something lies beyond or there is no sky.
vec3 sky_beyond(vec3 origin, vec3 dir, float t_end) {
    if (!has_sky()) {
        return vec3(0.0);
    }
    SceneHit hit;
    if (scene_ray(origin, dir, t_end, 1.0e7, hit)) {
        return vec3(0.0);
    }
    return scene.sky.rgb;
}

// ---- Lamps -------------------------------------------------------------------
// A pixel or a ray hit visits the suns, then the point lamps listed in its lamp cell:
// those whose range (pack.rs lamp_range) can reach that cell. lamp_reaches is the
// exact test, so the result does not depend on the cell size.

// Suns: first list word and count.
uvec2 lamp_suns() {
    return uvec2(scene.grid_at.z, scene.tail_at.w);
}

// Point lamps that may reach p: first list word and count. Outside the lamp grid no
// point lamp reaches.
uvec2 lamp_cell_at(vec3 p) {
    ivec2 dims = ivec2(scene.grid_dims.zw);
    if (dims.x == 0 || dims.y == 0) {
        return uvec2(0u);
    }
    ivec2 c = ivec2(floor((p.xz - scene.lamp_grid.xy) / scene.lamp_grid.z));
    if (any(lessThan(c, ivec2(0))) || any(greaterThanEqual(c, dims))) {
        return uvec2(0u);
    }
    uint at = scene.grid_at.y + 2u * (uint(c.y) * uint(dims.x) + uint(c.x));
    return uvec2(grid_word(at), grid_word(at + 1u));
}

// True when the lamp's light reaches p at all: a sun always, a point lamp inside its
// range.
bool lamp_reaches(Lamp lamp, vec3 p) {
    if (lamp.pos.w > 0.5 || lamp.color.w <= 0.0) {
        return true;
    }
    vec3 d = lamp.pos.xyz - p;
    return dot(d, d) <= lamp.color.w * lamp.color.w;
}
