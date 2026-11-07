## Baseline vs 2b

cell 0.250 m cubic, 16 rays/cell/frame, history 32 frames, pixels 0.050 m, reference 1024 spp, seeds [11, 12, 13], warm 160 frames, 8 measured frames. Bytes per cell are **estimates** (payload + 12 B key/count/age).

### contact

| config | energy | blotch | leak | mean rel err | pop p99 | cells | rays/frame |
|---|---|---|---|---|---|---|---|
| plain (stage 2, f16) (36 B est) | 0.834 ± 0.001 | 0.0287 ± 0.0004 | +0.000 | 0.164 | 0.0170 | 1173 | 18768 |
| side bit only (36 B est) | 0.872 ± 0.001 | 0.0224 ± 0.0004 | +0.000 | 0.125 | 0.0169 | 1218 | 19488 |
| 2b as written (1-bit side, compact) (32 B est) | 0.879 ± 0.002 | 0.0257 ± 0.0002 | +0.000 | 0.121 | 0.0258 | 1218 | 19488 |
| axis key only (36 B est) | 0.920 ± 0.001 | 0.0148 ± 0.0003 | +0.000 | 0.080 | 0.0176 | 1271 | 20336 |
| plain + irradiance norm (38 B est) | 0.921 ± 0.001 | 0.0239 ± 0.0002 | +0.000 | 0.079 | 0.0208 | 1173 | 18768 |
| 2b recommended (axis, wiener, norm, compact) (34 B est) | 0.967 ± 0.001 | 0.0156 ± 0.0003 | +0.000 | 0.046 | 0.0229 | 1271 | 20336 |
| 2b recommended, exact encoding (96 B est) | 0.967 ± 0.001 | 0.0160 ± 0.0003 | +0.000 | 0.047 | 0.0236 | 1271 | 20336 |
| 2b recommended + read visibility (34 B est) | 0.967 ± 0.001 | 0.0156 ± 0.0003 | +0.000 | 0.046 | 0.0229 | 1271 | 20336 |

### near-wall

| config | energy | blotch | leak | mean rel err | pop p99 | cells | rays/frame |
|---|---|---|---|---|---|---|---|
| plain (stage 2, f16) (36 B est) | 0.827 ± 0.002 | 0.0292 ± 0.0003 | +0.000 | 0.165 | 0.0238 | 1143 | 18288 |
| side bit only (36 B est) | 0.864 ± 0.002 | 0.0244 ± 0.0002 | +0.000 | 0.129 | 0.0243 | 1174 | 18784 |
| 2b as written (1-bit side, compact) (32 B est) | 0.871 ± 0.002 | 0.0273 ± 0.0003 | +0.000 | 0.124 | 0.0350 | 1174 | 18784 |
| axis key only (36 B est) | 0.909 ± 0.002 | 0.0175 ± 0.0002 | +0.000 | 0.088 | 0.0246 | 1216 | 19456 |
| plain + irradiance norm (38 B est) | 0.934 ± 0.001 | 0.0246 ± 0.0004 | +0.000 | 0.076 | 0.0272 | 1143 | 18288 |
| 2b recommended (axis, wiener, norm, compact) (34 B est) | 0.967 ± 0.002 | 0.0183 ± 0.0004 | +0.000 | 0.052 | 0.0293 | 1216 | 19456 |
| 2b recommended, exact encoding (96 B est) | 0.967 ± 0.002 | 0.0187 ± 0.0004 | +0.000 | 0.052 | 0.0306 | 1216 | 19456 |
| 2b recommended + read visibility (34 B est) | 0.967 ± 0.002 | 0.0183 ± 0.0004 | +0.000 | 0.052 | 0.0293 | 1216 | 19456 |

### thin-wall

