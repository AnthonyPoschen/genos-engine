//! Analytic gates for specular reflections, mirrors, portals, and water.
//!
//! The renderer feature is not built. [`grade`] with [`Fixture::Live`] runs the
//! analytic reference and reports every metric `NOT-IMPLEMENTED`. [`Fixture::Good`]
//! and [`Fixture::Bad`] prove the same checks on a known candidate and a broken one.
//!
//! Thresholds are the gates in `docs/systems/reflections-design.md`. A metric passes
//! when its value is finite and `value <= threshold`.

use std::path::{Path, PathBuf};

/// Which candidate the checker grades.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixture {
    /// The engine feature is absent. Every metric is `NOT-IMPLEMENTED`.
    Live,
    /// The analytic candidate that meets every gate.
    Good,
    /// A deliberately broken candidate. Every metric fails.
    Bad,
}

impl Fixture {
    /// `live`, `good`, or `bad`.
    ///
    /// # Errors
    ///
    /// Returns an error when `text` is not `live`, `good`, or `bad`.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "live" => Ok(Self::Live),
            "good" => Ok(Self::Good),
            "bad" => Ok(Self::Bad),
            _ => Err("fixture is live, good, or bad".to_string()),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Good => "good",
            Self::Bad => "bad",
        }
    }
}

/// Pass, fail, or the feature is not in the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    NotImplemented,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotImplemented => "NOT-IMPLEMENTED",
        }
    }
}

/// One gate.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    pub name: &'static str,
    /// Engine or candidate measurement. `None` when the feature is absent.
    pub value: Option<f64>,
    /// Analytic reference. Set on the live report so the reference is not skipped.
    pub reference: Option<f64>,
    pub threshold: f64,
    pub status: Status,
}

/// The full gate report.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub fixture: Fixture,
    pub metrics: Vec<Metric>,
}

impl Report {
    /// `1` when any metric failed. `NOT-IMPLEMENTED` is not a failure.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        u8::from(self.metrics.iter().any(|metric| metric.status == Status::Fail))
    }

    /// One compact line per metric: name, value, threshold, status.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for metric in &self.metrics {
            out.push_str(metric.name);
            out.push_str(" value=");
            out.push_str(&fmt_num_opt(metric.value));
            out.push_str(" threshold=");
            out.push_str(&fmt_num(metric.threshold));
            out.push(' ');
            out.push_str(metric.status.label());
            out.push('\n');
        }
        out
    }

    /// JSON object written to `metrics.json`.
    #[must_use]
    pub fn json(&self) -> String {
        let mut out = String::from("{\n  \"fixture\": \"");
        out.push_str(self.fixture.key());
        out.push_str("\",\n  \"area\": \"reflections\",\n  \"metrics\": [\n");
        for (index, metric) in self.metrics.iter().enumerate() {
            if index > 0 {
                out.push_str(",\n");
            }
            out.push_str("    {\"name\": \"");
            out.push_str(metric.name);
            out.push_str("\", \"value\": ");
            out.push_str(&json_num_opt(metric.value));
            out.push_str(", \"reference\": ");
            out.push_str(&json_num_opt(metric.reference));
            out.push_str(", \"threshold\": ");
            out.push_str(&fmt_num(metric.threshold));
            out.push_str(", \"status\": \"");
            out.push_str(metric.status.label());
            out.push_str("\"}");
        }
        out.push_str("\n  ]\n}\n");
        out
    }
}

/// `--out` wins. The environment mirror applies only when `--out` was omitted.
/// The default is `./genos-debug/reflections`.
#[must_use]
pub fn resolve_out(cli: Option<&str>, env: Option<&str>) -> PathBuf {
    for candidate in [cli, env] {
        if let Some(dir) = candidate.map(str::trim).filter(|dir| !dir.is_empty()) {
            return PathBuf::from(dir);
        }
    }
    PathBuf::from("genos-debug").join("reflections")
}

