// Stages F2, F3, and F4a. The world cache is the only store. A cache round traces
// one floor ray for each live cell every LIGHTING_FILL_FLOOR_PERIOD frames,
// plus one ray every frame for a cell a mover is changing, plus fill_burst rays
// on the highest value cells. The mover rays come out of that same budget.
// Word 0 stays a positive enable; the plan writes the traced count. F4a writes a
// low-confidence seed into other cells of the same screen tile. The seed is not
// an own sample and it does not add a ray. Ring sharing is not this stage.
// The radii, the weight, the fold, and the share gates match crates/fill-ref.
//
// Param words, after the per-pixel seed payload:
// 0 enable (a positive fill_budget; not the ray count), 1 tile, 2 lod_near,
// 3 leash, 4 look_ahead, 5 cell_max, 6 far,
// 7 far_share, 8 offscreen_floor, 9 error_target, 10 prox_min, 11 change_weight,
// 12 warmup rays, 13 ray cap, 14 frame_dt, 15 base cell, 16 check_rate,
// 17 max_picks, 18 lambda_decay, 19 window_floor, 20 catchup_max, 21 check_sigma,
// 22 share (0 off, 1 tile), 23 min hit, 24 own cap, 25 plane, 26 normal,
// 27 weight cap, 28 cosine floor, 29 seed confidence, 30 receivers,
// 31 share handover, in own rays. 0 means the warmup count (word 12).
// 32 floor enable (0 off; any other value is one ray every 10 frames), 33 burst.

const float FILL_EPS = 1.0e-6;
const float FILL_HALF = 0.5;
const float FILL_TINY = 1.0e-4;
const float FILL_REAL_CAP = 1.0e20;
const float FILL_TWO = 2.0;
const float FILL_GOLDEN = 2.3999631;
const float FILL_VIEW_EPS = 0.05;
const float FILL_CELL_KEEP = 0.75;
const vec3 FILL_LUMA = vec3(0.2126, 0.7152, 0.0722);
const float FILL_PRI_SCALE = 32.0;
const float FILL_RAY_BIAS = 0.02;
const float FILL_SET_RAYS = 64.0;
const float FILL_VAR_SEED = 0.15;
const float FILL_VAR_EPS = 1.0e-8;
const float FILL_DEV_EPS = 1.0e-5;
const float FILL_FIRE_FLOOR = 0.1;
const float FILL_SHORT = 0.08;
const float FILL_VAR_RATE = 0.04;
const float FILL_OPEN = 0.02;
const float FILL_CATCH = 0.2;
const float FILL_K_DEN = 256.0;
const float FILL_VB_DEN = 32.0;
const float FILL_SUN_DEN = 16.0;
const float FILL_CHANGE_SCALE = 1.0e6;
// water::bucket: (log2(priority) + 16) / 18, then 62 steps above the zero bucket.
const float FILL_BUCKET_BIAS = 16.0;
const float FILL_BUCKET_SPAN = 18.0;
const float FILL_BUCKET_STEPS = 62.0;
// Commits a world cell still takes before it can lose the tile to a neighbour.
// One commit freezes the first direct-only sample; dark-fill needs the later
// ones, which read a cache that already holds a bounce.
const float FILL_T_EPS = 1.0e-3;
const float FILL_Q10 = 1023.0;
const float FILL_SHARE_COS0 = 0.05;
const vec3 FILL_MSME = vec3(0.299, 0.587, 0.114);

const uint LIGHTING_FILL_PARAMS = 34u;
const uint LIGHTING_FILL_TILE_CAP = 262144u;

uint lighting_fill_param_at() {
    return LIGHTING_SC_SEED_PAY_AT + pc.dims.x * pc.dims.y * 8u;
}
uint lighting_fill_lock_at() {
    return lighting_fill_param_at() + LIGHTING_FILL_PARAMS;
}
// Eight header words, then one word per off-screen bin, then three 64-bin
// histograms, then the water-fill plan (six 64-word rows), then three extra
// cursors (64 words each), then two hit words per cache ray and two link
// words per ray. The host sizes the same tail.
// Header: 0 floor-ray count, 1 burst-ray count, 2 check count, 3 check cursor,
// 4 candidate count, 5 traced-ray count, 6 planned near budget,
// 7 occupied-cell count.
const uint LIGHTING_FILL_OFF_CAP = 16384u;
const uint LIGHTING_FILL_HIST_WORDS = 192u;
const uint LIGHTING_FILL_PLAN_WORDS = 384u;
const uint LIGHTING_FILL_EXTRA_WORDS = 192u;
uint lighting_fill_header_at() {
    return lighting_fill_lock_at() + LIGHTING_CACHE_SLOTS;
}
uint lighting_fill_off_at() {
    return lighting_fill_header_at() + 8u;
}
uint lighting_fill_off_cap() {
    return LIGHTING_FILL_OFF_CAP;
}
uint lighting_fill_hist_at() {
    return lighting_fill_off_at() + LIGHTING_FILL_OFF_CAP;
}
uint lighting_fill_plan_at() {
    return lighting_fill_hist_at() + LIGHTING_FILL_HIST_WORDS;
}
uint lighting_fill_extra_at() {
    return lighting_fill_plan_at() + LIGHTING_FILL_PLAN_WORDS;
}
uint lighting_fill_hit_at() {
    return lighting_fill_extra_at() + LIGHTING_FILL_EXTRA_WORDS;
}
uint lighting_fill_rays_max() {
    return LIGHTING_CACHE_BATCH * LIGHTING_CACHE_RAYS;
}
uint lighting_fill_link_at() {
    return lighting_fill_hit_at() + lighting_fill_rays_max() * 2u;
}
// Tiles and picks sit past the link words. They used to share the young/mature
// list range, which aliases once the table is LIGHTING_FILL_TILE_CAP slots.
uint lighting_fill_tile_at() {
    return lighting_fill_link_at() + lighting_fill_rays_max() * 2u;
}
uint lighting_fill_pick_at() {
    return lighting_fill_tile_at() + LIGHTING_FILL_TILE_CAP;
}
uint lighting_fill_u(uint i) {
    return ckeys.key[lighting_fill_param_at() + i];
}
float lighting_fill_f(uint i) {
    return uintBitsToFloat(lighting_fill_u(i));
}
bool lighting_ray_fill_on() {
    return lighting_world_mode() == 3u && lighting_fill_u(0) > 0u;
}
// One floor ray per live cell every this many frames. A cell a mover is
// changing is the exception: it is due every frame, and that ray is counted
// out of the burst. 0 in the param word leaves the floor off.
const uint LIGHTING_FILL_FLOOR_PERIOD = 10u;
uint lighting_fill_rate() {
    if (lighting_fill_u(32) == 0u) {
        return 0u;
    }
    return LIGHTING_FILL_FLOOR_PERIOD;
}
// The plan writes this. Word 0 is only the enable.
uint lighting_fill_traced() {
    return ckeys.key[lighting_fill_header_at() + 5u];
}
uint lighting_fill_check_cap() {
    uint budget = lighting_fill_u(0);
    uint rate = max(lighting_fill_u(16), 1u);
    uint pick_cap = lighting_fill_rays_max();
    uint check_cap = budget / rate;
    if (check_cap == 0u && budget > 0u) {
        check_cap = 1u;
    }
    return min(check_cap, min(budget, pick_cap));
}
// Append a check ray. The cursor starts at 0 (the host clears the header).
// Past the cap the ray is dropped. The set under the cap does not depend on
// which thread arrives first.
void lighting_fill_push_check(uint slot) {
    if (slot >= LIGHTING_CACHE_SLOTS) {
        return;
    }
    uint i = atomicAdd(ckeys.key[lighting_fill_header_at() + 3u], 1u);
    if (i < lighting_fill_check_cap()) {
        ckeys.key[lighting_fill_pick_at() + i] = slot;
    }
}
uint lighting_fill_lod_max() {
    float ratio = lighting_fill_f(5) / max(lighting_fill_f(15), FILL_EPS);
    return uint(max(floor(log2(max(ratio, 1.0)) + FILL_HALF), 0.0));
}
uint lighting_fill_level(float d) {
    float near = lighting_fill_f(2);
    if (d <= near) {
        return 0u;
    }
    uint level = uint(ceil(log2(max(d / max(near, FILL_EPS), 1.0))));
    return min(level, lighting_fill_lod_max());
}
float lighting_fill_cell_size(uint level) {
    return min(lighting_fill_f(15) * exp2(float(level)), lighting_fill_f(5));
}
float lighting_fill_create(uint level) {
    // Both distances load on every call. The far load used to sit in the
    // coarsest-level branch. Lavapipe still executes that load when the
    // wave's mask is empty and faults on a null descriptor. The bounce
    // presented-frame gate does that: a level-0 leash is not the coarsest
    // level, so the branch is uniformly skipped.
    float far = lighting_fill_f(6);
    float grown = lighting_fill_f(2) * exp2(float(level));
    float at_cap = float(level >= lighting_fill_lod_max());
    return mix(grown, far, at_cap);
}
float lighting_fill_leash(uint level) {
    // Coarsest level included. fill_far alone would create and drop on a jitter.
    return lighting_fill_f(3) * lighting_fill_create(level);
}
uint lighting_fill_level_of_size(float size) {
    float base = lighting_fill_f(15);
    if (size <= base) {
        return 0u;
    }
    return min(uint(floor(log2(size / max(base, FILL_EPS)) + FILL_HALF)), lighting_fill_lod_max());
}
vec3 lighting_fill_fibonacci(uint i, uint n) {
    if (n == 0u) {
        return vec3(0.0, 1.0, 0.0);
    }
    float y = 1.0 - (float(i) + FILL_HALF) / float(n) * FILL_TWO;
    float r = sqrt(max(1.0 - y * y, 0.0));
    float phi = FILL_GOLDEN * float(i);
    return vec3(r * cos(phi), y, r * sin(phi));
}

