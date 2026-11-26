// Cone-traced cascades: the probe light a pixel gathers. Included by the
// gather pass (cone_shade.comp, a share of the pixels) and the fill pass
// (cone_fill.comp, the pixels the resolve found no match for), after
// lighting_common.glsl and cone_common.glsl.

// The irradiance a pixel at `pos` facing `n` reads from the voxel-layer
// probes of level `lod` around it.
// The debug views' red and grey.
const float CONE_VIEW_RED = 4.0;
const float CONE_VIEW_GREY = 0.25;

// What the last level_irradiance read, for the debug views: its probes'
// total weight, and the share of the directions facing the pixel that held
// no light.
float shade_weight = 0.0;
float shade_dark = 0.0;

// The pick's record of the probes of one level: for each candidate in the
// hash, its directions with no light, its lit ones and its mean radiance.
void pick_probes(uint at, int lod, ivec3 own, int count) {
    cwork.w[at] = uint(lod);
    cwork.w[at + 1u] = uint(count);
    cwork.w[at + 2u] = uint(own.x);
    cwork.w[at + 3u] = uint(own.y);
    cwork.w[at + 4u] = uint(own.z);
    cwork.w[at + 6u] = 1u;
    uint dirs = cone_dirs(0);
    for (uint k = 0u; k < 8u; k++) {
        uint r = at + 8u + k * CONE_PICK_PROBE;
        uint probe = cwork.w[r + 1u];
        if ((cwork.w[r] & 1u) == 0u || probe >= cone_cap(0)) {
            continue;
        }
        uint dark = 0u;
        uint lit = 0u;
        float luma = 0.0;
        float open = 0.0;
        float out_luma = 0.0;
        float out_dirs = 0.0;
        vec3 way = cone_axis(cone_key_face(cwork.w[cone_probe(0, probe)]));
        for (uint bin = 0u; bin < dirs; bin++) {
            vec3 l;
            float th;
            vec3 m;
            bool filled = cone_get(cone_merged_at(0, probe, bin), l, th, m);
            float y = dot(l, CONE_LUMA);
            open += filled ? th : 0.0;
            if (dot(cone_bin_dir(bin, cone_theta(0)), way) > 0.0) {
                out_luma += filled ? y : 0.0;
                out_dirs += 1.0;
            }
            if (filled && y > 0.0) {
                lit++;
                luma += y;
            } else {
                dark++;
            }
        }
        cwork.w[r + 4u] = dark;
        cwork.w[r + 5u] = floatBitsToUint(luma / float(dirs));
        cwork.w[r + 9u] = lit;
        cwork.w[r + 10u] = floatBitsToUint(open / float(dirs));
        cwork.w[r + 11u] = floatBitsToUint(out_luma / max(out_dirs, 1.0));
    }
}

// The lamps the pixel being gathered takes from its probes: each lamp's
// index plus one and the share of it they see (their rays' shares,
// cone_lamp.comp, by the probes' weights). The first CONE_PIXEL_LAMPS met.
uint gather_lamp[CONE_PIXEL_LAMPS];
float gather_seen[CONE_PIXEL_LAMPS];
void gather_lamps_clear() {
    for (uint k = 0u; k < CONE_PIXEL_LAMPS; k++) {
        gather_lamp[k] = 0u;
        gather_seen[k] = 0.0;
    }
}
void gather_lamp_add(uint id, float share) {
    for (uint k = 0u; k < CONE_PIXEL_LAMPS; k++) {
        if (gather_lamp[k] == 0u || gather_lamp[k] == id) {
            gather_lamp[k] = id;
            gather_seen[k] += share;
            return;
        }
    }
}

