// Compose's indirect read. Screen-probe placement, rays, and filtered SH are
// removed. The displayed indirect is the world cache. This function is the
// compose interface. The neighbour walk is out of this compile: maintenance
// requests the lattice and the drain fills it. A caller includes
// lighting_common.glsl first.
// `p` stays in the signature so compose and the sun compose keep one call.
//
// The displayed indirect is the 25 cm SH lattice at the true normal. The
// 12.5 cm texel, its trust mix, and the screen wavelet are not this read.
// A miss stays black: a missing cell is not a stand-in colour. This does
// not stamp the cell and it is not a second neighbour gather.
// Picture compose defines GENOS_LEVEL0_PICTURE and loads the published word.
#ifdef GENOS_LEVEL0_PICTURE
#include "lighting_cascade.glsl"
#endif

vec3 lighting_screen_probe_indirect(uvec2 p, vec3 pos, vec3 n) {
    lighting_px_probe_reads = 0u;
    lighting_px_probe_skipped = 1u;
    lighting_px_probes = vec3(-1.0);
    lighting_px_world_w = 0.0;
    lighting_px_bounce_found = false;
    lighting_sc_display = true;
    uint pixel = p.y * pc.dims.x + p.x;
    lighting_px_object_id = (p.x != ~0u && pixel < pc.dims.x * pc.dims.y) ? gbuf_object_id(pixel) : 0u;
    // Bounce is the 25 cm SH lattice at the true normal. The 12.5 cm texel,
    // its trust mix, and the screen wavelet are not this read. The claim is
    // the one cell the lattice blends. Without it the corners stay empty and
    // the picture never leaves the first frame.
    if (LIGHTING_SC_WRITES) {
        lighting_cache_find_sized(pos, n, true, LIGHTING_SC_HIT, pos);
    }
    // The picture reads the published level-0 merge. Maintenance, which does
    // not define GENOS_LEVEL0_PICTURE, still requests the lattice that the
    // publish pass reads. A hash walk in this compile faults lavapipe.
#ifdef GENOS_LEVEL0_PICTURE
    vec4 world = lighting_gi_on() ? lighting_cascade_published(pixel) : vec4(0.0);
#else
    vec4 world = lighting_sh_compose_lattice(pos, n, false);
#endif
    lighting_px_object_id = 0u;
    lighting_px_world_w = world.w;
    lighting_sc_display = false;
#ifndef GENOS_COMPOSE_MAINT
    atomicAdd(gstats.diag[LIGHTING_DIAG_PROBE_SKIP], 1u);
#endif
    // `p` is not a spacing. A sentinel stays black.
    if (p.x == ~0u) {
        return vec3(0.0);
    }
    if (!(world.w > LIGHTING_CACHE_HAS_W)) {
        return vec3(0.0);
    }
    lighting_px_bounce_found = true;
    return max(world.rgb, vec3(0.0));
}
