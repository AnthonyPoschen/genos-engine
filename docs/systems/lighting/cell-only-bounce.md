# Cell-only bounce

Decided 19 Nov 2026. This note is the intent. The commits that follow it are the code.

Bounce comes only from world cells. A pixel reads that bounce through the SH-L1 lattice, at the true normal. 25 cm cells are the finest level. Shadow edges stay exact. Bounce is smooth, stable, and mostly right across a surface. Pixel-perfect bounce is not the bar.

## What this removes

- The 12.5 cm texel pool as its own bounce store. The fine band that created those texels goes with it. SH texel detail, the ray cut that served that pool, and the trust mix `sets / (sets + 6)` go with it.
- The screen denoiser (the à-trous wavelet) and its hold. Bounce is smooth at the cell, so the picture does not filter it.
- The MSME / firefly estimator on the bounce path. A cell keeps one mean and its SH-L1.
- The separate mover texel pool. A mover is world cells that travel with the object, under the same history rule as a still cell.
- The fixed cap of four exact lamps. A full history reset, the opening full replace, the catch that refuses to move a mean, and the sample refuse that drops a usable ray.

Direct light has no denoiser. A shadow line is the shadow ray.

## One history rule

A cell never wipes its mean and starts from zero.

When the stored mean is junk, the cell adopts the confirming data as the new mean and a short count. That data is a fresh ray, a neighbour cell, or the parent cell, whichever one is usable.

When the stored mean and the fresh ray are both usable, the blend is by confidence. The ray's share is one over the count plus the ray. A ray that disagrees with the mean shortens that count in proportion to how far it sits outside the spread the cell already stored. Inside that spread the count is not shortened.

## LOD blend

A finer cell's weight grows with its own sets until it takes the pixel over. The blend is continuous. A coarse cell and a fine cell do not pop.

## Leak gates

Light does not pass through a surface, and it does not land where it cannot.

A parent cell is refused when it is stale, when its normal faces the opposite way, or when it sits more than half a cell off the surface plane. The lattice read keeps the same gates it already has: stale, back-facing, and more than half a cell off the plane.

## Exact lamps

Every lamp that meaningfully reaches a pixel gets one exact shadow ray. Meaningful is an importance cutoff on the light tree, not a count of lamps. Lamps below that cutoff are the weak remainder. That remainder stays one stochastic sample and keeps its history. It is not a second full sum on top of the exact lamps.

The knockout that removes the exact path stays. With that path removed, the pixel is the reservoir again, so the moving-lamp wall still grains.

## Networking

- Replicated state: none. Cells, the exact lamp sum, and the stochastic remainder stay on the machine that shades.
- Authority: the machine that shades.
- Prediction / rollback: none.
- Determinism: the same pose, the same lamps, and the same cell keys produce the same exact sum and the same adopted mean.
- Headless server: a server that does not shade leaves the cells uncomputed.
