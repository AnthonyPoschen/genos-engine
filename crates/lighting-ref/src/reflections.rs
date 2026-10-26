//! Gates for specular reflections, mirrors, portals, and water.
//!
//! Rows that have a frame equivalent read the presented picture and compare it
//! with [`crate::reflections_trace`], an f64 trace that does not call
//! `genos_math::optics`. `low_reflection_rays` is the atomic dispatch count.
//! The shoreline rows are the absolute error against an independent wave oracle.
//! `ssr_depth_rel`, `caustic_rel`, `ms_frac`, `vram_frac`, and `denoise_lag_frames`
//! stay `NOT-IMPLEMENTED`. Lavapipe timestamps are not the 4070 or 1060 budget.
//!
//! Thresholds are the gates in `docs/systems/reflections-design.md`. A metric passes
//! when its value is finite and `value <= threshold`. A presented-frame threshold
//! is at least two 8-bit codes (`2/255`). One code is `1/255`, and lavapipe and a
//! device GPU can round a blend to opposite sides of that code. A gate tighter
//! than two codes flakes. Dispatch counts and the wave oracle are not framebuffer
//! values, and they keep their own limits.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use genos_render::PictureFault;

use crate::presented;
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
    /// The presented frame, the ray counter, or the wave oracle. Live grades this.
    Module,
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
    /// `presented frame`, `GPU dispatch count`, `wave oracle`, or empty when absent.
    pub source: &'static str,
    row: Row,
}

/// GPU timestamps and the reflection-image byte count from the Medium frame.
///
/// `ms_frac` and `vram_frac` stay absent: lavapipe time is not the 4070 or
/// 1060 budget. These raw counters are what a later auto-tune would read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    pub gpu_ns: u64,
    pub vram_bytes: u64,
    /// Potato, Low, Medium, High view draws.
    pub views: [u32; 4],
}

/// The full gate report.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub fixture: Fixture,
    pub metrics: Vec<Metric>,
    /// Present when the fixture read the presented frame.
    pub capture: Option<Capture>,
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
            if !metric.source.is_empty() {
                out.push(' ');
                out.push_str(metric.source);
            }
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
            out.push_str("\", \"source\": \"");
            out.push_str(metric.source);
            out.push_str("\"}");
        }
        out.push_str("\n  ]");
        if let Some(capture) = self.capture {
            out.push_str(",\n  \"gpu_ns\": ");
            out.push_str(&capture.gpu_ns.to_string());
            out.push_str(",\n  \"vram_bytes\": ");
            out.push_str(&capture.vram_bytes.to_string());
            out.push_str(",\n  \"views\": [");
            for (index, views) in capture.views.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                out.push_str(&views.to_string());
            }
            out.push(']');
        }
        out.push_str("\n}\n");
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
            capture: capture_budget(),
        },
        Fixture::Bad => Report {
            fixture,
            metrics: measure(Kind::Bad),
            capture: None,
        },
        Fixture::Live => Report {
            fixture,
            metrics: good.clone(),
            capture: capture_budget(),
        },
    }
}

fn capture_budget() -> Option<Capture> {
    let shot = presented::shot().ok()?;
    Some(Capture {
        gpu_ns: shot.gpu_ns,
        vram_bytes: shot.vram_bytes,
        views: shot.views,
    })
}

/// Two 8-bit codes. Presented-frame gates sit on this floor.
const PRESENTED_FLOOR: f64 = 2.0 / 255.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Good,
    Bad,
}

