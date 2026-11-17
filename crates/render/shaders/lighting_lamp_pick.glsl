// Weighted point-lamp picking for the direct shade loop (lighting_direct.comp).
//
// Production screen pixels are the C1-C5 ReSTIR sample, not this file.
// The sun is not a sample. Hits use Talbot, Cline, and Egbert, EGSR 2005:
// systematic samples along the unshadowed prefix. The noise gate's random
// path draws fresh candidates here and does not merge a neighbour. The
// labelled exact sum walks every point lamp. This file does not trace.
// The shade loop's one trace_occluded is the only shadow ray.
//
// Each slot is six uints (24 bytes). Kind 2 is the exact sum, kind 0 a
// noise-gate sample, kind 3 the stored ReSTIR mean. The sample count is the
// low 24 bits of `m`; the kind is the top byte. Deviations from the papers
// are listed in docs/systems/lighting/README.md. The CPU twin of the checker
// merge stays in crates/render/src/lamp_pick.rs.

// At this many positive point lamps, or fewer, the noise-gate reuse row
// shades the sum. Production pixels do not: they take the ReSTIR sample.
// Matches EXACT_LIT_LAMPS.
const uint LAMP_PICK_EXACT_LIT = 8u;
// Record kind in the geo word. A cleared slot is all ones, which is neither.
const uint LAMP_GEO_RESTIR = 0u;
const uint LAMP_GEO_EXACT = 2u;
// Mean of the unbiased ReSTIR DI sample. This is the graded colour.
const uint LAMP_GEO_RESTIR_MEAN = 3u;
// Samples in that mean. Past this the colour stays, so a still surface
// converges, until the stored sample's visibility or target changes.
// Same length as LAMP_PICK_MEAN_CAP. A 32-frame cap froze the grain.
const uint RESTIR_MEAN_CAP = 4096u;
// No stored lamp. A real lamp index still fits in the low 16 bits.
const uint RESTIR_MEAN_NO_LAMP = 0xffffu;

// 12-bit log luma. One code is 2^(1/64), about 1.1%. A still lamp
// round-trips the same code. A wider ratio would keep a different integrand.
const float RESTIR_MEAN_LUMA_FLOOR = 1.0e-8;
const float RESTIR_MEAN_LOG_BIAS = 24.0;
const float RESTIR_MEAN_LOG_SCALE = 64.0;
const float RESTIR_MEAN_ROUND = 0.5;
const float RESTIR_MEAN_LOG_MAX = 4095.0;

uint restir_mean_quant(float luma) {
    if (!(luma > RESTIR_MEAN_LUMA_FLOOR)) {
        return 0u;
    }
    float q = (log2(luma) + RESTIR_MEAN_LOG_BIAS) * RESTIR_MEAN_LOG_SCALE;
    return uint(clamp(floor(q + RESTIR_MEAN_ROUND), 1.0, RESTIR_MEAN_LOG_MAX));
}

bool restir_mean_target_moved(uint stored_q, float luma) {
    return restir_mean_quant(luma) != stored_q;
}

// Lamp in 0..15, visibility in bit 16, 12-bit log luma in bits 17..28.
// LAMP_ALL would wipe the reservoir, so a missing lamp stays RESTIR_MEAN_NO_LAMP.
uint restir_mean_pack(uint lamp, bool vis, float luma) {
    uint id = lamp < RESTIR_MEAN_NO_LAMP ? lamp : RESTIR_MEAN_NO_LAMP;
    uint bit = vis ? 0x10000u : 0u;
    return id | bit | (restir_mean_quant(luma) << 17u);
}
// Reject history when the depth gap is both over this fraction and over this many metres.
const float LAMP_PICK_DEPTH_REL = 0.02;
const float LAMP_PICK_DEPTH_ABS = 0.05;
// Reject history when the normal dot is below this.
const float LAMP_PICK_NORMAL_MIN = 0.9;

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

