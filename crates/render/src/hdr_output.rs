//! CPU references the HDR passes call. No swapchain and no display dispatch yet.
//!
//! `AgX` follows Eary Chow's `AgX_LUT_Gen` (`AgX_Base_Rec2020` then the sRGB cube path):
//! `Rec.709` to `Rec.2020`, lower guard, inset, log2 from −10 to +6.5 around 0.18,
//! sigmoid, power 2.4, 40% hue mix, outset, back to `Rec.709`, power 2.4 code.

/// ITU-R BT.2408 graphics white.
pub const GRAPHICS_WHITE_NITS: f64 = 203.0;
/// scRGB code 1.0. Graphics white is `GRAPHICS_WHITE_NITS / SCRGB_NITS_PER_CODE` (2.5375, written 2.54).
pub const SCRGB_NITS_PER_CODE: f64 = 80.0;

pub const FORMAT_BGRA8: i32 = 44;
pub const FORMAT_RGBA8: i32 = 37;
pub const FORMAT_A2R10: i32 = 58;
pub const FORMAT_A2B10: i32 = 64;
pub const FORMAT_R16F: i32 = 97;
pub const COLOR_SPACE_SRGB: i32 = 0;
pub const COLOR_SPACE_SCRGB: i32 = 1_000_104_002;
pub const COLOR_SPACE_HDR10: i32 = 1_000_104_008;

const MID_GREY: f64 = 0.18;
const LOG_MIN: f64 = -10.0;
const LOG_MAX: f64 = 6.5;
const SIGMOID_SLOPE: f64 = 2.4;
const SIGMOID_POWER: f64 = 1.5;
const HUE_MIX: f64 = 0.40;
const GUARD_Y: [f64; 3] = [
    0.258_923_535_568_984_8,
    0.610_498_534_606_652_5,
    0.130_577_929_824_362_84,
];

const BT709_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.412_386_563_252_991_8,
        0.357_591_490_920_625_4,
        0.180_450_491_203_563_7,
    ],
    [
        0.212_636_821_677_323_8,
        0.715_182_981_841_250_7,
        0.072_180_196_481_425_5,
    ],
    [
        0.019_330_620_152_484,
        0.119_197_163_640_208_5,
        0.950_372_587_005_435_5,
    ],
];
const XYZ_TO_BT709: [[f64; 3]; 3] = [
    [
        3.241_003_232_976_359,
        -1.537_398_969_488_785_5,
        -0.498_615_881_996_362_9,
    ],
    [
        -0.969_224_252_202_516_4,
        1.875_929_983_695_176,
        0.041_554_226_340_084_72,
    ],
    [
        0.055_639_419_851_975_47,
        -0.204_011_206_123_909_94,
        1.057_148_977_187_533_3,
    ],
];
const XYZ_TO_BT2020: [[f64; 3]; 3] = [
    [
        1.716_663_427_795_880_5,
        -0.355_673_319_730_139_9,
        -0.253_368_087_890_247_8,
    ],
    [
        -0.666_673_836_198_886_9,
        1.616_455_739_824_698,
        0.015_768_297_096_133_7,
    ],
    [
        0.017_642_481_784_977_2,
        -0.042_776_976_382_753_2,
        0.942_243_281_018_430_8,
    ],
];
const BT2020_TO_XYZ: [[f64; 3]; 3] = [
    [
        0.636_953_506_785_074,
        0.144_619_184_669_233_1,
        0.168_855_853_922_873_4,
    ],
    [
        0.262_698_338_956_556,
        0.678_008_765_772_816_5,
        0.059_292_895_270_627_3,
    ],
    [0.0, 0.028_073_135_847_557, 1.060_827_234_950_570_7],
];
const INSET: [[f64; 3]; 3] = [
    [
        0.856_627_156_288_779_2,
        0.095_121_245_402_534_9,
        0.048_251_598_308_685_8,
    ],
    [
        0.137_318_972_283_551_67,
        0.761_241_987_009_080_6,
        0.101_439_040_707_367_48,
    ],
    [
        0.111_898_208_045_179_53,
        0.076_799_414_562_517_57,
        0.811_302_377_392_303,
    ],
];
const OUTSET: [[f64; 3]; 3] = [
    [
        1.127_100_569_630_12,
        -0.110_606_638_578_260_65,
        -0.016_493_931_051_859_02,
    ],
    [
        -0.141_329_754_421_353_37,
        1.157_823_685_473_212_7,
        -0.016_493_931_051_859_03,
    ],
    [
        -0.141_329_754_421_353_26,
        -0.110_606_638_578_260_59,
        1.251_936_392_999_614,
    ],
];

