// Compose's screen-probe indirect. Included only by the compose pipelines and
// the lavapipe fixture. It is not in lighting_common.glsl: that file is in
// every lighting shader, and this walk is the compose pixel loop.
// A caller includes lighting_common.glsl first.
// True when the displayed indirect is the neighbour read and nothing else.
// An axis pixel needs every weighted fine-texel corner to be an empty world
// texel whose neighbour read has a corner. A slant has no fine texel: flag
// 2048 returns that same read, and the skip is that read having a corner.
// Own samples, a parent, a resample, a miss, and a trust gate that hands the
// pixel back to the probes are not this. Trust is not the test. A mover is
// this when the world-cell read at its current position and true normal
// accepts a corner: that read is what the pixel shows, and it does not seed.
// cells_only is not this test. The toggle is applied later, inside
// lighting_world_irradiance_in, so a cells_only pixel can still count as a
// skip here and then show lighting_sh_read rather than the texel seed.
// Defined after lighting_probe_irradiance: the cache include is earlier.
bool lighting_sh_texel_detail_covers(vec3 pos, vec3 n, vec3 wpos, bool obj) {
    if (lighting_world_mode() != 3u) {
        return false;
    }
    if (obj) {
        // The mover arm shows lighting_sh_read and does not seed a world texel.
        // Detail off and Low stay on the probes. Claim stays false.
        float salt = lighting_obj_salt;
        lighting_obj_salt = 0.0;
        bool hit = lighting_sh_texel_detail_on()
            && lighting_sh_read(pos, n, false).w > LIGHTING_CACHE_HAS_W;
        lighting_obj_salt = salt;
        return hit;
    }
    if (lighting_sc_slanted(n)) {
        // The screen arm shows this read with or without the texel-detail bit.
        // Low is not world mode 3.
        return lighting_sh_on() && lighting_sh_read(pos, n, false).w > LIGHTING_CACHE_HAS_W;
    }
    if (!lighting_sh_texel_detail_on() || !lighting_sc_seed_split()) {
        return false;
    }
    // The same gates lighting_world_irradiance_in uses to drop trust to 0, or
    // to share an empty texel with the probes. Skipping the probe read is only
    // valid when the cell value is what the pixel shows.
    if ((lighting_sc_flags() & (32u | 512u)) != 0u) {
        return false;
    }
    if (!lighting_sc_light_quiet() && (lighting_sc_flags() & 1048576u) == 0u) {
        return false;
    }
    if (!LIGHTING_SC_KEEP_NEAR && lighting_near_change(pos)) {
        return false;
    }
    float size = lighting_cell_size(wpos);
    if (size > 0.2) {
        return false;
    }
    uint ax = lighting_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    vec2 uv = vec2(dot(pos, ta), dot(pos, tb)) / size - 0.5;
    vec2 g0 = floor(uv);
    vec2 f = uv - g0;
    bool any = false;
    [[dont_unroll]] for (uint k = 0u; k < 4u; k++) {
        float bil = ((k & 1u) == 1u ? f.x : 1.0 - f.x) * ((k >> 1u) == 1u ? f.y : 1.0 - f.y);
        if (bil <= 0.0) {
            continue;
        }
        vec2 c = g0 + vec2(float(k & 1u), float(k >> 1u)) + 0.5;
        vec3 q = pos + ta * (c.x * size - dot(pos, ta)) + tb * (c.y * size - dot(pos, tb));
        uint slot = lighting_cache_find_sized(q, n, false, size, pos);
        if (slot == ~0u || cache.v[3u * slot + 2u].w >= LIGHTING_CACHE_HAS_W || lighting_sc_stale(slot)) {
            return false;
        }
        if (lighting_sh_read(q, n, false).w <= LIGHTING_CACHE_HAS_W) {
            return false;
        }
        any = true;
    }
    return any;
}