void lighting_fill_free(uint slot) {
    atomicAdd(gstats.freed, 1u);
    ckeys.key[slot] = 0u;
    ckeys.key[lighting_fill_lock_at() + slot] = 0u;
    lighting_sc_forget(slot);
    cache.v[3u * slot + 1u] = vec4(0.0);
    cache.v[3u * slot + 2u] = vec4(0.0);
    // The next occupant must not compare against this surface.
    cache.v[LIGHTING_SC_GRAD_AT + slot] = vec4(0.0);
    cache.v[LIGHTING_SC_GRAD2_AT + slot] = vec4(0.0);
}

uint lighting_fill_seed(uint slot);
float lighting_fill_unit(float v);

// Confidence-weighted merge of one cell into a coarser one. `src_scale` is
// the source area over the destination area, so a child does not claim the
// whole parent. The destination's value rises by the size step. The source
// is freed by the caller.
void lighting_fill_merge_into(uint dst, uint src, float src_scale) {
    float scale = clamp(src_scale, 0.0, 1.0);
    vec4 d = cache.v[3u * dst + 2u];
    vec4 s = cache.v[3u * src + 2u];
    float wd = max(d.w, 0.0);
    float ws = max(s.w, 0.0) * scale;
    float w = wd + ws;
    if (!(w > 0.0)) {
        return;
    }
    vec3 mean = (d.rgb * wd + s.rgb * ws) / w;
    float ld = dot(max(d.rgb, vec3(0.0)), FILL_LUMA);
    float ls = dot(max(s.rgb, vec3(0.0)), FILL_LUMA);
    float lm = dot(max(mean, vec3(0.0)), FILL_LUMA);
    float vd = max(cache.v[LIGHTING_SC_EST_AT + 2u * dst].w, 0.0);
    float vs = max(cache.v[LIGHTING_SC_EST_AT + 2u * src].w, 0.0);
    float var = (wd * (vd + (ld - lm) * (ld - lm)) + ws * (vs + (ls - lm) * (ls - lm))) / w;
    cache.v[3u * dst + 2u] = vec4(mean, w);
    vec4 ea = cache.v[LIGHTING_SC_EST_AT + 2u * dst];
    if (!(wd > 0.0)) {
        ea.rgb = cache.v[LIGHTING_SC_EST_AT + 2u * src].rgb;
    }
    ea.w = var;
    cache.v[LIGHTING_SC_EST_AT + 2u * dst] = ea;
    vec4 eb = cache.v[LIGHTING_SC_EST_AT + 2u * dst + 1u];
    float rd = max(eb.w, 0.0);
    float rs = max(cache.v[LIGHTING_SC_EST_AT + 2u * src + 1u].w, 0.0) * scale;
    cache.v[LIGHTING_SC_EST_AT + 2u * dst + 1u] = vec4(eb.xyz, rd + rs);
    if (lighting_sh_on()) {
        vec3 c0;
        vec3 cx;
        vec3 cy;
        vec3 cz;
        vec3 s0;
        vec3 sx;
        vec3 sy;
        vec3 sz;
        lighting_sh_get(dst, c0, cx, cy, cz);
        lighting_sh_get(src, s0, sx, sy, sz);
        float inv = 1.0 / w;
        lighting_sh_put(dst, (c0 * wd + s0 * ws) * inv, (cx * wd + sx * ws) * inv, (cy * wd + sy * ws) * inv, (cz * wd + sz * ws) * inv);
    }
    vec4 g = cache.v[LIGHTING_SC_GRAD_AT + dst];
    uint want = lighting_fill_seed(dst);
    if (floatBitsToUint(g.z) != want) {
        g = vec4(0.0);
        g.z = uintBitsToFloat(want);
    }
    g.x = max(lighting_fill_unit(g.x), 1.0 - scale);
    cache.v[LIGHTING_SC_GRAD_AT + dst] = g;
}

// The cell is past the distance for its size. Merge it into the coarser cell
// that still covers the point, then drop it. A failed claim keeps it.
void lighting_fill_coarsen(uint slot, vec3 p, vec3 n, uint target) {
    float size = lighting_fill_cell_size(target);
    uint parent = lighting_cache_find_sized(p, n, true, size, p);
    if (parent == ~0u || parent == slot) {
        return;
    }
    atomicAdd(gstats.diag[LIGHTING_RESET_COARSEN], 1u);
    float child = cache.v[3u * slot + 1u].w;
    float scale = (child * child) / max(size * size, FILL_EPS);
    lighting_fill_merge_into(parent, slot, scale);
    lighting_fill_free(slot);
}

bool lighting_fill_pixel(vec3 p, out uint px, out uint py) {
    vec3 rel = p - scene.eye.xyz;
    float z = dot(rel, scene.view_forward.xyz);
    float t = scene.view_right.w;
    float aspect = scene.view_forward.w;
    if (z <= FILL_VIEW_EPS || t < FILL_EPS || aspect < FILL_EPS) {
        return false;
    }
    float ndc_x = dot(rel, scene.view_right.xyz) / (z * t * aspect);
    float ndc_y = dot(rel, scene.view_up.xyz) / (z * t);
    if (abs(ndc_x) > 1.0 || abs(ndc_y) > 1.0) {
        return false;
    }
    px = min(uint((ndc_x * FILL_HALF + FILL_HALF) * float(pc.dims.x)), pc.dims.x - 1u);
    py = min(uint((1.0 - (ndc_y * FILL_HALF + FILL_HALF)) * float(pc.dims.y)), pc.dims.y - 1u);
    return true;
}

// While a cell's shown count is under FILL_OPEN_SETS, a lower shown count
// outranks a higher one, so one slot id cannot pin the tile. After that the rank is the
// priority bucket. That bucket is the stored variance, so the cell does not
// drop to bucket 0 when the countdown ends.
const float FILL_OPEN_SETS = 48.0;
uint lighting_fill_pack(uint slot, uint q) {
    float shown = cache.v[3u * slot + 2u].w;
    uint open = shown < FILL_OPEN_SETS ? 1u : 0u;
    uint behind = shown >= FILL_OPEN_SETS ? 0u : min(uint(FILL_OPEN_SETS - shown), 63u);
    uint rank = open == 1u ? behind : (q & 63u);
    return (open << 27u) | (rank << 21u) | (slot + 1u);
}

uint lighting_fill_packed_q(uint w) {
    return (w >> 21u) & 63u;
}

uint lighting_fill_bucket(float pri) {
    if (!(pri > 0.0)) {
        return 0u;
    }
    float t = clamp((log2(max(pri, FILL_EPS)) + FILL_BUCKET_BIAS) / FILL_BUCKET_SPAN, 0.0, 1.0);
    return min(uint(floor(t * FILL_BUCKET_STEPS)) + 1u, 63u);
}

float lighting_fill_unit(float v) {
    return (v >= 0.0 && v <= 1.0) ? v : 0.0;
}

// Infinity compares equal to itself, so `x == x` does not reject it.
bool lighting_fill_real(float v) {
    return v == v && abs(v) <= FILL_REAL_CAP;
}

uint lighting_fill_seed(uint slot) {
    return max(lighting_hash(ckeys.key[slot] ^ 0x00C3E7u), 1u);
}

