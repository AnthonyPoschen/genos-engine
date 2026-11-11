// Stages F2, F3, and F4a. The world cache is the only store. A cache round spends
// fill_budget single rays. F3 water-fills them by priority and spends part of
// the budget on one same-seed check ray per fill_check_rate block. F4a writes a
// low-confidence seed into other cells of the same screen tile. The seed is not
// an own sample and it does not add a ray. Ring sharing is not this stage.
// The radii, the weight, the fold, and the share gates match crates/fill-ref.
//
// Param words, after the per-pixel seed payload:
// 0 budget, 1 tile, 2 lod_near, 3 leash, 4 look_ahead, 5 cell_max, 6 far,
// 7 far_share, 8 offscreen_floor, 9 error_target, 10 prox_min, 11 change_weight,
// 12 warmup rays, 13 ray cap, 14 frame_dt, 15 base cell, 16 check_rate,
// 17 max_picks, 18 lambda_decay, 19 window_floor, 20 catchup_max, 21 check_sigma,
// 22 share (0 off, 1 tile), 23 min hit, 24 own cap, 25 plane, 26 normal,
// 27 weight cap, 28 cosine floor, 29 seed confidence, 30 receivers.

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
const float FILL_T_EPS = 1.0e-3;
const float FILL_Q10 = 1023.0;
const float FILL_SHARE_COS0 = 0.05;
const vec3 FILL_MSME = vec3(0.299, 0.587, 0.114);

const uint LIGHTING_FILL_PARAMS = 31u;
const uint LIGHTING_FILL_TILE_CAP = 262144u;
const uint LIGHTING_FILL_TILE_AT = 2u * LIGHTING_CACHE_SLOTS - LIGHTING_FILL_TILE_CAP;
const uint LIGHTING_FILL_PICK_AT = LIGHTING_CACHE_SLOTS + 1u;

