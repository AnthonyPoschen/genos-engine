// Compose's indirect read. Screen-probe placement, rays, and filtered SH are
// removed. The displayed indirect is the world cache. This function is the
// compose interface. The neighbour walk is out of this compile: maintenance
// requests the lattice and the drain fills it. A caller includes
// lighting_common.glsl first.
// `p` stays in the signature so compose and the sun compose keep one call.
//
// A full-trust read is the fine world colour. Anything less mixes in the
// coarser world-cache cells (two size steps, bilinear, stale and off-plane
// cells left out), or a corner's SH irradiance when SH cells are on. Trust 0
// is that coarser colour, not black, so the first accepted texel does not pop
// from black to full luminance. One flat cell would paint the grid the banner
// cut measures, so the corners blend. This does not stamp the cell and it is
// not a second neighbour gather.

const float LIGHTING_COARSE_HALF = 0.5;
// Same plane gate as lighting_sc_bilerp_cover. A step or a ledge is another surface.
const float LIGHTING_COARSE_PLANE = 0.1;

void lighting_coarse_add(vec3 q, vec3 n, float size, float bil, inout vec3 sum, inout float wsum) {
    if (!(bil > 0.0)) {
        return;
    }
    uint s = lighting_cache_find_sized(q, n, false, size, q);
    if (s == ~0u) {
        return;
    }
    if (lighting_sc_stale(s)) {
        return;
    }
    if (abs(dot(cache.v[3u * s].xyz - q, n)) > LIGHTING_COARSE_PLANE) {
        return;
    }
    vec4 v = cache.v[3u * s + 2u];
    if (v.w <= LIGHTING_CACHE_HAS_W) {
        return;
    }
    vec3 e = max(v.rgb * lighting_sc_sun_gain(s), vec3(0.0));
    if (lighting_sh_on()) {
        e = lighting_sh_tilt(s, e, cache.v[3u * s + 1u].xyz, n);
    }
    sum += bil * e;
    wsum += bil;
}

// w is 1 when this size has a fresh lit corner. A miss is w 0, not black light.
// The four corners are written out. A loop of this lookup in compose faults lavapipe.
vec4 lighting_coarse_bilerp(vec3 pos, vec3 n, float size) {
    uint ax = lighting_axis(n) >> 1u;
    vec3 ta = vec3(0.0);
    vec3 tb = vec3(0.0);
    ta[(ax + 1u) % 3u] = 1.0;
    tb[(ax + 2u) % 3u] = 1.0;
    vec2 uv = vec2(dot(pos, ta), dot(pos, tb)) / size - vec2(LIGHTING_COARSE_HALF);
    vec2 g0 = floor(uv);
    vec2 f = uv - g0;
    vec2 base = g0 + vec2(LIGHTING_COARSE_HALF);
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    vec2 c = base;
    lighting_coarse_add(pos + ta * (c.x * size - dot(pos, ta)) + tb * (c.y * size - dot(pos, tb)), n, size, (1.0 - f.x) * (1.0 - f.y), sum, wsum);
    c = base + vec2(1.0, 0.0);
    lighting_coarse_add(pos + ta * (c.x * size - dot(pos, ta)) + tb * (c.y * size - dot(pos, tb)), n, size, f.x * (1.0 - f.y), sum, wsum);
    c = base + vec2(0.0, 1.0);
    lighting_coarse_add(pos + ta * (c.x * size - dot(pos, ta)) + tb * (c.y * size - dot(pos, tb)), n, size, (1.0 - f.x) * f.y, sum, wsum);
    c = base + vec2(1.0);
    lighting_coarse_add(pos + ta * (c.x * size - dot(pos, ta)) + tb * (c.y * size - dot(pos, tb)), n, size, f.x * f.y, sum, wsum);
    if (wsum <= 0.0) {
        return vec4(0.0);
    }
    return vec4(sum / wsum, 1.0);
}

// Two size steps, written out. A loop of this lookup in compose faults lavapipe.
vec3 lighting_coarse_world(vec3 pos, vec3 n) {
    float size = lighting_cell_size(pos);
    size += size;
    vec4 e = lighting_coarse_bilerp(pos, n, size);
    if (e.w > LIGHTING_CACHE_HAS_W) {
        return e.rgb;
    }
    size += size;
    e = lighting_coarse_bilerp(pos, n, size);
    if (e.w > LIGHTING_CACHE_HAS_W) {
        return e.rgb;
    }
    return vec3(0.0);
}

vec3 lighting_screen_probe_indirect(uvec2 p, vec3 pos, vec3 n) {
    lighting_px_probe_reads = 0u;
    lighting_px_probe_skipped = 1u;
    lighting_px_probes = vec3(-1.0);
    lighting_px_world_w = 0.0;
    lighting_sc_display = true;
    vec4 world = lighting_world_irradiance(pos, n);
    lighting_px_world_w = world.w;
    lighting_sc_display = false;
#ifndef GENOS_COMPOSE_MAINT
    atomicAdd(gstats.diag[LIGHTING_DIAG_PROBE_SKIP], 1u);
#endif
    // `p` is not a spacing. A sentinel stays black.
    if (p.x == ~0u) {
        return vec3(0.0);
    }
    float trust = clamp(world.w, 0.0, 1.0);
    if (trust >= 1.0) {
        return max(world.rgb, vec3(0.0));
    }
    vec3 coarse = lighting_coarse_world(pos, n);
    return max(mix(coarse, max(world.rgb, vec3(0.0)), trust), vec3(0.0));
}
