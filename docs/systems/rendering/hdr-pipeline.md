# HDR pipeline: presets, costs, and the headless reference

Status: **accepted** (Engine bot, 29 Oct 2026). ADR: [0027](../../adr/0027-hdr-presets-and-headless-curves.md). The full pipeline design is [`hdr-output-design.md`](../hdr-output-design.md) (ADR 0015). This page maps it onto the ADR 0023 presets and records the CPU reference.

## 1. Pipeline, in frame order

| Step | Input | Output | Decided in |
|---|---|---|---|
| Compose | lighting, materials | scene-referred Rec.709 linear, `R16G16B16A16_SFLOAT`, values above 1 kept | ADR 0015 §1 |
| Meter | scene image luma | log2 histogram, outer 10% clipped, EV100 | ADR 0015 §3 |
| Adapt | EV100 target | exposure scale `1 / (1.2 × 2^EV100)`, exponential step | ADR 0015 §3 |
| Bloom | scene image, before exposure | dual-filter pyramid added back, Karis weight on the first downsample, no threshold | ADR 0015 §5 |
| Display transform | exposed scene | AgX (default), ACES 2.0 (alternate), legacy shoulder (byte parity only) | ADR 0015 §4 |
| Encode | display-linear | SDR sRGB code, HDR10 PQ (203-nit white, 1000-nit peak), or scRGB (code 2.54 = 203 nits) | ADR 0015 §6 |

### Why AgX over ACES

- AgX's inset/outset keeps hue as saturated lights brighten; a red lamp goes towards white instead of skewing to orange. ACES 1.x skews hue, and the common Narkowicz fit is not ACES.
- AgX is a per-channel curve plus two 3×3 matrices, baked into a 32³ lattice, one texture read per pixel on every preset.
- ACES 2.0 stays an alternate for teams that grade against it. Its body is the Academy CTL or nothing.

### Why the dual-filter bloom

Thresholdless, so nothing pops as a pixel crosses a level. Energy-conserving, so bloom adds glow without brightening the frame. The Karis weight on the first downsample stops a single firefly pixel flashing. It costs a few bandwidth-bound passes at half resolution and below.

### Why histogram auto-exposure

Clipping 10% at each tail ignores the sun disc and black corners. EV100 is the same unit a physical camera writes, so a cutscene can switch to aperture, shutter and ISO with no jump.

## 2. Presets (ADR 0023 names)

| | Fallback (below Low gate) | Low | Medium | High | Ultra |
|---|---|---|---|---|---|
| Target | no GI device | GTX 1060 / RX 580 1080p, Steam Deck 800p | mid | RTX 4070 1440p | RTX 4070 and up |
| Exposure | fixed EV100 0 | auto, 64 bins | auto, 128 bins | auto, 256 bins | auto, 256 bins |
| Adapt down / up | hold | 0.50 s / 1.00 s | 0.40 s / 0.80 s | 0.40 s / 0.80 s | 0.40 s / 0.80 s |
| Bloom levels | 0 | 4 | 5 | 6 | 7 |
| Display | AgX | AgX | AgX | AgX | AgX |
| Output | SDR | SDR (Deck OLED: auto) | auto | auto | auto, peak from display |

## 3. Per-preset cost ceilings

Ceilings from the ADR 0015 design §7 at 1440p, carried to the new names. Ultra's 7th level adds a 20×11 texel level, under 0.01 MiB. Low's 1080p numbers are the 1440p numbers scaled by pixel count (0.5625). None of these are measured yet.

| | Fallback | Low | Medium | High | Ultra |
|---|---|---|---|---|---|
| Post VRAM, MiB (SDR) | 28.13 | 37.46 at 1440p, 21.07 at 1080p | 37.49 | 37.50 | 37.50 |
| Post VRAM with scRGB, MiB | n/a | n/a | 79.68 | 79.69 | 79.69 |
| ms ceiling, 4070 1440p | 0.25 | 0.80 | 1.00 | 1.20 | 1.25 |
| ms ceiling, 1060 1080p | 0.56 | 1.40 | n/a | n/a | n/a |
| ms ceiling, Steam Deck 800p | n/a | 1.50 | n/a | n/a | n/a |

The Deck's 40 fps target is a 25 ms frame (ADR 0023). 1.50 ms is 6% of it. The 4070's 500 fps basic-scene goal is a 2.0 ms frame. High post 1.20 ms plus the frozen lighting floor of 0.43 ms is 1.63 ms. A preset whose measured post time misses its ceiling drops a bloom level first, then the histogram size.

## 4. Headless reference

`crates/math/src/tonemap.rs` holds the scalar curves. `crates/math/tests/tonemap.rs` checks them with no GPU:

| Test | Checks |
|---|---|
| `ev100_zero_is_one_over_one_point_two` | exposure scale at EV100 0 and one stop up |
| `camera_f1_one_second_iso100_is_ev0`, `sunny_sixteen_is_about_ev15` | camera EV100 formula |
| `luminance_meter_matches_k_12_5` | meter constant K = 12.5 |
| `compensation_plus_one_doubles` | compensation direction |
| `still_scene_holds_exposure` | no drift on a still scene over 600 frames |
| `four_stop_adaptation_reaches_90_percent_in_window_without_overshoot` | 90% time = ln 10 × 0.4 s, inside 0.4 to 2.0 s, never past the target |
| `karis_weight_tames_a_firefly` | a 1000-luma pixel adds under 1 |
| `pq_matches_st2084_table`, `pq_is_monotone` | ST 2084 at 0, 100, 203, 1000, 10000 nits |
| `nit_map_pins_white_and_peak` | 203-nit white, 1000-nit peak, NaN to 0 |
| `agx_keeps_mid_grey`, `agx_is_monotone_and_bounded_in_its_log_range`, `agx_toe_lifts_less_than_linear_shoulder_compresses` | AgX fixed point, shape, range |

Run: `cargo test -p genos-math`.

## 5. Follow-ups

1. Point `hdr_output.rs` at `genos-math::tonemap` and delete its copies of these curves.
2. Rename the `HdrSettings` preset table from Potato..High to the ADR 0023 names.
3. Measure the §3 ceilings on the 4070 with `post_ms_device` and `post_vram_device`.

## References

- [ADR 0015](../../adr/0015-hdr-output.md) and [its design](../hdr-output-design.md)
- [ADR 0023](../../adr/0023-graphics-settings-presets.md)
- SMPTE ST 2084, ITU-R BT.2100, ITU-R BT.2408 (203-nit graphics white)
- Jimenez, "Next Generation Post Processing in Call of Duty: Advanced Warfare", SIGGRAPH 2014
- Sobotka, AgX; Lagarde and de Rousiers, "Moving Frostbite to PBR" (EV100), SIGGRAPH 2014