uint lighting_fill_param_at() {
    return LIGHTING_SC_SEED_PAY_AT + pc.dims.x * pc.dims.y * 8u;
}
uint lighting_fill_lock_at() {
    return lighting_fill_param_at() + LIGHTING_FILL_PARAMS;
}
// Four words, then two hit words per cache ray (distance, packed normal), then
// two link words per ray (next, slot + 1). The host sizes the same tail.
uint lighting_fill_header_at() {
    return lighting_fill_lock_at() + LIGHTING_CACHE_SLOTS;
}
uint lighting_fill_hit_at() {
    return lighting_fill_header_at() + 4u;
}
uint lighting_fill_rays_max() {
    return LIGHTING_CACHE_BATCH * LIGHTING_CACHE_RAYS;
}
uint lighting_fill_link_at() {
    return lighting_fill_hit_at() + lighting_fill_rays_max() * 2u;
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
    if (level >= lighting_fill_lod_max()) {
        return lighting_fill_f(6);
    }
    return lighting_fill_f(2) * exp2(float(level));
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
vec3 lighting_fill_look() {
    vec4 last = work.v[lighting_change_at() + 1u];
    if (last.w < FILL_HALF) {
        return vec3(0.0);
    }
    float dt = max(lighting_fill_f(14), FILL_TINY);
    return (scene.eye.xyz - last.xyz) * (lighting_fill_f(4) / dt);
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

// The fine cell is past its leash and a coarser cell still covers the point.
// Copy into an empty parent, then drop the fine one. A failed claim keeps it.
void lighting_fill_coarsen(uint slot, vec3 p, vec3 n, uint target) {
    float size = lighting_fill_cell_size(target);
    uint parent = lighting_cache_find_sized(p, n, true, size, p);
    if (parent == ~0u || parent == slot) {
        return;
    }
    if (cache.v[3u * parent + 2u].w < FILL_HALF) {
        cache.v[3u * parent + 2u] = cache.v[3u * slot + 2u];
        cache.v[LIGHTING_SC_EST_AT + 2u * parent] = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
        cache.v[LIGHTING_SC_EST_AT + 2u * parent + 1u] = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u];
        if (lighting_sh_on()) {
            vec3 c0;
            vec3 cx;
            vec3 cy;
            vec3 cz;
            lighting_sh_get(slot, c0, cx, cy, cz);
            lighting_sh_put(parent, c0, cx, cy, cz);
        }
    }
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

uint lighting_fill_q(uint slot, float d) {
    vec4 shown_v = cache.v[3u * slot + 2u];
    vec4 est = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
    float rays = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u].w;
    float mean = max(dot(shown_v.rgb, FILL_LUMA), FILL_TINY);
    float dev = sqrt(max(est.w, 0.0));
    float lam = lighting_fill_unit(cache.v[LIGHTING_SC_GRAD_AT + slot].x);
    float err = rays < FILL_HALF ? 1.0 : max(0.0, dev / mean - lighting_fill_f(9));
    float close = 1.0 / max(d, lighting_fill_f(10));
    return lighting_fill_bucket(close * (err + lighting_fill_f(11) * lam));
}

void lighting_fill_decay(uint slot) {
    uint want = lighting_fill_seed(slot);
    vec4 g = cache.v[LIGHTING_SC_GRAD_AT + slot];
    // A reused slot still holds the previous occupant's seed. Drop it before
    // priority, so the new surface starts with no lambda and no stored luma.
    if (floatBitsToUint(g.z) != want) {
        g = vec4(0.0);
        g.z = uintBitsToFloat(want);
        cache.v[LIGHTING_SC_GRAD2_AT + slot].z = 0.0;
    }
    g.x = lighting_fill_unit(g.x) * clamp(lighting_fill_f(18), 0.0, 1.0);
    cache.v[LIGHTING_SC_GRAD_AT + slot] = g;
}

// Drop a clipmap cell past its leash. 12.5 cm detail stays; the screen stage
// owns that size (S57). A stored cell inside the leash is kept, so the create
// edge does not thrash. Past the leash, coarsen, then evict.
void lighting_fill_visit(uint slot) {
    vec3 p;
    vec3 n;
    if (!lighting_sc_texel_world(slot, p, n)) {
        return;
    }
    float size = cache.v[3u * slot + 1u].w;
    float base = lighting_fill_f(15);
    uint last = floatBitsToUint(cache.v[3u * slot].w);
    uint shown = floatBitsToUint(cache.v[LIGHTING_WORLD_AT + slot].z);
    lighting_fill_decay(slot);
    float d = length(p - scene.eye.xyz);
    float extra = length(lighting_fill_look());
    if (size >= base * FILL_CELL_KEEP) {
        uint level = lighting_fill_level_of_size(size);
        float keep = lighting_fill_leash(level) + extra;
        if (d > keep) {
            uint target = level;
            uint cap = lighting_fill_lod_max();
            while (target < cap && d > lighting_fill_leash(target) + extra) {
                target++;
            }
            if (target > level && d <= lighting_fill_leash(target) + extra) {
                lighting_fill_coarsen(slot, p, n, target);
            } else if (d > lighting_fill_leash(target) + extra) {
                lighting_fill_free(slot);
            }
            return;
        }
    } else if (lighting_sc_expired(last, shown)) {
        lighting_fill_free(slot);
        return;
    }
    if (size <= LIGHTING_WRC_CELL_SIZE && lighting_compact_hold_texel_rays(slot)) {
        return;
    }
    uint px;
    uint py;
    uint q = lighting_fill_q(slot, d);
    uint level = lighting_fill_level_of_size(size);
    if (!lighting_fill_pixel(p, px, py)) {
        // Bit 31 marks a ranked off-screen cell. A list head never sets it.
        ckeys.key[lighting_fill_lock_at() + slot] = 0x80000000u | (q << 16u) | (level << 8u) | 1u;
        return;
    }
    uint tile = max(lighting_fill_u(1), 1u);
    uint tiles_x = (pc.dims.x + tile - 1u) / tile;
    uint ti = px / tile + (py / tile) * tiles_x;
    if (ti >= LIGHTING_FILL_TILE_CAP) {
        return;
    }
    // On-screen cells are the tile pick, not also an off-screen candidate.
    ckeys.key[lighting_fill_lock_at() + slot] = 0u;
    uint packed = (q << 21u) | (slot + 1u);
    atomicMax(ckeys.key[LIGHTING_FILL_TILE_AT + ti], packed);
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

uint lighting_fill_give(uint slot, uint q, uint base_m[64], inout uint extra_left[64], uint max_picks, uint at, uint end) {
    if (q == 0u || slot >= LIGHTING_CACHE_SLOTS || at >= end) {
        return at;
    }
    // cell_grant: max_picks is per cell. A screen cell can win several tiles.
    // Off-screen locks keep bit 31, and each slot is visited once.
    uint lock = lighting_fill_lock_at() + slot;
    uint lock_w = ckeys.key[lock];
    bool ranked = (lock_w & 0x80000000u) != 0u;
    uint prior = ranked ? 0u : lock_w;
    if (prior >= max_picks) {
        return at;
    }
    uint room = max_picks - prior;
    uint m = min(base_m[q], room);
    if (extra_left[q] > 0u && m < room) {
        m += 1u;
        extra_left[q] -= 1u;
    }
    uint emitted = 0u;
    for (uint k = 0u; k < 8u; k++) {
        if (k >= m || at >= end) {
            break;
        }
        ckeys.key[LIGHTING_FILL_PICK_AT + at] = slot;
        at++;
        emitted++;
    }
    if (!ranked) {
        ckeys.key[lock] = prior + emitted;
    }
    return at;
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

void lighting_fill_dispatch() {
    uint budget = lighting_fill_u(0);
    uint tile = max(lighting_fill_u(1), 1u);
    uint tiles_x = (pc.dims.x + tile - 1u) / tile;
    uint tiles_y = (pc.dims.y + tile - 1u) / tile;
    uint tiles = min(tiles_x * tiles_y, LIGHTING_FILL_TILE_CAP);
    uint rate = max(lighting_fill_u(16), 1u);
    uint side = 1u;
    while (side < 8u && side * side < rate) {
        side++;
    }
    uint area = side * side;
    uint max_picks = clamp(lighting_fill_u(17), 1u, 8u);
    uint pick_cap = LIGHTING_FILL_TILE_AT - LIGHTING_FILL_PICK_AT;
    uint check_cap = budget / rate;
    if (check_cap == 0u && budget > 0u) {
        check_cap = 1u;
    }
    check_cap = min(check_cap, min(budget, pick_cap));
    uint hist[64];
    for (uint i = 0u; i < 64u; i++) {
        hist[i] = 0u;
    }
    uint check_n = 0u;
    uint limit = LIGHTING_FILL_TILE_CAP;
    // checks_from_phase: the cap walks from this frame's phase and wraps, so
    // the tail of the table is not skipped every frame.
    uint tile_phase = tiles == 0u ? 0u : lighting_round() % tiles;
    for (uint i = 0u; i < limit; i++) {
        if (i >= tiles) {
            continue;
        }
        uint t = i + tile_phase;
        if (t >= tiles) {
            t -= tiles;
        }
        uint w = ckeys.key[LIGHTING_FILL_TILE_AT + t];
        if (w == 0u) {
            continue;
        }
        uint q = min(w >> 21u, 63u);
        if (q > 0u) {
            hist[q]++;
        }
        if (check_n >= check_cap) {
            continue;
        }
        uint tx = t % max(tiles_x, 1u);
        uint ty = t / max(tiles_x, 1u);
        uint bx = tx / side;
        uint by = ty / side;
        uint ox = tx - bx * side;
        uint oy = ty - by * side;
        uint h = lighting_hash(bx ^ lighting_hash(by ^ lighting_hash(lighting_round())));
        if (oy * side + ox == h % area) {
            ckeys.key[LIGHTING_FILL_PICK_AT + check_n] = (w & 2097151u) - 1u;
            check_n++;
        }
    }
    uint hist_far[64];
    uint hist_near[64];
    for (uint i = 0u; i < 64u; i++) {
        hist_far[i] = 0u;
        hist_near[i] = 0u;
    }
    uint lod_max = lighting_fill_lod_max();
    uint guard = LIGHTING_CACHE_SLOTS;
    uint slot_phase = lighting_round() & (guard - 1u);
    for (uint i = 0u; i < guard; i++) {
        uint s = (i + slot_phase) & (guard - 1u);
        uint w = ckeys.key[lighting_fill_lock_at() + s];
        if ((w & 0x80000000u) == 0u || ckeys.key[s] == 0u) {
            continue;
        }
        uint q = min((w >> 16u) & 63u, 63u);
        uint level = (w >> 8u) & 255u;
        if (q > 0u) {
            if (level == lod_max) {
                hist_far[q]++;
            } else {
                hist_near[q]++;
            }
        }
        if (check_n >= check_cap) {
            continue;
        }
        uint block = s / rate;
        uint local = s - block * rate;
        uint pick = lighting_hash(block ^ lighting_hash(lighting_round())) % rate;
        if (local == pick) {
            ckeys.key[LIGHTING_FILL_PICK_AT + check_n] = s;
            check_n++;
        }
    }
    uint spend = budget - check_n;
    uint floor_n = min(uint(ceil(float(spend) * lighting_fill_f(8))), spend);
    uint on_cap = spend - floor_n;
    uint base_m[64];
    uint extra_m[64];
    uint on_take = 0u;
    lighting_fill_plan(hist, on_cap, max_picks, base_m, extra_m, on_take);
    uint extra_left[64];
    for (uint i = 0u; i < 64u; i++) {
        extra_left[i] = extra_m[i];
    }
    uint at = check_n;
    uint end = min(check_n + on_take, pick_cap);
    for (uint t = 0u; t < limit; t++) {
        uint w = t < tiles ? ckeys.key[LIGHTING_FILL_TILE_AT + t] : 0u;
        ckeys.key[LIGHTING_FILL_TILE_AT + t] = 0u;
        if (w == 0u) {
            continue;
        }
        uint q = min(w >> 21u, 63u);
        at = lighting_fill_give((w & 2097151u) - 1u, q, base_m, extra_left, max_picks, at, end);
    }
    // The lock word is the resolve chain. The per-cell count must not stay in it.
    for (uint i = check_n; i < at; i++) {
        uint slot = ckeys.key[LIGHTING_FILL_PICK_AT + i];
        if (slot < LIGHTING_CACHE_SLOTS) {
            ckeys.key[lighting_fill_lock_at() + slot] = 0u;
        }
    }
    uint on = at - check_n;
    uint off = spend - on;
    uint far_n = min(uint(ceil(float(spend) * lighting_fill_f(7))), off);
    uint near_n = off - far_n;
    uint far_take = 0u;
    uint near_take = 0u;
    lighting_fill_plan(hist_far, far_n, max_picks, base_m, extra_m, far_take);
    for (uint i = 0u; i < 64u; i++) {
        extra_left[i] = extra_m[i];
    }
    end = min(at + far_take, pick_cap);
    uint far_got = 0u;
    for (uint s = 0u; s < guard; s++) {
        uint w = ckeys.key[lighting_fill_lock_at() + s];
        if ((w & 0x80000000u) == 0u || ckeys.key[s] == 0u) {
            continue;
        }
        uint level = (w >> 8u) & 255u;
        if (level != lod_max) {
            continue;
        }
        uint before = at;
        uint q = min((w >> 16u) & 63u, 63u);
        at = lighting_fill_give(s, q, base_m, extra_left, max_picks, at, end);
        far_got += at - before;
    }
    uint near_budget = min(near_n, spend - on - far_got);
    lighting_fill_plan(hist_near, near_budget, max_picks, base_m, extra_m, near_take);
    for (uint i = 0u; i < 64u; i++) {
        extra_left[i] = extra_m[i];
    }
    end = min(at + near_take, pick_cap);
    uint near_got = 0u;
    for (uint s = 0u; s < guard; s++) {
        uint w = ckeys.key[lighting_fill_lock_at() + s];
        if ((w & 0x80000000u) == 0u || ckeys.key[s] == 0u) {
            continue;
        }
        uint level = (w >> 8u) & 255u;
        if (level == lod_max) {
            continue;
        }
        uint before = at;
        uint q = min((w >> 16u) & 63u, 63u);
        at = lighting_fill_give(s, q, base_m, extra_left, max_picks, at, end);
        near_got += at - before;
    }
    uint ranked = far_got + near_got;
    uint placed = check_n + on + ranked;
    if (placed > budget) {
        placed = budget;
    }
    uint far_disc = far_n - min(far_got, far_n);
    ckeys.key[lighting_fill_header_at()] = ranked;
    ckeys.key[lighting_fill_header_at() + 1u] = far_disc;
    ckeys.key[lighting_fill_header_at() + 2u] = check_n;
    ckeys.key[LIGHTING_ARGS_AT + 15u] = on;
    ckeys.key[LIGHTING_ARGS_AT + 0u] = max((budget + 63u) / 64u, 1u);
    ckeys.key[LIGHTING_ARGS_AT + 1u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 2u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 3u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 4u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 5u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 6u] = budget;
    ckeys.key[LIGHTING_ARGS_AT + 7u] = on;
    ckeys.key[LIGHTING_ARGS_AT + 8u] = on;
    ckeys.key[LIGHTING_ARGS_AT + 9u] = on;
    ckeys.key[LIGHTING_ARGS_AT + 10u] = on;
    gstats.rays = budget;
    gstats.due = on;
    gstats.scouts = budget - on - check_n;
    gstats.pad4 = check_n;
    gstats.round_entries = (budget + 15u) / 16u;
}

bool lighting_fill_ray(uint ray, out vec3 origin, out vec3 dir, out vec3 n) {
    origin = vec3(0.0);
    dir = vec3(0.0, 1.0, 0.0);
    n = dir;
    if (ray < lighting_probe_rays()) {
        return false;
    }
    uint r = ray - lighting_probe_rays();
    uint budget = lighting_fill_u(0);
    if (r >= budget) {
        return false;
    }
    uint checks = ckeys.key[lighting_fill_header_at() + 2u];
    uint on = ckeys.key[LIGHTING_ARGS_AT + 15u];
    uint ranked = ckeys.key[lighting_fill_header_at()];
    uint placed = min(checks + on + ranked, budget);
    if (r < placed) {
        uint slot = ckeys.key[LIGHTING_FILL_PICK_AT + r];
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
    uint disc = max(budget - placed, 1u);
    uint i = (r - placed + lighting_round()) % disc;
    dir = lighting_fill_fibonacci(i, disc);
    origin = scene.eye.xyz;
    n = dir;
    return true;
}

// Trace writes the hit here. Direct then stores radiance in the hit slot.
void lighting_fill_stash(uint ray, float t, uint packed_n) {
    if (!lighting_ray_fill_on() || ray < lighting_probe_rays()) {
        return;
    }
    uint r = ray - lighting_probe_rays();
    if (r >= lighting_fill_u(0)) {
        return;
    }
    ckeys.key[lighting_fill_hit_at() + r * 2u] = floatBitsToUint(t);
    ckeys.key[lighting_fill_hit_at() + r * 2u + 1u] = packed_n;
}

float lighting_fill_shown(uint rays, uint slot) {
    float sets = float(rays) / FILL_SET_RAYS;
    if (rays > 0u) {
        sets = max(sets, 1.0);
    }
    return min(sets, lighting_sc_cap(slot));
}

// `e` is the mean of this frame's n samples (each `lk * cos * TAU`). One writer
// calls this, after every ray into the slot has been summed.
void lighting_fill_commit(uint slot, vec3 e, float n, vec3 lk_mean, vec3 shx, vec3 shy, vec3 shz, bool on_screen, float sun_luma) {
    if (slot >= LIGHTING_CACHE_SLOTS || ckeys.key[slot] == 0u || n <= 0.0) {
        return;
    }
    vec4 old = cache.v[3u * slot + 2u];
    vec4 b = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u];
    float rays0 = max(b.w, 0.0);
    uint warmup = max(lighting_fill_u(12), 1u);
    float calm = n / (rays0 + n);
    float k = lighting_fill_pace(slot, calm, calm, rays0, n);
    bool cell = cache.v[3u * slot + 1u].w > LIGHTING_WRC_CELL_SIZE;
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
    } else {
        mean = old.w < FILL_HALF ? e : mix(old.rgb, e, clamp(k, 0.0, 1.0));
    }
    float cap = float(lighting_fill_u(13));
    float rays_after = min(rays0 + n, max(cap, float(warmup)));
    cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u] = vec4(b.xyz, rays_after);
    if (cell && lighting_sh_on()) {
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
            float a = clamp(k, 0.0, 1.0);
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
        vec4 shown = vec4(mean, lighting_fill_shown(uint(rays_after), slot));
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
    uint budget = lighting_fill_u(0);
    uint checks = ckeys.key[lighting_fill_header_at() + 2u];
    uint on = ckeys.key[LIGHTING_ARGS_AT + 15u];
    uint placed = min(checks + on, budget);
    uint tile = max(lighting_fill_u(1), 1u);
    uint cap_n = min(lighting_fill_u(30), 8u);
    for (uint r = gid; r < budget; r += 64u) {
        if (r < checks || r >= placed) {
            continue;
        }
        uint slot = ckeys.key[LIGHTING_FILL_PICK_AT + r];
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
                    lighting_sc_set_seed(found[s], rgb, lighting_fill_f(29) * scale, clamp(rad.w, 0.0, 1.0));
                }
            }
        }
    }
}