fn measure(kind: Kind) -> Vec<Metric> {
    vec![
        gate(
            "mirror_reflect",
            mirror_reflect(kind),
            PRESENTED_FLOOR,
            "presented frame",
        ),
        gate(
            "mirror_point",
            mirror_point(kind),
            PRESENTED_FLOOR,
            "presented frame",
        ),
        gate("fresnel_abs", fresnel_abs(kind), 0.02, "presented frame"),
        gate(
            "fresnel_below_abs",
            fresnel_below_abs(kind),
            0.02,
            "presented frame",
        ),
        gate(
            "water_f0",
            water_f0(kind),
            PRESENTED_FLOOR,
            "presented frame",
        ),
        gate(
            "snell_transmit",
            snell_transmit(kind),
            PRESENTED_FLOOR,
            "presented frame",
        ),
        gate("ggx_energy", ggx_energy_err(kind), 0.03, "presented frame"),
        absent("ssr_depth_rel", 0.02),
        absent("denoise_lag_frames", 2.0),
        gate(
            "mirror_view_frac",
            mirror_view_frac(kind),
            PRESENTED_FLOOR,
            "presented frame",
        ),
        gate(
            "portal_leak",
            portal_leak(kind),
            PRESENTED_FLOOR,
            "presented frame",
        ),
        gate(
            "portal_point",
            portal_point(kind),
            PRESENTED_FLOOR,
            "presented frame",
        ),
        gate("beer_rel", beer_rel(kind), 0.02, "presented frame"),
        // Larger than an f32 sine residual, and far below the unclamped gap.
        gate(
            "shoreline_ratio",
            shoreline_ratio(kind),
            1.0e-4,
            "wave oracle",
        ),
        gate(
            "shoreline_deep_ratio",
            shoreline_deep_ratio(kind),
            1.0e-4,
            "wave oracle",
        ),
        absent("caustic_rel", 0.05),
        gate(
            "near_weighted_rel",
            near_weighted_rel(kind),
            0.03,
            "presented frame",
        ),
        gate(
            "flicker_reversal",
            flicker_reversal(kind),
            PRESENTED_FLOOR,
            "presented frame",
        ),
        gate(
            "low_reflection_rays",
            low_reflection_rays(kind),
            0.0,
            "GPU dispatch count",
        ),
        absent("ms_frac", 1.0),
        absent("vram_frac", 1.0),
    ]
}

fn gate(name: &'static str, value: f64, threshold: f64, source: &'static str) -> Metric {
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
        source,
        row: Row::Module,
    }
}

fn absent(name: &'static str, threshold: f64) -> Metric {
    Metric {
        name,
        value: None,
        reference: None,
        threshold,
        status: Status::NotImplemented,
        source: "",
        row: Row::Absent,
    }
}

fn broken(kind: Kind, fail: f64, faithful: impl FnOnce() -> f64) -> f64 {
    if kind == Kind::Bad {
        fail
    } else {
        faithful()
    }
}

fn mirror_reflect(kind: Kind) -> f64 {
    // The red reflection target, not the flat F0 patch and not the transmission strip.
    broken(kind, 1.0, || region_abs(0, 20, 32, 44))
}

fn mirror_point(kind: Kind) -> f64 {
    broken(kind, 1.0, || region_abs(0, 60, 80, 120))
}

fn fresnel_abs(kind: Kind) -> f64 {
    broken(kind, 0.059, || region_abs(8, 0, 80, 16))
}

fn fresnel_below_abs(kind: Kind) -> f64 {
    broken(kind, 1.0, || region_abs(80, 0, 160, 60))
}

fn water_f0(kind: Kind) -> f64 {
    // Glass F0 is 0.04. |0.04 − 0.0204| = 0.0196, which misses the band.
    broken(kind, 0.0196, read_water_f0)
}

fn snell_transmit(kind: Kind) -> f64 {
    broken(kind, 1.0, || region_abs(52, 48, 60, 56))
}

/// `((1.333 − 1) / (1.333 + 1))²` rounded to four decimals.
const WATER_F0: f64 = 0.0204;

/// Mean channel of the flat normal-incidence patch. Reflection is white and
/// transmission is black, so the pixel is the Fresnel weight.
fn read_water_f0() -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for y in 48..58 {
        for x in 2..14 {
            let pixel = presented_pixel(x, y);
            if !pixel[0].is_finite() {
                return f64::INFINITY;
            }
            sum += (pixel[0] + pixel[1] + pixel[2]) / 3.0;
            count += 1.0;
        }
    }
    if count == 0.0 {
        f64::INFINITY
    } else {
        (sum / count - WATER_F0).abs()
    }
}

fn ggx_energy_err(kind: Kind) -> f64 {
    broken(kind, 1.0, || {
        let got = presented_pixel(2, 2);
        (got[0] - 1.0).abs()
    })
}

