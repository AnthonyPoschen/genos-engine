//! Analytic gates for specular reflections, mirrors, portals, and water.
//!
//! Module rows call [`genos_math::optics`] (the GLSL mirror is
//! `crates/render/shaders/reflections.glsl`) or the CPU picture, and
//! [`Fixture::Live`] grades them. The picture rows compare that picture with
//! [`crate::reflections_trace`], an f64 oracle that does not call `optics` or
//! the shade. Comparator
//! rows stay `NOT-IMPLEMENTED` on live. `denoise_lag_frames` stays
//! `NOT-IMPLEMENTED` on every fixture until a reflection denoiser exists.
//!
//! Thresholds are the gates in `docs/systems/reflections-design.md`. A metric passes
//! when its value is finite and `value <= threshold`.

use std::path::{Path, PathBuf};

use genos_math::optics::{self, PortalLink, MIRROR_FRESNEL_CUTOFF};
use genos_math::{Mat3, Vec3};
use genos_render::PictureFault;

use crate::reflections_trace;

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

/// Who a row measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    /// [`genos_math::optics`]. Live grades this.
    Module,
    /// Good and bad only. Live is `NOT-IMPLEMENTED`.
    Comparator,
    /// No implementation. Every fixture is `NOT-IMPLEMENTED`.
    Absent,
}

/// One gate.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    pub name: &'static str,
    /// Engine or candidate measurement. `None` when the feature is absent.
    pub value: Option<f64>,
    /// Analytic reference. Set on a live comparator row.
    pub reference: Option<f64>,
    pub threshold: f64,
    pub status: Status,
    row: Row,
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
        u8::from(
            self.metrics
                .iter()
                .any(|metric| metric.status == Status::Fail),
        )
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
        Fixture::Live => Report {
            fixture,
            metrics: good.iter().map(live_row).collect(),
        },
    }
}

fn live_row(metric: &Metric) -> Metric {
    match metric.row {
        Row::Module | Row::Absent => metric.clone(),
        Row::Comparator => Metric {
            name: metric.name,
            value: None,
            reference: metric.value,
            threshold: metric.threshold,
            status: Status::NotImplemented,
            row: Row::Comparator,
        },
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Good,
    Bad,
}

fn measure(kind: Kind) -> Vec<Metric> {
    vec![
        gate("mirror_law_rad", mirror_law_rad(kind), 1.0e-4, Row::Module),
        gate("mirror_point_m", mirror_point_m(kind), 1.0e-5, Row::Module),
        gate("fresnel_abs", fresnel_abs(kind), 0.02, Row::Module),
        gate(
            "fresnel_below_abs",
            fresnel_below_abs(kind),
            0.02,
            Row::Module,
        ),
        gate("snell_rad", snell_rad(kind), 1.0e-4, Row::Module),
        gate("ggx_energy", ggx_energy_err(kind), 0.03, Row::Module),
        gate("ssr_depth_rel", ssr_depth_rel(kind), 0.02, Row::Comparator),
        absent("denoise_lag_frames", 2.0),
        gate("mirror_view_frac", mirror_view_frac(kind), 0.0, Row::Module),
        gate("portal_leak", portal_leak(kind), 0.0, Row::Module),
        gate("portal_point_m", portal_point_m(kind), 1.0e-5, Row::Module),
        gate("beer_rel", beer_rel(kind), 0.02, Row::Module),
        gate("shoreline_ratio", shoreline_ratio(kind), 0.75, Row::Module),
        gate("caustic_rel", caustic_rel(kind), 0.05, Row::Comparator),
        gate(
            "near_weighted_rel",
            near_weighted_rel(kind),
            0.03,
            Row::Module,
        ),
        gate(
            "flicker_reversal",
            flicker_reversal(kind),
            0.002,
            Row::Module,
        ),
        gate(
            "low_reflection_rays",
            low_reflection_rays(kind),
            0.0,
            Row::Module,
        ),
        gate("ms_frac", ms_frac(kind), 1.0, Row::Comparator),
        gate("vram_frac", vram_frac(kind), 1.0, Row::Comparator),
    ]
}

fn gate(name: &'static str, value: f64, threshold: f64, row: Row) -> Metric {
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
        row,
    }
}

fn absent(name: &'static str, threshold: f64) -> Metric {
    Metric {
        name,
        value: None,
        reference: None,
        threshold,
        status: Status::NotImplemented,
        row: Row::Absent,
    }
}