// The lamp sample of a check ray. The budget index and the frame are not in it.
float lighting_fill_check_u(uint slot) {
    uint key = slot < LIGHTING_CACHE_SLOTS ? ckeys.key[slot] : 0u;
    uint seed = lighting_fill_seed(slot);
    return lighting_unit(key ^ (seed * 0x85ebca6bu + 0x5bd1e995u));
}

// Rank is closeness times remaining error. On screen, off screen, and
// occluded share that rank. The floor ray covers every live cell.
uint lighting_fill_q(uint slot, float d) {
    vec4 shown_v = cache.v[3u * slot + 2u];
    vec4 est = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
    float rays = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u].w;
    float mean = max(dot(shown_v.rgb, FILL_LUMA), FILL_TINY);
    float dev = sqrt(max(est.w, 0.0));
    float lam = lighting_fill_unit(cache.v[LIGHTING_SC_GRAD_AT + slot].x);
    float err = rays < FILL_HALF ? 1.0 : max(0.0, dev / mean - lighting_fill_f(9));
    float close = 1.0 / max(d, lighting_fill_f(10));
    float pri = close * (err + lighting_fill_f(11) * lam);
    return lighting_fill_bucket(pri);
}

void lighting_fill_decay(uint slot) {
    uint want = lighting_fill_seed(slot);
    vec4 g = cache.v[LIGHTING_SC_GRAD_AT + slot];
    // A reused slot still holds the previous occupant's seed. Drop it before
    // priority, so the new surface starts with no lambda and no stored luma.
    if (floatBitsToUint(g.z) != want) {
        atomicAdd(gstats.diag[LIGHTING_RESET_DECAY], 1u);
        g = vec4(0.0);
        g.z = uintBitsToFloat(want);
        cache.v[LIGHTING_SC_GRAD2_AT + slot].z = 0.0;
    }
    g.x = lighting_fill_unit(g.x) * clamp(lighting_fill_f(18), 0.0, 1.0);
    cache.v[LIGHTING_SC_GRAD_AT + slot] = g;
}

// Drop a clipmap cell past its leash. A stored cell inside the leash is kept,
// so the create edge does not thrash. Past the leash, coarsen, then evict.
// A 12.5 cm texel claims the 25 cm parent and still takes its own floor ray.
void lighting_fill_visit(uint slot) {
    vec3 p;
    vec3 n;
    if (!lighting_sc_texel_world(slot, p, n)) {
        return;
    }
    float size = cache.v[3u * slot + 1u].w;
    float base = lighting_fill_f(15);
    lighting_fill_decay(slot);
    // Size follows distance from the camera. Behind the camera, off screen,
    // and occluded use the same leash. A cell inside it stays, including one
    // the view has left.
    float d = length(p - scene.eye.xyz);
    if (size >= base * FILL_CELL_KEEP) {
        uint level = lighting_fill_level_of_size(size);
        float keep = lighting_fill_leash(level);
        if (d > keep) {
            uint target = level;
            uint cap = lighting_fill_lod_max();
            while (target < cap && d > lighting_fill_leash(target)) {
                target++;
            }
            if (target > level && d <= lighting_fill_leash(target)) {
                lighting_fill_coarsen(slot, p, n, target);
            } else if (d > lighting_fill_leash(target)) {
                atomicAdd(gstats.diag[LIGHTING_RESET_LEASH], 1u);
                lighting_fill_free(slot);
            }
            return;
        }
    } else {
        float drop = lighting_fine_word(LIGHTING_FINE_DROP_AT);
        if (drop > 0.0 && d > drop) {
            lighting_fill_coarsen(slot, p, n, 0u);
            return;
        }
    }
    // Compose reads the 12.5 cm texel (world mode 3's lighting_cell_size).
    // The 25 cm parent is the SH prior, so the texel still claims it. The
    // texel takes its own floor ray: compose shows that texel, and a per-cell
    // floor does not steal the parent's ray.
    if (size >= base) {
        lighting_fine_spawn(slot);
    }
    // The 12.5 cm texel does not claim the 25 cm parent. That claim recreated
    // cells the leash had just coarsened, so the same surfaces were wiped
    // every frame. Compose reads the texel, which takes its own floor ray.
    if (size <= LIGHTING_WRC_CELL_SIZE && lighting_compact_hold_texel_rays(slot)) {
        return;
    }
    uint q = lighting_fill_q(slot, d);
    uint rate = lighting_fill_rate();
    if (q > 0u) {
        atomicAdd(ckeys.key[lighting_fill_hist_at() + q], 1u);
        uint packed = ((q & 63u) << 21u) | (slot + 1u);
        uint c = atomicAdd(ckeys.key[lighting_fill_header_at() + 4u], 1u);
        if (c < LIGHTING_FILL_TILE_CAP) {
            ckeys.key[lighting_fill_tile_at() + c] = packed;
        }
    }
    // Slot index, so the same cell is due again every period. A cell a mover
    // is changing is due every frame: that sample is what keeps or down-weights
    // the stored mean. Header word 6 counts those mover rays.
    bool mover_due = lighting_near_change(p);
    if (rate != 0u && (mover_due || (slot % rate) == (lighting_round() % rate))) {
        uint i = atomicAdd(ckeys.key[lighting_fill_header_at()], 1u);
        if (i < lighting_fill_rays_max()) {
            ckeys.key[lighting_fill_pick_at() + i] = slot;
            if (mover_due) {
                atomicAdd(ckeys.key[lighting_fill_header_at() + 6u], 1u);
            }
        }
    }
}

void lighting_fill_plan(uint hist[64], uint share, uint max_picks, inout uint base_m[64], inout uint extra_m[64], out uint take) {
    take = 0u;
    uint remaining = share;
    for (uint i = 0u; i < 64u; i++) {
        base_m[i] = 0u;
        extra_m[i] = 0u;
    }
    for (int level = 63; level >= 1; level--) {
        if (remaining == 0u) {
            break;
        }
        uint count = hist[uint(level)];
        if (count == 0u) {
            continue;
        }
        uint room = remaining / count;
        uint base = min(room, max_picks);
        uint used = base * count;
        if (used > remaining) {
            used = remaining;
        }
        uint leftover = remaining - used;
        uint extra_room = (max_picks - base) * count;
        uint extra = leftover;
        if (extra > extra_room) {
            extra = extra_room;
        }
        base_m[uint(level)] = base;
        extra_m[uint(level)] = extra;
        uint got = used + extra;
        if (got > remaining) {
            got = remaining;
        }
        remaining -= got;
        take += got;
    }
}

// The fair pass stops at max_picks. Rays still in the budget are more samples
// on those cells, up to one due-list entry (16), not view primaries.
void lighting_fill_pour_row(uint hist[64], uint at, uint cap, inout uint remaining) {
    for (int level = 63; level >= 1; level--) {
        if (remaining == 0u) {
            break;
        }
        uint count = hist[uint(level)];
        if (count == 0u) {
            continue;
        }
        uint have = ckeys.key[at + uint(level)];
        if (have >= cap) {
            continue;
        }
        uint room = cap - have;
        uint give = remaining / count;
        if (give > room) {
            give = room;
        }
        if (give == 0u) {
            continue;
        }
        ckeys.key[at + uint(level)] = have + give;
        remaining -= give * count;
    }
}

float lighting_fill_pace(uint slot, float calm, float k, float rays0, float n) {
    float lam = lighting_fill_unit(cache.v[LIGHTING_SC_GRAD_AT + slot].x);
    // Lambda 0 keeps the caller's weight: the calm n/(rays+n), or the MSME k.
    if (!(lam > 0.0)) {
        return k;
    }
    float wfloor = clamp(lighting_fill_f(19), 0.0, 1.0);
    float scale = 1.0 - lam * (1.0 - wfloor);
    float fast = n / max(rays0 * max(scale, 0.0) + n, FILL_EPS);
    float cap = lighting_fill_f(20);
    if (cap > 0.0) {
        fast = min(fast, cap);
    }
    float bound = max(max(cap, 0.0), calm);
    float target = min(max(calm, fast), bound);
    return max(k, mix(calm, target, lam));
}

