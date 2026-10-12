// Lamp loop and write for one direct-light point. Included in
// lighting_direct.comp's main (the shipped shader) or in lighting_direct_shade
// (the hit-light variant). One trace_occluded call. The includer declares the
// point's locals: idx, pixels, kept, fill_hit, probe_hit, h, ray, pos, n,
// n_hit, origin, hit_t, reflectance, two_sided, glow, list, count, tdl,
// tdl_sig, pix_sig.
    // Light cache rays (params.z bit 11): at most LIGHTING_CACHE_PICKS shadow rays,
    // lamps picked systematically in proportion to their unshadowed light, each
    // weighted by how often it is expected to be picked (unbiased, as
    // lamps_light_only). The pick offset hangs on the patch and the ray, so a
    // still scene relights to the same value.
    // S60: the screen probes' hits (kept strata and fills) pick lamps the same way
    // (LIGHTING_PROBE_PICKS): every kept hit of all LIGHTING_STRATA strata is relit every
    // frame, so with every lamp shadow-traced the light pass was ~65 % of the
    // frame (GENOS_GPU_TIMES). Lumen's final gather never shadow-traces lamps at
    // probe hits (it reads the surface cache's lit cards); stochastic lamp
    // selection in proportion to unshadowed light (as MegaLights / ReSTIR DI) is
    // the step that keeps our per-hit direct light.
    bool cache_pick = idx >= pixels + kept && !fill_hit && ((pc.params.z >> 11u) & 1u) != 0u;
    bool probe_pick = LIGHTING_PROBE_PICKS && probe_hit && ((pc.params.z >> 11u) & 1u) != 0u;
    bool tdl_check = tdl != ~0u && (tdl & 0x40000000u) != 0u;
    bool pick = ((cache_pick || probe_pick) && tdl == ~0u) || tdl_check;
    float total = 0.0;
    uint lit = 0u;
    float u = 0.0;
    if (pick) {
        if (cache_pick) {
            uint r = ray - lighting_probe_rays();
            uint ent = r / LIGHTING_CACHE_RAYS;
            if (lighting_world_mode() == 3u && ent >= lighting_grad_first() && ent < LIGHTING_CACHE_BATCH - LIGHTING_WRC_SCOUT_ENTRIES) {
                // S74A gradient check: the stored ray's lamp pick (same seed).
                uint gs = ckeys.key[LIGHTING_GRAD_LIST_AT + r - lighting_grad_first() * LIGHTING_CACHE_RAYS];
                u = lighting_unit(ckeys.key[gs] ^ (floatBitsToUint(cache.v[LIGHTING_SC_GRAD_AT + gs].w) * 0x85ebca6bu + 0x5bd1e995u));
            } else {
            uint slot = lighting_cache_batch_slot(ent);
            uint key = slot == ~0u ? 0u : ckeys.key[slot];
            uint q = slot == ~0u ? 0u : lighting_cache_entry_quarter(ent, slot);
            u = lighting_unit(key ^ ((q * LIGHTING_CACHE_RAYS + r % LIGHTING_CACHE_RAYS) * 0x85ebca6bu + 0x5bd1e995u));
            }
        } else if (tdl_check) {
            u = lighting_unit(lighting_hash((tdl & 0x3FFFFFFFu) ^ lighting_frame() * 0x9e3779b9u));
        } else {
            // A probe hit: the offset hangs on the hit's 25 cm cell and its index,
            // so a still hit relights to the same value (no crawl).
            u = lighting_unit(lighting_cell_hash(pos, n) ^ (h * 0x85ebca6bu + 0x27d4eb2fu));
        }
        [[dont_unroll]] for (uint k = 0u; k < count; k++) {
            vec3 target;
            vec3 light = lamp_light(lamp_list_get(list, k), origin, n, two_sided, target);
            float w = light.r + light.g + light.b;
            total += w;
            lit += w > 0.0 ? 1u : 0u;
        }
    }
    bool exact = !pick || lit <= LIGHTING_CACHE_PICKS;
    float step = total / float(LIGHTING_CACHE_PICKS);
    float run = 0.0;
    vec3 e = vec3(0.0);
    vec3 e_sun = vec3(0.0);
    [[dont_unroll]] for (uint k = 0u; k < count; k++) {
        if (idx < pixels) {
            lighting_dp(7u, 1u);
            uint pli = lamp_list_index(list, k);
            if (pli < LIGHTING_WRC_SNAP_LAMPS && pli < scene.lamp_count) {
                uint eat = LIGHTING_SHADOW_EPOCH_AT + 8u * pli;
                if (ckeys.key[eat] == ckeys.key[eat + 6u]) {
                    lighting_dp(6u, 1u);
                }
            }
        }
        vec3 target;
        vec3 light = lamp_light(lamp_list_get(list, k), origin, n, two_sided, target);
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
        // S74A: hits (not pixels) reuse a static lamp's shadow (lighting_cache.glsl).
        bool shadow_lit = false;
        if (times > 0.0) {
            uint li = idx >= pixels ? lamp_list_index(list, k) : LAMP_ALL;
            float sc = lighting_world_mode() == 3u && li < LAMP_ALL - 1u ? lighting_shadow_get(origin, n, li) : -1.0;
            if (sc >= 0.0) {
                shadow_lit = sc > 0.5;
            } else {
#if DP_DIVERGE != 0
                if (idx >= pixels && lighting_dp_diverge_skip(idx)) {
                    shadow_lit = true;
                } else
#endif
                {
                    lighting_dp(idx >= pixels ? 0u : 1u, 1u);
                    shadow_lit = !trace_occluded(origin, target);
                    if (li < LAMP_ALL - 1u && lighting_world_mode() == 3u) {
                        lighting_shadow_put(origin, n, li, shadow_lit);
                    }
                }
            }
        }
        if (shadow_lit) {
            vec3 add = exact ? light : light * (times * step / w);
            e += add;
            if (k < list.suns.y) {
                e_sun += add;
            }
        }
    }
    n = n_hit;
    if ((lighting_toggles() & LIGHTING_TOGGLE_BOUNCE_ONLY) != 0u) {
        e = vec3(0.0);
        e_sun = vec3(0.0);
    }
    if (tdl != ~0u && (lighting_toggles() & LIGHTING_TOGGLE_BOUNCE_ONLY) == 0u) {
        uint ts = tdl & 0x3FFFFFFFu;
        if (tdl_check) {
            // Confirm: within LIGHTING_TDL_TOL of the cached value -> keep it,
            // nudged towards the check (an unbiased estimate, so drift is
            // tracked), under the new signature. Else escalate: the entry is
            // dropped and the next use recomputes it exactly (budget); this
            // hit takes the check's estimate.
            vec4 d = cache.v[LIGHTING_TDL_AT + ts];
            vec3 c = lighting_unpack_rgb9e5(floatBitsToUint(d.x));
            float lc = dot(c, vec3(0.2126, 0.7152, 0.0722));
            float le = dot(e, vec3(0.2126, 0.7152, 0.0722));
            if (abs(le - lc) <= LIGHTING_TDL_TOL * max(max(le, lc), 1.0e-3)) {
                lighting_dg(48u, 1u);
                vec3 m = mix(c, e, 0.25);
                cache.v[LIGHTING_TDL_AT + ts] = vec4(uintBitsToFloat(lighting_pack_rgb9e5(m)), d.y, uintBitsToFloat(tdl_sig), uintBitsToFloat(lighting_frame()));
                e = m;
                e_sun = m * d.y;
            } else {
                lighting_dg(49u, 1u);
                cache.v[LIGHTING_TDL_AT + ts].z = 0.0;
            }
        } else if ((tdl & 0x80000000u) != 0u) {
            vec4 d = cache.v[LIGHTING_TDL_AT + ts];
            e = lighting_unpack_rgb9e5(floatBitsToUint(d.x));
            e_sun = e * d.y;
        } else {
            float le = dot(e, vec3(0.2126, 0.7152, 0.0722));
            float fs = le > 0.0 ? clamp(dot(e_sun, vec3(0.2126, 0.7152, 0.0722)) / le, 0.0, 1.0) : 0.0;
            cache.v[LIGHTING_TDL_AT + ts] = vec4(uintBitsToFloat(lighting_pack_rgb9e5(e)), fs, uintBitsToFloat(tdl_sig), uintBitsToFloat(lighting_frame()));
        }
    }
    if (idx < pixels) {
        uint emitted = gbuf.px[idx].w;
        outc.px[idx] = emitted == ~0u ? 0u : emitted;
        gbuf.px[idx].w = lighting_pack_rgb9e5(e);
        if (LIGHTING_PIX_REUSE && lighting_world_mode() == 3u) {
            uint c = (idx & 1u) * 2u;
            work.v[lighting_pix_at() + idx / 2u][c] = uintBitsToFloat(pix_sig);
            work.v[lighting_pix_at() + idx / 2u][c + 1u] = uintBitsToFloat(gbuf.px[idx].w);
        }
    } else {
        lighting_hit_sunfrac = 0.0;
        lighting_bounce_path = 2u;
        vec3 bounce = lighting_cache_irradiance_at(pos, n, hit_t);
        e += bounce;
        e_sun += lighting_hit_sunfrac * bounce;
        if (fill_hit) {
            lighting_put_fill_light(h, reflectance * e + glow);
        } else if (probe_hit && lighting_check_within_noise(h, pos, n, reflectance, reflectance * e + glow)) {
            lighting_chg(CHG_KEPT_HELD, 1u);
        } else if (probe_hit) {
            lighting_put_kept_light_d(h, reflectance * e + glow, tdl == ~0u ? 2u : ((tdl & 0x80000000u) != 0u ? 0u : 1u));
        } else {
            // w: its sun/sky share as luminance (S63A).
            work.v[lighting_rad_at(ray)] = vec4(reflectance * e + glow, dot(reflectance * e_sun, vec3(0.2126, 0.7152, 0.0722)));
        }
    }