// In-tile seeds. Off, or a frozen cache, writes nothing. Every thread takes the
// same branch, so the barriers below stay inside the workgroup.
void lighting_fill_share(uint gid) {
    if (lighting_fill_u(22) != 1u) {
        return;
    }
    if ((lighting_toggles() & LIGHTING_TOGGLE_FREEZE_CACHE) != 0u) {
        return;
    }
    lighting_fill_share_pass(gid, 0u);
    memoryBarrierBuffer();
    barrier();
    lighting_fill_share_pass(gid, 1u);
    memoryBarrierBuffer();
    barrier();
    lighting_fill_share_pass(gid, 2u);
}

// One slot, one writer. Every ray that landed is on the list, then the head
// folds them with weight n / (rays + n). The hit distance is the stash trace
// wrote; direct has already overwritten the hit slot with radiance.
void lighting_fill_resolve(uint gid) {
    if (gid >= 64u) {
        return;
    }
    uint budget = lighting_fill_u(0);
    uint checks = ckeys.key[lighting_fill_header_at() + 2u];
    uint on = ckeys.key[LIGHTING_ARGS_AT + 15u];
    uint ranked = ckeys.key[lighting_fill_header_at()];
    uint placed = min(checks + on + ranked, budget);
    uint far_disc = ckeys.key[lighting_fill_header_at() + 1u];
    uint lod_max = lighting_fill_lod_max();
    for (uint r = gid; r < budget; r += 64u) {
        ckeys.key[lighting_fill_link_at() + r * 2u] = 0u;
        ckeys.key[lighting_fill_link_at() + r * 2u + 1u] = 0u;
        uint work_ray = lighting_probe_rays() + r;
        vec4 rad = work.v[lighting_rad_at(work_ray)];
        if (lighting_ray_none(rad)) {
            continue;
        }
        uint slot = ~0u;
        if (r < placed) {
            slot = ckeys.key[LIGHTING_FILL_PICK_AT + r];
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
            float d = length(pos - (scene.eye.xyz + lighting_fill_look()));
            if (d > lighting_fill_f(6)) {
                continue;
            }
            uint level = lighting_fill_level(d);
            if ((r - placed) < far_disc && level < lod_max) {
                continue;
            }
            slot = lighting_cache_find_sized(pos, hn, true, lighting_fill_cell_size(level), pos);
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
    memoryBarrierBuffer();
    barrier();
    for (uint r = gid; r < budget; r += 64u) {
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
    memoryBarrierBuffer();
    barrier();
    lighting_fill_share(gid);
}
