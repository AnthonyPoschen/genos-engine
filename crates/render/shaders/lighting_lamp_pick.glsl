// Weighted point-lamp picking for the direct shade loop (lighting_direct.comp).
//
// The sun is not a sample. Point lamps and the flame use Talbot, Cline, and
// Egbert, EGSR 2005: systematic samples along the unshadowed prefix. Pixel
// reuse combines reservoirs with Bitterli et al., TOG 2020, Algorithm 6 as
// printed (Z counts a source only when its target of the selected lamp is
// positive). This file does not trace. The shade loop's one trace_occluded
// is the only shadow ray.
//
// Deviations from the paper are listed in docs/systems/lighting/README.md.
// The CPU twin is crates/render/src/lamp_pick.rs.

layout(std430, set = 0, binding = 12) buffer LampReservoirs {
    uvec4 v[];
} lamp_res;

struct LampReservoir {
    uint lamp;
    float w_sum;
    uint m;
    float target;
};

LampReservoir lamp_res_empty() {
    LampReservoir r;
    r.lamp = LAMP_ALL;
    r.w_sum = 0.0;
    r.m = 0u;
    r.target = 0.0;
    return r;
}

uint lamp_pick_candidates() {
    return clamp(pc.lamp.y & 255u, 1u, 32u);
}

uint lamp_pick_rays() {
    return clamp((pc.lamp.y >> 8u) & 255u, 1u, 4u);
}

bool lamp_pick_unbiased() {
    return ((pc.lamp.y >> 16u) & 1u) != 0u;
}

bool lamp_pick_diag() {
    return ((pc.lamp.y >> 17u) & 1u) != 0u;
}

LampReservoir lamp_res_load(uint pixel, uint slot) {
    uvec4 w = lamp_res.v[slot * pc.dims.x * pc.dims.y + pixel];
    LampReservoir r;
    r.lamp = w.x;
    r.w_sum = uintBitsToFloat(w.y);
    r.m = w.z;
    r.target = uintBitsToFloat(w.w);
    // A cleared slot is all ones. Lamp 0 is a real lamp, so empty is LAMP_ALL,
    // and a leftover M must not enter the merge.
    if (r.lamp == LAMP_ALL) {
        r.w_sum = 0.0;
        r.m = 0u;
        r.target = 0.0;
    }
    return r;
}

void lamp_res_store(uint pixel, uint slot, LampReservoir r) {
    lamp_res.v[slot * pc.dims.x * pc.dims.y + pixel] =
        uvec4(r.lamp, floatBitsToUint(r.w_sum), r.m, floatBitsToUint(r.target));
}

// Same-pixel history. No motion vectors and no history G-buffer: a point that
// leaves its pixel, or sits in a mover bound, is a miss. GI off has no last
// camera in the work buffer, so it is a miss too.
bool lamp_pick_history_ok(vec3 pos, uvec2 pix) {
    if (!lighting_gi_on()) {
        return false;
    }
    if (lighting_near_change(pos)) {
        return false;
    }
    vec4 eye = work.v[lighting_change_at() + 1u];
    if (eye.w < 0.5) {
        return false;
    }
    vec3 right = work.v[lighting_change_at() + 2u].xyz;
    float t = work.v[lighting_change_at() + 2u].w;
    vec3 up = work.v[lighting_change_at() + 3u].xyz;
    vec3 fwd = work.v[lighting_change_at() + 4u].xyz;
    float aspect = work.v[lighting_change_at() + 4u].w;
    if (dot(fwd, fwd) < 0.5 || t == 0.0) {
        return false;
    }
    vec3 rel = pos - eye.xyz;
    float z = dot(rel, fwd);
    if (z <= 1.0e-3) {
        return false;
    }
    vec2 ndc = vec2(dot(rel, right) / (z * t * aspect), dot(rel, up) / (z * t));
    // Same crop as lighting_eye_dir. A guard band would otherwise shift the
    // history test off the lit pixel by the crop.
    float dw = float(pc.dims.x);
    float dh = float(pc.dims.y);
    float ox = 0.0;
    float oy = 0.0;
    if (pc.frame.x != 0u && pc.frame.y != 0u) {
        uint crop = pc.schedule.w;
        dw = float(pc.frame.x);
        dh = float(pc.frame.y);
        ox = float(crop & 0xffffu);
        oy = float(crop >> 16u);
    }
    vec2 px = vec2((ndc.x + 1.0) * 0.5 * dw + ox, (1.0 - ndc.y) * 0.5 * dh + oy);
    return ivec2(floor(px)) == ivec2(pix);
}

uint lamp_pick_find(LampList list, uint count, uint lamp) {
    if (lamp == LAMP_ALL) {
        return ~0u;
    }
    [[dont_unroll]] for (uint k = 0u; k < count; k++) {
        if (gstats.ctrl_max_lights != 0u && k >= gstats.ctrl_max_lights) {
            break;
        }
        if (lamp_list_index(list, k) == lamp) {
            return k;
        }
    }
    return ~0u;
}

float lamp_pick_phat(LampList list, uint count, uint lamp, vec3 origin, vec3 nrm, bool two_sided) {
    uint k = lamp_pick_find(list, count, lamp);
    if (k == ~0u) {
        return 0.0;
    }
    vec3 target;
    vec3 light = lamp_light(lamp_list_get(list, k), origin, nrm, two_sided, target);
    return light.r + light.g + light.b;
}