fn to64(value: Vec3) -> [f64; 3] {
    [f64::from(value.x), f64::from(value.y), f64::from(value.z)]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
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

fn mirror_law_rad(kind: Kind) -> f64 {
    let cases = [
        (Vec3::new(0.6, -0.8, 0.0), Vec3::Y, [0.6_f64, 0.8, 0.0]),
        (Vec3::new(0.0, -1.0, 0.0), Vec3::Y, [0.0_f64, 1.0, 0.0]),
    ];
    let mut max_err: f64 = 0.0;
    for (incident, normal, baked) in cases {
        let candidate = if kind == Kind::Good {
            to64(optics::mirror_dir(incident, normal))
        } else {
            to64(incident)
        };
        max_err = max_err.max(angle(candidate, baked));
    }
    max_err
}

fn mirror_point_m(kind: Kind) -> f64 {
    let diag = Vec3::new(0.0, 1.0, 1.0).normalize();
    let cases = [
        (Vec3::new(1.0, 2.0, 3.0), Vec3::Y, [1.0_f64, -2.0, 3.0]),
        (Vec3::new(0.0, 1.0, 0.0), diag, [0.0_f64, 0.0, -1.0]),
    ];
    let mut max_err: f64 = 0.0;
    for (point, normal, baked) in cases {
        let candidate = if kind == Kind::Good {
            to64(optics::mirror_point(point, normal))
        } else {
            let along = f64::from(normal.dot(point));
            let n = to64(normal);
            let p = to64(point);
            [
                p[0] - along * n[0],
                p[1] - along * n[1],
                p[2] - along * n[2],
            ]
        };
        max_err = max_err.max(dist(candidate, baked));
    }
    max_err
}

fn fresnel_schlick(ior: f64, cos_i: f64) -> f64 {
    let f0 = ((ior - 1.0) / (ior + 1.0)).powi(2);
    let facing = 1.0 - cos_i.clamp(0.0, 1.0);
    f0 + (1.0 - f0) * facing.powi(5)
}

fn fresnel_abs(kind: Kind) -> f64 {
    // Baked from an independent evaluation of the Fresnel equations.
    let table: [(f32, f32, f64); 12] = [
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
            f64::from(optics::fresnel_dielectric(ior, cos_i))
        } else {
            fresnel_schlick(f64::from(ior), f64::from(cos_i))
        };
        max_err = max_err.max((value - baked).abs());
    }
    max_err
}

fn fresnel_below_abs(kind: Kind) -> f64 {
    // From water into air: eta = n_air / n_water. Baked independently.
    // cos 0.50 and 0 are past the critical angle, so the reflectance is 1.
    let eta_below: f32 = 1.0 / 1.333;
    let table: [(f32, f64); 6] = [
        (1.00, 0.020_373_187_842),
        (0.90, 0.022_696_188_765),
        (0.75, 0.068_207_186_263),
        (0.70, 0.162_572_148_230),
        (0.50, 1.0),
        (0.00, 1.0),
    ];
    let mut max_err: f64 = 0.0;
    for (cos_i, baked) in table {
        let eta = if kind == Kind::Good { eta_below } else { 1.333 };
        let value = f64::from(optics::fresnel_dielectric(eta, cos_i));
        max_err = max_err.max((value - baked).abs());
    }
    max_err
}

