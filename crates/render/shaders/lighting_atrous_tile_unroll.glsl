// Eight origins, straight-line. Lavapipe faults on a barrier inside a loop and
// on a barrier inside an if, even when every thread takes the same branch.
// Unused slots are ~0u, and load/apply return before touching the footprint.
lighting_tile_load(0u, halo, stride, nfoot, src);
barrier();
lighting_tile_apply(0u, kstep, halo, stride, it);
barrier();
lighting_tile_load(1u, halo, stride, nfoot, src);
barrier();
lighting_tile_apply(1u, kstep, halo, stride, it);
barrier();
lighting_tile_load(2u, halo, stride, nfoot, src);
barrier();
lighting_tile_apply(2u, kstep, halo, stride, it);
barrier();
lighting_tile_load(3u, halo, stride, nfoot, src);
barrier();
lighting_tile_apply(3u, kstep, halo, stride, it);
barrier();
lighting_tile_load(4u, halo, stride, nfoot, src);
barrier();
lighting_tile_apply(4u, kstep, halo, stride, it);
barrier();
lighting_tile_load(5u, halo, stride, nfoot, src);
barrier();
lighting_tile_apply(5u, kstep, halo, stride, it);
barrier();
lighting_tile_load(6u, halo, stride, nfoot, src);
barrier();
lighting_tile_apply(6u, kstep, halo, stride, it);
barrier();
lighting_tile_load(7u, halo, stride, nfoot, src);
barrier();
lighting_tile_apply(7u, kstep, halo, stride, it);
barrier();
