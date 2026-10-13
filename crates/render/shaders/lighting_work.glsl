// Moving-scene work cut (docs/systems/lighting/work-reduction-design.md).
// Phase 1 tags a surface pixel or a probe full effort or settled and changes
// no shaded work. Later phases read the same control word.
//
// Control word at LIGHTING_WORK_AT (ckeys, after the a-trous meta):
//   bit 0     full effort (today's path for every probe and texel)
//   bits 8-11 phase, 0..7
// Sun step at LIGHTING_WORK_AT + 1..6 (host, every frame):
//   1  bit 0 small step (amortise indirect), bit 1 a direct patch follows
//      the list compose, bit 2 this dispatch is that patch.
//      Bits 8-15 are N. Bits 16-23 are this frame's slice (frame % N).
//   2..4 committed sun direction (the direction its rays travel)
//   5 committed global sun/sky light, 6 the live one
// Lamp motion sits at LIGHTING_WORK_LAMP_AT, 8 words per lamp, 128 lamps:
//   0 prev valid, 1..3 previous position, 4..6 latest position, 7 frame noted.

const uint LIGHTING_WORK_META = 391u; // lighting_gpu.rs LIGHTING_ATROUS_META_WORDS
const uint LIGHTING_WORK_AT = LIGHTING_ATROUS_PPM_AT + LIGHTING_WORK_META;
const uint LIGHTING_WORK_LAMP_AT = LIGHTING_WORK_AT + 8u;
const uint LIGHTING_WORK_LAMP_CAP = 128u;
const float LIGHTING_WORK_FULL = 0.25; // A-SVGF full-effort grade

uint lighting_work_control() {
    return ckeys.key[LIGHTING_WORK_AT];
}
uint lighting_work_phase() {
    return (lighting_work_control() >> 8u) & 15u;
}
bool lighting_work_full() {
    return (lighting_work_control() & 1u) != 0u;
}
bool lighting_work_counting() {
    return lighting_work_phase() >= 1u && !lighting_work_full();
}

// Once per cache round (lighting_write_args, one thread). A second call in the
// same frame keeps the previous position: the frame stamp makes it idempotent.
void lighting_work_note_lamps() {
    uint n = min(scene.lamp_count, LIGHTING_WORK_LAMP_CAP);
    uint frame = lighting_frame();
    for (uint i = 0u; i < n; i++) {
        uint base = LIGHTING_WORK_LAMP_AT + 8u * i;
        if (ckeys.key[base + 7u] == frame) {
            continue;
        }
        if (ckeys.key[base + 7u] != 0u) {
            ckeys.key[base + 1u] = ckeys.key[base + 4u];
            ckeys.key[base + 2u] = ckeys.key[base + 5u];
            ckeys.key[base + 3u] = ckeys.key[base + 6u];
            ckeys.key[base] = 1u;
        }
        Lamp l = scene_lamp(i);
        ckeys.key[base + 4u] = floatBitsToUint(l.pos.x);
        ckeys.key[base + 5u] = floatBitsToUint(l.pos.y);
        ckeys.key[base + 6u] = floatBitsToUint(l.pos.z);
        ckeys.key[base + 7u] = frame;
    }
}

bool lighting_work_lamp_moved(uint i, out vec3 prev, out vec3 now) {
    uint base = LIGHTING_WORK_LAMP_AT + 8u * i;
    prev = vec3(0.0);
    now = prev;
    if (ckeys.key[base] == 0u) {
        return false;
    }
    prev = vec3(uintBitsToFloat(ckeys.key[base + 1u]), uintBitsToFloat(ckeys.key[base + 2u]), uintBitsToFloat(ckeys.key[base + 3u]));
    now = vec3(uintBitsToFloat(ckeys.key[base + 4u]), uintBitsToFloat(ckeys.key[base + 5u]), uintBitsToFloat(ckeys.key[base + 6u]));
    return length(now - prev) > 0.01;
}

// Tube of radius `motion` around the segment, starting off the surface.
// Floor and roof bits are ignored. The occluder that contains `from` is skipped.
// Wider than the swept triangle: a pixel the triangle would leave quiet can
// still be tagged full effort.
bool lighting_work_tube(vec3 from, vec3 to, float radius) {
    vec3 d = to - from;
    float len = length(d);
    if (len < 1.0e-3) {
        return false;
    }
    float start = min(0.15, len * 0.5);
    float reach = len - start;
    if (reach < 1.0e-3) {
        return false;
    }
    vec3 dir = d / len;
    uint n = min(uint(ceil(reach / 0.5)) + 1u, 12u);
    vec3 e = vec3(max(radius, 0.05));
    for (uint s = 0u; s < n; s++) {
        float t = start + reach * (float(s) + 0.5) / float(n);
        vec3 p = from + dir * t;
        if ((scene_candidates_skip(p - e, p + e, true, from) & SCENE_OCC_BIT) != 0u) {
            return true;
        }
    }
    return false;
}