void lighting_fill_note_check(uint slot, float l_now) {
    vec4 g = cache.v[LIGHTING_SC_GRAD_AT + slot];
    uint want = lighting_fill_seed(slot);
    if (floatBitsToUint(g.z) != want) {
        g = vec4(0.0);
        g.z = uintBitsToFloat(want);
        cache.v[LIGHTING_SC_GRAD2_AT + slot].z = 0.0;
    }
    bool have = cache.v[LIGHTING_SC_GRAD2_AT + slot].z > FILL_HALF && lighting_fill_real(g.y);
    float lam = lighting_fill_unit(g.x);
    // A non-finite sample must not stick. The decayed lambda stays, and the
    // stored luma stays only when it is a real number. min(1, NaN) is 1, so an
    // infinite luma would raise lambda and then stick for the next check.
    if (!lighting_fill_real(l_now)) {
        if (!lighting_fill_real(g.y)) {
            g.y = 0.0;
            have = false;
        }
        g.x = lam;
        cache.v[LIGHTING_SC_GRAD_AT + slot] = g;
        cache.v[LIGHTING_SC_GRAD2_AT + slot].z = have ? 1.0 : 0.0;
        return;
    }
    if (have) {
        float denom = max(max(abs(l_now), abs(g.y)), FILL_TINY);
        float d = min(1.0, abs(l_now - g.y) / denom);
        lam = lighting_fill_unit(d - FILL_TWO * max(lighting_fill_f(21), 0.0));
    }
    g.x = lam;
    g.y = l_now;
    g.z = uintBitsToFloat(want);
    cache.v[LIGHTING_SC_GRAD_AT + slot] = g;
    cache.v[LIGHTING_SC_GRAD2_AT + slot].z = 1.0;
}

shared uint lighting_fill_scan[64];
shared uint lighting_fill_wg_start;

void lighting_fill_tile_grid(out uint tiles, out uint tiles_x) {
    uint tile = max(lighting_fill_u(1), 1u);
    tiles_x = (pc.dims.x + tile - 1u) / tile;
    uint tiles_y = (pc.dims.y + tile - 1u) / tile;
    tiles = min(tiles_x * tiles_y, LIGHTING_FILL_TILE_CAP);
}

// Inclusive workgroup scan. One atomic per group reserves the output range.
// Every thread in the group calls this; the barriers are the scan.
uint lighting_fill_wg_place(uint m, uint cursor) {
    uint local = gl_LocalInvocationID.x;
    lighting_fill_scan[local] = m;
    barrier();
    for (uint offset = 1u; offset < 64u; offset <<= 1u) {
        uint v = lighting_fill_scan[local];
        if (local >= offset) {
            v += lighting_fill_scan[local - offset];
        }
        barrier();
        lighting_fill_scan[local] = v;
        barrier();
    }
    if (local == 63u) {
        lighting_fill_wg_start = atomicAdd(ckeys.key[cursor], lighting_fill_scan[63]);
    }
    barrier();
    if (m == 0u) {
        return 0u;
    }
    return lighting_fill_wg_start + lighting_fill_scan[local] - m;
}

// True when this slot won its screen tile. That emit is its grant for the
// round, so the off-screen histogram must not plan a second one.
bool lighting_fill_holds_tile(uint slot) {
    vec3 p;
    vec3 n;
    if (!lighting_sc_texel_world(slot, p, n)) {
        return false;
    }
    uint px;
    uint py;
    if (!lighting_fill_pixel(p, px, py)) {
        return false;
    }
    uint tile = max(lighting_fill_u(1), 1u);
    uint tiles_x = (pc.dims.x + tile - 1u) / tile;
    uint ti = px / tile + (py / tile) * tiles_x;
    if (ti >= LIGHTING_FILL_TILE_CAP) {
        return false;
    }
    uint w = ckeys.key[lighting_fill_tile_at() + ti];
    return w != 0u && (w & 2097151u) == slot + 1u;
}

// The visit already counted value buckets. This dispatch only publishes its
// width. The fill harness reads that width; it is not a tile contest.
void lighting_fill_hist(uint gid) {
    if (gid == 0u) {
        gstats.pad3 = gl_NumWorkGroups.x;
    }
}

void lighting_fill_store_picks(uint m, uint slot, uint dest) {
    for (uint k = 0u; k < 64u; k++) {
        if (k >= m) {
            break;
        }
        ckeys.key[lighting_fill_pick_at() + dest + k] = slot;
    }
}

// One thread per candidate the visit listed. prior >= max_picks is the burst
// cap: the floor ray is already stored, and this pass adds only the burst.
void lighting_fill_emit_on(uint gid) {
    // prior >= max_picks is the per-cell burst cap. The floor ray is separate.
    if (!lighting_ray_fill_on()) {
        return;
    }
    uint n = ckeys.key[lighting_fill_header_at() + 4u];
    uint m = 0u;
    uint slot = 0u;
    if (gid < n && gid < LIGHTING_FILL_TILE_CAP) {
        uint w = ckeys.key[lighting_fill_tile_at() + gid];
        uint q = w >> 21u;
        slot = (w & 2097151u) - 1u;
        uint max_picks = clamp(lighting_fill_u(17), 1u, 64u);
        if (q > 0u && slot < LIGHTING_CACHE_SLOTS && ckeys.key[slot] != 0u) {
            uint base = ckeys.key[lighting_fill_plan_at() + q];
            uint extra = ckeys.key[lighting_fill_plan_at() + 64u + q];
            uint want = base;
            if (want > max_picks) {
                want = max_picks;
            }
            if (extra > 0u && want < max_picks) {
                uint ticket = atomicAdd(ckeys.key[lighting_fill_extra_at() + q], 1u);
                if (ticket < extra) {
                    want += 1u;
                }
            }
            m = want;
        }
    }
    uint place = lighting_fill_wg_place(m, lighting_fill_header_at() + 1u);
    if (m == 0u) {
        return;
    }
    uint dest = ckeys.key[lighting_fill_header_at()] + place;
    uint room = lighting_fill_rays_max();
    if (dest >= room) {
        return;
    }
    if (dest + m > room) {
        m = room - dest;
    }
    lighting_fill_store_picks(m, slot, dest);
}

// The off-screen bin contest is gone. Burst picks are lighting_fill_emit_on.
void lighting_fill_emit_off(uint gid) {
    if (gid == ~0u) {
        return;
    }
}

void lighting_fill_dispatch() {
    // One thread. The value histogram is already in memory. Emitting the burst
    // is a later parallel dispatch. This function does not walk tiles or slots.
    uint max_picks = clamp(lighting_fill_u(17), 1u, 64u);
    uint burst = lighting_fill_u(33);
    // Mover rays are already in the floor count. The part past the 1/10 share
    // comes out of the burst, so the round stays inside the device budget.
    uint mover_rays = ckeys.key[lighting_fill_header_at() + 6u];
    uint mover_extra = mover_rays - mover_rays / LIGHTING_FILL_FLOOR_PERIOD;
    if (burst > mover_extra) {
        burst -= mover_extra;
    } else {
        burst = 0u;
    }
    gstats.diag[LIGHTING_DIAG_MOVER_RAYS] = mover_rays;
    uint hist[64];
    uint serial = 0u;
    uint hist_at = lighting_fill_hist_at();
    for (uint i = 0u; i < 64u; i++) {
        hist[i] = ckeys.key[hist_at + i];
        serial += 1u;
    }
    uint base_m[64];
    uint extra_m[64];
    uint burst_take = 0u;
    lighting_fill_plan(hist, burst, max_picks, base_m, extra_m, burst_take);
    uint plan = lighting_fill_plan_at();
    for (uint i = 0u; i < 64u; i++) {
        ckeys.key[plan + i] = base_m[i];
        ckeys.key[plan + 64u + i] = extra_m[i];
        serial += 2u;
    }
    uint floor_n = ckeys.key[lighting_fill_header_at()];
    uint cap = lighting_fill_rays_max();
    if (floor_n > cap) {
        floor_n = cap;
    }
    ckeys.key[lighting_fill_header_at()] = floor_n;
    uint room = cap - floor_n;
    if (burst_take > room) {
        burst_take = room;
    }
    uint total = floor_n + burst_take;
    ckeys.key[lighting_fill_header_at() + 2u] = 0u;
    ckeys.key[lighting_fill_header_at() + 5u] = total;
    ckeys.key[LIGHTING_ARGS_AT + 0u] = max((total + 63u) / 64u, 1u);
    ckeys.key[LIGHTING_ARGS_AT + 1u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 2u] = 1u;
    // The resolve used to be one group of 64, striding the whole ray list.
    // That is the cache-column cost. One thread per traced ray, same count
    // as the trace. The six phases are separate dispatches.
    ckeys.key[LIGHTING_ARGS_AT + 3u] = max((total + 63u) / 64u, 1u);
    ckeys.key[LIGHTING_ARGS_AT + 4u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 5u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 6u] = total;
    ckeys.key[LIGHTING_ARGS_AT + 7u] = total;
    ckeys.key[LIGHTING_ARGS_AT + 8u] = total;
    ckeys.key[LIGHTING_ARGS_AT + 9u] = total;
    ckeys.key[LIGHTING_ARGS_AT + 10u] = total;
    gstats.rays = 0u;
    gstats.pad2 = serial;
    gstats.due = floor_n;
    gstats.scouts = burst_take;
    gstats.round_entries = (total + 15u) / 16u;
}

