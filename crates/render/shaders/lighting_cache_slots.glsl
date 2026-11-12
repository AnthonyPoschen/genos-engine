// GI light cache sizes (lighting_cache.glsl); lighting_gpu.rs mirrors them.
// Specialization constant 1. 0 in surface_cache.slots keeps this default:
// four rounds of LIGHTING_SC_BASE_BATCH (65536). Texels stay one size.
// Pipeline creation overrides it with the resolved slot count.
layout(constant_id = 1) const uint LIGHTING_CACHE_SLOTS = 262144u;
const uint LIGHTING_CACHE_SEARCH = 16u;
// Patches relit per frame, and rays per patch (a square; 16 left each patch a
// fixed error of its own that probes carried onto the walls they light as
// blotches, gi-check room).
// Entries a round (S59D: 65536 of 16 rays, the 1M rays a round of 16384 x 64;
// S59E: 98304; S61A back to 65536, the world space radiance cache takes the bursts).
// S66A: the batch holds a boost (1.5x); the normal ceiling is
// LIGHTING_SC_BASE_BATCH (lighting_cache.glsl), so unused entries cost no rays.
const uint LIGHTING_CACHE_BATCH = 98304u;
// S59: 16, a quarter of a texel's 64-ray set (lighting_cache.glsl, few rays a relight).
const uint LIGHTING_CACHE_RAYS = 16u;
// Shadow rays per light cache ray hit when lamp picks are on (lighting_direct.comp).
const uint LIGHTING_CACHE_PICKS = 2u;
// S60: the screen probes' hits pick lamps too (lighting_direct.comp).
// S69C: kept probe hits relit on their stratum's frame and halfway round only.
const bool LIGHTING_KEPT_RELIGHT = true;
const bool LIGHTING_PROBE_PICKS = false; // S60A: no gain measured (light pass unchanged)
// Frames a patch lives without a read (30 s at 60 fps).
const uint LIGHTING_CACHE_LIFE = 1800u;
// ADR 0039 stage 2 SH-L1 cell storage (lighting_cache.glsl lighting_sh_put),
// a low-spec knob: true is fp16 (12 halves, 24 B a slot, 1.5 vec4 regions),
// false is fp32 (12 floats, 48 B a slot, 3 vec4 regions). lighting_gpu.rs
// LIGHTING_SH_HALF mirrors it and sizes the cache buffer from it. The SH is
// summed from the relight rays (LIGHTING_CACHE_RAYS a batch entry), no rays of
// its own, so the ray count knob is LIGHTING_CACHE_RAYS.
const bool LIGHTING_SH_HALF = true;
