// Compose's indirect read. Screen-probe placement, rays, and filtered SH are
// removed. The displayed indirect is the world cache: texel detail when that
// layer has a value, otherwise the SH-L1 cell read inside the same walk.
// This function is the compose interface. PR #211 (open, not merged) still
// takes the neighbour walk out of this compile. A caller includes
// lighting_common.glsl first.
// `p` stays in the signature so compose and the sun compose keep one call.
// The walk is `lighting_world_irradiance` only. A second cell read is not
// added here.
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
    // A rejected world read stays black. Stale, back-facing, and off-plane
    // cells are not replaced by another estimator. `p` is not a spacing.
    if (p.x == ~0u) {
        return vec3(0.0);
    }
    return max(world.rgb, vec3(0.0));
}
