## Variants

cell 0.250 m, 16 rays/cell/frame, history 32 frames, reference 1024 spp, seeds [11, 12, 13], warm 160, 8 measured frames. Bytes are **est** engine RGB bytes per cell incl. 12 B key/count/age.

### contact

| config | energy | blotch | leak | mean rel err | pop p99 | cells | rays/frame |
|---|---|---|---|---|---|---|---|
| axis+norm (38 B) | 0.966 ± 0.001 | 0.0157 ± 0.0003 | +0.000 | 0.047 | 0.0213 | 1271 | 20336 |
| axis+norm+compact (26 B) | 0.966 ± 0.001 | 0.0157 ± 0.0003 | +0.000 | 0.047 | 0.0222 | 1271 | 20336 |
| axis+pos+wiener+compact (33 B) | 0.926 ± 0.001 | 0.0148 ± 0.0004 | +0.000 | 0.076 | 0.0186 | 1271 | 20336 |
| axis+pos+wiener+norm+compact (34 B) | 0.967 ± 0.001 | 0.0156 ± 0.0003 | +0.000 | 0.046 | 0.0229 | 1271 | 20336 |
| axis+pos+wiener+norm+compact+vis (34 B) | 0.967 ± 0.001 | 0.0156 ± 0.0003 | +0.000 | 0.046 | 0.0229 | 1271 | 20336 |

### near-wall

| config | energy | blotch | leak | mean rel err | pop p99 | cells | rays/frame |
|---|---|---|---|---|---|---|---|
| axis+norm (38 B) | 0.967 ± 0.002 | 0.0183 ± 0.0004 | +0.000 | 0.052 | 0.0281 | 1216 | 19456 |
| axis+norm+compact (26 B) | 0.967 ± 0.002 | 0.0184 ± 0.0004 | +0.000 | 0.052 | 0.0286 | 1216 | 19456 |
| axis+pos+wiener+compact (33 B) | 0.915 ± 0.002 | 0.0175 ± 0.0003 | +0.000 | 0.083 | 0.0253 | 1216 | 19456 |
| axis+pos+wiener+norm+compact (34 B) | 0.967 ± 0.002 | 0.0183 ± 0.0004 | +0.000 | 0.052 | 0.0293 | 1216 | 19456 |
| axis+pos+wiener+norm+compact+vis (34 B) | 0.967 ± 0.002 | 0.0183 ± 0.0004 | +0.000 | 0.052 | 0.0293 | 1216 | 19456 |

### thin-wall

| config | energy | blotch | leak | mean rel err | pop p99 | cells | rays/frame |
|---|---|---|---|---|---|---|---|
| axis+norm (38 B) | 0.967 ± 0.001 | 0.0439 ± 0.0006 | +0.366 | 0.110 | 0.0258 | 1556 | 24896 |
| axis+norm+compact (26 B) | 0.967 ± 0.001 | 0.0440 ± 0.0006 | +0.366 | 0.110 | 0.0267 | 1556 | 24896 |
| axis+pos+wiener+compact (33 B) | 0.916 ± 0.000 | 0.0469 ± 0.0002 | +0.364 | 0.129 | 0.0253 | 1556 | 24896 |
| axis+pos+wiener+norm+compact (34 B) | 0.968 ± 0.001 | 0.0377 ± 0.0006 | +0.263 | 0.097 | 0.0291 | 1556 | 24896 |
| axis+pos+wiener+norm+compact+vis (34 B) | 0.968 ± 0.001 | 0.0323 ± 0.0005 | +0.114 | 0.083 | 0.0288 | 1556 | 24896 |

### shadow-edge

| config | energy | blotch | leak | mean rel err | pop p99 | cells | rays/frame |
|---|---|---|---|---|---|---|---|
| axis+norm (38 B) | 0.967 ± 0.001 | 0.0259 ± 0.0006 | +0.000 | 0.069 | 0.0482 | 1304 | 20864 |
| axis+norm+compact (26 B) | 0.967 ± 0.001 | 0.0259 ± 0.0006 | +0.000 | 0.069 | 0.0481 | 1304 | 20864 |
| axis+pos+wiener+compact (33 B) | 0.893 ± 0.001 | 0.0243 ± 0.0005 | +0.000 | 0.104 | 0.0401 | 1304 | 20864 |
| axis+pos+wiener+norm+compact (34 B) | 0.968 ± 0.001 | 0.0260 ± 0.0007 | +0.000 | 0.069 | 0.0487 | 1304 | 20864 |
| axis+pos+wiener+norm+compact+vis (34 B) | 0.968 ± 0.001 | 0.0260 ± 0.0007 | +0.000 | 0.069 | 0.0487 | 1304 | 20864 |

### lamp-off

| config | lag | linger at +120 |
|---|---|---|
| axis+norm | 0.993 | 0.0057 |
| axis+norm+compact | 0.993 | 0.0057 |
| axis+pos+wiener+compact | 0.953 | 0.0054 |
| axis+pos+wiener+norm+compact | 0.993 | 0.0057 |
| axis+pos+wiener+norm+compact+vis | 0.993 | 0.0057 |

