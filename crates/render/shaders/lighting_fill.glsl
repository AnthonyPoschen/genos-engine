// One allocator. The world cache is the only store. A cache round spends
// fill_budget single rays on one value per cell, highest rank first. A cell
// with no rays starts at 1. The value falls as the mean converges. Word 8 is
// the trickle floor: that fraction is reserved first and drawn by every cell
// in range, including a converged cell. A disagreeing trickle ray raises the
// value through the same-seed check. There is no tile winner, no off-screen
// bin, no far/near split, and no in-tile seed.
// The radii, the weight, and the fold match crates/fill-ref.
//
// Param words, after the per-pixel seed payload:
// 0 budget, 1 tile, 2 lod_near, 3 leash, 4 look_ahead, 5 cell_max, 6 far,
// 7 far_share (not a second budget), 8 trickle floor, 9 error_target, 10 prox_min, 11 change_weight,
// 12 warmup rays, 13 ray cap, 14 frame_dt, 15 base cell, 16 check_rate,
// 17 max_picks, 18 lambda_decay, 19 window_floor, 20 catchup_max, 21 check_sigma,
// 22 share (0 off, 1 tile), 23 min hit, 24 own cap, 25 plane, 26 normal,
// 27 weight cap, 28 cosine floor, 29 seed confidence, 30 receivers,
// 31 share handover, in own rays. 0 means the warmup count (word 12).

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

const uint LIGHTING_FILL_PARAMS = 32u;
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
// Header: 0 placed count, 1 unused, 2 unused (trickle is a bit on the pick),
// 3 trickle cursor, 4 unused, 5 trickle floor, 6 ranked rays planned,
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

