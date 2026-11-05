//! The comparisons the ADR 0039 addendum gates on, as markdown.

use std::fmt::Write as _;

use crate::cache::{CacheConfig, Selection, TexelFeedback};
use crate::harness::{self, Bench, Metrics, Timing, HEADER};

/// Static scenes graded for baseline vs 2b.
pub const STATIC_SCENES: [&str; 4] = ["contact", "near-wall", "thin-wall", "shadow-edge"];

/// Knobs every comparison takes (CLI arguments in the binary).
#[derive(Clone, Copy, Debug)]
pub struct Knobs {
    pub cell_size: f64,
    pub rays_per_cell: usize,
    pub history_frames: f64,
    /// Frames after the lamp goes off at which linger is read.
    pub linger_after: usize,
}

impl Default for Knobs {
    fn default() -> Self {
        Self {
            cell_size: 0.25,
            rays_per_cell: 16,
            history_frames: 32.0,
            linger_after: 120,
        }
    }
}

impl Knobs {
    fn apply(self, mut c: CacheConfig) -> CacheConfig {
        c.cell_size = self.cell_size;
        c.rays_per_cell = self.rays_per_cell;
        c.history_frames = self.history_frames;
        c
    }

    /// Stage 2 as on master: plain SH-L1, fp16, no side bit.
    #[must_use]
    pub fn plain(self) -> CacheConfig {
        self.apply(CacheConfig::plain(self.cell_size, self.rays_per_cell))
    }

    /// 2b as first written: 1-bit side key, centroid, clamped gradient,
    /// hit distance, compact encoding.
    #[must_use]
    pub fn stage_2b_literal(self) -> CacheConfig {
        self.apply(CacheConfig::stage_2b(self.cell_size, self.rays_per_cell))
    }

    /// Recommended 2b: 6-class axis key, Wiener-shrunk gradient, irradiance
    /// normalisation, compact encoding.
    #[must_use]
    pub fn stage_2b(self) -> CacheConfig {
        CacheConfig {
            axis_key: true,
            wiener: true,
            irr_norm: true,
            ..self.stage_2b_literal()
        }
    }
}

/// Labelled configs `compare` grades, in table order. Index 0 is the
/// baseline; [`RECOMMENDED`] is the 2b the gates are read on.
#[must_use]
pub fn compare_configs(k: Knobs) -> Vec<(&'static str, CacheConfig)> {
    let p = k.plain();
    let lit = k.stage_2b_literal();
    let rec = k.stage_2b();
    vec![
        ("plain (stage 2, f16)", p),
        (
            "side bit only",
            CacheConfig {
                side_bit: true,
                ..p
            },
        ),
        ("2b as written (1-bit side, compact)", lit),
        (
            "axis key only",
            CacheConfig {
                side_bit: true,
                axis_key: true,
                ..p
            },
        ),
        (
            "plain + irradiance norm",
            CacheConfig {
                irr_norm: true,
                ..p
            },
        ),
        ("2b recommended (axis, wiener, norm, compact)", rec),
        (
            "2b recommended, exact encoding",
            CacheConfig {
                encoding: crate::sh::Encoding::Exact,
                ..rec
            },
        ),
        (
            "2b recommended + read visibility",
            CacheConfig { vis: true, ..rec },
        ),
    ]
}

/// Index of the recommended 2b in [`compare_configs`].
pub const RECOMMENDED: usize = 5;

/// One F2 run: hard-spot bias (mean, seed sd) and energy.
#[derive(Clone, Copy, Debug, Default)]
pub struct F2Run {
    pub skew: f64,
    pub selection_label: &'static str,
    pub energy: f64,
    pub hard: (f64, f64),
}

