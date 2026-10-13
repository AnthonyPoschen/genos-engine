// Eight origins, each in its own uniform branch. A barrier inside a loop faults lavapipe.
if (tile_n > 0u) {
    lighting_tile_load(0u, halo, stride, nfoot, src);
    barrier();
    lighting_tile_apply(0u, kstep, halo, stride, it);
    barrier();
}
if (tile_n > 1u) {
    lighting_tile_load(1u, halo, stride, nfoot, src);
    barrier();
    lighting_tile_apply(1u, kstep, halo, stride, it);
    barrier();
}
if (tile_n > 2u) {
    lighting_tile_load(2u, halo, stride, nfoot, src);
    barrier();
    lighting_tile_apply(2u, kstep, halo, stride, it);
    barrier();
}
if (tile_n > 3u) {
    lighting_tile_load(3u, halo, stride, nfoot, src);
    barrier();
    lighting_tile_apply(3u, kstep, halo, stride, it);
    barrier();
}
if (tile_n > 4u) {
    lighting_tile_load(4u, halo, stride, nfoot, src);
    barrier();
    lighting_tile_apply(4u, kstep, halo, stride, it);
    barrier();
}
if (tile_n > 5u) {
    lighting_tile_load(5u, halo, stride, nfoot, src);
    barrier();
    lighting_tile_apply(5u, kstep, halo, stride, it);
    barrier();
}
if (tile_n > 6u) {
    lighting_tile_load(6u, halo, stride, nfoot, src);
    barrier();
    lighting_tile_apply(6u, kstep, halo, stride, it);
    barrier();
}
if (tile_n > 7u) {
    lighting_tile_load(7u, halo, stride, nfoot, src);
    barrier();
    lighting_tile_apply(7u, kstep, halo, stride, it);
    barrier();
}
