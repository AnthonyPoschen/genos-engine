// Weighted point-lamp picking for the direct shade loop (lighting_direct.comp).
//
// The sun is not a sample. Point lamps and the flame use Talbot, Cline, and
// Egbert, EGSR 2005: systematic samples along the unshadowed prefix. Unbiased
// pixel reuse combines reservoirs with pairwise MIS (Bitterli et al. 2022;
// Wyman, SIGGRAPH 2023). The Jacobian is 1 (point lamps). The biased path is
// Bitterli et al. 2020 section 3 plus the M clamp, not Algorithm 6. This file
// does not trace. The shade loop's one trace_occluded is the only shadow ray.
//
// Low keeps one reservoir (temporal only, radius 0). Medium and High keep
// the current and previous slots. Each slot is two uvec4s: the reservoir, then
// depth along that frame's forward, the packed normal, the record kind, and
// the shaded direct (rgb9e5). Kind 2 is the exact sum, kind 1 a trusted copy
// of one, kind 0 a ReSTIR sample. The high 8 bits of `m` are the hold count;
// the merge reads the low 24 bits. Deviations from the papers are listed in
// docs/systems/lighting/README.md. The CPU twin is crates/render/src/lamp_pick.rs.

// A settled exact record carries this in the high 8 bits of `m`, so the next
// quiet held frame takes the S78J early-out. Matches LAMP_PICK_HOLD_FRAMES.
const uint LIGHTING_DIRECT_HOLD = 32u;
// At this many positive point lamps, or fewer, a pixel shades the sum. Above
// it, one visibility sample is cheaper than the sum. Matches EXACT_LIT_LAMPS.
const uint LAMP_PICK_EXACT_LIT = 8u;
// Record kind in the geo word. A cleared slot is all ones, which is neither.
const uint LAMP_GEO_RESTIR = 0u;
const uint LAMP_GEO_COPY = 1u;
const uint LAMP_GEO_EXACT = 2u;

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

uint lamp_res_frames(LampReservoir r) {
    return r.m >> 24u;
}

uint lamp_res_samples(LampReservoir r) {
    return r.m & 0x00ffffffu;
}

// Radius 0 is one slot, read and written in place. A neighbour merge needs
// the previous frame left untouched, so it ping-pongs two slots.
bool lamp_res_one_slot() {
    return pc.lamp.z == 0u;
}

uint lamp_res_slot_now() {
    return lamp_res_one_slot() ? 0u : (pc.params.w & 1u);
}

uint lamp_res_slot_prev() {
    return lamp_res_one_slot() ? 0u : ((pc.params.w + 1u) & 1u);
}