/// 13-tap dual-filter downsample. Offsets are source texels. Weights sum to 1.
const BLOOM_TAPS: [(i32, i32, f64); 13] = [
    (0, 0, 0.125),
    (-1, 1, 0.125),
    (1, 1, 0.125),
    (-1, -1, 0.125),
    (1, -1, 0.125),
    (0, 2, 0.0625),
    (0, -2, 0.0625),
    (-2, 0, 0.0625),
    (2, 0, 0.0625),
    (-2, 2, 0.03125),
    (2, 2, 0.03125),
    (-2, -2, 0.03125),
    (2, -2, 0.03125),
];

/// Host that presents the swapchain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PresentHost {
    Linux,
    Windows,
    Mac,
}

/// Chosen swapchain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HdrSwapchain {
    Sdr,
    Hdr10,
    Scrgb,
}

/// One `VkSurfaceFormatKHR` pair the gate can see.
#[derive(Clone, Copy)]
pub struct ListedFormat {
    pub format: i32,
    pub color_space: i32,
}

#[must_use]
pub const fn scrgb_code(nits: f64) -> f64 {
    nits / SCRGB_NITS_PER_CODE
}

/// Compose stores this radiance. No shoulder and no clamp at 1.
#[must_use]
pub const fn scene_referred(radiance: [f64; 3]) -> [f64; 3] {
    radiance
}

/// One read of the scene-referred image. `rgb` is three channels per pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneRead {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<f64>,
    pub distance_m: Vec<f64>,
}

/// Build a grade image from RGBA32F texels. Each pixel goes through [`scene_referred`].
#[must_use]
pub fn scene_read_from_rgba(width: u32, height: u32, rgba: &[f32]) -> SceneRead {
    let pixels = usize::try_from(width)
        .unwrap_or(0)
        .saturating_mul(usize::try_from(height).unwrap_or(0));
    let mut rgb = Vec::with_capacity(pixels.saturating_mul(3));
    let mut distance_m = Vec::with_capacity(pixels);
    for pixel in rgba.chunks(4).take(pixels) {
        let r = f64::from(pixel.first().copied().unwrap_or(0.0));
        let g = f64::from(pixel.get(1).copied().unwrap_or(0.0));
        let b = f64::from(pixel.get(2).copied().unwrap_or(0.0));
        let stored = scene_referred([r, g, b]);
        rgb.push(stored[0]);
        rgb.push(stored[1]);
        rgb.push(stored[2]);
        distance_m.push(f64::from(pixel.get(3).copied().unwrap_or(0.0)));
    }
    SceneRead {
        width,
        height,
        rgb,
        distance_m,
    }
}

/// `|stored − 4| / 4` for one scene-referred channel whose radiance is 4.
#[must_use]
pub fn scene_peak_error(stored: f64) -> f64 {
    (stored - 4.0).abs() / 4.0
}

/// Legacy shoulder. The body is [`crate::mesh::tone`].
#[must_use]
pub fn legacy_display_channel(channel: f64) -> f64 {
    f64::from(crate::mesh::tone(narrow_f32(channel)))
}

/// Legacy shoulder on a scene-referred pixel.
#[must_use]
pub fn legacy_display(scene: [f64; 3]) -> [f64; 3] {
    [
        legacy_display_channel(scene[0]),
        legacy_display_channel(scene[1]),
        legacy_display_channel(scene[2]),
    ]
}

/// Which curve the display pass runs. `Aces` stays empty until the Academy CTL.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Agx,
    Aces,
    Legacy,
}

