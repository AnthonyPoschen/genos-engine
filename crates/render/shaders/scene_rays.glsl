// One description of the scene for every ray and every visibility test.
//
// Direct light, shadows, probe rays, bounce hits, world probes, the volume bake and
// the picture all ask the same three questions here: what does this ray hit first
// (scene_ray), is this segment clear (scene_occluded), and is this point inside
// something (scene_inside). Every surface type answers through the same path:
// the occluder list (walls, solids, stand-ins), the floor plane and the roof plane.
// A new primitive goes in scene_ray and scene_inside and nowhere else.
//
// Needs the scene block (scene_data.glsl) declared first, and
// GL_EXT_control_flow_attributes.
//
// Every loop here runs over data the scene block sizes (cells, list entries), with a
// bound read from the block and [[dont_unroll]]. These functions are inlined at every
// ray and shadow test, so a loop with a large constant bound (the DDA once had 4096)
// invites a driver to unroll it into each copy: the NVIDIA compiler grew past 17 GB
// and never finished the pipeline. A data bound cannot be unrolled.
//
// The occluders sit in a sparse box centered on the camera. The cell is 2 m. A ray
// jumps empty 16 m bricks. Inside a brick, one test covers the fine cells that
// repeat the same shapes. The floor plane and the roof plane stay single tests.
// A ray that leaves the box misses.

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

// Origin of the camera box. occ_grid is (origin.x, origin.z, cell, origin.y).
vec3 occ_box_origin() {
    return vec3(scene.occ_grid.x, scene.occ_grid.w, scene.occ_grid.y);
}

bool occ_grid_empty() {
    return scene.occ_count == 0u || scene.occ_grid.z <= 0.0 || scene.grid_dims.x == 0u || grid_word(6u) == 0u;
}

// Header: fine counts, step cap, coarse counts x y, brick count, coarse count z.
// Word 8 on is one entry per 16 m brick. A brick is 512 fine slots, a record count,
// then records (local, list word, count). A fine slot is 0 or the list's count word.

uint occ_local_index(ivec3 cell) {
    return uint(cell.x & 7) + 8u * uint(cell.z & 7) + 64u * uint(cell.y & 7);
}

// Brick word for a coarse cell, when any shape overlaps that 16 m brick.
bool occ_brick(ivec3 coarse, out uint brick) {
    ivec3 n = ivec3(int(grid_word(4u)), int(grid_word(5u)), int(grid_word(7u)));
    brick = 0u;
    if (any(lessThan(coarse, ivec3(0))) || any(greaterThanEqual(coarse, n))) {
        return false;
    }
    uint at = 8u + uint((coarse.z * n.y + coarse.y) * n.x + coarse.x);
    brick = grid_word(at);
    return brick != 0u;
}

// Shape list for one stored 2 m cell inside `brick`. A high bit is an empty-cell jump.
bool occ_fine(uint brick, ivec3 cell, out uint list, out uint n) {
    uint at = grid_word(brick + occ_local_index(cell));
    list = 0u;
    n = 0u;
    if (at == 0u || (at & 0x80000000u) != 0u) {
        return false;
    }
    n = grid_word(at) & 255u;
    list = at + 1u;
    return n != 0u;
}

