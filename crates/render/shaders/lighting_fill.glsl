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
const uint LIGHTING_FILL_TILE_AT = 2u * LIGHTING_CACHE_SLOTS - LIGHTING_FILL_TILE_CAP;
const uint LIGHTING_FILL_PICK_AT = LIGHTING_CACHE_SLOTS + 1u;

uint lighting_fill_param_at() {
    return LIGHTING_SC_SEED_PAY_AT + pc.dims.x * pc.dims.y * 8u;
}
uint lighting_fill_lock_at() {
    return lighting_fill_param_at() + LIGHTING_FILL_PARAMS;
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
    if (level >= lighting_fill_lod_max()) {
        return lighting_fill_f(6);
    }
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
    lighting_sc_forget(slot);
    cache.v[3u * slot + 1u] = vec4(0.0);
    cache.v[3u * slot + 2u] = vec4(0.0);
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

// Drop a clipmap cell past its leash. 12.5 cm detail stays; the screen stage
// owns that size (S57). A stored cell inside the leash is kept, so the create
// edge does not thrash.
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
    if (size >= base * FILL_CELL_KEEP) {
        uint level = lighting_fill_level_of_size(size);
        if (d > lighting_fill_leash(level) || d > lighting_fill_f(6)) {
            lighting_fill_free(slot);
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
    if (!lighting_fill_pixel(p, px, py)) {
        return;
    }
    uint tile = max(lighting_fill_u(1), 1u);
    uint tiles_x = (pc.dims.x + tile - 1u) / tile;
    uint ti = px / tile + (py / tile) * tiles_x;
    if (ti >= LIGHTING_FILL_TILE_CAP) {
        return;
    }
    vec4 shown_v = cache.v[3u * slot + 2u];
    vec4 est = cache.v[LIGHTING_SC_EST_AT + 2u * slot];
    float mean = max(dot(shown_v.rgb, FILL_LUMA), FILL_TINY);
    float dev = sqrt(max(est.w, 0.0));
    float err = max(0.0, dev / mean - lighting_fill_f(9));
    float close = 1.0 / max(d, lighting_fill_f(10));
    // lambda is 0 until F3. change_weight is uploaded and not read here.
    float pri = close * err;
    uint q = uint(clamp(pri * FILL_PRI_SCALE, 1.0, FILL_PRI_MAX));
    uint packed = (q << 21u) | (slot + 1u);
    atomicMax(ckeys.key[LIGHTING_FILL_TILE_AT + ti], packed);
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
        uint w = ckeys.key[LIGHTING_FILL_TILE_AT + t];
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
        uint w = t < tiles ? ckeys.key[LIGHTING_FILL_TILE_AT + t] : 0u;
        ckeys.key[LIGHTING_FILL_TILE_AT + t] = 0u;
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
        ckeys.key[LIGHTING_FILL_PICK_AT + n] = (w & 2097151u) - 1u;
        n++;
    }
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
    if (r < on) {
        uint slot = ckeys.key[LIGHTING_FILL_PICK_AT + r];
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
    uint off = max(budget - on, 1u);
    uint i = (r - on + lighting_round()) % off;
    dir = lighting_fill_fibonacci(i, off);
    origin = scene.eye.xyz;
    n = dir;
    return true;
}

float lighting_fill_shown(uint rays, uint slot) {
    float sets = float(rays) / FILL_SET_RAYS;
    if (rays > 0u) {
        sets = max(sets, 1.0);
    }
    return min(sets, lighting_sc_cap(slot));
}

void lighting_fill_commit(uint slot, vec3 lk, float cos_theta, vec3 sample_dir, bool on_screen, float sun_luma) {
    if (slot >= LIGHTING_CACHE_SLOTS || ckeys.key[slot] == 0u) {
        return;
    }
    uint mark = lighting_frame() + 1u;
    uint prev = atomicExchange(ckeys.key[lighting_fill_lock_at() + slot], mark);
    if (prev == mark) {
        return;
    }
    vec4 old = cache.v[3u * slot + 2u];
    float n = 1.0;
    vec3 e = lk * cos_theta * LIGHTING_TAU / n;
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
        float wr = LIGHTING_TAU / n;
        vec3 n0 = lk * (wr * LIGHTING_SH_Y0);
        vec3 nx = lk * sample_dir.x * (wr * LIGHTING_SH_Y1);
        vec3 ny = lk * sample_dir.y * (wr * LIGHTING_SH_Y1);
        vec3 nz = lk * sample_dir.z * (wr * LIGHTING_SH_Y1);
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
    float el = dot(lk, FILL_LUMA);
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

void lighting_fill_resolve(uint gid) {
    if (gid >= 64u) {
        return;
    }
    uint budget = lighting_fill_u(0);
    uint on = ckeys.key[LIGHTING_ARGS_AT + 15u];
    for (uint r = gid; r < budget; r += 64u) {
        uint work_ray = lighting_probe_rays() + r;
        vec4 rad = work.v[lighting_rad_at(work_ray)];
        if (lighting_ray_none(rad)) {
            continue;
        }
        vec3 lk = lighting_ray_light(rad);
        if (r < on) {
            uint slot = ckeys.key[LIGHTING_FILL_PICK_AT + r];
            vec3 p;
            vec3 n;
            if (slot >= LIGHTING_CACHE_SLOTS || !lighting_sc_texel_world(slot, p, n)) {
                continue;
            }
            uint rk = lighting_sc_ray_key(slot, ckeys.key[slot]);
            uint stratum = (lighting_hash(rk) + lighting_round()) % 64u;
            vec3 dir = lighting_dir(rk, n, stratum, 64u);
            lighting_fill_commit(slot, lk, max(dot(dir, n), 0.0), dir, true, rad.w);
            continue;
        }
        vec4 hit = work.v[lighting_hit_at(work_ray)];
        if (hit.x < 0.0) {
            continue;
        }
        vec3 dir;
        vec3 origin;
        vec3 nn;
        if (!lighting_fill_ray(work_ray, origin, dir, nn)) {
            continue;
        }
        vec3 pos = origin + dir * hit.x;
        vec3 n = lighting_unpack_normal(floatBitsToUint(hit.y));
        float d = length(pos - (scene.eye.xyz + lighting_fill_look()));
        if (d > lighting_fill_f(6)) {
            continue;
        }
        float size = lighting_fill_cell_size(lighting_fill_level(d));
        uint slot = lighting_cache_find_sized(pos, n, true, size, pos);
        if (slot == ~0u) {
            continue;
        }
        vec3 sample_dir = -dir;
        lighting_fill_commit(slot, lk, max(dot(sample_dir, n), 0.0), sample_dir, false, rad.w);
    }
}