// A new cell (no rays) is value 1. After that the value is the remaining
// error plus the measured change. `screen` is 1 on screen and 0.25 off it,
// so both sit in this one ranking.
uint lighting_fill_q(uint slot, float d, float screen) {
    vec4 shown_v = cache.v[3u * slot + 2u];
    vec4 est = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
    float rays = cache.v[LIGHTING_SC_EST_AT + 2u * slot + 1u].w;
    float mean = max(dot(shown_v.rgb, FILL_LUMA), FILL_TINY);
    float dev = sqrt(max(est.w, 0.0));
    float lam = lighting_fill_unit(cache.v[LIGHTING_SC_GRAD_AT + slot].x);
    float err = max(0.0, dev / mean - lighting_fill_f(9));
    float value = rays < FILL_HALF ? 1.0 : max(0.0, err + lighting_fill_f(11) * lam);
    float close = 1.0 / max(d, lighting_fill_f(10));
    return lighting_fill_bucket(screen * close * value);
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

// Drop a clipmap cell past its leash. A stored cell inside the leash is kept,
// so the create edge does not thrash. Past the leash, coarsen, then evict.
// A slot finer than the base world cell does not take a ray: the visit claims
// that cell and returns. Compose reads the cell while the screen texel is empty.
void lighting_fill_visit(uint slot) {
    if (slot < LIGHTING_CACHE_SLOTS) {
        ckeys.key[lighting_fill_lock_at() + slot] = 0u;
    }
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
            // Compose reads the base world cell on screen, including past the
            // clipmap leash. Coarsen and evict only the off-screen copy.
            uint px_keep;
            uint py_keep;
            if (!lighting_fill_pixel(p, px_keep, py_keep)) {
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
        }
    } else if (lighting_sc_expired(last, shown)) {
        lighting_fill_free(slot);
        return;
    }
    // The world cell is the store for a screen texel. A 12.5 cm screen texel
    // outranks its parent while it is young, so the parent stays empty and
    // compose shows a direct-only mean. Claim the parent; its own visit
    // schedules the rays. An off-screen band child is a candidate in this
    // same histogram. A child the frustum can see is not: it would take the
    // parent's rays and the presented frame would stop filling.
    uint seen_x;
    uint seen_y;
    bool seen = lighting_fill_pixel(p, seen_x, seen_y);
    if (size >= base && !seen) {
        lighting_fine_spawn(slot);
    }
    if (size < base) {
        lighting_cache_find_sized(p, n, true, base, p);
        if (!(cache.v[LIGHTING_SC_SHOWN_AT + slot].y > LIGHTING_FINE_VALID) || seen) {
            return;
        }
    }
    if (size <= LIGHTING_WRC_CELL_SIZE && lighting_compact_hold_texel_rays(slot)) {
        return;
    }
    // One histogram. The lock's low 6 bits are the bucket. Bit 6 marks a cell
    // inside the allocator's range, including a converged cell (bucket 0),
    // so the trickle floor can still draw it.
    uint px;
    uint py;
    float screen = lighting_fill_pixel(p, px, py) ? 1.0 : 0.25;
    uint q = lighting_fill_q(slot, d, screen);
    ckeys.key[lighting_fill_lock_at() + slot] = (q & 63u) | 64u;
    if (q > 0u) {
        atomicAdd(ckeys.key[lighting_fill_hist_at() + q], 1u);
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

uint lighting_fill_pick_slot(uint word) {
    return word & 1048575u;
}

bool lighting_fill_pick_trickle(uint word) {
    return (word & 0x80000000u) != 0u;
}

void lighting_fill_store_picks(uint m, uint slot, uint dest, uint flag) {
    for (uint k = 0u; k < 64u; k++) {
        if (k >= m) {
            break;
        }
        ckeys.key[lighting_fill_pick_at() + dest + k] = flag | slot;
    }
}

// The visit already counted one histogram. This dispatch places nothing.
void lighting_fill_hist(uint gid) {
    if (!lighting_ray_fill_on()) {
        return;
    }
    if (gid == 0u) {
        gstats.pad3 = gl_NumWorkGroups.x;
    }
}

// One thread per cell. Ranked rays come from the cell's bucket. A cell the
// ranked spend missed, including a converged cell, may take one trickle ray.
// Bit 31 of the pick word marks that ray so resolve can raise the value.
void lighting_fill_emit_on(uint slot) {
    uint m = 0u;
    uint flag = 0u;
    if (lighting_ray_fill_on() && slot < LIGHTING_CACHE_SLOTS && ckeys.key[slot] != 0u) {
        uint mark = ckeys.key[lighting_fill_lock_at() + slot];
        if ((mark & 64u) != 0u) {
            uint q = mark & 63u;
            if (q > 0u) {
                uint base = ckeys.key[lighting_fill_plan_at() + q];
                uint extra = ckeys.key[lighting_fill_plan_at() + 64u + q];
                m = base;
                if (extra > 0u) {
                    uint ticket = atomicAdd(ckeys.key[lighting_fill_extra_at() + q], 1u);
                    if (ticket < extra) {
                        m += 1u;
                    }
                }
            }
            if (m == 0u) {
                uint budget = lighting_fill_u(0);
                float floor_f = lighting_fill_f(8);
                if (!(floor_f > 0.0)) {
                    floor_f = 0.0;
                }
                uint floor_n = min(uint(ceil(float(budget) * floor_f)), budget);
                uint rate = max(lighting_fill_u(16), 1u);
                uint turn = lighting_hash(slot ^ lighting_hash(lighting_round())) % rate;
                if (turn == 0u && floor_n > 0u) {
                    uint ticket = atomicAdd(ckeys.key[lighting_fill_header_at() + 3u], 1u);
                    if (ticket < floor_n) {
                        m = 1u;
                        flag = 0x80000000u;
                    }
                }
            }
        }
    }
    uint place = lighting_fill_wg_place(m, lighting_fill_header_at());
    if (m == 0u) {
        return;
    }
    lighting_fill_store_picks(m, slot, place, flag);
}

// The off-screen bin grant is gone. The slot walk above is the only spend.
void lighting_fill_emit_off(uint gid) {
    if (gid == ~0u) {
        return;
    }
}

void lighting_fill_dispatch() {
    // One thread. The histogram is already in memory. Emitting the rays is
    // a later parallel dispatch. This function does not walk tiles or slots.
    uint budget = lighting_fill_u(0);
    uint max_picks = clamp(lighting_fill_u(17), 1u, 8u);
    uint hist[64];
    uint serial = 0u;
    uint hist_at = lighting_fill_hist_at();
    for (uint i = 0u; i < 64u; i++) {
        hist[i] = ckeys.key[hist_at + i];
        serial += 1u;
    }
    float floor_f = lighting_fill_f(8);
    if (!(floor_f > 0.0)) {
        floor_f = 0.0;
    }
    uint floor_n = min(uint(ceil(float(budget) * floor_f)), budget);
    uint ranked_budget = budget - floor_n;
    uint base_m[64];
    uint extra_m[64];
    uint take = 0u;
    lighting_fill_plan(hist, ranked_budget, max_picks, base_m, extra_m, take);
    uint plan = lighting_fill_plan_at();
    for (uint i = 0u; i < 64u; i++) {
        ckeys.key[plan + i] = base_m[i];
        ckeys.key[plan + 64u + i] = extra_m[i];
        serial += 2u;
    }
    uint left = ranked_budget - take;
    lighting_fill_pour_row(hist, plan, 64u, left);
    serial += 64u;
    ckeys.key[lighting_fill_header_at() + 1u] = 0u;
    ckeys.key[lighting_fill_header_at() + 2u] = 0u;
    ckeys.key[lighting_fill_header_at() + 5u] = floor_n;
    ckeys.key[lighting_fill_header_at() + 6u] = take;
    ckeys.key[LIGHTING_ARGS_AT + 0u] = max((budget + 63u) / 64u, 1u);
    ckeys.key[LIGHTING_ARGS_AT + 1u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 2u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 3u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 4u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 5u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 6u] = budget;
    ckeys.key[LIGHTING_ARGS_AT + 7u] = budget;
    ckeys.key[LIGHTING_ARGS_AT + 8u] = budget;
    ckeys.key[LIGHTING_ARGS_AT + 9u] = budget;
    ckeys.key[LIGHTING_ARGS_AT + 10u] = budget;
    gstats.rays = 0u;
    gstats.pad4 = floor_n;
    gstats.pad2 = serial;
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
    uint placed = min(ckeys.key[lighting_fill_header_at()], budget);
    if (r < placed) {
        uint word = ckeys.key[lighting_fill_pick_at() + r];
        uint slot = lighting_fill_pick_slot(word);
        if (slot >= LIGHTING_CACHE_SLOTS || ckeys.key[slot] == 0u) {
            return false;
        }
        vec3 p;
        if (!lighting_sc_texel_world(slot, p, n)) {
            return false;
        }
        origin = p + n * FILL_RAY_BIAS;
        if (lighting_fill_pick_trickle(word)) {
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
    if (cell && n > 0.0) {
        blend = sample_add / max(sample_t + sample_add, FILL_EPS);
        if (lighting_sc_dark_fill() && sample_t < FILL_HALF * LIGHTING_WRC_CAP) {
            float ratio = sample_t / max(sample_t + sample_add, FILL_EPS);
            blend = 1.0 - ratio * ratio * ratio;
        }
        bool urgent = ((pc.params.z >> 5u) & 1u) != 0u;
        vec3 cp = lighting_sc_texel_pos(slot);
        bool local = (urgent && lighting_near_change(cp)) || lighting_wrc_lamp_changed(cp, lighting_sc_updated(slot));
        if (local) {
            blend = max(blend, LIGHTING_WRC_LOCAL_ALPHA * sample_add);
        } else if (!lighting_sc_light_quiet()) {
            blend = max(blend, LIGHTING_WRC_CHANGE_ALPHA * sample_add);
        }
        if (lighting_lamp_noted()) {
            blend = 1.0;
        }
        blend = min(blend, 1.0);
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
        mean = old.w < FILL_HALF ? e : mix(old.rgb, e, blend);
        // The texel MSME fold above is `!cell`. A world cell still has to
        // store short variance or its priority falls to 0 after one ray.
        lighting_fill_note_var(slot, e);
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
        float shown_w = min(sample_t + sample_add, lighting_sc_cap(slot));
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

void lighting_fill_resolve(uint gid) {
    if (gid >= 64u) {
        return;
    }
    uint budget = lighting_fill_u(0);
    uint placed = min(ckeys.key[lighting_fill_header_at()], budget);
    // The emit pass counted picks in header word 0. The chain starts empty.
    for (uint r = gid; r < placed; r += 64u) {
        uint slot = lighting_fill_pick_slot(ckeys.key[lighting_fill_pick_at() + r]);
        if (slot < LIGHTING_CACHE_SLOTS) {
            ckeys.key[lighting_fill_lock_at() + slot] = 0u;
        }
    }
    memoryBarrierBuffer();
    barrier();
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
            uint word = ckeys.key[lighting_fill_pick_at() + r];
            slot = lighting_fill_pick_slot(word);
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
                if (lighting_fill_pick_trickle(ckeys.key[lighting_fill_pick_at() + cur])) {
                    saw_check = true;
                    check_luma = dot(lk, FILL_LUMA);
                } else if (cur < placed) {
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
}

// The birth frame often runs before the parent cell has a sample. Until the
// child has its own sets, replace the stored prior when the parent has more
// samples than the one already kept. A child the frustum can see is left as
// it was: the screen push publishes that snapshot, and it does not track the
// parent after it becomes visible.
#ifndef GENOS_COMPOSE_NO_WALK
void lighting_fine_adopt(uint slot) {
    if (cache.v[3u * slot + 1u].w > LIGHTING_HIT_PATCH) {
        return;
    }
    if (!(cache.v[LIGHTING_SC_SHOWN_AT + slot].y > LIGHTING_FINE_VALID)) {
        return;
    }
    if (floor(max(cache.v[3u * slot + 2u].w, 0.0)) >= LIGHTING_FINE_READY_SETS) {
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
