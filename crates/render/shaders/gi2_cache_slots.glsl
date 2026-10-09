// GI v2 light cache sizes (gi2_cache.glsl); gi2_gpu.rs mirrors them.
const uint GI2_CACHE_SLOTS = 65536u;
const uint GI2_CACHE_SEARCH = 8u;
// Patches relit per frame, and rays per patch (a square).
const uint GI2_CACHE_BATCH = 8192u;
const uint GI2_CACHE_RAYS = 16u;
// Frames a patch lives without a read (30 s at 60 fps).
const uint GI2_CACHE_LIFE = 1800u;
