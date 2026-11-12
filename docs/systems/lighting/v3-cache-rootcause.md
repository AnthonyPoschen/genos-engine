# Surface-cache root cause (still hall)

Branch `cursor/still-converge-642e` at `9c45c72a`. Lavapipe, hall pose of `still_converge` (eye `(14.5, 1.7, 23.5)`, yaw `-0.3554`, pitch `0.08`), High, `sc_settle` 64, 320×180, 96 still frames. The trace is `Renderer::read_cache_cells`: the hash key, the slot, the stored point, the irradiance and its sample count, and the filtered field, next to the presented pixel and the CPU path tracer at the same hit.

## Root cause

Compact's lit-from offer writes the **cell centre** along the normal, not the face the texel was claimed on.

`lighting_sc_offer_hit` decodes the two tangent offsets and leaves the axis at `lighting_sc_offer_origin`, the centre of the hash cell. That centre is up to half a 12.5 cm texel off the face. When it falls behind the face, the sample point is inside the solid. Cache rays start at `position + normal * 0.02` (`lighting_direct.comp`). A centre 2.5 cm behind the face plus a 2 cm bias is still 5 mm inside the wall, so the set comes back black. The running mean (and the k² / multi-scale estimator, which trusts later sets) then drives the texel to black. More rays make it blacker. The pixel shows that texel once it has its own samples, so the blotch does not clean up.

The hall roof is the plane `y = 3`. That is a cell boundary, so the centre **is** the face. Roof texels stay on the plane and match the path reference by frame 16. A wall that is not on a boundary does not. Same material, same colour, different brightness, because one of them is being lit from inside the solid.

`9c45c72a` is not this bug. It stopped the host settle bit from cancelling the cache round, so a still frame keeps spending rays. Those rays are real. They are aimed at points inside the wall, which is why the cost went up and the picture did not.

## What the four checks showed

History is not reset. For every watched wall and roof cell, both the 12.5 cm texel and the 25 cm cell, the key and the slot were the same from the frame the cell was claimed through frame 96. The sample count climbed from 0 to about 46. There were no weight drops and no slot changes. A probe window can hold one to three other keys (`foreign`), and `copies` of our key stayed 1. Not an eviction, not a re-key, not a hash collision overwriting the cell.

The wall/roof split is not the hemisphere, the normal bucket, or a cosine term. Both store the axis normal (`+X`, `-X`, or `-Y`). `lighting_hemi_dir` for those axes has a non-negative dot with the normal. The split is whether the cell centre lands on the face:

| Point | Face | Cell centre along the normal | Where the centre sits | Stored after frame 2 | Frame 96 |
|---|---|---|---|---|---|
| Wall `(8, 98)` | x = 0.150, normal `+X` | x = 0.125 | 2.5 cm inside the solid | x = 0.125, same key `539892721` slot `355612` | weight 46, irradiance Y **0.000**, field Y 0.004, pixel Y flickers 0.17–0.31 (reference 0.322) |
| Wall `(296, 152)` | x = 16.850, normal `-X` | x = 16.875 | 2.5 cm inside the solid | x = 16.875, key `983563858` slot `809017` | weight 46, irradiance Y 0.471, pixel Y **0.018** (reference 0.151) |
| Wall `(104, 116)` | z = 17.100, normal `+Z` | z = 17.125 | 2.5 cm out in the air | z = 17.125 | weight 46, irradiance Y 1.437, pixel Y 0.166 (reference 0.278) |
| Wall `(248, 98)` | x = 17.000, normal `-X` | x = 17.000 | on the face | x = 17.000 | weight 47, irradiance Y 1.091, pixel Y 0.086 (reference 0.079) |
| Roof `(8, 8)` | y = 3.000, normal `-Y` | y = 3.000 | on the face | y = 3.000 | weight 46, irradiance Y 1.996, pixel Y **0.452** (reference 0.452) |
| Roof `(104, 8)` | y = 3.000 | y = 3.000 | on the face | y = 3.000 | pixel Y **0.510** (reference 0.509) |
| Roof `(200, 8)` | y = 3.000 | y = 3.000 | on the face | y = 3.000 | pixel Y 0.384 (reference 0.376) |
| Roof `(296, 44)` | y = 3.000 | y = 3.000 | on the face | y = 3.000 | pixel Y 0.202 (reference 0.256) |

The CPU path tracer at the same hits does **not** say the walls are black. One-bounce uniform-hemisphere irradiance Y is 0.247 at the dark wall `(8, 98)` and 0.483 at `(296, 152)`. Full outgoing (direct plus bounces) is 0.463 and 0.342. The ceiling is brighter than the walls in that tracer too (outgoing 0.43–3.67, reference display 0.26–0.51), because it sees the lamps. The engine's broken walls are the ones whose centre is behind the face. The wall that sits on a cell boundary matches the reference (0.086 against 0.079).

Rays do deposit into the cells compose reads. On the dark texel the sample count and the `updated` frame advance every frame or two, on that same key. They deposit black. The 25 cm cell at the same hit is a different key, stays on the face, and accumulates real irradiance (Y 1.22 at the dark wall, 1.16 at `(296, 152)`, weight ~47). Compose of a fine texel that has its own samples shows the texel, not that 25 cm cell. The bright cell is not what the pixel is reading. `shown` and the field stamp on the texel track the frame; on the 25 cm cell `shown` stays 0.

Nothing writes the right value at the end. At frame 96 the off-plane texel is still black (Y 0.000) with 46 samples. The pixel at `(8, 98)` sits near 0.013 through frame 48, then flickers between about 0.17 and 0.31 while the texel and its filtered field stay near 0. That flicker is the spatial filter borrowing from neighbours, not the cell converging. Roof pixels that are on the plane are right by frame 16 and do not move after that. Host settle flips true around frame 80 and the ray count stays at about 1.29 million per frame, so settle is not a second read path and it is not what finally lights the wall.

Lavapipe frame cost is not the 18.1 → 27.2 ms figure (that was the RTX 4070). The ray counter here is the same fact as the 369k rays/frame on that machine: a still view keeps tracing. From frame 8 through frame 96 the count stayed between 1.27 million and 1.31 million rays per frame.

## Fix

`lighting_sc_offer_hit` keeps the axis coordinate the claim stored. The offer still moves the point on that plane by the tangent offsets. `lighting_sc_centre` already projects the centre onto the claimer's face; the offer apply was the path that threw the projection away.

The regression is `an_off_plane_wall_texel_stays_on_the_face` in `examples/stress/tests/cache_rootcause.rs`. Before the fix, four still frames stored the wall at the cell centre:

```
face 16.8500 centre 16.8750 stored 16.8750
stored axis 16.8750 left the face 16.8500
```

After the fix, the same four frames store the face and not the centre:

```
face 16.8500 centre 16.8750 stored 16.8500
```