bool lighting_fill_ray(uint ray, out vec3 origin, out vec3 dir, out vec3 n) {
    origin = vec3(0.0);
    dir = vec3(0.0, 1.0, 0.0);
    n = dir;
    if (ray < lighting_probe_rays()) {
        return false;
    }
    uint r = ray - lighting_probe_rays();
    uint budget = lighting_fill_traced();
    if (r >= budget) {
        return false;
    }
    uint checks = ckeys.key[lighting_fill_header_at() + 2u];
    uint on = ckeys.key[lighting_fill_header_at()];
    uint ranked = ckeys.key[lighting_fill_header_at() + 1u];
    uint placed = min(checks + on + ranked, budget);
    if (r >= placed) {
        return false;
    }
    uint slot = ckeys.key[lighting_fill_pick_at() + r];
    if (slot >= LIGHTING_CACHE_SLOTS || ckeys.key[slot] == 0u) {
        return false;
    }
    vec3 p;
    if (!lighting_sc_texel_world(slot, p, n)) {
        return false;
    }
    origin = p + n * FILL_RAY_BIAS;
    if (r < checks) {
        // Same seed every frame. The round is not part of the direction.
        uint seed = floatBitsToUint(cache.v[LIGHTING_SC_GRAD_AT + slot].z);
        if (seed == 0u) {
            seed = max(lighting_hash(ckeys.key[slot] ^ 0x00C3E7u), 1u);
        }
        dir = lighting_dir(seed, n, seed % 64u, 64u);
    } else {
        uint rk = lighting_sc_ray_key(slot, ckeys.key[slot]);
        uint stratum = (lighting_hash(rk) + lighting_round() + r) % 64u;
        dir = lighting_dir(rk, n, stratum, 64u);
    }
    return true;
}

// Trace writes the hit here. Direct then stores radiance in the hit slot.
void lighting_fill_stash(uint ray, float t, uint packed_n) {
    if (!lighting_ray_fill_on() || ray < lighting_probe_rays()) {
        return;
    }
    uint r = ray - lighting_probe_rays();
    if (r >= lighting_fill_traced()) {
        return;
    }
    ckeys.key[lighting_fill_hit_at() + r * 2u] = floatBitsToUint(t);
    ckeys.key[lighting_fill_hit_at() + r * 2u + 1u] = packed_n;
}

// World cells skip the texel MSME fold. Priority still reads est.w, so a cell
// with no stored variance has error 0 after its first ray and leaves the
// histogram. This keeps a short variance on the cell until the mean settles.
void lighting_fill_note_var(uint slot, vec3 e) {
    const vec3 msmel = vec3(0.299, 0.587, 0.114);
    vec4 a = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
    float luma = max(dot(e, msmel), FILL_TINY);
    if (!(a.w > FILL_VAR_EPS)) {
        float seed = FILL_VAR_SEED * luma;
        a = vec4(e, seed * seed + FILL_VAR_EPS);
    } else {
        vec3 sm = mix(a.rgb, e, FILL_SHORT);
        float open_m = dot((e - a.rgb) * (e - sm), msmel);
        float var = mix(a.w, max(open_m, 0.0), FILL_VAR_RATE);
        a = vec4(sm, max(var, FILL_VAR_EPS));
    }
    cache.v[LIGHTING_SC_EST_AT + 2u * slot] = a;
}

// Own rays at which a fill seed's display weight is 0. A zero word is the
// warmup count. This is not the 16-set handover.
uint lighting_fill_seed_drop() {
    uint drop = lighting_fill_u(31);
    if (drop == 0u) {
        drop = max(lighting_fill_u(12), 1u);
    }
    return max(drop, 1u);
}

bool lighting_fill_seed_due(uint slot) {
    if (slot >= LIGHTING_CACHE_SLOTS) {
        return false;
    }
    float rays = max(cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u].w, 0.0);
    return rays >= float(lighting_fill_seed_drop());
}

// Clear a fill seed once the cell has enough of its own rays. Other seeds
// are not marked, so their 16-set fade stays. Runs in this commit, which
// already owns the param words.
void lighting_fill_seed_spent(uint slot) {
    if (!lighting_sc_fill_marked(cache.v[LIGHTING_SC_SPARSE_AT + slot].z)) {
        return;
    }
    if (lighting_fill_seed_due(slot)) {
        lighting_sc_clear_seed(slot);
    }
}