// `share` is this level's part of the pixel's light (its blend weight).
vec3 level_irradiance(int lod, vec3 pos, vec3 n, float share) {
    uint probes[8];
    float weights[8];
    ivec3 own = cone_cell(pos, n, cone_spacing(lod, 0));
    int count = cone_weights(0, lod, cone_face(n), pos, (pc.lamp.x & 2u) != 0u, pos + n * CONE_SURFACE_LIFT, own, probes, weights);
    if (cone_pick_at != 0u) {
        pick_probes(cone_pick_at, lod, own, count);
    }
    shade_weight = 0.0;
    for (int k = 0; k < count; k++) {
        shade_weight += weights[k];
    }
    // Each lamp's share seen, over the probes read.
    for (int k = 0; k < count && shade_weight > 0.0; k++) {
        uint list = cone_targets_of(0, probes[k]);
        uint seen = cone_seen_of(probes[k]);
        for (uint t = 0u; t < CONE_TARGETS; t++) {
            uint id = cwork.w[list + t];
            if (id == 0u) {
                break;
            }
            gather_lamp_add(id, share * weights[k] / shade_weight * uintBitsToFloat(cwork.w[seen + t]));
        }
    }
    float faced = 0.0;
    float unlit = 0.0;
    uint theta = cone_theta(0);
    uint dirs = cone_dirs(0);
    vec3 sum = vec3(0.0);
    float weight = 0.0;
    for (uint bin = 0u; bin < dirs; bin++) {
        vec3 d = cone_bin_dir(bin, theta);
        float cosine = dot(d, n);
        if (cosine <= 0.0) {
            continue;
        }
        vec3 v = vec3(0.0);
        vec3 m = vec3(0.0);
        float wsum = 0.0;
        for (int k = 0; k < count; k++) {
            vec3 l;
            float th;
            vec3 mm;
            if (cone_get(cone_merged_at(0, probes[k], bin), l, th, mm)) {
                v += l * weights[k];
                m += mm * weights[k];
                wsum += weights[k];
            }
        }
        faced += 1.0;
        if (wsum <= 0.0 || dot(v, CONE_LUMA) <= 0.0) {
            unlit += 1.0;
        }
        if (wsum <= 0.0) {
            continue;
        }
        v /= wsum;
        m /= wsum;
        // The light's own cosine when it is known (the bin mean cosine).
        float held = length(m);
        float y = dot(v, CONE_LUMA);
        float lit = cosine;
        if (held > CONE_TINIER && y > CONE_TINIER) {
            float spread = min(held / y, 1.0);
            lit = max(dot(m / held, n), 0.0) * spread + cosine * (1.0 - spread);
        }
        sum += v * lit;
        weight += cosine;
    }
    shade_dark = faced > 0.0 ? unlit / faced : 1.0;
    return weight > 0.0 ? sum * (CONE_PI / weight) : vec3(0.0);
}

// The probe light of the surface at `pixel` (`pos`, facing `n`): the
// irradiance of the voxel-layer probes about it, at its level and, in a
// blend band, the one under it; or, with a debug view on
// (cone_cascades.view), that view's colour. When `pick` and the pixel is the
// picked one (push dims.w, its index plus one), what it reads goes to the
// work buffer for cone_pick.
vec3 cone_pixel_gather(uint pixel, vec3 pos, vec3 n, bool pick) {
    int lo;
    int hi;
    float w_hi;
    cone_pixel_levels(pos, lo, hi, w_hi);
    bool picked = pick && pc.dims.w != 0u && pixel + 1u == pc.dims.w;
    if (picked) {
        for (uint k = 1u; k < CONE_PICK_HEAD + 2u * CONE_PICK_LEVEL; k++) {
            cwork.w[CONE_PICK + k] = 0u;
        }
        cwork.w[CONE_PICK + 1u] = uint(lo);
        cwork.w[CONE_PICK + 2u] = uint(hi);
        cwork.w[CONE_PICK + 3u] = floatBitsToUint(w_hi);
        cone_put3(CONE_PICK + 4u, pos);
        cone_put3(CONE_PICK + 7u, n);
        cwork.w[CONE_PICK + 10u] = uint(cone_face(n));
        cone_pick_at = CONE_PICK + CONE_PICK_HEAD;
    }
    bool two = lo != hi && w_hi < 1.0;
    gather_lamps_clear();
    vec3 irradiance = level_irradiance(hi, pos, n, two ? w_hi : 1.0);
    float read_weight = shade_weight;
    float least_weight = shade_weight;
    float dark = shade_dark;
    if (lo != hi && w_hi < 1.0) {
        if (picked) {
            cone_pick_at = CONE_PICK + CONE_PICK_HEAD + CONE_PICK_LEVEL;
        }
        irradiance = mix(level_irradiance(lo, pos, n, 1.0 - w_hi), irradiance, w_hi);
        read_weight = mix(shade_weight, read_weight, w_hi);
        least_weight = min(least_weight, shade_weight);
        dark = mix(shade_dark, dark, w_hi);
    }
    cone_pick_at = 0u;
    if (picked) {
        cone_put3(CONE_PICK + 12u, irradiance);
        cwork.w[CONE_PICK + 11u] = floatBitsToUint(field_sun(pos, n));
        cwork.w[CONE_PICK] = pixel + 1u;
    }
    uint view = (pc.lamp.x >> CONE_VIEW_SHIFT) & 3u;
    if (view == CONE_VIEW_COVERAGE) {
        return least_weight <= 0.0 ? vec3(CONE_VIEW_RED, 0.0, 0.0) : (least_weight <= CONE_PASS_FLOOR ? vec3(CONE_VIEW_RED, CONE_VIEW_RED, 0.0) : vec3(read_weight));
    }
    if (view == CONE_VIEW_DARK) {
        return mix(vec3(CONE_VIEW_GREY), vec3(CONE_VIEW_RED, 0.0, 0.0), dark);
    }
    return irradiance;
}