ivec3 occ_cell_at(vec3 origin, vec3 dir, float t) {
    vec3 p = origin + dir * (t + 1.0e-3);
    return ivec3(floor((p - occ_box_origin()) / scene.occ_grid.z));
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
        vec3 org = occ_box_origin();
        float cell = scene.occ_grid.z;
        ivec3 dims = ivec3(int(scene.grid_dims.x), int(grid_word(1u)), int(scene.grid_dims.y));
        ivec3 c0 = clamp(ivec3(floor((min(lo, hi) - org) / cell)), ivec3(0), dims - 1);
        ivec3 c1 = clamp(ivec3(floor((max(lo, hi) - org) / cell)), ivec3(0), dims - 1);
        ivec3 b0 = c0 >> 3;
        ivec3 b1 = c1 >> 3;
        // A neighbourhood covers a few 16 m bricks. Empty bricks are one hash miss.
        [[dont_unroll]] for (int cy = b0.y; cy <= b1.y && mask == 0u; cy++) {
            [[dont_unroll]] for (int cz = b0.z; cz <= b1.z && mask == 0u; cz++) {
                [[dont_unroll]] for (int cx = b0.x; cx <= b1.x && mask == 0u; cx++) {
                    uint brick;
                    if (!occ_brick(ivec3(cx, cy, cz), brick)) {
                        continue;
                    }
                    int x0 = max(c0.x, cx << 3);
                    int x1 = min(c1.x, (cx << 3) + 7);
                    int y0 = max(c0.y, cy << 3);
                    int y1 = min(c1.y, (cy << 3) + 7);
                    int z0 = max(c0.z, cz << 3);
                    int z1 = min(c1.z, (cz << 3) + 7);
                    if (x1 < x0 || y1 < y0 || z1 < z0) {
                        continue;
                    }
                    uint nx = uint(x1 - x0 + 1);
                    uint ny = uint(y1 - y0 + 1);
                    uint nz = uint(z1 - z0 + 1);
                    uint total = nx * ny * nz;
                    [[dont_unroll]] for (uint i = 0u; i < total && mask == 0u; i++) {
                        uint ix = uint(x0) + (i % nx);
                        uint yz = i / nx;
                        uint iy = uint(y0) + (yz % ny);
                        uint iz = uint(z0) + (yz / ny);
                        uint list;
                        uint list_n;
                        if (!occ_fine(brick, ivec3(ix, iy, iz), list, list_n)) {
                            continue;
                        }
                        [[dont_unroll]] for (uint k = 0u; k < list_n && mask == 0u; k++) {
                            Occ occ = scene_occ(grid_word(list + k));
                            if (occ_meets(occ, lo, hi) && !(skip && occ_inside(occ, skip_in))) {
                                mask |= SCENE_OCC_BIT;
                            }
                        }
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

// Test the occluders of one stored cell against the ray, keeping the nearest hit.
void occ_test_list(uint list, uint n, vec3 origin, vec3 dir, float t0, inout SceneHit hit, inout bool found) {
    [[dont_unroll]] for (uint k = 0u; k < n; k++) {
        Occ occ = scene_occ(grid_word(list + k));
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

// Jump the stored cells from t0 to hit.t. Stop when the nearest hit is closer than
// the next cell. The step cap is header word 3. A ray that leaves the box misses.
void occ_walk(vec3 origin, vec3 dir, float t0, inout SceneHit hit, inout bool found) {
    if (occ_grid_empty()) {
        return;
    }
    vec3 org = occ_box_origin();
    float cell = scene.occ_grid.z;
    vec3 box_hi = org + vec3(float(scene.grid_dims.x), float(grid_word(1u)), float(scene.grid_dims.y)) * cell;
    float t_enter = t0;
    float box_leave = hit.t;
    for (int axis = 0; axis < 3; axis++) {
        float d = dir[axis];
        float o = origin[axis];
        if (abs(d) < 1.0e-8) {
            if (o < org[axis] || o > box_hi[axis]) {
                return;
            }
            continue;
        }
        float ta = (org[axis] - o) / d;
        float tb = (box_hi[axis] - o) / d;
        t_enter = max(t_enter, min(ta, tb));
        box_leave = min(box_leave, max(ta, tb));
    }
    if (t_enter >= box_leave) {
        return;
    }
    float t = max(t_enter, t0);
    ivec3 c = occ_cell_at(origin, dir, t);
    ivec3 dims = ivec3(int(scene.grid_dims.x), int(grid_word(1u)), int(scene.grid_dims.y));
    uint safety = max(grid_word(3u), 1u);
    [[dont_unroll]] for (uint i = 0u; i < safety; i++) {
        float limit = min(hit.t, box_leave);
        if (t >= limit - 1.0e-4) {
            return;
        }
        if (any(lessThan(c, ivec3(0))) || any(greaterThanEqual(c, dims))) {
            return;
        }
        ivec3 coarse = c >> 3;
        uint brick;
        if (occ_brick(coarse, brick)) {
            // A miss crosses every fine cell that repeats this cell's shapes.
            ivec3 lo = c;
            ivec3 hi = c + ivec3(1);
            uint slot = grid_word(brick + occ_local_index(c));
            if (slot != 0u && (slot & 0x80000000u) == 0u) {
                uint raw = grid_word(slot);
                occ_test_list(slot + 1u, raw & 255u, origin, dir, t0, hit, found);
                ivec3 base = coarse << 3;
                lo = base + ivec3(int((raw >> 8u) & 7u), int((raw >> 11u) & 7u), int((raw >> 14u) & 7u));
                hi = base + ivec3(int((raw >> 17u) & 7u), int((raw >> 20u) & 7u), int((raw >> 23u) & 7u)) + ivec3(1);
            }
            float leave = 1.0e30;
            for (int axis = 0; axis < 3; axis++) {
                if (abs(dir[axis]) < 1.0e-8) {
                    continue;
                }
                int face = dir[axis] > 0.0 ? hi[axis] : lo[axis];
                float hit_t = (org[axis] + float(face) * cell - origin[axis]) / dir[axis];
                if (hit_t > t + 1.0e-5) {
                    leave = min(leave, hit_t);
                }
            }
            if ((found && hit.t <= leave) || leave <= t + 1.0e-5 || leave >= limit) {
                return;
            }
            t = leave;
            c = occ_cell_at(origin, dir, t);
        } else {
            ivec3 base = coarse << 3;
            float jump = 1.0e30;
            for (int axis = 0; axis < 3; axis++) {
                if (abs(dir[axis]) < 1.0e-8) {
                    continue;
                }
                int face = dir[axis] > 0.0 ? base[axis] + 8 : base[axis];
                float hit_t = (org[axis] + float(face) * cell - origin[axis]) / dir[axis];
                if (hit_t > t + 1.0e-5) {
                    jump = min(jump, hit_t);
                }
            }
            if (jump >= limit - 1.0e-4 || jump <= t + 1.0e-5) {
                return;
            }
            t = jump;
            c = occ_cell_at(origin, dir, t);
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

// True when anything opaque lies between a and b. Unlike scene_occluded a start
// inside a solid is not tested: a caller with many segments from one point tests it
// once.
bool scene_segment_blocked(vec3 a, vec3 b) {
    vec3 delta = b - a;
    float dist = length(delta);
    if (dist < 1.0e-3) {
        return false;
    }
    SceneHit hit;
    return scene_ray(a, delta / dist, 1.0e-4, dist - 1.0e-3, hit);
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
    vec3 org = occ_box_origin();
    vec3 box_hi = org + vec3(float(scene.grid_dims.x), float(grid_word(1u)), float(scene.grid_dims.y)) * scene.occ_grid.z;
    if (any(lessThan(p, org)) || any(greaterThan(p, box_hi))) {
        return false;
    }
    ivec3 cell = ivec3(floor((p - org) / scene.occ_grid.z));
    uint brick;
    uint list;
    uint n;
    if (!occ_brick(cell >> 3, brick) || !occ_fine(brick, cell, list, n)) {
        return false;
    }
    [[dont_unroll]] for (uint k = 0u; k < n; k++) {
        if (occ_inside(scene_occ(grid_word(list + k)), p)) {
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
// occluder walk stops at the camera box, so the extra span tests stored cells
// inside that box and nothing past it.

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

// The suns, the point lamps of p's cell and the flame, as one list: a caller walks it
// in one loop and inlines its lamp shading (and the shadow ray in it) once, not once
// for the suns, again for the lamps and again for the flame.
struct LampList {
    uvec2 suns;
    uvec2 near;
};

LampList lamps_at(vec3 p) {
    LampList list;
    list.suns = lamp_suns();
    list.near = lamp_cell_at(p);
    return list;
}

uint lamp_list_size(LampList list) {
    return list.suns.y + list.near.y + (scene.fire_pos.w > 0.0 ? 1u : 0u);
}

// The flame is a point lamp with no range (color.w 0: lamp_reaches always passes).
Lamp lamp_list_get(LampList list, uint k) {
    uint lamps = list.suns.y + list.near.y;
    if (k >= lamps) {
        Lamp fire;
        fire.pos = vec4(scene.fire_pos.xyz, 0.0);
        fire.color = vec4(scene.fire_color.rgb * scene.fire_pos.w, 0.0);
        return fire;
    }
    uint word = k < list.suns.y ? list.suns.x + k : list.near.x + (k - list.suns.y);
    return scene_lamp(grid_word(word));
}

// A unit white lamp 7 m above a white painted floor stays near 0.37.
const float LAMP_UNIT = 72.0;
// Radius of a lamp bulb. Outside it a lamp falls off with the inverse square.
const float LAMP_RADIUS = 0.1;

// Light a lamp brings to a face at origin before any shadow test (zero out of its
// range or behind the face), and the point a shadow ray runs to. A two-sided face
// (a camera card) takes the light on either side.
vec3 lamp_light(Lamp lamp, vec3 origin, vec3 normal, bool two_sided, out vec3 target) {
    target = origin;
    if (!lamp_reaches(lamp, origin)) {
        return vec3(0.0);
    }
    vec3 toward;
    float dist2;
    if (lamp.pos.w > 0.5) {
        // A sun's xyz is the direction its rays travel. It uses the same unit as a
        // lamp 7 m away.
        toward = -normalize(lamp.pos.xyz);
        target = origin + toward * 80.0;
        dist2 = 49.0;
    } else {
        toward = lamp.pos.xyz - origin;
        float dist = max(length(toward), 1.0e-4);
        toward /= dist;
        target = lamp.pos.xyz;
        dist2 = dist * dist;
    }
    float nd = dot(toward, normal);
    if (two_sided) {
        nd = abs(nd);
    }
    if (nd <= 0.0) {
        return vec3(0.0);
    }
    return lamp.color.rgb * nd * LAMP_UNIT / max(dist2, LAMP_RADIUS * LAMP_RADIUS);
}

// Shadowed light at a face at origin from the lamps of its list. Where at most
// `picks` lamps light the face, each is shadow-tested: the exact sum. Where more do,
// `picks` of them are chosen in proportion to their unshadowed light, and a chosen
// lamp's shadowed light is divided by the number of times it is expected to be
// chosen: an unbiased estimate of the same sum for `picks` shadow rays, however many
// lamps light the face. The choice is systematic: points (j + u) / picks along the
// running total of the lamps' light, j = 0 .. picks - 1, u in [0, 1), so a lamp
// with that share s of the light is chosen floor or ceil of s * picks times, and a
// caller that averages many samples (a probe, a pixel's rays) varies u to cover them.
// The unshadowed light is cheap; the shadow rays are the cost.
vec3 lamps_light(vec3 origin, vec3 normal, bool two_sided, uint picks, float u) {
    // A face inside a solid sees no lamp: one test instead of one per shadow ray.
    if (scene_inside(origin)) {
        return vec3(0.0);
    }
    LampList list = lamps_at(origin);
    uint count = lamp_list_size(list);
    float total = 0.0;
    uint lit = 0u;
    [[dont_unroll]] for (uint k = 0u; k < count; k++) {
        vec3 target;
        vec3 light = lamp_light(lamp_list_get(list, k), origin, normal, two_sided, target);
        float w = light.r + light.g + light.b;
        total += w;
        lit += w > 0.0 ? 1u : 0u;
    }
    bool exact = lit <= picks;
    // Light per pick.
    float step = total / float(max(picks, 1u));
    float run = 0.0;
    vec3 sum = vec3(0.0);
    [[dont_unroll]] for (uint k = 0u; k < count && lit > 0u; k++) {
        vec3 target;
        vec3 light = lamp_light(lamp_list_get(list, k), origin, normal, two_sided, target);
        float w = light.r + light.g + light.b;
        if (w <= 0.0) {
            continue;
        }
        float times = 1.0;
        if (!exact) {
            float before = ceil(run / step - u);
            run += w;
            times = ceil(run / step - u) - before;
        }
        if (times > 0.0 && !scene_segment_blocked(origin, target)) {
            sum += exact ? light : light * (times * step / w);
        }
    }
    return sum;
}