/// Display transform after exposure. `None` is ACES, which is not the CTL yet.
#[must_use]
pub fn display_transform(scene: [f64; 3], mode: DisplayMode) -> Option<[f64; 3]> {
    match mode {
        DisplayMode::Legacy => Some(legacy_display(scene)),
        DisplayMode::Agx => Some(agx_rec709(scene)),
        DisplayMode::Aces => aces2_rec709(scene),
    }
}

/// Rec.709 relative luminance.
#[must_use]
pub fn rec709_y(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Rec.709 `Y` of one scene-referred pixel. The grade calls this.
#[must_use]
pub fn scene_luma(rgb: [f64; 3]) -> f64 {
    rec709_y(scene_referred(rgb))
}

/// Mean relative luminance error. The grade calls this on scene-referred `Y`.
#[must_use]
pub fn grade_relative(candidate: &[f64], reference: &[f64]) -> f64 {
    let mut n = 0.0;
    let mut sum = 0.0;
    for sample in reference {
        sum += *sample;
        n += 1.0;
    }
    let mean = if n == 0.0 { 0.0 } else { sum / n };
    let mut err = 0.0;
    let mut count = 0.0;
    for (cand, refer) in candidate.iter().zip(reference.iter()) {
        let denom = refer.max(0.25 * mean).max(0.02);
        err += (cand - refer).abs() / denom;
        count += 1.0;
    }
    if count == 0.0 {
        1.0
    } else {
        err / count
    }
}

/// Largest channel. A display shoulder peaks near 0.78 and fails the 1.5 floor.
#[must_use]
pub fn grade_peak(values: &[f64]) -> f64 {
    let mut peak = 0.0;
    for value in values {
        if *value > peak {
            peak = *value;
        }
    }
    peak
}

/// Weight of one grade sample. Under 2 m the weight is 4. Farther it is 1.
#[must_use]
pub fn grade_weight(distance_m: f64) -> f64 {
    if distance_m < 2.0 {
        4.0
    } else {
        1.0
    }
}

/// `(4 × e_near + e_far) / 5`. One near sample and one far sample.
#[must_use]
pub fn near_grade(near: f64, far: f64) -> f64 {
    (grade_weight(0.0) * near + grade_weight(3.0) * far) / (grade_weight(0.0) + grade_weight(3.0))
}

#[allow(
    clippy::as_conversions,
    reason = "mesh::tone takes f32; the legacy display is that function"
)]
#[allow(
    clippy::cast_possible_truncation,
    reason = "mesh::tone takes f32; the legacy display is that function"
)]
fn narrow_f32(value: f64) -> f32 {
    value as f32
}

#[must_use]
pub fn ev100_from_luminance(luminance: f64) -> f64 {
    (luminance * (100.0 / 12.5)).log2()
}

#[must_use]
pub fn exposure_from_ev100(ev100: f64) -> f64 {
    1.0 / (1.2 * ev100.exp2())
}

#[must_use]
pub fn camera_ev100(aperture: f64, shutter: f64, iso: f64) -> f64 {
    ((aperture * aperture) / shutter * 100.0 / iso).log2()
}

#[must_use]
pub fn exposure_from_camera(aperture: f64, shutter: f64, iso: f64) -> f64 {
    exposure_from_ev100(camera_ev100(aperture, shutter, iso))
}

/// Log-luminance histogram, 256 bins, outer 10% dropped on each tail.
#[must_use]
pub fn histogram_ev100(samples: &[f64]) -> f64 {
    histogram_ev100_bins(samples, 256)
}

/// Bin counts for [`histogram_ev100_bins`]. The dump writes these counts.
#[must_use]
pub fn histogram_counts(samples: &[f64], bins: usize) -> Vec<u32> {
    let bins = bins.clamp(1, 4096);
    let mut counts = vec![0_u32; bins];
    for sample in samples {
        if *sample <= 0.0 {
            continue;
        }
        let index = bin_of(*sample, bins);
        if let Some(count) = counts.get_mut(index) {
            *count += 1;
        }
    }
    counts
}