// `e` is the mean of this frame's n samples (each `lk * cos * TAU`). One writer
// calls this, after every ray into the slot has been summed.
void lighting_fill_commit(uint slot, vec3 e, float n, vec3 lk_mean, vec3 shx, vec3 shy, vec3 shz, bool on_screen, float sun_luma) {
    if (slot >= LIGHTING_CACHE_SLOTS || ckeys.key[slot] == 0u || n <= 0.0) {
        return;
    }
    vec4 old = cache.v[3u * slot + 2u];
    // World mode 3 returns from the cache pass before its own note, on both
    // settings of the fill budget. This commit is the writer that stored the
    // sample. The raw sample against the stored mean, not the blended mean.
    lighting_reach_note_cell(slot, old.rgb, e);
    vec4 b = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u];
    float rays0 = max(b.w, 0.0);
    uint warmup = max(lighting_fill_u(12), 1u);
    float calm = n / (rays0 + n);
    float k = lighting_fill_pace(slot, calm, calm, rays0, n);
    bool cell = cache.v[3u * slot + 1u].w > LIGHTING_WRC_CELL_SIZE;
    // A world cell's deposit is one cache sample, the same unit a due-list
    // relight adds (one 16-ray entry). The blend is that pass's dark-fill
    // weight. rays/64 never reached a trusted set count, so the picture stayed
    // on the probe.
    float sample_t = old.w < FILL_HALF ? 0.0 : lighting_sc_count(slot);
    float sample_add = n > 0.0 ? max(floor(n / float(LIGHTING_CACHE_RAYS)), 1.0) : 0.0;
    float blend = clamp(k, 0.0, 1.0);
    bool accept = true;
    if (cell && n > 0.0) {
        blend = sample_add / max(sample_t + sample_add, FILL_EPS);
        if (lighting_sc_dark_fill() && sample_t < FILL_HALF * LIGHTING_WRC_CAP) {
            float ratio = sample_t / max(sample_t + sample_add, FILL_EPS);
            blend = 1.0 - ratio * ratio * ratio;
        }
        // A high value (the cell wants rays) raises the new sample's weight.
        // A lamp note, a 1 mm move, and the 1/8 and 1/32 floors do not.
        float value = lighting_history_value(slot, old.rgb, e);
        // -1 is an invalid sample. It does not lower the value and does not blend.
        accept = value >= 0.0;
        blend = accept ? clamp(max(blend, value), 0.0, 1.0) : 0.0;
    }
    vec3 mean = old.rgb;
    if (!cell && (lighting_sc_flags() & 1048576u) != 0u && lighting_sc_msme() && rays0 >= float(warmup) && old.w > FILL_HALF) {
        const vec3 msmel = vec3(0.299, 0.587, 0.114);
        vec4 a = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
        if (b.x <= 0.0) {
            a = vec4(old.rgb, pow(FILL_VAR_SEED * dot(old.rgb, msmel), FILL_TWO) + FILL_VAR_EPS);
            b = vec4(1.0, 1.0, 0.0, rays0);
        }
        float dev0 = sqrt(max(FILL_DEV_EPS, a.w));
        float sigma = lighting_sc_firefly_sigma();
        vec3 y = e - max(e - (FILL_FIRE_FLOOR + a.rgb + sigma * dev0), vec3(0.0));
        float sb = FILL_SHORT;
        float vr = FILL_VAR_RATE;
        vec3 sm = mix(a.rgb, e, sb);
        float open_m = dot((e - a.rgb) * (e - sm), msmel);
        vec3 ys = mix(a.rgb, y, sb);
        float clip_m = dot((y - a.rgb) * (y - ys), msmel);
        float var = mix(a.w, max(open_m, max(clip_m, 0.0)), vr);
        float dev = sqrt(max(FILL_DEV_EPS, var));
        float rho = dot(abs(old.rgb - sm), msmel) / max(FILL_DEV_EPS, dev);
        float inc = mix(b.y, rho, FILL_SHORT);
        k = clamp(smoothstep(0.0, 1.0, rho * max(FILL_RAY_BIAS, inc - FILL_CATCH)), 1.0 / FILL_K_DEN, 1.0) * b.x;
        float vb = mix(b.x, clamp(dot(msmel, FILL_HALF * sm / vec3(max(FILL_DEV_EPS, dev))), 1.0 / FILL_PRI_SCALE, 1.0), FILL_FIRE_FLOOR);
        k = max(k, 1.0 / FILL_K_DEN);
        k = lighting_fill_pace(slot, calm, k, rays0, n);
        mean = mix(old.rgb, e, clamp(k, 0.0, 1.0));
        cache.v[LIGHTING_SC_EST_AT + 2u * slot] = vec4(sm, var);
        b = vec4(vb, inc, k, b.w);
    } else if (cell) {
        mean = (accept && old.w < FILL_HALF) ? e : mix(old.rgb, e, blend);
        // The texel MSME fold above is `!cell`. A world cell still has to
        // store short variance or its priority falls to 0 after one ray.
        // An invalid sample does not.
        if (accept) {
            lighting_fill_note_var(slot, e);
        }
    } else {
        mean = old.w < FILL_HALF ? e : mix(old.rgb, e, clamp(k, 0.0, 1.0));
    }
    float cap = float(lighting_fill_u(13));
    float rays_after = min(rays0 + n, max(cap, float(warmup)));
    cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u] = vec4(b.xyz, rays_after);
    if (cell && lighting_sh_on() && (accept || old.w >= LIGHTING_CACHE_HAS_W)) {
        float wr = LIGHTING_TAU;
        vec3 n0 = lk_mean * (wr * LIGHTING_SH_Y0);
        vec3 nx = shx * (wr * LIGHTING_SH_Y1);
        vec3 ny = shy * (wr * LIGHTING_SH_Y1);
        vec3 nz = shz * (wr * LIGHTING_SH_Y1);
        if (old.w >= LIGHTING_CACHE_HAS_W) {
            vec3 o0;
            vec3 ox;
            vec3 oy;
            vec3 oz;
            lighting_sh_get(slot, o0, ox, oy, oz);
            float a = cell ? blend : clamp(k, 0.0, 1.0);
            n0 = mix(o0, n0, a);
            nx = mix(ox, nx, a);
            ny = mix(oy, ny, a);
            nz = mix(oz, nz, a);
        }
        lighting_sh_put(slot, n0, nx, ny, nz);
    }
    float el = dot(lk_mean, FILL_LUMA);
    float fe = el > FILL_EPS ? clamp(sun_luma / el, 0.0, 1.0) : 0.0;
    float fo = old.w > FILL_HALF ? lighting_sc_sunfrac_now(slot) : fe;
    float fnew = mix(fo, fe, max(k, 1.0 / FILL_SUN_DEN));
    float now = lighting_ls_now();
    cache.v[LIGHTING_SC_SUN_AT + slot] = vec4(fnew, now < 0.0 ? cache.v[LIGHTING_SC_SUN_AT + slot].y : now, 0.0, 0.0);
    cache.v[3u * slot].w = uintBitsToFloat(max(lighting_frame(), 1u));
    cache.v[LIGHTING_SC_OBJ_AT + slot].y = uintBitsToFloat(lighting_frame());
    if ((lighting_toggles() & LIGHTING_TOGGLE_FREEZE_CACHE) == 0u) {
        float shown_w = min(sample_t + (accept ? sample_add : 0.0), lighting_sc_cap(slot));
        vec4 shown = vec4(mean, shown_w);
        if (gstats.chg[31] != 0u) {
            lighting_chg(CHG_TEX_WRITE, 1u);
            lighting_chg(CHG_TEX_D0 + lighting_chg_bin(old.rgb, shown.rgb), 1u);
        }
        cache.v[3u * slot + 2u] = shown;
        if (slot < LIGHTING_CACHE_SLOTS && cache.v[3u * slot + 1u].w > LIGHTING_HIT_PATCH
            && lighting_pack_rgb9e5(old.rgb) != lighting_pack_rgb9e5(shown.rgb)) {
            lighting_compose_mark_slot(slot);
        }
    }
    // Off-screen deposits do not hold the picture open. On-screen cells still
    // inside the warm-up do. After that, the blended step is the change.
    if (on_screen && rays_after < float(warmup)) {
        atomicMax(gstats.max_change, 1000000u);
    } else if (on_screen && rays0 >= float(warmup)) {
        const vec3 lw = FILL_LUMA;
        float lo = dot(old.rgb, lw);
        float ln = dot(mean, lw);
        float change = abs(ln - lo) / (max(lo, ln) + FILL_VIEW_EPS);
        atomicMax(gstats.max_change, uint(clamp(change, 0.0, 1.0) * FILL_CHANGE_SCALE));
    }
    atomicAdd(gstats.relit, 1u);
    lighting_fill_seed_spent(slot);
}

// Newest lamp-change frame that still reaches p, or 0. Same reach test as
// lighting_lamp_delta. `updated` is the mean commit (`lighting_sc_updated`),
// not the visit word: a bounce read refreshes that word without a new mean.
// A share whose frame is not after this stamp stays home.
uint lighting_fill_stamp(vec3 p, uint updated) {
    uint stamp = 0u;
    [[dont_unroll]] for (uint i = 0u; i < LIGHTING_WRC_RING; i++) {
        uint at = LIGHTING_WRC_RING_AT + LIGHTING_WRC_RING_W * i;
        uint f = ckeys.key[at + 8u];
        if (f == 0u || int(lighting_frame() - f) > int(LIGHTING_WRC_CHANGE_FRAMES) || int(updated - f) >= 0) {
            continue;
        }
        vec3 a = vec3(uintBitsToFloat(ckeys.key[at]), uintBitsToFloat(ckeys.key[at + 1u]), uintBitsToFloat(ckeys.key[at + 2u]));
        vec3 b = vec3(uintBitsToFloat(ckeys.key[at + 3u]), uintBitsToFloat(ckeys.key[at + 4u]), uintBitsToFloat(ckeys.key[at + 5u]));
        float range = uintBitsToFloat(ckeys.key[at + 6u]);
        if (min(length(p - a), length(p - b)) >= range) {
            continue;
        }
        stamp = max(stamp, f);
    }
    return stamp;
}

float lighting_fill_view_depth(vec3 p) {
    return max(dot(p - scene.eye.xyz, scene.view_forward.xyz), FILL_T_EPS);
}

float lighting_fill_share_cos(float v) {
    float floor_c = lighting_fill_f(28);
    if (!(floor_c > 0.0)) {
        floor_c = FILL_SHARE_COS0;
    }
    return v <= floor_c ? floor_c : v;
}

// Reconnection weight, crates/fill-ref/src/share.rs jacobian. 0 rejects it.
float lighting_fill_share_jacobian(vec3 origin, vec3 n_o, vec3 hit, vec3 n_h, float t, vec3 recv, vec3 n_j) {
    vec3 to_o = hit - origin;
    vec3 to_j = hit - recv;
    float len_o = length(to_o);
    float len_j = length(to_j);
    if (len_o <= FILL_EPS || len_j <= FILL_EPS) {
        return 0.0;
    }
    vec3 dir_o = to_o / len_o;
    vec3 dir_j = to_j / len_j;
    float cos_theta_o = lighting_fill_share_cos(dot(n_o, dir_o));
    float cos_theta_j = lighting_fill_share_cos(dot(n_j, dir_j));
    float cos_phi_o = lighting_fill_share_cos(dot(n_h, -dir_o));
    float cos_phi_j = lighting_fill_share_cos(dot(n_h, -dir_j));
    float t_hit = max(t, FILL_T_EPS);
    float t2 = t_hit * t_hit;
    float d2 = len_j * len_j;
    return (cos_theta_j / cos_theta_o) * ((cos_phi_j / d2) / (cos_phi_o / t2));
}

// Own-count expiry. 0 means the cell takes no more seeds. The scale is trust,
// not a darkening of the stored light.
float lighting_fill_share_scale(uint recv) {
    float rays = max(cache.v[LIGHTING_SC_EST_AT + 2u * recv + 1u].w, 0.0);
    float own_cap = float(max(lighting_fill_u(24), 1u));
    return max(0.0, 1.0 - rays / own_cap);
}

