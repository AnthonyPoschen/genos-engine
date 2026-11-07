## Stage 2c subdivision on the dolly

Parent cell 0.250 m, children 0.125 m, 16 rays/cell/frame, 160 warm frames at the start pose, then 600 frames (dolly 300, then held); pop max is over those 600 frames, seeds [11, 12]. Subdiv defaults: Subdiv { split_dist: 1.5, hysteresis: 1.3, tau: Some(0.5), n_split: 128.0, merge_frames: 30, min_dwell: 30, seed: Model, seed_n: 8.0, fade_k: 32.0, boost_min_n: 64.0, boost_rays: 16 }.

| config | B/cell est | energy | blotch (split pixels) | pop max | pop steady | splits | merges | toggles after hold | catch-up rays | peak cells | rays |
|---|---|---|---|---|---|---|---|---|---|---|---|
| static 2b, parent size | 34 | 0.999 | 0.0170 | 0.0276 | 0.0217 | 0 | 0 | 0 | 0 | 1271 | 15455360 |
| static 2b, child size | 34 | 0.994 | 0.0125 | 0.0232 | 0.0205 | 0 | 0 | 0 | 0 | 4889 | 59450240 |
| 2c, zero-seeded children | 34 | 0.998 | 0.0124 | 0.0344 | 0.0205 | 334 | 158 | 0 | 40096 | 2187 | 21442120 |
| 2c, parent-copy seed | 34 | 0.998 | 0.0123 | 0.0334 | 0.0205 | 332 | 156 | 0 | 39872 | 2187 | 21434904 |
| 2c, 2b-model seed | 34 | 0.998 | 0.0123 | 0.0330 | 0.0205 | 332 | 156 | 0 | 39872 | 2187 | 21434928 |
| 2c, 2b-model seed worth 32 samples | 34 | 0.998 | 0.0124 | 0.0317 | 0.0206 | 332 | 156 | 0 | 28144 | 2187 | 21417760 |

### Skewed origins (selection skew 8, 1/q weighted)

| config | energy | blotch (split pixels) |
|---|---|---|
| 2c, parent-copy seed | 0.998 | 0.0119 |
| 2c, 2b-model seed | 0.998 | 0.0119 |

### lamp-off with subdivision

| config | lag | linger at +120 |
|---|---|---|
| static 2b | 0.994 | 0.0057 |
| 2c, 2b-model seed | 0.993 | 0.0050 |