/// Same meter as [`histogram_ev100`] with `bins` (Low uses 64, Medium 128, High 256).
#[must_use]
pub fn histogram_ev100_bins(samples: &[f64], bins: usize) -> f64 {
    let bins = bins.clamp(1, 4096);
    let counts = histogram_counts(samples, bins);
    let total: u32 = counts.iter().copied().sum();
    if total == 0 {
        return 0.0;
    }
    let drop = floor_u32(f64::from(total) * 0.10);
    let mut skip = drop;
    let mut keep = total.saturating_sub(drop.saturating_mul(2));
    if keep == 0 {
        keep = total;
        skip = 0;
    }
    let mut acc = 0.0;
    let mut seen = 0_u32;
    for (index, count) in counts.iter().enumerate() {
        let mut left = *count;
        if skip > 0 {
            let cut = skip.min(left);
            skip -= cut;
            left -= cut;
        }
        if left == 0 || seen >= keep {
            continue;
        }
        let take = left.min(keep - seen);
        acc += f64::from(take) * bin_center(index, bins);
        seen += take;
    }
    if seen == 0 {
        return 0.0;
    }
    acc / f64::from(seen) + 8.0_f64.log2()
}

/// Exposure after a compensation in stops. `+1` doubles the scale.
#[must_use]
pub fn exposure_with_compensation(ev100: f64, compensation: f64) -> f64 {
    exposure_from_ev100(ev100 - compensation)
}

/// One adaptation step. Holds when the relative gap is under 0.001.
/// `tau_down` is the scene getting brighter (scale falling).
#[must_use]
pub fn step_exposure(scale: f64, target: f64, dt: f64, tau_down: f64, tau_up: f64) -> f64 {
    if !target.is_finite() || target < 0.0 {
        return scale;
    }
    if !scale.is_finite() || scale <= 0.0 {
        return target;
    }
    let rel = (target - scale).abs() / scale.abs().max(1.0e-6);
    if rel < 0.001 {
        return scale;
    }
    let tau = if target < scale { tau_down } else { tau_up };
    let tau = if tau > 0.0 { tau } else { dt.abs().max(1.0e-6) };
    let gain = 1.0 - (-dt.abs() / tau).exp();
    scale + (target - scale) * gain
}

/// Max relative change over 8 identical frames, starting on the target.
#[must_use]
pub fn exposure_still_change() -> f64 {
    let target = exposure_from_ev100(0.0);
    let mut scale = target;
    let mut worst = 0.0;
    let mut prev = scale;
    for _ in 0..8 {
        scale = step_exposure(scale, target, 1.0 / 60.0, 0.40, 0.80);
        let step = (scale - prev).abs() / prev.abs().max(1.0e-6);
        if step > worst {
            worst = step;
        }
        prev = scale;
    }
    worst
}

/// Distance of the 90% time of a 4-stop darkening outside `[0.40, 2.00]` seconds.
#[must_use]
pub fn exposure_adapt_outside() -> f64 {
    let (t90, _) = adapt_four_stops();
    if t90 < 0.40 {
        0.40 - t90
    } else if t90 > 2.00 {
        t90 - 2.00
    } else {
        0.0
    }
}

/// How far a 4-stop adaptation steps past the target. The exponential stays short of it.
#[must_use]
pub fn exposure_overshoot() -> f64 {
    adapt_four_stops().1
}

fn adapt_four_stops() -> (f64, f64) {
    let dt = 1.0 / 60.0;
    let start = exposure_from_ev100(0.0);
    let target = exposure_from_ev100(4.0);
    let span = start - target;
    let mut scale = start;
    let mut time = 0.0;
    let mut t90 = 10.0;
    let mut saw = false;
    let mut overshoot = 0.0;
    for _ in 0..600 {
        scale = step_exposure(scale, target, dt, 0.40, 0.80);
        time += dt;
        if span != 0.0 {
            let progress = (start - scale) / span;
            if progress > 1.0 {
                let past = progress - 1.0;
                if past > overshoot {
                    overshoot = past;
                }
            }
            if !saw && progress >= 0.90 {
                t90 = time;
                saw = true;
            }
        }
    }
    (t90, overshoot)
}

#[must_use]
pub fn bloom_downsample_weight_sum() -> f64 {
    let mut sum = 0.0;
    for (_, _, weight) in BLOOM_TAPS {
        sum += weight;
    }
    sum
}