// phat_q_* is the unshadowed luminance at the current pixel. phat_hist_* is
// that luminance at the reservoir's source pixel. Same-pixel history passes
// the same pair twice.
LampReservoir lamp_pick_merge(
    LampReservoir canonical,
    LampReservoir history,
    float phat_q_can,
    float phat_q_hist,
    float phat_hist_can,
    float phat_hist_hist,
    bool unbiased,
    uint history_cap,
    float u
) {
    if (canonical.m == 0u || canonical.lamp == LAMP_ALL) {
        if (history.m == 0u) {
            return lamp_res_empty();
        }
        return history;
    }
    uint hist_m = history.m;
    if (!unbiased) {
        hist_m = min(history.m, max(history_cap, 1u));
    }
    float phat_can = max(phat_q_can, 0.0);
    float phat_h = 0.0;
    if (history.lamp != LAMP_ALL) {
        phat_h = max(phat_q_hist, 0.0);
    }
    float w_can = 0.0;
    if (canonical.target > 0.0 && phat_can > 0.0) {
        w_can = canonical.w_sum * phat_can / canonical.target;
    }
    float w_hist = 0.0;
    if (hist_m > 0u && history.target > 0.0 && phat_h > 0.0) {
        float clamped = float(hist_m) / float(max(history.m, 1u));
        w_hist = history.w_sum * phat_h / history.target * clamped;
    }
    float wsum = 0.0;
    uint lamp = LAMP_ALL;
    float target = 0.0;
    if (w_can > 0.0) {
        wsum = w_can;
        lamp = canonical.lamp;
        target = phat_can;
    }
    if (w_hist > 0.0) {
        wsum += w_hist;
        if (wsum > 0.0 && u < w_hist / wsum) {
            lamp = history.lamp;
            target = phat_h;
        }
    }
    if (lamp == LAMP_ALL || target <= 0.0) {
        return lamp_res_empty();
    }
    bool selected_can = lamp == canonical.lamp;
    float phat_q_sel = selected_can ? phat_can : phat_h;
    float phat_hist_sel = selected_can ? phat_hist_can : phat_hist_hist;
    bool can_counts = canonical.m > 0u && phat_q_sel > 0.0;
    bool hist_counts = hist_m > 0u && phat_hist_sel > 0.0;
    uint z = unbiased
        ? canonical.m * uint(can_counts) + hist_m * uint(hist_counts)
        : canonical.m + hist_m;
    if (z == 0u) {
        z = 1u;
    }
    uint m_out = canonical.m + hist_m;
    LampReservoir outr;
    outr.lamp = lamp;
    outr.w_sum = float(m_out) * wsum / float(z);
    outr.m = m_out;
    outr.target = target;
    return outr;
}

// Unshadowed point lamps and the flame. Suns stay out of this sum.
void lamp_pick_point_sum(LampList list, uint count, vec3 origin, vec3 nrm, bool two_sided, out float total, out uint lit) {
    total = 0.0;
    lit = 0u;
    [[dont_unroll]] for (uint k = 0u; k < count; k++) {
        if (gstats.ctrl_max_lights != 0u && k >= gstats.ctrl_max_lights) {
            break;
        }
        if (k < list.suns.y) {
            continue;
        }
        vec3 target;
        vec3 light = lamp_light(lamp_list_get(list, k), origin, nrm, two_sided, target);
        float w = light.r + light.g + light.b;
        total += w;
        lit += w > 0.0 ? 1u : 0u;
    }
}

// One systematic sample along the point-lamp prefix (rays = 1).
uint lamp_pick_one(LampList list, uint count, vec3 origin, vec3 nrm, bool two_sided, float total, uint lit, float u, out float phat) {
    phat = 0.0;
    if (lit == 0u || total <= 0.0) {
        return ~0u;
    }
    bool exact = lit <= 1u;
    float step = total;
    float run = 0.0;
    uint found = ~0u;
    [[dont_unroll]] for (uint k = 0u; k < count; k++) {
        if (gstats.ctrl_max_lights != 0u && k >= gstats.ctrl_max_lights) {
            break;
        }
        if (k < list.suns.y) {
            continue;
        }
        vec3 target;
        vec3 light = lamp_light(lamp_list_get(list, k), origin, nrm, two_sided, target);
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
        if (times > 0.0) {
            found = k;
            phat = w;
        }
    }
    return found;
}

// Pixel samples include the frame only while reuse is on, so a still frame
// without reuse does not flicker. Hits keep the cell or patch hash, with no
// frame, so a still hit does not crawl.
float lamp_pick_u(uint idx, uint pixels, uint kept, bool fill_hit, bool reuse_pix, uint ray, uint h, vec3 pos, vec3 nrm) {
    if (idx < pixels) {
        return lighting_unit(reuse_pix ? (idx ^ pc.params.w) : idx);
    }
    if (idx >= pixels + kept && !fill_hit) {
        uint r = ray - lighting_probe_rays();
        uint ent = r / LIGHTING_CACHE_RAYS;
        if (lighting_world_mode() == 3u && ent >= lighting_grad_first() && ent < LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES) {
            uint gs = ckeys.key[LIGHTING_GRAD_LIST_AT + r - lighting_grad_first() * LIGHTING_CACHE_RAYS];
            return lighting_unit(ckeys.key[gs] ^ (floatBitsToUint(cache.v[LIGHTING_SC_GRAD_AT + gs].w) * 0x85ebca6bu + 0x5bd1e995u));
        }
        uint slot = lighting_cache_batch_slot(ent);
        uint key = slot == ~0u ? 0u : ckeys.key[slot];
        uint q = slot == ~0u ? 0u : lighting_cache_entry_quarter(ent, slot);
        return lighting_unit(key ^ ((q * LIGHTING_CACHE_RAYS + r % LIGHTING_CACHE_RAYS) * 0x85ebca6bu + 0x5bd1e995u));
    }
    return lighting_unit(lighting_cell_hash(pos, nrm) ^ (h * 0x85ebca6bu + 0x27d4eb2fu));
}
