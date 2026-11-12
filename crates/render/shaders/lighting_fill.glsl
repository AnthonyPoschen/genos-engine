// Stage F2. The world cache is the only store. A cache round spends fill_budget
// single rays: one need-weighted pick per screen tile, and the rest on a
// 360-degree clipmap around the player. Each ray deposits the frame it lands.
// The radii match crates/fill-ref/src/sched.rs.
//
// Param words, after the per-pixel seed payload:
// 0 budget, 1 tile, 2 lod_near, 3 leash, 4 look_ahead, 5 cell_max, 6 far,
// 7 far_share, 8 offscreen_floor, 9 error_target, 10 prox_min, 11 change_weight,
// 12 warmup rays, 13 ray cap, 14 frame_dt, 15 base cell.

const float FILL_EPS = 1.0e-6;
const float FILL_HALF = 0.5;
const float FILL_TINY = 1.0e-4;
const float FILL_TWO = 2.0;
const float FILL_GOLDEN = 2.3999631;
const float FILL_VIEW_EPS = 0.05;
const float FILL_CELL_KEEP = 0.75;
const vec3 FILL_LUMA = vec3(0.2126, 0.7152, 0.0722);
const float FILL_PRI_SCALE = 32.0;
const float FILL_PRI_MAX = 63.0;
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

const uint LIGHTING_FILL_PARAMS = 16u;
const uint LIGHTING_FILL_TILE_CAP = 262144u;

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

uint lighting_fill_q(uint slot, float d) {
    vec4 shown_v = cache.v[3u * slot + 2u];
    vec4 est = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
    float mean = max(dot(shown_v.rgb, FILL_LUMA), FILL_TINY);
    float dev = sqrt(max(est.w, 0.0));
    float err = max(0.0, dev / mean - lighting_fill_f(9));
    float close = 1.0 / max(d, lighting_fill_f(10));
    // lambda is 0 until F3. change_weight is uploaded and not read here.
    float pri = close * err;
    return uint(clamp(pri * FILL_PRI_SCALE, 1.0, FILL_PRI_MAX));
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
    atomicMax(ckeys.key[lighting_fill_tile_at() + ti], packed);
}