#[must_use]
pub fn bloom_upsample_weight_sum() -> f64 {
    0.25 + 4.0 * 0.125 + 4.0 * 0.0625
}

#[must_use]
pub fn bloom_downsample(src: &[f64], width: usize, height: usize) -> Image {
    let dst_w = (width / 2).max(1);
    let dst_h = (height / 2).max(1);
    let mut pixels = vec![0.0; dst_w.saturating_mul(dst_h)];
    for y in 0..dst_h {
        for x in 0..dst_w {
            let u = (usize_to_f64(x) + 0.5) / usize_to_f64(dst_w);
            let v = (usize_to_f64(y) + 0.5) / usize_to_f64(dst_h);
            let mut acc = 0.0;
            for (dx, dy, weight) in BLOOM_TAPS {
                let su = u + f64::from(dx) / usize_to_f64(width.max(1));
                let sv = v + f64::from(dy) / usize_to_f64(height.max(1));
                acc += weight * bilinear(src, width, height, su, sv);
            }
            if let Some(slot) = pixels.get_mut(y.saturating_mul(dst_w).saturating_add(x)) {
                *slot = acc;
            }
        }
    }
    Image {
        pixels,
        width: dst_w,
        height: dst_h,
    }
}

#[must_use]
pub fn bloom_upsample(src: &Image, dst_w: usize, dst_h: usize) -> Image {
    let mut pixels = vec![0.0; dst_w.saturating_mul(dst_h)];
    for y in 0..dst_h {
        for x in 0..dst_w {
            let u = (usize_to_f64(x) + 0.5) / usize_to_f64(dst_w.max(1));
            let v = (usize_to_f64(y) + 0.5) / usize_to_f64(dst_h.max(1));
            let mut acc = 0.0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let weight = tent_weight(dx, dy);
                    let su = u + f64::from(dx) / usize_to_f64(src.width.max(1));
                    let sv = v + f64::from(dy) / usize_to_f64(src.height.max(1));
                    acc += weight * bilinear(&src.pixels, src.width, src.height, su, sv);
                }
            }
            if let Some(slot) = pixels.get_mut(y.saturating_mul(dst_w).saturating_add(x)) {
                *slot = acc;
            }
        }
    }
    Image {
        pixels,
        width: dst_w,
        height: dst_h,
    }
}

/// One-channel image the bloom reference reads and writes.
pub struct Image {
    pub pixels: Vec<f64>,
    pub width: usize,
    pub height: usize,
}

/// SMPTE ST 2084 inverse EOTF. Nits in, code in 0..1 out. Zero nits is `c1^m2`, not a hard 0.
#[must_use]
pub fn pq_st2084(nits: f64) -> f64 {
    let c1 = 3424.0 / 4096.0;
    let c2 = 2413.0 / 128.0;
    let c3 = 2392.0 / 128.0;
    let m1 = 2610.0 / 16384.0;
    let m2 = 2523.0 / 32.0;
    let y = (nits / 10_000.0).max(0.0);
    let y_m = if y == 0.0 { 0.0 } else { y.powf(m1) };
    ((c1 + c2 * y_m) / (1.0 + c3 * y_m)).powf(m2)
}

/// `AgX` sRGB-cube code for scene-linear `Rec.709`. The display pass calls this for `display=agx`.
#[must_use]
pub fn agx_rec709(scene: [f64; 3]) -> [f64; 3] {
    let rec2020 = mul(XYZ_TO_BT2020, mul(BT709_TO_XYZ, scene));
    let formed = agx_base_rec2020(rec2020);
    let rec709 = mul(XYZ_TO_BT709, mul(BT2020_TO_XYZ, formed));
    let guarded = compensate_low_side(rec709);
    [
        encode_power(guarded[0]),
        encode_power(guarded[1]),
        encode_power(guarded[2]),
    ]
}

/// ACES 2.0 Rec.709 display (`Output.Academy.Rec709-D65_100nit_in_Rec709-D65_sRGB-Piecewise`).
/// `None` until this body is the Academy CTL. Not a Narkowicz fit.
#[must_use]
pub const fn aces2_rec709(_scene_linear_rec709: [f64; 3]) -> Option<[f64; 3]> {
    None
}

