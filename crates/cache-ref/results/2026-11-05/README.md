# cache-ref results, 5 Nov 2026

CPU reference runs behind ADR 0039 addendum A. Reference: path tracer with
NEE and Russian roulette, 1024 spp, pixels 0.05 m. Byte and MiB figures are
estimates. Commands (from the repo root, release build):

```text
cache-ref compare  --spp 1024 --seeds 11,12,13 --out /tmp/cr-c1
cache-ref variants --spp 1024 --seeds 11,12,13 --out /tmp/cr-c1 \
    --configs "axis+norm;axis+norm+compact;axis+pos+wiener+compact;axis+pos+wiener+norm+compact;axis+pos+wiener+norm+compact+vis"
cache-ref subdiv   --spp 1024 --seeds 11,12 --frames 600 --out /tmp/cr-c1
cache-ref sweep    --spp 1024 --seeds 11,12 --cells 0.30,0.25,0.21,0.177,0.125,0.088,0.0625 --out /tmp/cr-c1
```

The subdiv run's lines say "dolly 300, then held" and "pop max is over those
600 frames"; the static rows' blotch is over the same split-pixel mask.