// 2 fringe, 1 full effort (missing or a big grade), 3 settled.
uint lighting_work_px_tag(vec3 pos, vec3 n) {
    uint slot = lighting_cache_find_sized(pos, n, false, LIGHTING_SC_TEXEL, pos);
    bool published = slot != ~0u && cache.v[3u * slot + 2u].w > 0.5;
    float lam = slot == ~0u ? 1.0 : lighting_sc_lambda(slot);
    uint lamps = min(scene.lamp_count, LIGHTING_WORK_LAMP_CAP);
    bool fringe = false;
    for (uint i = 0u; i < lamps && !fringe; i++) {
        vec3 prev;
        vec3 now;
        if (!lighting_work_lamp_moved(i, prev, now)) {
            continue;
        }
        Lamp l = scene_lamp(i);
        if (l.pos.w > 0.5) {
            fringe = true;
            break;
        }
        float motion = length(now - prev);
        fringe = lighting_work_tube(pos, now, motion);
    }
    if (!fringe) {
        vec4 head = work.v[lighting_change_at()];
        if (head.y > 0.5) {
            fringe = true;
        } else {
            uint changes = min(uint(head.x + 0.5), LIGHTING_CHANGES);
            for (uint c = 0u; c < changes && !fringe; c++) {
                vec4 b = work.v[lighting_change_at() + LIGHTING_CHANGE_HEAD + c];
                fringe = length(pos - b.xyz) < b.w + 0.5;
            }
        }
    }
    if (fringe) {
        return 2u;
    }
    if (!published || lam >= LIGHTING_WORK_FULL) {
        return 1u;
    }
    return 3u;
}

shared uint work_spx;
shared uint work_sset;
shared uint work_sfr;
shared uint work_smiss;

// Every thread of the compose workgroup calls this. Tag 0 means no surface.
uint lighting_work_count_px(bool surface, vec3 pos, vec3 n) {
    if (gl_LocalInvocationIndex == 0u) {
        work_spx = 0u;
        work_sset = 0u;
        work_sfr = 0u;
        work_smiss = 0u;
    }
    barrier();
    uint tag = 0u;
    if (surface && lighting_work_counting()) {
        tag = lighting_work_px_tag(pos, n);
        atomicAdd(work_spx, 1u);
        if (tag == 3u) {
            atomicAdd(work_sset, 1u);
        } else if (tag == 2u) {
            atomicAdd(work_sfr, 1u);
        } else {
            atomicAdd(work_smiss, 1u);
        }
    }
    barrier();
    if (gl_LocalInvocationIndex == 0u && work_spx > 0u && lighting_work_counting()) {
        atomicAdd(gstats.work_pages, 1u);
        if (work_sset == work_spx) {
            atomicAdd(gstats.work_settled_pages, 1u);
        }
        atomicAdd(gstats.work_px, work_spx);
        atomicAdd(gstats.work_settled_px, work_sset);
        atomicAdd(gstats.work_fringe, work_sfr);
        atomicAdd(gstats.work_missing, work_smiss);
    }
    return tag;
}

// A small directional-sun step. Indirect light catches up over N frames.
// Direct sun light and its shadows are not on this path.
const uint LIGHTING_SUN_SMALL = 1u;
const uint LIGHTING_SUN_FOLLOWS = 2u;
const uint LIGHTING_SUN_PATCH = 4u;

uint lighting_sun_flags() {
    return ckeys.key[LIGHTING_WORK_AT + 1u];
}
bool lighting_sun_small() {
    return (lighting_sun_flags() & LIGHTING_SUN_SMALL) != 0u && !lighting_work_full();
}
bool lighting_sun_patch_follows() {
    return (lighting_sun_flags() & LIGHTING_SUN_FOLLOWS) != 0u;
}
bool lighting_sun_patch_now() {
    return (lighting_sun_flags() & LIGHTING_SUN_PATCH) != 0u;
}
uint lighting_sun_n() {
    return max((lighting_sun_flags() >> 8u) & 255u, 1u);
}
uint lighting_sun_slice_id() {
    return (lighting_sun_flags() >> 16u) & 255u;
}
vec3 lighting_sun_committed_dir() {
    return vec3(uintBitsToFloat(ckeys.key[LIGHTING_WORK_AT + 2u]), uintBitsToFloat(ckeys.key[LIGHTING_WORK_AT + 3u]), uintBitsToFloat(ckeys.key[LIGHTING_WORK_AT + 4u]));
}
float lighting_sun_committed_light() {
    return uintBitsToFloat(ckeys.key[LIGHTING_WORK_AT + 5u]);
}
float lighting_sun_live_light() {
    return uintBitsToFloat(ckeys.key[LIGHTING_WORK_AT + 6u]);
}
// One band of probe rows, or one band of texel pages, per frame.
bool lighting_sun_probe_slice(uint probe) {
    uint cols = max(pc.dims.z, 1u);
    return (probe / cols) % lighting_sun_n() == lighting_sun_slice_id();
}
bool lighting_sun_slot_slice(uint slot) {
    // Spatial page column. The hashed page id is scattered, and an a-trous
    // 5x5 of that set fills the field.
    ivec4 c = lighting_sc_cell(slot);
    uint page_x = uint(c.x) >> 3u;
    return page_x % lighting_sun_n() == lighting_sun_slice_id();
}
// A held probe outside this frame's slice keeps the light it already has.
bool lighting_sun_hold_kept(uint h) {
    if (!lighting_sun_small()) {
        return false;
    }
    uint rays = lighting_rays();
    uint pk = h / rays / LIGHTING_STRATA;
    if (lighting_sun_probe_slice(pk) || !lighting_stratum_held(pk)) {
        return false;
    }
    if (floatBitsToUint(work.v[lighting_probe_at(pk) + 1u].w) == lighting_frame() + LIGHTING_STRATA) {
        return false;
    }
    return floatBitsToUint(work.v[lighting_kept_light_at() + h / 4u][h % 4u]) != 0u;
}
bool lighting_sun_hold_fill(uint h) {
    if (!lighting_sun_small()) {
        return false;
    }
    uint pk = h / lighting_rays() / LIGHTING_STRATA;
    if (lighting_sun_probe_slice(pk)) {
        return false;
    }
    return floatBitsToUint(work.v[lighting_fill_light_at() + h / 4u][h % 4u]) != 0u;
}