/// Everything `compare` measured.
#[derive(Clone, Debug, Default)]
pub struct Compare {
    pub labels: Vec<&'static str>,
    /// Per scene, metrics per config in [`compare_configs`] order.
    pub scenes: Vec<(&'static str, Vec<Metrics>)>,
    /// Lamp-off timing per config.
    pub timing: Vec<Timing>,
    /// F3 on `lamp-off`: plain timing with texel feedback.
    pub f3: Timing,
    /// F2 on `contact`: uniform, then unweighted and weighted per skew.
    pub f2: Vec<F2Run>,
    /// Hard-spot pixel count for F2.
    pub f2_pixels: usize,
}

/// Run baseline vs 2b, the lamp-off timeline and both must-fail variants.
#[must_use]
pub fn compare(b: &Bench, k: Knobs, skews: &[f64]) -> (String, Compare) {
    let mut out = String::new();
    let mut res = Compare::default();
    let list = compare_configs(k);
    res.labels = list.iter().map(|(l, _)| *l).collect();
    let _ = writeln!(
        out,
        "## Baseline vs 2b\n\ncell {:.3} m cubic, {} rays/cell/frame, history {} frames, pixels {:.3} m, reference {} spp, seeds {:?}, warm {} frames, {} measured frames. Bytes per cell are **estimates** (payload + 12 B key/count/age).\n",
        k.cell_size, k.rays_per_cell, k.history_frames, b.pixel_size, b.ref_spp, b.seeds, b.warm_frames, b.samples
    );
    for name in STATIC_SCENES {
        let Some(s) = b.prepare(name) else { continue };
        let _ = writeln!(out, "### {name}\n\n{HEADER}");
        let mut ms = Vec::new();
        for (label, cfg) in &list {
            let m = harness::run_static(b, &s, *cfg);
            harness::row(
                &mut out,
                &format!("{label} ({} B est)", cfg.engine_bytes_per_cell()),
                &m,
            );
            ms.push(m);
        }
        out.push('\n');
        res.scenes.push((name, ms));
    }
    if let Some(s) = b.prepare("lamp-off") {
        let _ = writeln!(
            out,
            "### lamp-off\n\n| config | lag (energy at frame 300) | linger at +{} |\n|---|---|---|",
            k.linger_after
        );
        for (label, cfg) in &list {
            let t = harness::run_lamp_off(b, &s, *cfg, k.linger_after);
            let _ = writeln!(out, "| {label} | {:.3} | {:.4} |", t.lag, t.linger);
            res.timing.push(t);
        }
        let f3 = CacheConfig {
            texel: Some(TexelFeedback {
                alpha: 0.99,
                beta: 0.5,
            }),
            ..k.plain()
        };
        res.f3 = harness::run_lamp_off(b, &s, f3, k.linger_after);
        let _ = writeln!(
            out,
            "| F3 texel feedback (must fail) | {:.3} | {:.4} |\n",
            res.f3.lag, res.f3.linger
        );
    }
    if let Some(s) = b.prepare("contact") {
        let hard: Vec<usize> = crate::cache::hardness(&s.scene)
            .iter()
            .enumerate()
            .filter(|(i, h)| s.scene.pixels[*i].valid && **h >= 0.5)
            .map(|(i, _)| i)
            .collect();
        res.f2_pixels = hard.len();
        let mut runs = vec![(0.0, "uniform", Selection::Uniform)];
        for &sk in skews {
            runs.push((sk, "skew unweighted (must fail)", Selection::SkewUnweighted));
            runs.push((
                sk,
                "skew with 1/q weights (control)",
                Selection::SkewWeighted,
            ));
        }
        let _ = writeln!(
            out,
            "### F2 adaptive origins on contact (plain cells)\n\nBias on hard spots (hardness >= 0.5, {} pixels) vs the reference, mean ± seed sd; delta is against uniform selection.\n\n| selection | skew | energy | hard-spot bias | delta vs uniform |\n|---|---|---|---|---|",
            hard.len()
        );
        for (skew, label, selection) in runs {
            let cfg = CacheConfig {
                skew,
                selection,
                ..k.plain()
            };
            let m = harness::run_static(b, &s, cfg);
            let hb = harness::region_bias_seeds(b, &s, cfg, &hard);
            let base = res.f2.first().map_or(hb.0, |u| u.hard.0);
            let _ = writeln!(
                out,
                "| {label} | {skew} | {:.3} | {:+.2}% ± {:.2} | {:+.2}% |",
                m.energy,
                hb.0 * 100.0,
                hb.1 * 100.0,
                (hb.0 - base) * 100.0
            );
            res.f2.push(F2Run {
                skew,
                selection_label: label,
                energy: m.energy,
                hard: hb,
            });
        }
        out.push('\n');
    }
    (out, res)
}

impl Knobs {
    /// A config from a `+`-joined spec over plain: `side` (1-bit side key),
    /// `axis` (6-class key, implies `side`), `pos` (centroid, gradient, hit
    /// distance), `wiener`, `vis` (read visibility weight), `norm` (irradiance normalisation), `bcc`, and an
    /// encoding `exact`, `f16` or `compact`. `plain` alone is stage 2.
    #[must_use]
    pub fn spec(self, spec: &str) -> Option<CacheConfig> {
        let mut c = self.plain();
        for t in spec.split('+') {
            match t.trim() {
                "plain" => {}
                "side" => c.side_bit = true,
                "axis" => {
                    c.side_bit = true;
                    c.axis_key = true;
                }
                "pos" => c.position = true,
                "wiener" => c.wiener = true,
                "norm" => c.irr_norm = true,
                "vis" => c.vis = true,
                "bcc" => c.layout = crate::lattice::Layout::Bcc,
                other => c.encoding = crate::sh::Encoding::parse(other)?,
            }
        }
        Some(c)
    }
}

/// Default spec list for `variants`.
pub const DEFAULT_SPECS: &str =
    "plain;side;side+pos+compact;axis;axis+pos+compact;axis+pos+wiener+compact;plain+norm;axis+norm;axis+pos+wiener+norm+compact";

/// Every spec on every static scene, plus lamp-off timing.
#[must_use]
pub fn variants(b: &Bench, k: Knobs, specs: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "## Variants\n\ncell {:.3} m, {} rays/cell/frame, history {} frames, reference {} spp, seeds {:?}, warm {}, {} measured frames. Bytes are **est** engine RGB bytes per cell incl. 12 B key/count/age.\n",
        k.cell_size, k.rays_per_cell, k.history_frames, b.ref_spp, b.seeds, b.warm_frames, b.samples
    );
    let list: Vec<(&str, CacheConfig)> = specs
        .split(';')
        .filter_map(|s| k.spec(s).map(|c| (s.trim(), c)))
        .collect();
    for name in STATIC_SCENES {
        let Some(s) = b.prepare(name) else { continue };
        let _ = writeln!(out, "### {name}\n\n{HEADER}");
        for (label, cfg) in &list {
            let m = harness::run_static(b, &s, *cfg);
            harness::row(
                &mut out,
                &format!("{label} ({} B)", cfg.engine_bytes_per_cell()),
                &m,
            );
        }
        out.push('\n');
    }
    if let Some(s) = b.prepare("lamp-off") {
        let _ = writeln!(
            out,
            "### lamp-off\n\n| config | lag | linger at +{} |\n|---|---|---|",
            k.linger_after
        );
        for (label, cfg) in &list {
            let t = harness::run_lamp_off(b, &s, *cfg, k.linger_after);
            let _ = writeln!(out, "| {label} | {:.3} | {:.4} |", t.lag, t.linger);
        }
        out.push('\n');
    }
    out
}