fn snell_rad(kind: Kind) -> f64 {
    let normal = Vec3::Y;
    let water: f32 = 1.0 / 1.333;
    let baked_oblique = [0.450_112_528_132_033_f64, -0.892_971_842_791_579_8, 0.0];
    let cases = [
        (Vec3::new(0.6, -0.8, 0.0), water, Some(baked_oblique)),
        (Vec3::new(0.0, -1.0, 0.0), water, Some([0.0_f64, -1.0, 0.0])),
        (Vec3::new(0.95, -0.05, 0.0).normalize(), 1.333, None),
    ];
    let mut max_err: f64 = 0.0;
    for (incident, eta, baked) in cases {
        let candidate = if kind == Kind::Good {
            optics::refract_dir(incident, normal, eta).map(to64)
        } else {
            Some(to64(optics::mirror_dir(incident, normal)))
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
    let alphas = [0.25_f32, 0.5, 0.7];
    let mut max_err: f64 = 0.0;
    for alpha in alphas {
        let energy = ggx_integral(alpha, kind == Kind::Bad);
        max_err = max_err.max((energy - 1.0).abs());
    }
    max_err
}

fn ggx_integral(alpha: f32, drop_pi: bool) -> f64 {
    let steps: u16 = 4096;
    let dmu = 1.0_f32 / f32::from(steps);
    let mut sum: f64 = 0.0;
    for i in 0..steps {
        let mu = (f32::from(i) + 0.5) * dmu;
        let density = if drop_pi {
            let a2 = alpha * alpha;
            let den = mu * mu * (a2 - 1.0) + 1.0;
            a2 / (den * den)
        } else {
            optics::ggx_ndf(mu, alpha)
        };
        sum += f64::from(density * mu);
    }
    sum * f64::from(dmu) * 2.0 * std::f64::consts::PI
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

fn mirror_view_frac(kind: Kind) -> f64 {
    // Potato, Low, Medium, High. Counts are baked for cutoff 0.02.
    // The water chain stops after one bounce. The bright chain fills the cap.
    let caps = [0u32, 1, 2, 3];
    let chains: [(&[f32], [u32; 4]); 2] = [
        (&[0.0204, 0.0597], [0, 1, 1, 1]),
        (&[0.5, 0.5, 0.5, 0.5, 0.5], [0, 1, 2, 3]),
    ];
    let mut max_frac: f64 = 0.0;
    for (chain, expected) in chains {
        for (index, cap) in caps.into_iter().enumerate() {
            let emitted = if kind == Kind::Good {
                optics::mirror_views(chain, cap, MIRROR_FRESNEL_CUTOFF)
            } else {
                let count = u32::try_from(chain.len()).unwrap_or(0);
                count.min(cap)
            };
            max_frac = max_frac.max(view_mismatch(emitted, expected[index], cap));
        }
    }
    max_frac
}

fn view_mismatch(emitted: u32, expected: u32, cap: u32) -> f64 {
    if cap == 0 {
        if emitted == 0 {
            0.0
        } else {
            99.0
        }
    } else {
        f64::from(emitted.abs_diff(expected)) / f64::from(cap)
    }
}

fn fault_of(kind: Kind) -> PictureFault {
    match kind {
        Kind::Good => PictureFault::default(),
        Kind::Bad => PictureFault::all(),
    }
}

fn portal_leak(kind: Kind) -> f64 {
    genos_render::portal_leak(fault_of(kind))
}

/// Quarter-turn about Y into a quarter-turn about X. Neither basis is identity,
/// so dropping `transpose(from_basis)` moves the point.
fn turned_portal() -> PortalLink {
    PortalLink {
        from_origin: Vec3::ZERO,
        from_basis: Mat3::from_cols([0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]),
        to_origin: Vec3::new(10.0, 0.0, 0.0),
        to_basis: Mat3::from_cols([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0]),
    }
}

/// The rigid map with `from_basis` applied instead of its inverse.
fn portal_point_no_inverse(link: PortalLink, point: Vec3) -> Vec3 {
    let local = link.from_basis.transform(point - link.from_origin);
    let flipped = Vec3::new(local.x, local.y, -local.z);
    link.to_origin + link.to_basis.transform(flipped)
}

fn portal_point_m(kind: Kind) -> f64 {
    // Hand map: inverse quarter-turn Y, flip Z, quarter-turn X, then +(10, 0, 0).
    let cases = [
        (Vec3::new(1.0, 2.0, 3.0), [7.0_f64, 1.0, 2.0]),
        (Vec3::new(0.0, 0.0, 1.0), [9.0_f64, 0.0, 0.0]),
    ];
    let link = turned_portal();
    let mut max_err: f64 = 0.0;
    for (point, baked) in cases {
        let got = if kind == Kind::Good {
            to64(optics::portal_point(link, point))
        } else {
            to64(portal_point_no_inverse(link, point))
        };
        max_err = max_err.max(dist(got, baked));
    }
    max_err
}

fn beer_rel(kind: Kind) -> f64 {
    let sigma: f32 = 0.5;
    // `exp(-sigma * distance)`, baked independently.
    let table: [(f32, f64); 4] = [
        (0.0, 1.0),
        (1.0, 0.606_530_659_713),
        (2.0, 0.367_879_441_171),
        (4.0, 0.135_335_283_237),
    ];
    let mut max_err: f64 = 0.0;
    for (distance, baked) in table {
        let value = if kind == Kind::Good {
            f64::from(optics::beer(sigma, distance))
        } else {
            f64::from((1.0 - sigma * distance).max(0.0))
        };
        max_err = max_err.max((value - baked).abs() / baked.max(1.0e-6));
    }
    max_err
}

fn shoreline_ratio(kind: Kind) -> f64 {
    genos_render::shoreline_ratio(fault_of(kind))
}

fn caustic_rel(kind: Kind) -> f64 {
    // Comparator only. The flat-water function lives in `optics` and is not this row.
    let truth: f64 = 0.979_626_812_158;
    let bins = if kind == Kind::Good {
        [truth; 8]
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
    near_weighted(fault_of(kind))
}

fn near_weighted(fault: PictureFault) -> f64 {
    let engine = genos_render::shade_picture(0, fault);
    let reference = reflections_trace::trace();
    if engine.color.len() != reference.color.len() || reference.depth.len() != engine.color.len() {
        return f64::INFINITY;
    }
    let mut num: f64 = 0.0;
    let mut den: f64 = 0.0;
    for (index, pixel) in engine.color.iter().enumerate() {
        let rel = pixel_rel(*pixel, reference.color[index]);
        let depth = reference.depth[index];
        let weight = 1.0 / depth.max(0.25);
        num += weight * rel;
        den += weight;
    }
    if den <= 0.0 {
        f64::INFINITY
    } else {
        num / den
    }
}

fn pixel_rel(engine: [f32; 3], reference: [f64; 3]) -> f64 {
    let mut sum = 0.0;
    for channel in 0..3 {
        let err = (f64::from(engine[channel]) - reference[channel]).abs();
        let scale = reference[channel].abs().max(1.0e-3);
        sum += err / scale;
    }
    sum / 3.0
}

fn flicker_reversal(kind: Kind) -> f64 {
    flicker(fault_of(kind))
}

fn flicker(fault: PictureFault) -> f64 {
    let index = still_index();
    let reference = reflections_trace::still_pixel(index);
    let mut samples = [0.0; 8];
    for (frame, sample) in samples.iter_mut().enumerate() {
        let frame_u = u32::try_from(frame).unwrap_or(0);
        let picture = genos_render::shade_picture(frame_u, fault);
        let pixel = picture.color.get(index).copied().unwrap_or([0.0; 3]);
        let mean = f64::from(pixel[0] + pixel[1] + pixel[2]) / 3.0;
        *sample = mean - reference;
    }
    reversal(&samples)
}

fn reversal(samples: &[f64]) -> f64 {
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

fn still_index() -> usize {
    let tile = usize::try_from(genos_render::TILE).unwrap_or(8);
    let mid = tile / 2;
    mid * tile + mid
}

fn low_reflection_rays(kind: Kind) -> f64 {
    f64::from(genos_render::scheduled_low_rays(fault_of(kind)))
}

struct BudgetRow {
    cap_ms: f64,
    used_ms: f64,
    cap_mb: f64,
    used_mb: f64,
}

fn budget_rows(kind: Kind) -> [BudgetRow; 4] {
    let mut rows = [
        BudgetRow {
            cap_ms: 0.05,
            used_ms: 0.04,
            cap_mb: 0.0,
            used_mb: 0.0,
        },
        BudgetRow {
            cap_ms: 0.40,
            used_ms: 0.30,
            cap_mb: 8.0,
            used_mb: 6.2,
        },
        BudgetRow {
            cap_ms: 0.70,
            used_ms: 0.55,
            cap_mb: 16.0,
            used_mb: 9.5,
        },
        BudgetRow {
            cap_ms: 1.00,
            used_ms: 0.80,
            cap_mb: 32.0,
            used_mb: 21.1,
        },
    ];
    if kind == Kind::Bad {
        rows[3].used_ms = 2.5;
        rows[3].used_mb = 80.0;
    }
    rows
}

fn cap_ratio(used: f64, cap: f64) -> f64 {
    if cap <= 0.0 {
        if used <= 0.0 {
            0.0
        } else {
            99.0
        }
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
    use super::{grade, resolve_out, write_report, Fixture, Row, Status};

    #[test]
    fn a_portal_without_the_inverse_misses_the_baked_turn() {
        let report_good = grade(Fixture::Good);
        let report_bad = grade(Fixture::Bad);
        let good = report_good
            .metrics
            .iter()
            .find(|metric| metric.name == "portal_point_m")
            .unwrap_or_else(|| panic!("portal_point_m"));
        let bad = report_bad
            .metrics
            .iter()
            .find(|metric| metric.name == "portal_point_m")
            .unwrap_or_else(|| panic!("portal_point_m"));
        let good_value = good.value.unwrap_or_else(|| panic!("good value"));
        let bad_value = bad.value.unwrap_or_else(|| panic!("bad value"));
        assert!(good_value <= 1.0e-5, "{good_value}");
        assert!(bad_value > 1.0e-5, "{bad_value}");
    }

    #[test]
    fn the_path_tracer_matches_the_faithful_picture() {
        let engine = genos_render::shade_picture(0, genos_render::PictureFault::default());
        let reference = crate::reflections_trace::trace();
        assert_eq!(engine.color.len(), reference.color.len());
        let mut max_err: f64 = 0.0;
        for (got, truth) in engine.color.iter().zip(reference.color.iter()) {
            for channel in 0..3 {
                max_err = max_err.max((f64::from(got[channel]) - truth[channel]).abs());
            }
        }
        assert!(max_err < 1.0e-4, "{max_err}");
        assert!(
            crate::reflections_trace::still_pixel(super::still_index()).abs() > 0.1,
            "the still pixel must be lit so a blink can fail"
        );
    }

    #[test]
    fn a_near_bias_fails_near_weighted_rel() {
        let faithful = super::near_weighted(genos_render::PictureFault::default());
        let planted = super::near_weighted(genos_render::PictureFault::only(
            genos_render::PictureFault::NEAR_BIAS,
        ));
        assert!(faithful <= 0.03, "{faithful}");
        assert!(planted > 0.03, "{planted}");
    }

    #[test]
    fn a_blink_fails_flicker_reversal() {
        let still = super::flicker(genos_render::PictureFault::default());
        let planted = super::flicker(genos_render::PictureFault::only(
            genos_render::PictureFault::BLINK,
        ));
        assert!(still <= 0.002, "{still}");
        assert!(planted > 0.002, "{planted}");
    }

    #[test]
    fn good_passes_every_implemented_gate() {
        let report = grade(Fixture::Good);
        assert_eq!(report.metrics.len(), 19);
        assert_eq!(report.exit_code(), 0);
        for metric in &report.metrics {
            if metric.name == "denoise_lag_frames" {
                assert_eq!(metric.status, Status::NotImplemented, "{}", metric.name);
                assert!(metric.value.is_none(), "{}", metric.name);
                continue;
            }
            assert_eq!(metric.status, Status::Pass, "{}", metric.name);
            let value = metric.value.unwrap_or_else(|| panic!("{}", metric.name));
            assert!(value <= metric.threshold, "{}", metric.name);
        }
        let text = report.summary();
        assert_eq!(text.lines().count(), 19);
        assert!(!text.contains("FAIL"));
        assert_eq!(text.matches("NOT-IMPLEMENTED").count(), 1);
    }

    #[test]
    fn bad_fails_every_implemented_gate() {
        let report = grade(Fixture::Bad);
        assert_eq!(report.exit_code(), 1);
        for metric in &report.metrics {
            if metric.name == "denoise_lag_frames" {
                assert_eq!(metric.status, Status::NotImplemented, "{}", metric.name);
                continue;
            }
            assert_eq!(metric.status, Status::Fail, "{}", metric.name);
            let value = metric.value.unwrap_or_else(|| panic!("{}", metric.name));
            assert!(value > metric.threshold, "{}", metric.name);
        }
    }

    #[test]
    fn live_grades_the_module_and_leaves_comparators() {
        let report = grade(Fixture::Live);
        assert_eq!(report.exit_code(), 0);
        let good = grade(Fixture::Good);
        for (live, proven) in report.metrics.iter().zip(good.metrics.iter()) {
            assert!(
                (live.threshold - proven.threshold).abs() <= f64::EPSILON,
                "{}",
                live.name
            );
            if live.name == "denoise_lag_frames" {
                assert_eq!(live.status, Status::NotImplemented, "{}", live.name);
                assert!(live.value.is_none() && live.reference.is_none());
                continue;
            }
            if proven.row == Row::Module {
                assert_eq!(live.status, Status::Pass, "{}", live.name);
                assert_eq!(live.value, proven.value, "{}", live.name);
                continue;
            }
            assert_eq!(live.status, Status::NotImplemented, "{}", live.name);
            assert!(live.value.is_none(), "{}", live.name);
            assert_eq!(live.reference, proven.value, "{}", live.name);
        }
        let text = report.summary();
        assert!(text.contains("fresnel_below_abs"));
        assert!(text.contains("denoise_lag_frames value=na"));
        assert!(text.contains("caustic_rel value=na"));
        assert!(text.contains("mirror_view_frac value=0"));
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
        let text =
            std::fs::read_to_string(dir.join("metrics.json")).unwrap_or_else(|err| panic!("{err}"));
        assert!(text.contains("\"fixture\": \"good\""));
        assert!(text.contains("\"status\": \"PASS\""));
        assert!(text.contains("denoise_lag_frames"));
        assert!(text.contains("NOT-IMPLEMENTED"));
        std::fs::remove_dir_all(&dir).unwrap_or_else(|err| panic!("{err}"));
    }
}