// All five gates. A miss returns 0. The value is the clamped reconnection,
// before the own-count scale. That scale multiplies the seed confidence.
float lighting_fill_share_accept(uint origin_slot, uint recv, vec3 origin_p, vec3 n_o, vec3 hit, vec3 n_h, float t, vec3 recv_p, vec3 n_j) {
    if (recv == ~0u || recv == origin_slot || recv >= LIGHTING_CACHE_SLOTS || ckeys.key[recv] == 0u) {
        return 0.0;
    }
    float gap = abs(dot(n_o, recv_p - origin_p));
    float depth = max(lighting_fill_view_depth(origin_p), lighting_fill_view_depth(recv_p));
    if (gap >= lighting_fill_f(25) * depth) {
        return 0.0;
    }
    if (dot(n_o, n_j) < lighting_fill_f(26)) {
        return 0.0;
    }
    if (lighting_axis(n_o) != lighting_axis(n_j)) {
        return 0.0;
    }
    if (t < lighting_fill_f(23)) {
        return 0.0;
    }
    if (!(lighting_fill_share_scale(recv) > 0.0)) {
        return 0.0;
    }
    if (lighting_fill_seed_due(recv)) {
        return 0.0;
    }
    uint updated = lighting_sc_updated(recv);
    uint stamp = lighting_fill_stamp(recv_p, updated);
    if (stamp != 0u && lighting_frame() <= stamp) {
        return 0.0;
    }
    float w = lighting_fill_share_jacobian(origin_p, n_o, hit, n_h, t, recv_p, n_j);
    w = min(w, lighting_fill_f(27));
    if (!(w > 0.0) || w > FILL_REAL_CAP) {
        return 0.0;
    }
    return w;
}

// Higher weight wins. The same weight keeps the lower ray. Matches share_claim.
uint lighting_fill_share_claim(float w, uint ray) {
    float cap = lighting_fill_f(27);
    if (!(cap > 0.0)) {
        cap = 1.0;
    }
    uint q = uint(clamp(w / cap, 0.0, 1.0) * FILL_Q10);
    uint pack = (q << 22u) | ((~ray) & 0x3FFFFFu);
    return pack == 0u ? 1u : pack;
}

vec3 lighting_fill_share_rgb(uint recv, vec3 contrib) {
    vec4 b = cache.v[LIGHTING_SC_EST_AT + 2u * recv + 1u];
    float shown = cache.v[3u * recv + 2u].w;
    contrib = max(contrib, vec3(0.0));
    // A claim-zero estimator (b.x == 0) is not a bound of 0.1. The own fold
    // clamps only after warmup rays, a started short mean, and a shown value.
    if (!(b.x > 0.0 && b.w >= float(max(lighting_fill_u(12), 1u)) && shown > FILL_HALF)) {
        return contrib;
    }
    vec4 a = cache.v[LIGHTING_SC_EST_AT + 2u * recv];
    float mean_l = dot(max(a.rgb, vec3(0.0)), FILL_MSME);
    float dev = sqrt(max(a.w, 0.0));
    float cap = FILL_FIRE_FLOOR + mean_l + lighting_sc_firefly_sigma() * dev;
    float luma = dot(contrib, FILL_MSME);
    if (luma > cap && luma > 0.0) {
        contrib *= cap / luma;
    }
    return contrib;
}

// phase 0 clears the receiver lock, phase 1 elects, phase 2 writes the seed.
// The three passes recompute the same tile, so the winner does not need a store.
void lighting_fill_share_pass(uint gid, uint phase) {
    uint budget = lighting_fill_traced();
    uint checks = ckeys.key[lighting_fill_header_at() + 2u];
    uint on = ckeys.key[lighting_fill_header_at()];
    uint placed = min(checks + on, budget);
    uint tile = max(lighting_fill_u(1), 1u);
    uint cap_n = min(lighting_fill_u(30), 8u);
    uint stride = gl_WorkGroupSize.x * gl_NumWorkGroups.x;
    if (stride == 0u) {
        stride = 64u;
    }
    for (uint r = gid; r < budget; r += stride) {
        if (r < checks || r >= placed) {
            continue;
        }
        uint slot = ckeys.key[lighting_fill_pick_at() + r];
        if (slot >= LIGHTING_CACHE_SLOTS || ckeys.key[slot] == 0u) {
            continue;
        }
        vec3 origin_p;
        vec3 n_o;
        if (!lighting_sc_texel_world(slot, origin_p, n_o)) {
            continue;
        }
        uint px;
        uint py;
        if (!lighting_fill_pixel(origin_p, px, py)) {
            continue;
        }
        uint work_ray = lighting_probe_rays() + r;
        vec4 rad = work.v[lighting_rad_at(work_ray)];
        if (lighting_ray_none(rad)) {
            continue;
        }
        float t = uintBitsToFloat(ckeys.key[lighting_fill_hit_at() + r * 2u]);
        if (!(t > 0.0)) {
            continue;
        }
        vec3 dir = vec3(0.0, 1.0, 0.0);
        vec3 ray_o;
        vec3 ray_n;
        if (!lighting_fill_ray(work_ray, ray_o, dir, ray_n)) {
            continue;
        }
        vec3 n_h = lighting_unpack_normal(ckeys.key[lighting_fill_hit_at() + r * 2u + 1u]);
        vec3 hit = ray_o + dir * t;
        vec3 lk = lighting_ray_light(rad);
        vec3 irr = lk * max(dot(dir, n_o), 0.0) * LIGHTING_TAU;
        float size = cache.v[3u * slot + 1u].w;
        uint x0 = (px / tile) * tile;
        uint y0 = (py / tile) * tile;
        uint x1 = min(x0 + tile, pc.dims.x);
        uint y1 = min(y0 + tile, pc.dims.y);
        uint nfound = 0u;
        uint found[8];
        float found_w[8];
        for (uint s = 0u; s < 8u; s++) {
            found[s] = 0u;
            found_w[s] = 0.0;
        }
        bool stop = false;
        [[dont_unroll]] for (uint y = y0; y < y1 && !stop; y++) {
            [[dont_unroll]] for (uint x = x0; x < x1 && !stop; x++) {
                vec3 recv_p;
                vec3 n_j;
                vec3 albedo;
                bool two_sided;
                if (!lighting_surface(uvec2(x, y), recv_p, n_j, albedo, two_sided)) {
                    continue;
                }
                uint recv = lighting_cache_find_sized(recv_p, n_j, false, size, recv_p);
                float w = lighting_fill_share_accept(slot, recv, origin_p, n_o, hit, n_h, t, recv_p, n_j);
                if (!(w > 0.0)) {
                    continue;
                }
                bool seen = false;
                for (uint s = 0u; s < 8u; s++) {
                    if (s < nfound && found[s] == recv) {
                        seen = true;
                    }
                }
                if (seen || nfound >= cap_n) {
                    stop = nfound >= cap_n;
                    continue;
                }
                found[nfound] = recv;
                found_w[nfound] = w;
                nfound++;
            }
        }
        for (uint s = 0u; s < 8u; s++) {
            if (s >= nfound) {
                break;
            }
            uint lock = lighting_fill_lock_at() + found[s];
            uint word = lighting_fill_share_claim(found_w[s] * lighting_fill_share_scale(found[s]), r);
            if (phase == 0u) {
                atomicExchange(ckeys.key[lock], 0u);
            } else if (phase == 1u) {
                atomicMax(ckeys.key[lock], word);
            } else if (ckeys.key[lock] == word) {
                float scale = lighting_fill_share_scale(found[s]);
                vec3 rgb = lighting_fill_share_rgb(found[s], irr * found_w[s]);
                if (scale > 0.0 && dot(rgb, FILL_MSME) > 0.0) {
                    lighting_sc_set_fill_seed(found[s], rgb, lighting_fill_f(29) * scale, clamp(rad.w, 0.0, 1.0));
                }
            }
        }
    }
}

// In-tile seeds. Off, or a frozen cache, writes nothing. The host runs one
// phase per dispatch, so this does not barrier inside the workgroup.
void lighting_fill_share(uint gid, uint phase) {
    if (lighting_fill_u(22) != 1u) {
        return;
    }
    if ((lighting_toggles() & LIGHTING_TOGGLE_FREEZE_CACHE) != 0u) {
        return;
    }
    lighting_fill_share_pass(gid, phase);
}

