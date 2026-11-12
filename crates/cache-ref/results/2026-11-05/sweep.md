## Layout and lighting memory sweep

2b recommended, 16 rays/cell/frame (rays scale with cells), history 32, pixels 0.050 m, reference 1024 spp, seeds [11, 12]. Pool MiB are **est**: 34 B/cell × world cells, world cells = 407116 × (room cells / room cells at 25 cm cubic). Cell size is the cubic-equivalent density (BCC cube edge = h·2^(1/3)).

Cubic-equivalent cell size per pool at 34 B/cell: 
64 MiB → 0.114 m; 128 MiB → 0.080 m; 256 MiB → 0.057 m; 512 MiB → 0.040 m; 

| layout | cell (m) | pool MiB est | lookup fetches / ALU | contact energy | contact blotch | contact rel err | thin-wall energy | thin-wall blotch | thin-wall leak | room cells (contact) | rays/frame (contact) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| cubic | 0.3000 | 9.6 | 8 / 52 | 0.969 | 0.0158 | 0.045 | 0.970 | 0.0328 | +0.076 | 925 | 14800 |
| cubic | 0.2500 | 13.2 | 8 / 52 | 0.966 | 0.0158 | 0.047 | 0.969 | 0.0381 | +0.268 | 1271 | 20336 |
| cubic | 0.2100 | 18.5 | 8 / 52 | 0.966 | 0.0146 | 0.046 | 0.966 | 0.0256 | +0.035 | 1786 | 28576 |
| cubic | 0.1770 | 26.5 | 8 / 52 | 0.967 | 0.0131 | 0.045 | 0.967 | 0.0257 | +0.080 | 2554 | 40864 |
| cubic | 0.1250 | 50.8 | 8 / 52 | 0.962 | 0.0119 | 0.048 | 0.967 | 0.0236 | +0.062 | 4889 | 78224 |
| cubic | 0.0880 | 104.4 | 8 / 52 | 0.968 | 0.0099 | 0.045 | 0.973 | 0.0177 | +0.050 | 10051 | 160816 |
| cubic | 0.0625 | 202.2 | 8 / 52 | 0.964 | 0.0084 | 0.047 | 0.968 | 0.0168 | -0.026 | 19464 | 311424 |
| bcc | 0.3000 | 13.2 | 4 / 86 | 0.965 | 0.0157 | 0.047 | 0.964 | 0.0438 | +0.209 | 1268 | 20288 |
| bcc | 0.2500 | 18.2 | 4 / 86 | 0.966 | 0.0146 | 0.046 | 0.964 | 0.0364 | +0.163 | 1750 | 28000 |
| bcc | 0.2100 | 24.5 | 4 / 86 | 0.968 | 0.0140 | 0.045 | 0.966 | 0.0305 | +0.111 | 2362 | 37792 |
| bcc | 0.1770 | 34.7 | 4 / 86 | 0.966 | 0.0135 | 0.046 | 0.967 | 0.0237 | +0.025 | 3342 | 53472 |
| bcc | 0.1250 | 63.9 | 4 / 86 | 0.966 | 0.0116 | 0.046 | 0.970 | 0.0216 | +0.070 | 6152 | 98432 |
| bcc | 0.0880 | 112.4 | 4 / 86 | 0.967 | 0.0105 | 0.047 | 0.966 | 0.0195 | -0.051 | 10825 | 173200 |
| bcc | 0.0625 | 188.9 | 4 / 86 | 0.966 | 0.0091 | 0.047 | 0.968 | 0.0182 | -0.008 | 18185 | 290960 |

Equal accuracy: pool each layout needs for contact blotch ≤ 0.0092 (110 % of cubic's best in this sweep), log-interpolated:

- cubic: 138.8 MiB est
- bcc: 179.5 MiB est