// Compose's screen-probe indirect. A covered pixel does not read probe SH:
// the neighbour read is the displayed indirect, so the probe read cannot
// change it. A miss, a mover whose world-cell read rejects, Low, and a slant
// whose read is empty still read the probes. Place, trace, gather, and
// filter stay.
vec3 lighting_screen_probe_indirect(uvec2 p, vec3 pos, vec3 n) {
    lighting_px_probe_reads = 0u;
    lighting_px_probe_skipped = 0u;
    lighting_px_world_w = 0.0;
    uint mode = lighting_world_mode();
    uint om = mode == 3u ? lighting_mover_on(pos, n) : ~0u;
    bool covered = mode != 0u && lighting_sh_texel_detail_covers(pos, n, pos, om != ~0u);
    float tile = float(pc.params.y);
    vec2 g = (vec2(p) + 0.5) / tile - 0.5;
    ivec2 g0 = ivec2(floor(g));
    vec2 fr = g - vec2(g0);
    float dist = length(pos - scene.eye.xyz);
    float plane = 0.03 + 0.01 * dist;
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    ivec2 grid = ivec2(pc.dims.zw);
    if (!covered) {
        [[dont_unroll]] for (uint k = 0u; k < 4u; k++) {
            ivec2 c = g0 + ivec2(int(k & 1u), int(k >> 1u));
            if (any(lessThan(c, ivec2(0))) || any(greaterThanEqual(c, grid))) {
                continue;
            }
            uint probe = uint(c.y) * pc.dims.z + uint(c.x);
            vec4 pp = work.v[lighting_probe_at(probe)];
            if (pp.w < 0.5) {
                continue;
            }
            vec3 pn = work.v[lighting_probe_at(probe) + 1u].xyz;
            float bil = ((k & 1u) == 1u ? fr.x : 1.0 - fr.x) * ((k >> 1u) == 1u ? fr.y : 1.0 - fr.y);
            float w = (bil + 1.0e-3) * pow(max(dot(pn, n), 0.0), 8.0)
                * exp(-abs(dot(pp.xyz - pos, n)) / plane);
            if (w <= 1.0e-6) {
                continue;
            }
            sum += w * lighting_probe_irradiance(probe, n);
            wsum += w;
            lighting_px_probe_reads += 1u;
        }
        // Displaying compose only. Maintenance calls this function on the same
        // pixels and must not add, or one frame pixel becomes two counts.
        // diag[0] is a covers() skip, which cells_only can take before the
        // picture shows lighting_sh_read. A fixture total is one dispatch.
#ifndef GENOS_COMPOSE_MAINT
        atomicAdd(gstats.diag[LIGHTING_DIAG_PROBE_EVAL], 1u);
        atomicAdd(gstats.diag[LIGHTING_DIAG_PROBE_READS], lighting_px_probe_reads);
#endif
    } else {
        lighting_px_probes = vec3(-1.0);
        lighting_px_probe_skipped = 1u;
#ifndef GENOS_COMPOSE_MAINT
        atomicAdd(gstats.diag[LIGHTING_DIAG_PROBE_SKIP], 1u);
#endif
    }
    const float CACHE_WEIGHT = 0.05;
    lighting_sc_display = true;
    // The hit read does not apply the neighbour gates. With SH cells on, a
    // weak probe must not be filled with a cell the neighbour read rejected.
    if (!covered && wsum < CACHE_WEIGHT && !lighting_sh_on()) {
        float w = CACHE_WEIGHT - wsum;
        sum += w * lighting_cache_irradiance(pos, n);
        wsum += w;
    }
    vec3 probes = sum / max(wsum, 1.0e-20);
    if (mode != 0u) {
        if (!covered) {
            lighting_px_probes = wsum > CACHE_WEIGHT + 1.0e-4 ? probes : vec3(-1.0);
        }
        vec4 world = lighting_world_irradiance(pos, n);
        lighting_px_world_w = world.w;
        lighting_sc_display = false;
        if (covered) {
            return world.rgb;
        }
        return mix(probes, world.rgb, clamp(world.w, 0.0, 1.0));
    }
    lighting_sc_display = false;
    return probes;
}