// One slot, one writer. Every ray that landed is on the list, then the head
// folds them with weight n / (rays + n). The hit distance is the stash trace
// wrote; direct has already overwritten the hit slot with radiance.
//
// pc.lamp.x is the phase. The host dispatches one thread per traced ray and
// a buffer barrier between phases, which is the old single-workgroup barrier.
// 0 clears locks, 1 links the rays, 2 commits, 3-5 are the in-tile share
// passes. A phase past that writes nothing.
void lighting_fill_resolve(uint gid) {
    uint phase = pc.lamp.x;
    if (phase >= 3u) {
        if (lighting_fill_u(22) != 1u) {
            return;
        }
        if ((lighting_toggles() & LIGHTING_TOGGLE_FREEZE_CACHE) != 0u) {
            return;
        }
        if (phase > 5u) {
            return;
        }
        lighting_fill_share(gid, phase - 3u);
        return;
    }
    uint budget = lighting_fill_traced();
    uint checks = ckeys.key[lighting_fill_header_at() + 2u];
    uint on = ckeys.key[lighting_fill_header_at()];
    uint ranked = ckeys.key[lighting_fill_header_at() + 1u];
    uint placed = min(checks + on + ranked, budget);
    uint stride = gl_WorkGroupSize.x * gl_NumWorkGroups.x;
    if (stride == 0u) {
        stride = 64u;
    }
    if (phase == 0u) {
        // The emit passes counted picks in the lock word. The chain starts empty.
        for (uint r = gid; r < placed; r += stride) {
            uint slot = ckeys.key[lighting_fill_pick_at() + r];
            if (slot < LIGHTING_CACHE_SLOTS) {
                ckeys.key[lighting_fill_lock_at() + slot] = 0u;
            }
        }
        return;
    }
    if (phase == 1u) {
    for (uint r = gid; r < budget; r += stride) {
        ckeys.key[lighting_fill_link_at() + r * 2u] = 0u;
        ckeys.key[lighting_fill_link_at() + r * 2u + 1u] = 0u;
        uint work_ray = lighting_probe_rays() + r;
        vec4 rad = work.v[lighting_rad_at(work_ray)];
        if (lighting_ray_none(rad)) {
            continue;
        }
        uint slot = ~0u;
        if (r < placed) {
            slot = ckeys.key[lighting_fill_pick_at() + r];
        } else {
            float t = uintBitsToFloat(ckeys.key[lighting_fill_hit_at() + r * 2u]);
            if (t < 0.0) {
                continue;
            }
            vec3 dir;
            vec3 origin;
            vec3 nn;
            if (!lighting_fill_ray(lighting_probe_rays() + r, origin, dir, nn)) {
                continue;
            }
            vec3 pos = origin + dir * t;
            vec3 hn = lighting_unpack_normal(ckeys.key[lighting_fill_hit_at() + r * 2u + 1u]);
            float d = length(pos - scene.eye.xyz);
            if (d > lighting_fill_f(6)) {
                continue;
            }
            uint level = lighting_fill_level(d);
            // A camera primary is not a cosine hemisphere sample. Folding it
            // with TAU stores the shaded hit as irradiance and the hall moves
            // away from the reference. Claiming the cell is the deposit; the
            // hemisphere samples are what enter the mean.
            lighting_cache_find_sized(pos, hn, true, lighting_fill_cell_size(level), pos);
            continue;
        }
        if (slot >= LIGHTING_CACHE_SLOTS || ckeys.key[slot] == 0u) {
            continue;
        }
        uint prev = atomicExchange(ckeys.key[lighting_fill_lock_at() + slot], r + 1u);
        if ((prev & 0x80000000u) != 0u) {
            prev = 0u;
        }
        ckeys.key[lighting_fill_link_at() + r * 2u] = prev;
        ckeys.key[lighting_fill_link_at() + r * 2u + 1u] = slot + 1u;
    }
        return;
    }
    for (uint r = gid; r < budget; r += stride) {
        uint slot1 = ckeys.key[lighting_fill_link_at() + r * 2u + 1u];
        if (slot1 == 0u) {
            continue;
        }
        uint slot = slot1 - 1u;
        if (ckeys.key[lighting_fill_lock_at() + slot] != r + 1u) {
            continue;
        }
        vec3 sum_e = vec3(0.0);
        vec3 sum_lk = vec3(0.0);
        vec3 sum_x = vec3(0.0);
        vec3 sum_y = vec3(0.0);
        vec3 sum_z = vec3(0.0);
        float sum_sun = 0.0;
        float count = 0.0;
        bool saw_on = false;
        bool saw_check = false;
        float check_luma = 0.0;
        uint cur = r;
        uint steps = 0u;
        while (steps < budget) {
            uint work_ray = lighting_probe_rays() + cur;
            vec4 rad = work.v[lighting_rad_at(work_ray)];
            vec3 lk = lighting_ray_light(rad);
            vec3 dir = vec3(0.0, 1.0, 0.0);
            vec3 origin;
            vec3 nn;
            bool aimed = lighting_fill_ray(work_ray, origin, dir, nn);
            float cos_t = 0.0;
            if (aimed && cur < placed) {
                cos_t = max(dot(dir, nn), 0.0);
            } else if (aimed) {
                vec3 hn = lighting_unpack_normal(ckeys.key[lighting_fill_hit_at() + cur * 2u + 1u]);
                cos_t = max(dot(-dir, hn), 0.0);
                dir = -dir;
            }
            if (aimed) {
                sum_e += lk * cos_t * LIGHTING_TAU;
                sum_lk += lk;
                sum_x += lk * dir.x;
                sum_y += lk * dir.y;
                sum_z += lk * dir.z;
                sum_sun += rad.w;
                count += 1.0;
                if (cur < checks) {
                    saw_check = true;
                    check_luma = dot(lk, FILL_LUMA);
                } else if (cur < checks + on) {
                    saw_on = true;
                }
            }
            uint nxt = ckeys.key[lighting_fill_link_at() + cur * 2u];
            steps++;
            if (nxt == 0u) {
                break;
            }
            cur = nxt - 1u;
        }
        float inv = 1.0 / max(count, 1.0);
        if (saw_check) {
            lighting_fill_note_check(slot, check_luma);
        }
        lighting_fill_commit(slot, sum_e * inv, count, sum_lk * inv, sum_x * inv, sum_y * inv, sum_z * inv, saw_on, sum_sun * inv);
    }
}

// The birth frame often runs before the parent cell has a sample. An empty
// child takes that prior once. A child that already has its own samples keeps
// them: replacing the prior every frame was a wipe of a surface that is still
// there. A child the frustum can see is left as it was.
#ifndef GENOS_COMPOSE_NO_WALK
void lighting_fine_adopt(uint slot) {
    if (cache.v[3u * slot + 1u].w > LIGHTING_HIT_PATCH) {
        return;
    }
    if (!(cache.v[LIGHTING_SC_SHOWN_AT + slot].y > LIGHTING_FINE_VALID)) {
        return;
    }
    if (cache.v[3u * slot + 2u].w > 0.0) {
        return;
    }
    vec3 p;
    vec3 n;
    if (!lighting_sc_texel_world(slot, p, n)) {
        return;
    }
    vec3 pr;
    float pn;
    float pv;
    float ps;
    lighting_fine_prior(p, n, pr, pn, pv, ps);
    float have = cache.v[LIGHTING_SC_SHOWN_AT + slot].z;
    if (!(pn > have + LIGHTING_SC_SET_HALF)) {
        return;
    }
    atomicAdd(gstats.diag[LIGHTING_RESET_ADOPT], 1u);
    lighting_sc_set_seed(slot, pr, 1.0, ps);
    vec4 mark = cache.v[LIGHTING_SC_SHOWN_AT + slot];
    mark.y = 1.0;
    mark.z = pn;
    mark.w = pv;
    cache.v[LIGHTING_SC_SHOWN_AT + slot] = mark;
    lighting_fine_publish(slot);
}
#endif
// Off-screen, the field is the fading parent prior until four own sets. That
// pass stores the child's own field and clears the mark. Later off-screen
// samples keep the store on the current own mean until a pixel shows the
// child. A child the frustum can see is published by the screen push.
void lighting_fine_world_filter(uint slot) {
    if (slot >= LIGHTING_CACHE_SLOTS || cache.v[3u * slot + 1u].w > LIGHTING_HIT_PATCH) {
        return;
    }
    uint px;
    uint py;
    if (lighting_fill_pixel(lighting_sc_texel_pos(slot), px, py)) {
        return;
    }
#ifndef GENOS_COMPOSE_NO_WALK
    lighting_fine_adopt(slot);
#endif
    if (lighting_fine_has_prior(slot)) {
        lighting_fine_publish(slot);
    } else if (cache.v[LIGHTING_SC_SHOWN_AT + slot].y > LIGHTING_FINE_VALID
        && floor(max(cache.v[3u * slot + 2u].w, 0.0)) >= LIGHTING_FINE_READY_SETS
        && floatBitsToUint(cache.v[LIGHTING_WORLD_AT + slot].z) == 0u) {
        lighting_fine_store_own(slot);
    }
}