/// Write `metrics.json` under `dir`.
///
/// # Errors
///
/// Returns an error when the directory cannot be created or the file cannot be written.
pub fn write_report(dir: &Path, report: &Report) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let path = dir.join("metrics.json");
    std::fs::write(&path, report.json()).map_err(|err| err.to_string())
}

/// Grade one fixture.
#[must_use]
pub fn grade(fixture: Fixture) -> Report {
    let good = measure(Kind::Good);
    match fixture {
        Fixture::Good => Report {
            fixture,
            metrics: good,
        },
        Fixture::Bad => Report {
            fixture,
            metrics: measure(Kind::Bad),
        },
        Fixture::Live => {
            if good.iter().any(|metric| metric.status != Status::Pass) {
                return Report {
                    fixture,
                    metrics: good,
                };
            }
            let metrics = good.iter().map(live_metric).collect();
            Report { fixture, metrics }
        }
    }
}

fn live_metric(metric: &Metric) -> Metric {
    Metric {
        name: metric.name,
        value: None,
        reference: metric.value,
        threshold: metric.threshold,
        status: Status::NotImplemented,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Good,
    Bad,
}

fn measure(kind: Kind) -> Vec<Metric> {
    vec![
        gate("mirror_law_rad", mirror_law_rad(kind), 1.0e-4),
        gate("mirror_point_m", mirror_point_m(kind), 1.0e-5),
        gate("fresnel_abs", fresnel_abs(kind), 0.02),
        gate("snell_rad", snell_rad(kind), 1.0e-4),
        gate("ggx_energy", ggx_energy_err(kind), 0.03),
        gate("ssr_depth_rel", ssr_depth_rel(kind), 0.02),
        gate("denoise_lag_frames", denoise_lag_frames(kind), 2.0),
        gate("mirror_view_frac", mirror_view_frac(kind), 1.0),
        gate("portal_leak", portal_leak(kind), 0.0),
        gate("portal_point_m", portal_point_m(kind), 1.0e-5),
        gate("beer_rel", beer_rel(kind), 0.02),
        gate("shoreline_ratio", shoreline_ratio(kind), 0.75),
        gate("caustic_rel", caustic_rel(kind), 0.05),
        gate("near_weighted_rel", near_weighted_rel(kind), 0.03),
        gate("flicker_reversal", flicker_reversal(kind), 0.002),
        gate("low_reflection_rays", low_reflection_rays(kind), 0.0),
        gate("ms_frac", ms_frac(kind), 1.0),
        gate("vram_frac", vram_frac(kind), 1.0),
    ]
}

fn gate(name: &'static str, value: f64, threshold: f64) -> Metric {
    let status = if value.is_finite() && value <= threshold {
        Status::Pass
    } else {
        Status::Fail
    };
    Metric {
        name,
        value: Some(value),
        reference: None,
        threshold,
        status,
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let len = length(a);
    if len <= f64::MIN_POSITIVE {
        return [0.0, 0.0, 0.0];
    }
    [a[0] / len, a[1] / len, a[2] / len]
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    length([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
}

fn angle(a: [f64; 3], b: [f64; 3]) -> f64 {
    let da = length(a);
    let db = length(b);
    if da <= f64::MIN_POSITIVE || db <= f64::MIN_POSITIVE {
        return std::f64::consts::PI;
    }
    let cosine = (dot(a, b) / (da * db)).clamp(-1.0, 1.0);
    cosine.acos()
}

/// GLSL `reflect`: `I - 2 dot(N, I) N`. `I` points at the surface.
fn reflect_dir(incident: [f64; 3], normal: [f64; 3]) -> [f64; 3] {
    let d = dot(normal, incident);
    [
        incident[0] - 2.0 * d * normal[0],
        incident[1] - 2.0 * d * normal[1],
        incident[2] - 2.0 * d * normal[2],
    ]
}

fn mirror_law_rad(kind: Kind) -> f64 {
    // Baked from the reflection formula, independent of `reflect_dir`.
    let cases = [
        (
            [0.6, -0.8, 0.0],
            [0.0, 1.0, 0.0],
            [0.6, 0.8, 0.0],
        ),
        (
            [0.0, -1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ),
    ];
    let mut max_err: f64 = 0.0;
    for (incident, normal, baked) in cases {
        let candidate = if kind == Kind::Good {
            reflect_dir(incident, normal)
        } else {
            incident
        };
        max_err = max_err.max(angle(candidate, baked));
    }
    max_err
}

fn mirror_point(point: [f64; 3], normal: [f64; 3], factor: f64) -> [f64; 3] {
    let d = dot(point, normal);
    [
        point[0] - factor * d * normal[0],
        point[1] - factor * d * normal[1],
        point[2] - factor * d * normal[2],
    ]
}

fn mirror_point_m(kind: Kind) -> f64 {
    let factor = if kind == Kind::Good { 2.0 } else { 1.0 };
    let axis_n = [0.0, 1.0, 0.0];
    let diag_n = normalize([0.0, 1.0, 1.0]);
    let cases = [
        ([1.0, 2.0, 3.0], axis_n, [1.0, -2.0, 3.0]),
        ([0.0, 1.0, 0.0], diag_n, [0.0, 0.0, -1.0]),
    ];
    let mut max_err: f64 = 0.0;
    for (point, normal, baked) in cases {
        max_err = max_err.max(dist(mirror_point(point, normal, factor), baked));
    }
    max_err
}

/// Unpolarized dielectric Fresnel. `ior` is the transmitted index. Incident index is 1.
fn fresnel_exact(ior: f64, cos_i: f64) -> f64 {
    let cos_i = cos_i.clamp(0.0, 1.0);
    let sin2_i = (1.0 - cos_i * cos_i).max(0.0);
    let sin2_t = sin2_i / (ior * ior);
    if sin2_t >= 1.0 {
        return 1.0;
    }
    let cos_t = (1.0 - sin2_t).sqrt();
    let rs = (cos_i - ior * cos_t) / (cos_i + ior * cos_t);
    let rp = (ior * cos_i - cos_t) / (ior * cos_i + cos_t);
    0.5 * (rs * rs + rp * rp)
}

fn fresnel_schlick(ior: f64, cos_i: f64) -> f64 {
    let f0 = ((ior - 1.0) / (ior + 1.0)).powi(2);
    let facing = 1.0 - cos_i.clamp(0.0, 1.0);
    f0 + (1.0 - f0) * facing.powi(5)
}

fn fresnel_abs(kind: Kind) -> f64 {
    // Baked from an independent evaluation of the Fresnel equations.
    let table = [
        (1.333, 1.00, 0.020_373_187_842),
        (1.333, 0.75, 0.025_282_650_921),
        (1.333, 0.50, 0.059_690_919_175),
        (1.333, 0.25, 0.223_342_309_908),
        (1.333, 0.10, 0.539_842_100_728),
        (1.333, 0.00, 1.0),
        (1.5, 1.00, 0.040_000_000_000),
        (1.5, 0.75, 0.046_780_747_425),
        (1.5, 0.50, 0.089_186_712_802),
        (1.5, 0.25, 0.264_190_373_434),
        (1.5, 0.10, 0.571_592_520_342),
        (1.5, 0.00, 1.0),
    ];
    let mut max_err: f64 = 0.0;
    for (ior, cos_i, baked) in table {
        let value = if kind == Kind::Good {
            fresnel_exact(ior, cos_i)
        } else {
            fresnel_schlick(ior, cos_i)
        };
        max_err = max_err.max((value - baked).abs());
    }
    max_err
}

/// GLSL `refract`. `None` is total internal reflection.
fn refract(incident: [f64; 3], normal: [f64; 3], eta: f64) -> Option<[f64; 3]> {
    let d = dot(normal, incident);
    let k = 1.0 - eta * eta * (1.0 - d * d);
    if k < 0.0 {
        return None;
    }
    let scale = eta * d + k.sqrt();
    Some([
        eta * incident[0] - scale * normal[0],
        eta * incident[1] - scale * normal[1],
        eta * incident[2] - scale * normal[2],
    ])
}

fn snell_rad(kind: Kind) -> f64 {
    let normal = [0.0, 1.0, 0.0];
    let water = 1.0 / 1.333;
    let oblique = [0.6, -0.8, 0.0];
    let baked_oblique = [0.450_112_528_132_033, -0.892_971_842_791_579_8, 0.0];
    let straight = [0.0, -1.0, 0.0];
    let grazing = normalize([0.95, -0.05, 0.0]);
    let cases = [
        (oblique, water, Some(baked_oblique)),
        (straight, water, Some([0.0, -1.0, 0.0])),
        (grazing, 1.333, None),
    ];
    let mut max_err: f64 = 0.0;
    for (incident, eta, baked) in cases {
        let candidate = if kind == Kind::Good {
            refract(incident, normal, eta)
        } else {
            Some(reflect_dir(incident, normal))
        };
        let err = match (candidate, baked) {
            (None, None) => 0.0,
            (Some(got), Some(expect)) => angle(got, expect),
            _ => std::f64::consts::PI,
        };
        max_err = max_err.max(err);
    }
    max_err
}

/// `|∫ D cos dω − 1|` for the GGX NDF. The broken candidate drops the `π` in `D`.
fn ggx_energy_err(kind: Kind) -> f64 {
    let alphas = [0.25, 0.5, 0.7];
    let mut max_err: f64 = 0.0;
    for alpha in alphas {
        let energy = ggx_integral(alpha, kind == Kind::Bad);
        max_err = max_err.max((energy - 1.0).abs());
    }
    max_err
}

fn ggx_integral(alpha: f64, drop_pi: bool) -> f64 {
    let steps = 4096u32;
    let dmu = 1.0 / f64::from(steps);
    let a2 = alpha * alpha;
    let mut sum: f64 = 0.0;
    for i in 0..steps {
        let mu = (f64::from(i) + 0.5) * dmu;
        let den = mu * mu * (a2 - 1.0) + 1.0;
        let mut density = a2 / (den * den);
        if !drop_pi {
            density /= std::f64::consts::PI;
        }
        sum += density * mu;
    }
    sum * dmu * 2.0 * std::f64::consts::PI
}

fn depth_buffer() -> [f64; 64] {
    let mut depth = [8.0; 64];
    let mut i = 24;
    while i < 40 {
        depth[i] = 3.0;
        i += 1;
    }
    depth
}

fn ssr_march(depth: &[f64], dz: f64) -> Option<f64> {
    for (i, surface) in depth.iter().enumerate() {
        let Ok(index) = u32::try_from(i) else {
            return None;
        };
        let z = 2.0 + dz * f64::from(index);
        if z >= *surface {
            return Some(*surface);
        }
    }
    None
}

fn depth_rel(got: Option<f64>, expect: Option<f64>) -> f64 {
    match (got, expect) {
        (None, None) => 0.0,
        (Some(got), Some(expect)) => (got - expect).abs() / expect.max(1.0e-6),
        _ => 1.0,
    }
}

fn ssr_depth_rel(kind: Kind) -> f64 {
    let depth = depth_buffer();
    let hit = if kind == Kind::Good {
        ssr_march(&depth, 0.15)
    } else {
        Some(8.0)
    };
    let miss = if kind == Kind::Good {
        ssr_march(&depth, -0.5)
    } else {
        Some(8.0)
    };
    depth_rel(hit, Some(3.0)).max(depth_rel(miss, None))
}

fn denoise_lag_frames(kind: Kind) -> f64 {
    let mut shaded = 0.2;
    let mut frame = 0u32;
    while frame < 120 {
        let sample = if frame >= 30 { 0.8 } else { 0.2 };
        if kind == Kind::Good {
            shaded = sample;
        } else {
            shaded += 0.08 * (sample - shaded);
        }
        if frame >= 30 && shaded >= 0.74 {
            return f64::from(frame - 30);
        }
        frame += 1;
    }
    120.0
}

fn mirror_view_frac(kind: Kind) -> f64 {
    // Potato, Low, Medium, High extra-view caps. The request is 8 views.
    let caps = [0u32, 1, 2, 3];
    let request = 8u32;
    let mut max_frac: f64 = 0.0;
    for cap in caps {
        let emitted = if kind == Kind::Good {
            request.min(cap)
        } else {
            request
        };
        let frac = if cap == 0 {
            if emitted == 0 { 0.0 } else { 99.0 }
        } else {
            f64::from(emitted) / f64::from(cap)
        };
        max_frac = max_frac.max(frac);
    }
    max_frac
}

fn portal_leak(kind: Kind) -> f64 {
    let samples = [-2.0, -1.0, -0.5, 0.5, 1.0, 2.0];
    let mut wrong = 0u32;
    for z in samples {
        let behind = z < 0.0;
        let drawn = if kind == Kind::Good { !behind } else { true };
        if behind == drawn {
            wrong += 1;
        }
    }
    f64::from(wrong) / f64::from(u32::try_from(samples.len()).unwrap_or(1))
}

fn portal_map(point: [f64; 3], translate: bool, flip: bool) -> [f64; 3] {
    let x = if translate { point[0] + 10.0 } else { point[0] };
    let z = if flip { -point[2] } else { point[2] };
    [x, point[1], z]
}

fn portal_point_m(kind: Kind) -> f64 {
    let translate = kind == Kind::Good;
    let flip = kind == Kind::Good;
    let cases = [
        ([1.0, 2.0, 3.0], [11.0, 2.0, -3.0]),
        ([0.0, 0.0, 1.0], [10.0, 0.0, -1.0]),
    ];
    let mut max_err: f64 = 0.0;
    for (point, baked) in cases {
        max_err = max_err.max(dist(portal_map(point, translate, flip), baked));
    }
    max_err
}

fn beer_rel(kind: Kind) -> f64 {
    let sigma: f64 = 0.5;
    // `exp(-sigma * distance)`, baked independently.
    let table = [
        (0.0, 1.0),
        (1.0, 0.606_530_659_713),
        (2.0, 0.367_879_441_171),
        (4.0, 0.135_335_283_237),
    ];
    let mut max_err: f64 = 0.0;
    for (distance, baked) in table {
        let value = if kind == Kind::Good {
            (-sigma * distance).exp()
        } else {
            (1.0 - sigma * distance).max(0.0)
        };
        max_err = max_err.max((value - baked).abs() / baked.max(1.0e-6));
    }
    max_err
}

fn shoreline_ratio(kind: Kind) -> f64 {
    let deep: f64 = 0.4;
    let cap = 0.75;
    let depths = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0];
    let mut max_ratio: f64 = 0.0;
    for depth in depths {
        let height = if kind == Kind::Good {
            deep.min(cap * depth)
        } else {
            deep
        };
        max_ratio = max_ratio.max(height / depth);
    }
    max_ratio
}

fn water_transmittance() -> f64 {
    let ior: f64 = 1.333;
    let f0 = ((ior - 1.0) / (ior + 1.0)).powi(2);
    1.0 - f0
}

fn caustic_rel(kind: Kind) -> f64 {
    // Flat water, sun at normal incidence. Baked `1 - F0` for IOR 1.333.
    let truth: f64 = 0.979_626_812_158;
    let transmit = water_transmittance();
    let bins = if kind == Kind::Good {
        [transmit, transmit, transmit, transmit, transmit, transmit, transmit, transmit]
    } else {
        [0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0]
    };
    let mut sum: f64 = 0.0;
    for bin in bins {
        sum += (bin - truth).abs() / truth.max(1.0e-6);
    }
    sum / f64::from(u32::try_from(bins.len()).unwrap_or(1))
}

fn near_weighted_rel(kind: Kind) -> f64 {
    let (near_err, far_err) = if kind == Kind::Good {
        (0.0, 0.25)
    } else {
        (0.4, 0.0)
    };
    let samples: [(f64, f64); 8] = [
        (0.5, near_err),
        (0.5, near_err),
        (0.5, near_err),
        (0.5, near_err),
        (20.0, far_err),
        (20.0, far_err),
        (20.0, far_err),
        (20.0, far_err),
    ];
    let mut num: f64 = 0.0;
    let mut den: f64 = 0.0;
    for (depth, err) in samples {
        let weight = 1.0 / depth.max(0.25);
        num += weight * err;
        den += weight;
    }
    if den <= 0.0 {
        return f64::INFINITY;
    }
    num / den
}

fn flicker_reversal(kind: Kind) -> f64 {
    let steady: [f64; 8] = [0.4, 0.4, 0.4, 0.4, 0.4, 0.4, 0.4, 0.4];
    let blink: [f64; 8] = [0.5, 0.3, 0.5, 0.3, 0.5, 0.3, 0.5, 0.3];
    let samples = if kind == Kind::Good { &steady } else { &blink };
    let mut sum: f64 = 0.0;
    let mut reversals = 0u32;
    let mut i = 0;
    while i + 2 < samples.len() {
        let first: f64 = samples[i + 1] - samples[i];
        let second: f64 = samples[i + 2] - samples[i + 1];
        if first * second < 0.0 {
            sum += first.abs().min(second.abs());
            reversals += 1;
        }
        i += 1;
    }
    if reversals == 0 {
        0.0
    } else {
        sum / f64::from(reversals)
    }
}

fn low_reflection_rays(kind: Kind) -> f64 {
    if kind == Kind::Good {
        0.0
    } else {
        64.0
    }
}

struct BudgetRow {
    cap_ms: f64,
    used_ms: f64,
    cap_mb: f64,
    used_mb: f64,
}

fn budget_rows(kind: Kind) -> [BudgetRow; 4] {
    let mut rows = [
        BudgetRow { cap_ms: 0.05, used_ms: 0.04, cap_mb: 0.0, used_mb: 0.0 },
        BudgetRow { cap_ms: 0.40, used_ms: 0.30, cap_mb: 8.0, used_mb: 6.2 },
        BudgetRow { cap_ms: 0.70, used_ms: 0.55, cap_mb: 16.0, used_mb: 9.5 },
        BudgetRow { cap_ms: 1.00, used_ms: 0.80, cap_mb: 32.0, used_mb: 21.1 },
    ];
    if kind == Kind::Bad {
        rows[3].used_ms = 2.5;
        rows[3].used_mb = 80.0;
    }
    rows
}

fn cap_ratio(used: f64, cap: f64) -> f64 {
    if cap <= 0.0 {
        if used <= 0.0 { 0.0 } else { 99.0 }
    } else {
        used / cap
    }
}

fn max_ratio(kind: Kind, memory: bool) -> f64 {
    let mut max_frac: f64 = 0.0;
    for row in budget_rows(kind) {
        let frac = if memory {
            cap_ratio(row.used_mb, row.cap_mb)
        } else {
            cap_ratio(row.used_ms, row.cap_ms)
        };
        max_frac = max_frac.max(frac);
    }
    max_frac
}

fn ms_frac(kind: Kind) -> f64 {
    max_ratio(kind, false)
}

fn vram_frac(kind: Kind) -> f64 {
    max_ratio(kind, true)
}

fn fmt_num(value: f64) -> String {
    if !value.is_finite() {
        return "inf".to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs();
    if !(1.0e-3..1.0e4).contains(&magnitude) {
        return format!("{value:.3e}");
    }
    trim_zeros(format!("{value:.6}"))
}

fn fmt_num_opt(value: Option<f64>) -> String {
    value.map_or_else(|| "na".to_string(), fmt_num)
}

fn json_num_opt(value: Option<f64>) -> String {
    match value {
        Some(value) if value.is_finite() => fmt_num(value),
        Some(_) | None => "null".to_string(),
    }
}

fn trim_zeros(raw: String) -> String {
    if !raw.contains('.') {
        return raw;
    }
    let trimmed = raw.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
#[expect(clippy::panic, reason = "a failed fixture aborts the test")]
mod tests {
    use super::{grade, resolve_out, write_report, Fixture, Status};

    #[test]
    fn good_passes_every_gate() {
        let report = grade(Fixture::Good);
        assert_eq!(report.metrics.len(), 18);
        assert_eq!(report.exit_code(), 0);
        for metric in &report.metrics {
            assert_eq!(metric.status, Status::Pass, "{}", metric.name);
            let value = metric.value.unwrap_or_else(|| panic!("{}", metric.name));
            assert!(value <= metric.threshold, "{}", metric.name);
        }
        let text = report.summary();
        assert_eq!(text.lines().count(), 18);
        assert!(!text.contains("FAIL"));
        assert!(!text.contains("NOT-IMPLEMENTED"));
    }

    #[test]
    fn bad_fails_every_gate() {
        let report = grade(Fixture::Bad);
        assert_eq!(report.exit_code(), 1);
        for metric in &report.metrics {
            assert_eq!(metric.status, Status::Fail, "{}", metric.name);
            let value = metric.value.unwrap_or_else(|| panic!("{}", metric.name));
            assert!(value > metric.threshold, "{}", metric.name);
        }
        let text = report.summary();
        assert!(!text.contains("PASS"));
        assert!(!text.contains("NOT-IMPLEMENTED"));
    }

    #[test]
    fn live_marks_every_metric_unimplemented() {
        let report = grade(Fixture::Live);
        assert_eq!(report.exit_code(), 0);
        let good = grade(Fixture::Good);
        for (live, proven) in report.metrics.iter().zip(good.metrics.iter()) {
            assert_eq!(live.status, Status::NotImplemented, "{}", live.name);
            assert!(live.value.is_none(), "{}", live.name);
            assert_eq!(live.reference, proven.value, "{}", live.name);
            assert!(
                (live.threshold - proven.threshold).abs() <= f64::EPSILON,
                "{}",
                live.name
            );
        }
        let text = report.summary();
        assert!(text.contains("value=na"));
        assert!(!text.contains("PASS"));
        assert!(!text.contains("FAIL"));
    }

    #[test]
    fn out_argument_wins_over_the_mirror() {
        assert_eq!(
            resolve_out(Some("genos-debug/reflections/cli"), Some("/tmp/env")),
            std::path::PathBuf::from("genos-debug/reflections/cli")
        );
        assert_eq!(
            resolve_out(None, Some("genos-debug/reflections/env")),
            std::path::PathBuf::from("genos-debug/reflections/env")
        );
        assert_eq!(
            resolve_out(Some("  "), Some("genos-debug/reflections/env")),
            std::path::PathBuf::from("genos-debug/reflections/env")
        );
        assert_eq!(
            resolve_out(None, None),
            std::path::PathBuf::from("genos-debug/reflections")
        );
    }

    #[test]
    fn report_json_lands_in_the_chosen_directory() {
        let dir = std::env::temp_dir().join(format!("genos-reflections-{}", std::process::id()));
        let report = grade(Fixture::Good);
        write_report(&dir, &report).unwrap_or_else(|err| panic!("{err}"));
        let text = std::fs::read_to_string(dir.join("metrics.json")).unwrap_or_else(|err| panic!("{err}"));
        assert!(text.contains("\"fixture\": \"good\""));
        assert!(text.contains("\"status\": \"PASS\""));
        assert!(!text.contains("NOT-IMPLEMENTED"));
        std::fs::remove_dir_all(&dir).unwrap_or_else(|err| panic!("{err}"));
    }
}