void lighting_fill_dispatch() {
    uint budget = lighting_fill_u(0);
    uint tile = max(lighting_fill_u(1), 1u);
    uint tiles_x = (pc.dims.x + tile - 1u) / tile;
    uint tiles_y = (pc.dims.y + tile - 1u) / tile;
    uint tiles = min(tiles_x * tiles_y, LIGHTING_FILL_TILE_CAP);
    uint floor_n = uint(ceil(float(budget) * lighting_fill_f(8)));
    floor_n = min(floor_n, budget);
    uint on_cap = budget - floor_n;
    uint hist[64];
    for (uint i = 0u; i < 64u; i++) {
        hist[i] = 0u;
    }
    uint nonempty = 0u;
    uint limit = LIGHTING_FILL_TILE_CAP;
    for (uint t = 0u; t < limit; t++) {
        if (t >= tiles) {
            continue;
        }
        uint w = ckeys.key[lighting_fill_tile_at() + t];
        if (w == 0u) {
            continue;
        }
        nonempty++;
        hist[min(w >> 21u, 63u)]++;
    }
    uint on = min(nonempty, on_cap);
    uint cutoff = 0u;
    uint of_cut = on;
    if (nonempty > on_cap) {
        uint left = on;
        for (int q = 63; q >= 0; q--) {
            uint hq = hist[uint(q)];
            if (hq >= left) {
                cutoff = uint(q);
                of_cut = left;
                break;
            }
            left -= hq;
        }
    }
    uint n = 0u;
    uint at_cut = 0u;
    for (uint t = 0u; t < limit; t++) {
        uint w = t < tiles ? ckeys.key[lighting_fill_tile_at() + t] : 0u;
        ckeys.key[lighting_fill_tile_at() + t] = 0u;
        if (w == 0u || n >= on) {
            continue;
        }
        uint q = min(w >> 21u, 63u);
        if (nonempty > on_cap) {
            if (q < cutoff) {
                continue;
            }
            if (q == cutoff) {
                if (at_cut >= of_cut) {
                    continue;
                }
                at_cut++;
            }
        }
        ckeys.key[lighting_fill_pick_at() + n] = (w & 2097151u) - 1u;
        n++;
    }
    uint off = budget - n;
    uint far_n = min(uint(ceil(float(budget) * lighting_fill_f(7))), off);
    uint near_n = off - far_n;
    uint hist_far[64];
    uint hist_near[64];
    for (uint i = 0u; i < 64u; i++) {
        hist_far[i] = 0u;
        hist_near[i] = 0u;
    }
    uint n_far = 0u;
    uint n_near = 0u;
    uint live = 0u;
    uint lod_max = lighting_fill_lod_max();
    uint guard = LIGHTING_CACHE_SLOTS;
    for (uint s = 0u; s < guard; s++) {
        uint w = ckeys.key[lighting_fill_lock_at() + s];
        if (ckeys.key[s] != 0u && cache.v[3u * s + 1u].w > 0.0) {
            live++;
        }
        if ((w & 0x80000000u) == 0u || ckeys.key[s] == 0u) {
            continue;
        }
        uint q = min((w >> 16u) & 63u, 63u);
        uint level = (w >> 8u) & 255u;
        if (level == lod_max) {
            hist_far[q]++;
            n_far++;
        } else {
            hist_near[q]++;
            n_near++;
        }
    }
    uint far_take = min(n_far, far_n);
    uint near_take = min(n_near, near_n);
    uint pick_cap = lighting_fill_rays_max();
    uint room = pick_cap - min(n, pick_cap);
    if (far_take + near_take > room) {
        if (far_take > room) {
            far_take = room;
            near_take = 0u;
        } else {
            near_take = room - far_take;
        }
    }
    uint far_cut = 0u;
    uint far_of = far_take;
    if (n_far > far_take) {
        uint left = far_take;
        for (int q = 63; q >= 0; q--) {
            uint hq = hist_far[uint(q)];
            if (hq >= left) {
                far_cut = uint(q);
                far_of = left;
                break;
            }
            left -= hq;
        }
    }
    uint near_cut = 0u;
    uint near_of = near_take;
    if (n_near > near_take) {
        uint left = near_take;
        for (int q = 63; q >= 0; q--) {
            uint hq = hist_near[uint(q)];
            if (hq >= left) {
                near_cut = uint(q);
                near_of = left;
                break;
            }
            left -= hq;
        }
    }
    uint ranked = 0u;
    uint far_got = 0u;
    uint near_got = 0u;
    uint far_at = 0u;
    uint near_at = 0u;
    for (uint s = 0u; s < guard; s++) {
        uint w = ckeys.key[lighting_fill_lock_at() + s];
        if ((w & 0x80000000u) == 0u || ckeys.key[s] == 0u) {
            continue;
        }
        uint q = min((w >> 16u) & 63u, 63u);
        uint level = (w >> 8u) & 255u;
        bool take = false;
        if (level == lod_max) {
            if (far_got < far_take) {
                if (n_far > far_take) {
                    if (q < far_cut) {
                        take = false;
                    } else if (q == far_cut) {
                        if (far_at < far_of) {
                            far_at++;
                            take = true;
                        }
                    } else {
                        take = true;
                    }
                } else {
                    take = true;
                }
                if (take) {
                    far_got++;
                }
            }
        } else if (near_got < near_take) {
            if (n_near > near_take) {
                if (q < near_cut) {
                    take = false;
                } else if (q == near_cut) {
                    if (near_at < near_of) {
                        near_at++;
                        take = true;
                    }
                } else {
                    take = true;
                }
            } else {
                take = true;
            }
            if (take) {
                near_got++;
            }
        }
        if (!take || n + ranked >= pick_cap) {
            continue;
        }
        ckeys.key[lighting_fill_pick_at() + n + ranked] = s;
        ranked++;
    }
    ckeys.key[lighting_fill_header_at()] = ranked;
    ckeys.key[lighting_fill_header_at() + 1u] = far_n - far_got;
    ckeys.key[lighting_fill_header_at() + 2u] = live;
    ckeys.key[LIGHTING_ARGS_AT + 15u] = n;
    ckeys.key[LIGHTING_ARGS_AT + 0u] = max((budget + 63u) / 64u, 1u);
    ckeys.key[LIGHTING_ARGS_AT + 1u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 2u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 3u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 4u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 5u] = 1u;
    ckeys.key[LIGHTING_ARGS_AT + 6u] = budget;
    ckeys.key[LIGHTING_ARGS_AT + 7u] = n;
    ckeys.key[LIGHTING_ARGS_AT + 8u] = n;
    ckeys.key[LIGHTING_ARGS_AT + 9u] = n;
    ckeys.key[LIGHTING_ARGS_AT + 10u] = n;
    gstats.rays = budget;
    gstats.due = n;
    gstats.scouts = budget - n;
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
    uint on = ckeys.key[LIGHTING_ARGS_AT + 15u];
    uint ranked = ckeys.key[lighting_fill_header_at()];
    if (r < on + ranked) {
        uint slot = ckeys.key[lighting_fill_pick_at() + r];
        if (slot >= LIGHTING_CACHE_SLOTS || ckeys.key[slot] == 0u) {
            return false;
        }
        vec3 p;
        if (!lighting_sc_texel_world(slot, p, n)) {
            return false;
        }
        origin = p + n * FILL_RAY_BIAS;
        uint rk = lighting_sc_ray_key(slot, ckeys.key[slot]);
        uint stratum = (lighting_hash(rk) + lighting_round()) % 64u;
        dir = lighting_dir(rk, n, stratum, 64u);
        return true;
    }
    uint disc = max(budget - min(on + ranked, budget), 1u);
    uint i = (r - on - ranked + lighting_round()) % disc;
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
    float k = n / (rays0 + n);
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

// One slot, one writer. Every ray that landed is on the list, then the head
// folds them with weight n / (rays + n). The hit distance is the stash trace
// wrote; direct has already overwritten the hit slot with radiance.
void lighting_fill_resolve(uint gid) {
    if (gid >= 64u) {
        return;
    }
    uint budget = lighting_fill_u(0);
    uint on = ckeys.key[LIGHTING_ARGS_AT + 15u];
    uint ranked = ckeys.key[lighting_fill_header_at()];
    uint placed = min(on + ranked, budget);
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
                if (cur < on) {
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
        lighting_fill_commit(slot, sum_e * inv, count, sum_lk * inv, sum_x * inv, sum_y * inv, sum_z * inv, saw_on, sum_sun * inv);
    }
}