// The noise gate sets this so a still view keeps tracing the random path.
// Production leaves it clear and does not take this hold: a restir product
// pixel shades the reservoir on a quiet frame. Bit 19 skips this signature
// hold so a failed prepare shades the labelled exact sum.
bool lamp_pick_hold_off() {
    return ((pc.lamp.y >> 18u) & 1u) != 0u;
}

// Shade every point lamp. The noise gate's baseline is this sum, not lamp pick off.
bool lamp_pick_force_exact() {
    return ((pc.lamp.y >> 19u) & 1u) != 0u;
}

// Fault injection for the noise gate: uniform pick, shade weight 1, no MIS.
bool lamp_pick_broken() {
    return ((pc.lamp.y >> 20u) & 1u) != 0u;
}

// Running mean of fresh unbiased samples while the hold is off. Past this the
// picture stays put and the pixel keeps tracing, so the grade sees a settled
// integral and a non-zero ray count. R, G, and blue stay full floats: the
// step at a few thousand samples is under an fp16 ulp, an rgb9e5 mean went
// dark, and fp16 blue missed the noise-gate mean. View depth is fp16. The
// normal is an octahedral snorm8, expanded back to snorm16 on load. Two
// slots stay when a neighbour can be reprojected.
const uint LAMP_PICK_MEAN_CAP = 4096u;
// lamp, f32 weight, M|kind, f32 target, f32 blue (or rgb9e5 bits), fp16 depth|snorm8 normal.
const uint LAMP_RES_WORDS = 6u;
const float LAMP_SNORM8_SCALE = 127.0;

layout(std430, set = 0, binding = 12) buffer LampReservoirs {
    uint w[];
} lamp_res;

uint lamp_res_index(uint pixel, uint slot) {
    return (slot * pc.dims.x * pc.dims.y + pixel) * LAMP_RES_WORDS;
}

LampReservoir lamp_res_load(uint pixel, uint slot) {
    uint base = lamp_res_index(pixel, slot);
    LampReservoir r;
    r.lamp = lamp_res.w[base];
    r.w_sum = uintBitsToFloat(lamp_res.w[base + 1u]);
    r.m = lamp_res.w[base + 2u] & 0x00ffffffu;
    r.target = uintBitsToFloat(lamp_res.w[base + 3u]);
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
    uint base = lamp_res_index(pixel, slot);
    uint kind = lamp_res.w[base + 2u] & 0xff000000u;
    lamp_res.w[base] = r.lamp;
    lamp_res.w[base + 1u] = floatBitsToUint(r.w_sum);
    lamp_res.w[base + 2u] = (r.m & 0x00ffffffu) | kind;
    lamp_res.w[base + 3u] = floatBitsToUint(r.target);
}

// glslang has no packSnorm2x8. Low 8 bits are x, the next 8 are y.
uint lamp_float_to_snorm8(float c) {
    int q = int(round(clamp(c, -1.0, 1.0) * LAMP_SNORM8_SCALE));
    if (q < 0) {
        return uint(q + 256);
    }
    return uint(q);
}

float lamp_snorm8_to_float(uint bits) {
    uint b = bits & 0xffu;
    int q = int(b);
    if (b >= 128u) {
        q -= 256;
    }
    return clamp(float(q) / LAMP_SNORM8_SCALE, -1.0, 1.0);
}

uint lamp_pack_snorm2x8(vec2 e) {
    return lamp_float_to_snorm8(e.x) | (lamp_float_to_snorm8(e.y) << 8u);
}

vec3 lamp_unpack_oct8(uint bits) {
    vec2 e = vec2(lamp_snorm8_to_float(bits), lamp_snorm8_to_float(bits >> 8u));
    vec3 n = vec3(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        n.xy = lighting_oct_wrap(n.xy);
    }
    return normalize(n);
}