/// Linux/Wayland prefers HDR10, then scRGB, then SDR.
/// Windows prefers scRGB, then HDR10, then SDR.
/// macOS takes the first HDR pair `MoltenVK` listed, else SDR.
#[must_use]
pub fn pick_hdr_output(host: PresentHost, listed: &[ListedFormat]) -> HdrSwapchain {
    match host {
        PresentHost::Linux => prefer(&[HdrSwapchain::Hdr10, HdrSwapchain::Scrgb], listed),
        PresentHost::Windows => prefer(&[HdrSwapchain::Scrgb, HdrSwapchain::Hdr10], listed),
        PresentHost::Mac => first_hdr(listed),
    }
}

fn prefer(order: &[HdrSwapchain], listed: &[ListedFormat]) -> HdrSwapchain {
    for candidate in order {
        if listed
            .iter()
            .copied()
            .any(|item| matches_mode(*candidate, item))
        {
            return *candidate;
        }
    }
    HdrSwapchain::Sdr
}

fn first_hdr(listed: &[ListedFormat]) -> HdrSwapchain {
    for item in listed {
        if matches_mode(HdrSwapchain::Hdr10, *item) {
            return HdrSwapchain::Hdr10;
        }
        if matches_mode(HdrSwapchain::Scrgb, *item) {
            return HdrSwapchain::Scrgb;
        }
    }
    HdrSwapchain::Sdr
}

fn matches_mode(mode: HdrSwapchain, item: ListedFormat) -> bool {
    match mode {
        HdrSwapchain::Hdr10 => {
            item.color_space == COLOR_SPACE_HDR10
                && (item.format == FORMAT_A2B10 || item.format == FORMAT_A2R10)
        }
        HdrSwapchain::Scrgb => item.color_space == COLOR_SPACE_SCRGB && item.format == FORMAT_R16F,
        HdrSwapchain::Sdr => {
            item.color_space == COLOR_SPACE_SRGB
                && (item.format == FORMAT_BGRA8 || item.format == FORMAT_RGBA8)
        }
    }
}

fn agx_base_rec2020(color: [f64; 3]) -> [f64; 3] {
    let guarded = compensate_low_side(color);
    let inset = mul(INSET, guarded);
    let pre = rgb_to_hsv(inset);
    let logged = [
        log_encode(inset[0]),
        log_encode(inset[1]),
        log_encode(inset[2]),
    ];
    let curved = [sigmoid(logged[0]), sigmoid(logged[1]), sigmoid(logged[2])];
    let linear = [
        power_fwd(curved[0]),
        power_fwd(curved[1]),
        power_fwd(curved[2]),
    ];
    let mut post = rgb_to_hsv(linear);
    post[0] = lerp_hue(pre[0], post[0], HUE_MIX);
    mul(OUTSET, hsv_to_rgb(post))
}

fn compensate_low_side(rgb: [f64; 3]) -> [f64; 3] {
    let y = dot(rgb, GUARD_Y);
    let peak = rgb[0].max(rgb[1]).max(rgb[2]);
    let inverse = [peak - rgb[0], peak - rgb[1], peak - rgb[2]];
    let max_inverse = inverse[0].max(inverse[1]).max(inverse[2]);
    let y_inverse = dot(inverse, GUARD_Y);
    let y_compensate = max_inverse - y_inverse + y;

    let min_rgb = rgb[0].min(rgb[1]).min(rgb[2]);
    let offset = (-min_rgb).max(0.0);
    let shifted = [rgb[0] + offset, rgb[1] + offset, rgb[2] + offset];
    let peak_shifted = shifted[0].max(shifted[1]).max(shifted[2]);
    let inverse_shifted = [
        peak_shifted - shifted[0],
        peak_shifted - shifted[1],
        peak_shifted - shifted[2],
    ];
    let max_inverse_shifted = inverse_shifted[0]
        .max(inverse_shifted[1])
        .max(inverse_shifted[2]);
    let y_new = max_inverse_shifted - dot(inverse_shifted, GUARD_Y) + dot(shifted, GUARD_Y);
    let ratio = if y_new > y_compensate {
        y_compensate / y_new
    } else {
        1.0
    };
    [shifted[0] * ratio, shifted[1] * ratio, shifted[2] * ratio]
}