| config | energy | blotch | leak | mean rel err | pop p99 | cells | rays/frame |
|---|---|---|---|---|---|---|---|
| plain (stage 2, f16) (36 B est) | 0.672 ± 0.001 | 0.1100 ± 0.0003 | -0.099 | 0.281 | 0.0185 | 1385 | 22160 |
| side bit only (36 B est) | 0.849 ± 0.001 | 0.0939 ± 0.0002 | +0.783 | 0.200 | 0.0228 | 1466 | 23456 |
| 2b as written (1-bit side, compact) (32 B est) | 0.852 ± 0.001 | 0.0867 ± 0.0003 | +0.579 | 0.182 | 0.0340 | 1466 | 23456 |
| axis key only (36 B est) | 0.912 ± 0.000 | 0.0528 ± 0.0002 | +0.483 | 0.143 | 0.0216 | 1556 | 24896 |
| plain + irradiance norm (38 B est) | 0.789 ± 0.001 | 0.0846 ± 0.0003 | +0.235 | 0.222 | 0.0274 | 1385 | 22160 |
| 2b recommended (axis, wiener, norm, compact) (34 B est) | 0.968 ± 0.001 | 0.0377 ± 0.0006 | +0.263 | 0.097 | 0.0291 | 1556 | 24896 |
| 2b recommended, exact encoding (96 B est) | 0.968 ± 0.001 | 0.0365 ± 0.0004 | +0.241 | 0.096 | 0.0290 | 1556 | 24896 |
| 2b recommended + read visibility (34 B est) | 0.968 ± 0.001 | 0.0323 ± 0.0005 | +0.114 | 0.083 | 0.0288 | 1556 | 24896 |

### shadow-edge

| config | energy | blotch | leak | mean rel err | pop p99 | cells | rays/frame |
|---|---|---|---|---|---|---|---|
| plain (stage 2, f16) (36 B est) | 0.749 ± 0.000 | 0.0315 ± 0.0008 | +0.000 | 0.226 | 0.0351 | 1219 | 19504 |
| side bit only (36 B est) | 0.837 ± 0.000 | 0.0289 ± 0.0008 | +0.000 | 0.152 | 0.0378 | 1250 | 20000 |
| 2b as written (1-bit side, compact) (32 B est) | 0.852 ± 0.001 | 0.0322 ± 0.0012 | +0.000 | 0.141 | 0.0488 | 1250 | 20000 |
| axis key only (36 B est) | 0.878 ± 0.001 | 0.0242 ± 0.0004 | +0.000 | 0.115 | 0.0398 | 1304 | 20864 |
| plain + irradiance norm (38 B est) | 0.859 ± 0.000 | 0.0303 ± 0.0006 | +0.000 | 0.132 | 0.0421 | 1219 | 19504 |
| 2b recommended (axis, wiener, norm, compact) (34 B est) | 0.968 ± 0.001 | 0.0260 ± 0.0007 | +0.000 | 0.069 | 0.0487 | 1304 | 20864 |
| 2b recommended, exact encoding (96 B est) | 0.968 ± 0.001 | 0.0264 ± 0.0007 | +0.000 | 0.070 | 0.0502 | 1304 | 20864 |
| 2b recommended + read visibility (34 B est) | 0.968 ± 0.001 | 0.0260 ± 0.0007 | +0.000 | 0.069 | 0.0487 | 1304 | 20864 |

### lamp-off

| config | lag (energy at frame 300) | linger at +120 |
|---|---|---|
| plain (stage 2, f16) | 0.852 | 0.0030 |
| side bit only | 0.893 | 0.0040 |
| 2b as written (1-bit side, compact) | 0.901 | 0.0041 |
| axis key only | 0.946 | 0.0054 |
| plain + irradiance norm | 0.945 | 0.0045 |
| 2b recommended (axis, wiener, norm, compact) | 0.993 | 0.0057 |
| 2b recommended, exact encoding | 0.994 | 0.0057 |
| 2b recommended + read visibility | 0.993 | 0.0057 |
| F3 texel feedback (must fail) | 0.269 | 0.2319 |

### F2 adaptive origins on contact (plain cells)

Bias on hard spots (hardness >= 0.5, 968 pixels) vs the reference, mean ± seed sd; delta is against uniform selection.

| selection | skew | energy | hard-spot bias | delta vs uniform |
|---|---|---|---|---|
| uniform | 0 | 0.834 | -29.43% ± 0.11 | +0.00% |
| skew unweighted (must fail) | 8 | 0.829 | -30.35% ± 0.05 | -0.92% |
| skew with 1/q weights (control) | 8 | 0.834 | -29.40% ± 0.17 | +0.03% |
| skew unweighted (must fail) | 64 | 0.826 | -30.83% ± 0.05 | -1.40% |
| skew with 1/q weights (control) | 64 | 0.833 | -29.50% ± 0.15 | -0.07% |

