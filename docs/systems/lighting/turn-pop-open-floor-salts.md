# Turn pop and open floor: measured across probe patterns

Two `gpu_frame` tests, `a_turn_toward_the_lamp_does_not_pop_the_bounce` and
`an_open_floor_falls_off_smoothly_and_a_shadow_keeps_colored_bounce`, pass or
fail with the probe turn pattern (`lighting_cell_hash` salt). Any salt is an
equally valid pattern, so the shipped salt is 0 and the limits stay as written.

Lavapipe, 28 Oct 2026, salts 0-11, limits unchanged:

| Test | Passes | Failing values | Limit |
|---|---|---|---|
| Turn pop (worst channel gap, held vs settled frame) | 7/12 (1, 3, 5, 6, 8-11) | 17.8-32.5 | < 16 |
| Open floor (largest reversal on an 8-point line) | 5/12 (4, 6, 7, 8, 11) | 0.7-1.4 codes | < 0.5 (drop 1.3-1.9) |

Both pass only at salts 6, 8 and 11. At salt 0 both fail.

## Why they fail

- **Turn pop is a real defect.** The worst point is the floor beside the red solid
  (-1, 0, 0): the first frame after a half-turn shows 6-21, the settled frame 38.
  New screen probes start without the light their region had, so it pops. This
  matches the side pop-ins seen on the 4070 when turning; it is not lavapipe-only.
- **Open floor is estimator noise against a strict limit.** The bounce drop along
  the line is only 1.3-1.9 codes, so the limit falls to the 0.5-code floor the
  test allows for dither. The bounce estimate's own spatial noise (about 1 code,
  set by the ray pattern) is larger than that.

Neither needs the 4070 to explain it, and neither is fixed by picking a salt.
The turn pop needs warm-started probes (lighting plan: edge pop-ins). The open
floor needs a less noisy bounce at the shadow edge, or a limit derived from the
path-traced reference's own falloff and noise.