fn log_encode(x: f64) -> f64 {
    if x <= 0.0 {
        return f64::NEG_INFINITY;
    }
    let lg2 = (x / MID_GREY).log2();
    (lg2 - LOG_MIN) / (LOG_MAX - LOG_MIN)
}

fn sigmoid(x: f64) -> f64 {
    let x_pivot = (-LOG_MIN) / (LOG_MAX - LOG_MIN);
    let y_pivot = MID_GREY.powf(1.0 / SIGMOID_SLOPE);
    let toe_x = linear_breakpoint(0.0, SIGMOID_SLOPE, x_pivot);
    let toe_y = linear_breakpoint(0.0, SIGMOID_SLOPE, y_pivot);
    let shoulder_x = linear_breakpoint(0.0, SIGMOID_SLOPE, x_pivot);
    let shoulder_y = linear_breakpoint(0.0, SIGMOID_SLOPE, y_pivot);
    let scale_toe = -scale_factor(1.0 - 0.0, 1.0 - 0.0, 1.0 - toe_x, 1.0 - toe_y);
    let scale_shoulder = scale_factor(1.0, 1.0, shoulder_x, shoulder_y);
    let intercept = toe_y - SIGMOID_SLOPE * toe_x;
    if x < toe_x {
        exponential_curve(x, scale_toe, toe_x, toe_y)
    } else if x <= shoulder_x {
        SIGMOID_SLOPE * x + intercept
    } else {
        exponential_curve(x, scale_shoulder, shoulder_x, shoulder_y)
    }
}

fn linear_breakpoint(numerator: f64, slope: f64, coordinate: f64) -> f64 {
    numerator / (slope * slope + 1.0).sqrt() + coordinate
}

fn scale_factor(limit_x: f64, limit_y: f64, transition_x: f64, transition_y: f64) -> f64 {
    let span_x = limit_x - transition_x;
    let span_y = limit_y - transition_y;
    let term_a = (SIGMOID_SLOPE * span_x).powf(-SIGMOID_POWER);
    let term_b = (SIGMOID_SLOPE * span_x / span_y).powf(SIGMOID_POWER) - 1.0;
    (term_a * term_b).powf(-1.0 / SIGMOID_POWER)
}

fn exponential_curve(x: f64, scale: f64, transition_x: f64, transition_y: f64) -> f64 {
    let inner = SIGMOID_SLOPE * (x - transition_x) / scale;
    scale * exponential(inner) + transition_y
}

fn exponential(x: f64) -> f64 {
    x / (1.0 + signed_pow(x, SIGMOID_POWER)).powf(1.0 / SIGMOID_POWER)
}

fn signed_pow(x: f64, power: f64) -> f64 {
    if x < 0.0 {
        -(-x).powf(power)
    } else {
        x.powf(power)
    }
}

fn power_fwd(x: f64) -> f64 {
    if x <= 0.0 {
        0.0
    } else {
        x.powf(2.4)
    }
}

fn encode_power(x: f64) -> f64 {
    let encoded = if x <= 0.0 { 0.0 } else { x.powf(1.0 / 2.4) };
    encoded.clamp(0.0, 1.0)
}

fn rgb_to_hsv(rgb: [f64; 3]) -> [f64; 3] {
    let peak = rgb[0].max(rgb[1]).max(rgb[2]);
    let floor = rgb[0].min(rgb[1]).min(rgb[2]);
    let delta = peak - floor;
    let saturation = if peak == 0.0 { 0.0 } else { delta / peak };
    let hue = if delta == 0.0 {
        0.0
    } else if rgb[0] >= rgb[1] && rgb[0] >= rgb[2] {
        let mut sector = (rgb[1] - rgb[2]) / delta;
        if sector < 0.0 {
            sector += 6.0;
        }
        sector / 6.0
    } else if rgb[1] >= rgb[2] {
        ((rgb[2] - rgb[0]) / delta + 2.0) / 6.0
    } else {
        ((rgb[0] - rgb[1]) / delta + 4.0) / 6.0
    };
    [hue, saturation, peak]
}