fn mirror_view_frac(kind: Kind) -> f64 {
    broken(kind, 1.0, || {
        let albedo = [0.28, 0.30, 0.34];
        fraction(120, 60, 160, 120, |pixel| !near(pixel, albedo))
    })
}

fn portal_leak(kind: Kind) -> f64 {
    broken(kind, 1.0, || {
        let leak = [0.95, 0.25, 0.85];
        fraction(0, 60, 80, 120, |pixel| near(pixel, leak))
    })
}

fn portal_point(kind: Kind) -> f64 {
    broken(kind, 1.0, || region_abs(80, 60, 120, 120))
}

fn beer_rel(kind: Kind) -> f64 {
    broken(kind, 1.0, || region_rel(40, 20, 80, 44))
}

fn shoreline_ratio(kind: Kind) -> f64 {
    broken(kind, 1.0, || {
        let engine = genos_render::shoreline_ratio(PictureFault::default());
        let (oracle, _) = reflections_trace::shore_oracle();
        (engine - oracle).abs()
    })
}

fn shoreline_deep_ratio(kind: Kind) -> f64 {
    broken(kind, 1.0, || {
        let engine = genos_render::shoreline_deep_ratio(PictureFault::default());
        let (_, oracle) = reflections_trace::shore_oracle();
        (engine - oracle).abs()
    })
}

fn near_weighted_rel(kind: Kind) -> f64 {
    broken(kind, 1.0, weighted)
}

fn flicker_reversal(kind: Kind) -> f64 {
    broken(kind, 0.1, || match presented::shot() {
        Ok(shot) => shot.flicker,
        Err(_) => f64::INFINITY,
    })
}

fn low_reflection_rays(kind: Kind) -> f64 {
    broken(kind, 64.0, || match presented::shot() {
        Ok(shot) => f64::from(shot.rays[1]),
        Err(_) => f64::INFINITY,
    })
}

fn weighted() -> f64 {
    let Ok((shot, trace)) = frame() else {
        return f64::INFINITY;
    };
    let mut num = 0.0;
    let mut den = 0.0;
    for (index, pixel) in trace.color.iter().enumerate() {
        let width = ux(trace_width());
        let x = u32::try_from(index % width).unwrap_or(0);
        let y = u32::try_from(index / width).unwrap_or(0);
        let got = presented::sample(shot, x, y);
        let expect = quantize(*pixel);
        let rel = pixel_rel(got, expect);
        let depth = trace.depth.get(index).copied().unwrap_or(1.0);
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

fn region_abs(x0: u32, y0: u32, x1: u32, y1: u32) -> f64 {
    region(x0, y0, x1, y1, false)
}

fn region_rel(x0: u32, y0: u32, x1: u32, y1: u32) -> f64 {
    region(x0, y0, x1, y1, true)
}

fn region(x0: u32, y0: u32, x1: u32, y1: u32, relative: bool) -> f64 {
    let Ok((shot, trace)) = frame() else {
        return f64::INFINITY;
    };
    let width = trace_width();
    let mut max = 0.0_f64;
    for y in y0..y1 {
        for x in x0..x1 {
            let index = usize::try_from(y * width + x).unwrap_or(0);
            let Some(expect_lin) = trace.color.get(index).copied() else {
                return f64::INFINITY;
            };
            let got = presented::sample(shot, x, y);
            let expect = quantize(expect_lin);
            let err = if relative {
                pixel_rel(got, expect)
            } else {
                // An f32 shade on an 8-bit target can take the other code when
                // the f64 colour sits on a quantum boundary. That one step is
                // not an optical error. Anything farther is.
                presented_abs(got, expect_lin)
            };
            max = max.max(err);
        }
    }
    max
}

fn fraction(x0: u32, y0: u32, x1: u32, y1: u32, pred: impl Fn([f64; 3]) -> bool) -> f64 {
    let Ok(shot) = presented::shot() else {
        return f64::INFINITY;
    };
    let mut hits = 0u32;
    let mut count = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            count = count.saturating_add(1);
            if pred(presented::sample(shot, x, y)) {
                hits = hits.saturating_add(1);
            }
        }
    }
    if count == 0 {
        f64::INFINITY
    } else {
        f64::from(hits) / f64::from(count)
    }
}