// A quiet pixel changes its candidate each frame so the mean can converge.
// A listed mover freezes the seed: the shade does not boil while something moves.
uint lamp_pick_frame_mix() {
    return lighting_pix_quiet() ? pc.params.w : 0u;
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

uint lamp_res_index(uint pixel, uint slot) {
    return (slot * pc.dims.x * pc.dims.y + pixel) * 2u;
}

LampReservoir lamp_res_load(uint pixel, uint slot) {
    uvec4 w = lamp_res.v[lamp_res_index(pixel, slot)];
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
    lamp_res.v[lamp_res_index(pixel, slot)] =
        uvec4(r.lamp, floatBitsToUint(r.w_sum), r.m, floatBitsToUint(r.target));
}

uvec4 lamp_geo_load(uint pixel, uint slot) {
    return lamp_res.v[lamp_res_index(pixel, slot) + 1u];
}

void lamp_geo_store(uint pixel, uint slot, uvec4 geo) {
    lamp_res.v[lamp_res_index(pixel, slot) + 1u] = geo;
}

float lamp_res_w(LampReservoir r) {
    uint m = lamp_res_samples(r);
    if (m == 0u || r.target <= 0.0 || r.lamp == LAMP_ALL) {
        return 0.0;
    }
    return r.w_sum / (float(m) * r.target);
}

// Depth of `pos` along the camera that is shading this frame.
float lamp_pick_depth(vec3 pos) {
    return dot(pos - scene.eye.xyz, scene.view_forward.xyz);
}

// Motion vector into the previous frame. The returned pixel is in bounds.
// Depth (2% and 5 cm) and normal (dot > 0.9) rejection use the stored geo.
// A mover bound does not reject on its own. GI off has no last camera.
bool lamp_pick_reproject(vec3 pos, vec3 nrm, out uint hist_pixel, out float z_now) {
    hist_pixel = 0u;
    z_now = 0.0;
    if (!lighting_gi_on()) {
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
    z_now = z;
    if (z <= 1.0e-3) {
        return false;
    }
    vec2 ndc = vec2(dot(rel, right) / (z * t * aspect), dot(rel, up) / (z * t));
    vec2 px = vec2((ndc.x + 1.0) * 0.5 * float(pc.dims.x), (1.0 - ndc.y) * 0.5 * float(pc.dims.y));
    if (px.x < 0.0 || px.y < 0.0 || px.x >= float(pc.dims.x) || px.y >= float(pc.dims.y)) {
        return false;
    }
    hist_pixel = uint(floor(px.y)) * pc.dims.x + uint(floor(px.x));
    uvec4 geo = lamp_geo_load(hist_pixel, lamp_res_slot_prev());
    float z_prev = uintBitsToFloat(geo.x);
    if (!(z_prev > 0.0)) {
        return false;
    }
    float gap = abs(z - z_prev);
    if (gap / z > 0.02 && gap > 0.05) {
        return false;
    }
    vec3 n_prev = lighting_unpack_normal(geo.y);
    if (dot(n_prev, nrm) < 0.9) {
        return false;
    }
    return true;
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
    canonical.m = lamp_res_samples(canonical);
    history.m = lamp_res_samples(history);
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
    uint m_out = min(canonical.m + hist_m, 0x00ffffffu);
    LampReservoir outr;
    outr.lamp = lamp;
    outr.w_sum = float(m_out) * wsum / float(z);
    outr.m = m_out;
    outr.target = target;
    return outr;
}

// Pairwise MIS. J = 1. Identical domains with the same W keep W.
// phat_q_* is the unshadowed luminance at the current pixel.
// phat_hist_* is that luminance at the history pixel.
LampReservoir lamp_pick_pairwise(
    LampReservoir canonical,
    LampReservoir history,
    float phat_q_can,
    float phat_q_hist,
    float phat_hist_can,
    float phat_hist_hist,
    float u
) {
    canonical.m = lamp_res_samples(canonical);
    history.m = lamp_res_samples(history);
    if (canonical.m == 0u || canonical.lamp == LAMP_ALL) {
        if (history.m == 0u) {
            return lamp_res_empty();
        }
        return history;
    }
    float mass_c = float(canonical.m) * lamp_res_w(canonical) * max(phat_q_can, 0.0);
    float mass_h = 0.0;
    if (history.m > 0u && history.lamp != LAMP_ALL) {
        mass_h = float(history.m) * lamp_res_w(history) * max(phat_q_hist, 0.0);
    }
    float area = mass_c + mass_h;
    if (area <= 0.0) {
        return lamp_res_empty();
    }
    bool pick_h = mass_h > 0.0 && u < mass_h / area;
    float phat_q_sel = pick_h ? max(phat_q_hist, 0.0) : max(phat_q_can, 0.0);
    float phat_src = pick_h ? max(phat_hist_hist, 0.0) : max(phat_q_can, 0.0);
    float phat_other = pick_h ? max(phat_q_hist, 0.0) : max(phat_hist_can, 0.0);
    uint m_sel = pick_h ? history.m : canonical.m;
    uint m_other = pick_h ? canonical.m : history.m;
    float denom = float(m_sel) * phat_src + float(m_other) * phat_other;
    if (phat_q_sel <= 0.0 || phat_src <= 0.0 || denom <= 0.0) {
        return lamp_res_empty();
    }
    float w_new = area * phat_src / (phat_q_sel * denom);
    uint m_out = min(canonical.m + history.m, 0x00ffffffu);
    LampReservoir outr;
    outr.lamp = pick_h ? history.lamp : canonical.lamp;
    outr.w_sum = w_new * float(m_out) * phat_q_sel;
    outr.m = m_out;
    outr.target = phat_q_sel;
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