// Word 5. Depth is fp16 (the reject is 2% and 5 cm). The normal is an
// octahedral snorm8 (the reject is a 0.9 dot).
uint lamp_pack_geo_word(float depth, vec3 n) {
    float len = abs(n.x) + abs(n.y) + abs(n.z);
    if (len > 0.0) {
        n /= len;
    } else {
        n = vec3(0.0, 1.0, 0.0);
    }
    vec2 e = n.z >= 0.0 ? n.xy : lighting_oct_wrap(n.xy);
    uint depth16 = packHalf2x16(vec2(depth, 0.0)) & 0xffffu;
    uint normal8 = lamp_pack_snorm2x8(e) & 0xffffu;
    return depth16 | (normal8 << 16u);
}

uvec4 lamp_geo_load(uint pixel, uint slot) {
    uint base = lamp_res_index(pixel, slot);
    uint packed = lamp_res.w[base + 5u];
    float depth = unpackHalf2x16(packed & 0xffffu).x;
    uint kind = lamp_res.w[base + 2u] >> 24u;
    // Callers still unpack a snorm16 normal. Word 4 is the caller's bits:
    // a full float for both means, rgb9e5 for the exact path.
    return uvec4(floatBitsToUint(depth), lighting_pack_normal(lamp_unpack_oct8(packed >> 16u)), kind, lamp_res.w[base + 4u]);
}

void lamp_geo_store(uint pixel, uint slot, uvec4 geo) {
    uint base = lamp_res_index(pixel, slot);
    lamp_res.w[base + 4u] = geo.w;
    lamp_res.w[base + 5u] = lamp_pack_geo_word(uintBitsToFloat(geo.x), lighting_unpack_normal(geo.y));
    uint m = lamp_res.w[base + 2u] & 0x00ffffffu;
    lamp_res.w[base + 2u] = m | ((geo.z & 255u) << 24u);
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
// `only_px` of ~0u accepts any in-bounds pixel. Any other value returns
// before the geo load when the pixel is not that one, so a one-slot mean
// does not read a neighbour this dispatch is writing.
bool lamp_pick_reproject_at(vec3 pos, vec3 nrm, uint only_px, out uint hist_pixel, out float z_now) {
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
    if (only_px != ~0u && hist_pixel != only_px) {
        return false;
    }
    uvec4 geo = lamp_geo_load(hist_pixel, lamp_res_slot_prev());
    float z_prev = uintBitsToFloat(geo.x);
    if (!(z_prev > 0.0)) {
        return false;
    }
    float gap = abs(z - z_prev);
    if (gap / z > LAMP_PICK_DEPTH_REL && gap > LAMP_PICK_DEPTH_ABS) {
        return false;
    }
    vec3 n_prev = lighting_unpack_normal(geo.y);
    if (dot(n_prev, nrm) < LAMP_PICK_NORMAL_MIN) {
        return false;
    }
    return true;
}

bool lamp_pick_reproject(vec3 pos, vec3 nrm, out uint hist_pixel, out float z_now) {
    return lamp_pick_reproject_at(pos, nrm, ~0u, hist_pixel, z_now);
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

// One positive point lamp, each equally likely. `u` is in [0, 1). No target weight.
uint lamp_pick_uniform(LampList list, uint count, vec3 origin, vec3 nrm, bool two_sided, uint lit, float u) {
    if (lit == 0u) {
        return ~0u;
    }
    uint want = min(uint(floor(u * float(lit))), lit - 1u);
    uint seen = 0u;
    [[dont_unroll]] for (uint k = 0u; k < count; k++) {
        if (gstats.ctrl_max_lights != 0u && k >= gstats.ctrl_max_lights) {
            break;
        }
        if (k < list.suns.y) {
            continue;
        }
        vec3 target;
        vec3 light = lamp_light(lamp_list_get(list, k), origin, nrm, two_sided, target);
        if (light.r + light.g + light.b <= 0.0) {
            continue;
        }
        if (seen == want) {
            return k;
        }
        seen += 1u;
    }
    return ~0u;
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