fn near(pixel: [f64; 3], expect: [f64; 3]) -> bool {
    channel_abs(pixel, quantize(expect)) <= 1.0 / 255.0
}

fn presented_pixel(x: u32, y: u32) -> [f64; 3] {
    match presented::shot() {
        Ok(shot) => presented::sample(shot, x, y),
        Err(_) => [f64::INFINITY; 3],
    }
}

fn frame() -> Result<
    (
        &'static presented::Shot,
        &'static reflections_trace::Reference,
    ),
    (),
> {
    let shot = presented::shot().map_err(|_| ())?;
    let trace = traced();
    if shot.width != trace_width() || shot.height != trace_height() {
        return Err(());
    }
    if trace.color.len() != ux(trace_width()) * ux(trace_height()) {
        return Err(());
    }
    Ok((shot, trace))
}

fn traced() -> &'static reflections_trace::Reference {
    static TRACE: OnceLock<reflections_trace::Reference> = OnceLock::new();
    TRACE.get_or_init(reflections_trace::trace)
}

fn ux(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
}

fn trace_width() -> u32 {
    reflections_trace::WIDTH
}

fn trace_height() -> u32 {
    reflections_trace::HEIGHT
}

fn quantize(pixel: [f64; 3]) -> [f64; 3] {
    [
        quant_channel(pixel[0]),
        quant_channel(pixel[1]),
        quant_channel(pixel[2]),
    ]
}

fn quant_channel(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    (value.clamp(0.0, 1.0) * 255.0).round() / 255.0
}

fn presented_abs(got: [f64; 3], linear: [f64; 3]) -> f64 {
    let expect = quantize(linear);
    let mut max = 0.0_f64;
    for channel in 0..3 {
        let err = (got[channel] - expect[channel]).abs();
        if err == 0.0 || (err <= 1.0 / 255.0 + 1.0e-9 && on_quantum_boundary(linear[channel])) {
            continue;
        }
        max = max.max(err);
    }
    max
}

fn on_quantum_boundary(value: f64) -> bool {
    let scaled = value.clamp(0.0, 1.0) * 255.0;
    let frac = scaled - scaled.floor();
    (frac - 0.5).abs() < 0.02
}

fn channel_abs(got: [f64; 3], expect: [f64; 3]) -> f64 {
    let mut max = 0.0_f64;
    for channel in 0..3 {
        max = max.max((got[channel] - expect[channel]).abs());
    }
    max
}

