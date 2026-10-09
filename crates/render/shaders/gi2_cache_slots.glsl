// GI v2 light cache sizes (gi2_cache.glsl); gi2_gpu.rs mirrors them.
const uint GI2_CACHE_SLOTS = 262144u;
const uint GI2_CACHE_SEARCH = 16u;
// Patches relit per frame, and rays per patch (a square).
const uint GI2_CACHE_BATCH = 16384u;
const uint GI2_CACHE_RAYS = 16u;
// Shadow rays per light cache ray hit when lamp picks are on (gi2_light.comp).
const uint GI2_CACHE_PICKS = 2u;
// Frames a patch lives without a read (30 s at 60 fps).
const uint GI2_CACHE_LIFE = 1800u;
