// Direct-pass setup through the TDL decision. Included as the body of
// lighting_direct.comp's main. A return leaves that pass. The lamp loop is
// lighting_direct_shade.glsl, so each compiled variant has one trace site.
    uint idx = gl_GlobalInvocationID.x;
    uint pixels = pc.dims.x * pc.dims.y;
    uint mode = lighting_mode();
    uint kept = lighting_kept_hits();
    uint cache_rays = LIGHTING_CACHE_BATCH * LIGHTING_CACHE_RAYS;
    if (lighting_dense()) {
        // S70A: this round's light cache rays only, live entries only.
        if (idx >= lighting_dense_count() * LIGHTING_CACHE_RAYS) {
            return;
        }
        idx = pixels + kept + lighting_dense_entry(idx / LIGHTING_CACHE_RAYS) * LIGHTING_CACHE_RAYS + idx % LIGHTING_CACHE_RAYS;
    } else if (mode == 2u) {
        idx += pixels + kept;
    }
    // Then (not on the async queue) one point per fill slot (lighting_fill_ray).
    uint fills = pixels + kept + (mode == 1u ? 0u : cache_rays);
    uint points = fills + (mode == 2u ? 0u : kept);
    if (idx >= points) {
        return;
    }
    // A kept probe hit (index h), a light cache ray (ray) or a fill (index h).
    bool fill_hit = idx >= fills;
    bool probe_hit = (idx >= pixels && idx < pixels + kept) || fill_hit;
    uint h = fill_hit ? idx - fills : idx - pixels;
    uint ray = idx < pixels + kept || fill_hit ? 0u : lighting_probe_rays() + (idx - pixels - kept);
    vec3 pos;
    vec3 n;
    float hit_t = 1.0e9;
    vec3 reflectance = vec3(1.0);
    bool two_sided = false;
    vec3 glow = vec3(0.0);
    if (idx < pixels) {
        vec3 albedo;
        if (!lighting_surface(uvec2(idx % pc.dims.x, idx / pc.dims.x), pos, n, albedo, two_sided)) {
            return;
        }
    } else {
        vec4 rec;
        vec3 origin;
        vec3 dir;
        vec3 from_n;
        bool live;
        if (probe_hit) {
            uint rays = lighting_rays();
            uint ps = h / rays;
            if (fill_hit) {
                live = lighting_fill_ray(ps / LIGHTING_STRATA, ps % LIGHTING_STRATA, h % rays, origin, dir, from_n);
                if (!live) {
                    return;
                }
                rec = work.v[lighting_fill_at(h)];
                if (rec.x < -0.75) {
                    lighting_put_fill_light(h, has_sky() ? scene.sky.rgb : vec3(0.0));
                    return;
                }
            } else {
                rec = work.v[lighting_kept_at(h)];
                live = lighting_stratum_ray(ps / LIGHTING_STRATA, ps % LIGHTING_STRATA, h % rays, origin, dir, from_n);
                if (live && rec.x < -0.75) {
                    lighting_put_kept_light_d(h, has_sky() ? scene.sky.rgb : vec3(0.0), 2u);
                    return;
                }
            }
        } else {
            rec = work.v[lighting_hit_at(ray)];
            live = lighting_ray(ray, origin, dir, from_n);
        }
        if (rec.x < 0.0 || !live) {
            return;
        }
        pos = origin + dir * rec.x;
        hit_t = rec.x;
        n = lighting_unpack_normal(floatBitsToUint(rec.y));
        reflectance = unpackUnorm4x8(floatBitsToUint(rec.z)).rgb * (1.0 / LIGHTING_PI);
        glow = lighting_unpack_rgb9e5(floatBitsToUint(rec.w));
    }
    // S69C (Anthony: build-up starved; GENOS_GPU_TIMES, default stress scene at
    // 1920x1080: this pass was 70 % of the frame and the kept hits of all
    // LIGHTING_STRATA strata, relit every frame, ~10 ms of it). A kept hit is relit
    // the frame its stratum is traced and once more halfway round
    // (LIGHTING_KEPT_RELIGHT), so its light is at most LIGHTING_STRATA / 2 frames old;
    // near a change (a mover, re-traced kept rays) every frame as before.
    if (LIGHTING_KEPT_RELIGHT && probe_hit && !fill_hit) {
        uint s = (h / lighting_rays()) % LIGHTING_STRATA;
        uint cur = lighting_stratum();
        bool urgent = ((pc.params.z >> 5u) & 1u) != 0u;
        if (s != cur && s != (cur + LIGHTING_STRATA / 2u) % LIGHTING_STRATA && !(urgent && lighting_near_change(pos))) {
            return;
        }
        // S78D: the halfway relight only where the light should have moved: a
        // lamp that moved reaches the hit since its trace, or the sun / sky
        // changed by more than 1 % (a change near a mover is relit above).
        // S78K: a held probe's current stratum: only the rays re-traced this
        // frame (its check, change crossings) are new hits; the rest are lit
        // as the halfway relight below.
        uint pk = h / lighting_rays() / LIGHTING_STRATA;
        bool held_old = s == cur && lighting_stratum_held(pk);
        if (held_old) {
            vec3 ho;
            vec3 hd;
            vec3 hnn;
            lighting_stratum_ray(pk, s, h % lighting_rays(), ho, hd, hnn);
            held_old = !lighting_held_traced(pk, h % lighting_rays(), ho, hd, hit_t);
            // S78U: the check ray whose trace confirmed its hit is relit like
            // the other held hits (only when a lamp / the sun moved): a drift
            // of the shared surface cache is no reason to change the probe.
            if (!held_old && LIGHTING_CHECK_NOISE && h % lighting_rays() == lighting_hold_check_k(pk)
                && floatBitsToUint(work.v[lighting_probe_at(pk) + 1u].w) != lighting_frame() + LIGHTING_STRATA
                && !lighting_held_crosses(ho, hd, hit_t)) {
                held_old = true;
            }
        }
        if (LIGHTING_KEPT_SHOULD && (s != cur || held_old) && lighting_world_mode() == 3u && !(urgent && lighting_near_change(pos))) {
            uint traced = lighting_frame() - LIGHTING_STRATA / 2u;
            if (!lighting_wrc_lamp_changed(pos, traced) && abs(lighting_ls_ratio(traced) - 1.0) <= 0.01) {
                return;
            }
        }
    }
    vec3 origin = pos + n * 0.02;
    LampList list = lamps_at(origin);
    uint count = lamp_list_size(list);
    // S78J: a pixel whose point and lamps match last frame's, in a quiet frame.
    uint pix_sig = 0u;
    if (LIGHTING_PIX_REUSE && idx < pixels && lighting_world_mode() == 3u) {
        pix_sig = lighting_pix_sig(pos, n, two_sided, list);
        vec4 pv = work.v[lighting_pix_at() + idx / 2u];
        uint c = (idx & 1u) * 2u;
        if (pix_sig != 0u && floatBitsToUint(pv[c]) == pix_sig && lighting_pix_quiet()) {
            uint emitted = gbuf.px[idx].w;
            outc.px[idx] = emitted == ~0u ? 0u : emitted;
            gbuf.px[idx].w = floatBitsToUint(pv[c + 1u]);
            return;
        }
    }
    // S75 Lumen surface-cache direct lighting for hits (lighting_cache.glsl).
    uint tdl = ~0u;
    uint tdl_sig = 0u;
    vec3 n_hit = n;
    if (LIGHTING_TDL && idx >= pixels && lighting_world_mode() == 3u) {
        uint ts = lighting_cache_find_sized(pos, n, false, LIGHTING_SC_TEXEL, pos);
        vec3 tp;
        vec3 tn;
        if (ts == ~0u) {
            lighting_dp(4u, 1u);
        } else if (lighting_sc_texel_world(ts, tp, tn)) {
            LampList tl = lamps_at(tp + tn * 0.02);
            tdl_sig = lighting_tdl_sig(ts, tl);
            vec4 d = cache.v[LIGHTING_TDL_AT + ts];
            if (tdl_sig != 0u && floatBitsToUint(d.z) == tdl_sig && lighting_frame() - floatBitsToUint(d.w) < LIGHTING_TDL_STALE) {
                count = 0u;
                tdl = ts | 0x80000000u;
                lighting_dg(40u, 1u);
            } else if (LIGHTING_TDL_CHECK && tdl_sig != 0u && floatBitsToUint(d.z) != 0u) {
                // S78E: an entry past LIGHTING_TDL_STALE (same signature) is checked
                // like a changed one, not recomputed: age alone is no reason.
                // S76C: a lamp in range changed (the signature moved): a check,
                // the texel's light with picked lamps (LIGHTING_CACHE_PICKS shadow
                // rays) at its centre, against the cached value.
                lighting_dg(47u, 1u);
                tdl = ts | 0x40000000u;
                origin = tp + tn * 0.02;
                n = tn;
                list = tl;
                count = lamp_list_size(list);
            } else if (tdl_sig != 0u) {
                if (atomicAdd(ckeys.key[LIGHTING_TDL_COUNT_AT], 1u) < LIGHTING_TDL_BUDGET) {
                    lighting_dg(41u, 1u);
                    tdl = ts;
                    origin = tp + tn * 0.02;
                    n = tn;
                    list = tl;
                    count = lamp_list_size(list);
                } else {
                    lighting_dp(5u, 1u);
                }
            }
        }
    }