fn pixel_rel(got: [f64; 3], expect: [f64; 3]) -> f64 {
    let mut sum = 0.0;
    for channel in 0..3 {
        let err = (got[channel] - expect[channel]).abs();
        let scale = expect[channel].abs().max(1.0e-3);
        sum += err / scale;
    }
    sum / 3.0
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
    fn a_portal_without_the_inverse_misses_the_baked_turn() {
        let good = metric(Fixture::Good, "portal_point");
        let bad = metric(Fixture::Bad, "portal_point");
        assert!(good <= 2.0 / 255.0, "{good}");
        assert!(bad > 2.0 / 255.0, "{bad}");
    }

    #[test]
    fn a_forced_shore_cap_fails_deep_water() {
        let (oracle_crest, oracle_deep) = crate::reflections_trace::shore_oracle();
        let open = genos_render::shoreline_deep_ratio(genos_render::PictureFault::default());
        let forced = genos_render::shoreline_deep_ratio(genos_render::PictureFault::only(
            genos_render::PictureFault::FORCE_CAP,
        ));
        assert!(
            (open - oracle_deep).abs() < 1.0e-4,
            "{open} vs {oracle_deep}"
        );
        assert!((forced - oracle_deep).abs() > 1.0e-4, "{forced}");
        let boundary = genos_render::shoreline_ratio(genos_render::PictureFault::default());
        assert!(
            (boundary - oracle_crest).abs() < 1.0e-4,
            "{boundary} vs {oracle_crest}"
        );
        let unclamped = genos_render::shoreline_ratio(genos_render::PictureFault::only(
            genos_render::PictureFault::SKIP_SHORE,
        ));
        assert!((unclamped - oracle_crest).abs() > 1.0, "{unclamped}");
    }

    #[test]
    fn a_glass_f0_fails_the_water_band() {
        let good = metric(Fixture::Good, "water_f0");
        let bad = metric(Fixture::Bad, "water_f0");
        assert!(good <= 2.0 / 255.0, "{good}");
        assert!(bad > 2.0 / 255.0, "{bad}");
    }

    #[test]
    fn the_presented_frame_matches_the_trace() {
        let err = metric(Fixture::Good, "near_weighted_rel");
        assert!(err <= 0.03, "{err}");
        let rays = metric(Fixture::Good, "low_reflection_rays");
        assert!(rays <= 0.0, "{rays}");
    }

    #[test]
    fn a_broken_candidate_fails_the_picture_rows() {
        assert!(metric(Fixture::Bad, "near_weighted_rel") > 0.03);
        assert!(metric(Fixture::Bad, "low_reflection_rays") > 0.0);
        assert!(metric(Fixture::Bad, "flicker_reversal") > 2.0 / 255.0);
    }

    #[test]
    fn good_passes_every_implemented_gate() {
        let report = grade(Fixture::Good);
        assert_eq!(report.metrics.len(), 21);
        assert_eq!(report.exit_code(), 0);
        let mut absent = 0;
        for metric in &report.metrics {
            if metric.status == Status::NotImplemented {
                assert!(metric.value.is_none(), "{}", metric.name);
                absent += 1;
                continue;
            }
            assert_eq!(metric.status, Status::Pass, "{}", metric.name);
            let value = metric.value.unwrap_or_else(|| panic!("{}", metric.name));
            assert!(value <= metric.threshold, "{} {value}", metric.name);
        }
        assert_eq!(absent, 5, "ssr, denoise, caustic, ms, vram");
        let text = report.summary();
        assert_eq!(text.lines().count(), 21);
        assert!(!text.contains("FAIL"));
        assert!(!text.contains("GPU frame readback"));
        assert!(text.contains("presented frame"));
        assert!(text.contains("GPU dispatch count"));
        assert!(text.contains("wave oracle"));
    }

    #[test]
    fn bad_fails_every_implemented_gate() {
        let report = grade(Fixture::Bad);
        assert_eq!(report.exit_code(), 1);
        for metric in &report.metrics {
            if metric.status == Status::NotImplemented {
                continue;
            }
            assert_eq!(metric.status, Status::Fail, "{}", metric.name);
            let value = metric.value.unwrap_or_else(|| panic!("{}", metric.name));
            assert!(value > metric.threshold, "{}", metric.name);
        }
    }

    #[test]
    fn live_grades_the_presented_frame_and_leaves_the_gaps() {
        let report = grade(Fixture::Live);
        assert_eq!(report.exit_code(), 0);
        let good = grade(Fixture::Good);
        for (live, proven) in report.metrics.iter().zip(good.metrics.iter()) {
            assert!(
                (live.threshold - proven.threshold).abs() <= f64::EPSILON,
                "{}",
                live.name
            );
            if proven.status == Status::NotImplemented {
                assert_eq!(live.status, Status::NotImplemented, "{}", live.name);
                assert!(live.value.is_none(), "{}", live.name);
                continue;
            }
            assert_eq!(live.status, Status::Pass, "{}", live.name);
            assert_eq!(live.value, proven.value, "{}", live.name);
        }
        let text = report.summary();
        assert!(text.contains("fresnel_below_abs"));
        assert!(text.contains("denoise_lag_frames value=na"));
        assert!(text.contains("caustic_rel value=na"));
        assert!(text.contains("ms_frac value=na"));
        assert!(text.contains("vram_frac value=na"));
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

    fn metric(fixture: Fixture, name: &str) -> f64 {
        grade(fixture)
            .metrics
            .into_iter()
            .find(|metric| metric.name == name)
            .unwrap_or_else(|| panic!("{name}"))
            .value
            .unwrap_or_else(|| panic!("{name}"))
    }
}