fn hsv_to_rgb(hsv: [f64; 3]) -> [f64; 3] {
    let hue = hsv[0].rem_euclid(1.0);
    let saturation = hsv[1];
    let value = hsv[2];
    if saturation == 0.0 {
        return [value, value, value];
    }
    let sector_f = hue * 6.0;
    let sector = sector_f.floor();
    let fraction = sector_f - sector;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * fraction);
    let t = value * (1.0 - saturation * (1.0 - fraction));
    if sector < 1.0 {
        [value, t, p]
    } else if sector < 2.0 {
        [q, value, p]
    } else if sector < 3.0 {
        [p, value, t]
    } else if sector < 4.0 {
        [p, q, value]
    } else if sector < 5.0 {
        [t, p, value]
    } else {
        [value, p, q]
    }
}

fn lerp_hue(start: f64, end: f64, t: f64) -> f64 {
    let delta = end - start;
    let wrapped = if delta > 0.5 {
        end - 1.0
    } else if delta < -0.5 {
        end + 1.0
    } else {
        end
    };
    (start + t * (wrapped - start)).rem_euclid(1.0)
}

fn mul(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn tent_weight(dx: i32, dy: i32) -> f64 {
    if dx == 0 && dy == 0 {
        0.25
    } else if dx == 0 || dy == 0 {
        0.125
    } else {
        0.0625
    }
}

fn bilinear(src: &[f64], width: usize, height: usize, u: f64, v: f64) -> f64 {
    if width == 0 || height == 0 {
        return 0.0;
    }
    let px = u * usize_to_f64(width) - 0.5;
    let py = v * usize_to_f64(height) - 0.5;
    let x0 = floor_i32(px);
    let y0 = floor_i32(py);
    let tx = px - f64::from(x0);
    let ty = py - f64::from(y0);
    let s00 = at(src, width, height, x0, y0);
    let s10 = at(src, width, height, x0.saturating_add(1), y0);
    let s01 = at(src, width, height, x0, y0.saturating_add(1));
    let s11 = at(
        src,
        width,
        height,
        x0.saturating_add(1),
        y0.saturating_add(1),
    );
    s00 * (1.0 - tx) * (1.0 - ty) + s10 * tx * (1.0 - ty) + s01 * (1.0 - tx) * ty + s11 * tx * ty
}

fn at(src: &[f64], width: usize, height: usize, x: i32, y: i32) -> f64 {
    if x < 0 || y < 0 {
        return 0.0;
    }
    let Ok(xu) = usize::try_from(x) else {
        return 0.0;
    };
    let Ok(yu) = usize::try_from(y) else {
        return 0.0;
    };
    if xu >= width || yu >= height {
        return 0.0;
    }
    src.get(yu.saturating_mul(width).saturating_add(xu))
        .copied()
        .unwrap_or(0.0)
}

fn floor_i32(value: f64) -> i32 {
    let mut whole = value.floor();
    let mut acc = 0_i32;
    if whole >= 0.0 {
        while whole >= 1.0 {
            whole -= 1.0;
            acc = acc.saturating_add(1);
        }
    } else {
        while whole <= -1.0 {
            whole += 1.0;
            acc = acc.saturating_sub(1);
        }
    }
    acc
}

fn usize_to_f64(value: usize) -> f64 {
    match u32::try_from(value) {
        Ok(narrow) => f64::from(narrow),
        Err(_) => f64::from(u32::MAX),
    }
}

fn bin_of(luma: f64, bins: usize) -> usize {
    let log_l = luma.log2().clamp(-16.0, 15.999);
    let scaled = (log_l + 16.0) / 32.0 * usize_to_f64(bins);
    let index = floor_usize(scaled);
    if index >= bins {
        bins - 1
    } else {
        index
    }
}

fn bin_center(index: usize, bins: usize) -> f64 {
    let width = 32.0 / usize_to_f64(bins);
    -16.0 + (usize_to_f64(index) + 0.5) * width
}

fn floor_usize(value: f64) -> usize {
    usize::try_from(floor_u32(value)).unwrap_or(0)
}

fn floor_u32(value: f64) -> u32 {
    u32::try_from(floor_i32(value.max(0.0))).unwrap_or(u32::MAX)
}
