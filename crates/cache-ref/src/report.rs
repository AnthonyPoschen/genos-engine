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

/// Scenes the layout and lighting-memory sweep grades (smooth and thin-wall).
pub const SWEEP_SCENES: [&str; 2] = ["contact", "thin-wall"];

/// World size the lighting-memory figures are scaled to: cells a whole world
/// needs at the 25 cm cubic reference, as a parameter. The default is ADR
/// 0036's measured worst live slot count on High (407,116), taken as if every
/// slot were a 25 cm cell. Real slots mix 25 cm cells and 12.5 cm texels, so
/// this overstates the memory a given cell size needs (conservative).
pub const WORLD_CELLS_AT_25CM: f64 = 407_116.0;

/// One layout × cell-size point of the sweep.
#[derive(Clone, Copy, Debug)]
pub struct SweepPoint {
    pub layout: crate::lattice::Layout,
    pub cell_size: f64,
    /// Estimated world pool in MiB at this density.
    pub pool_mb: f64,
    /// Per [`SWEEP_SCENES`] scene.
    pub metrics: [Metrics; 2],
}

/// Lighting memory in MiB (est) that a cell count scaled to the world needs.
#[must_use]
pub fn pool_mb(room_cells: f64, room_cells_ref: f64, world_cells: f64, bytes: u32) -> f64 {
    world_cells * room_cells / room_cells_ref.max(1.0) * f64::from(bytes) / 1_048_576.0
}

/// Cubic-equivalent cell size whose world pool fits `mb` (est), from the
/// 1/h² surface scaling.
#[must_use]
pub fn cell_for_pool(mb: f64, world_cells: f64, bytes: u32, ref_cell: f64) -> f64 {
    let cells = mb * 1_048_576.0 / f64::from(bytes);
    ref_cell * (world_cells / cells).sqrt()
}

/// Cubic vs BCC over cell sizes with the recommended 2b, and the lighting
/// memory each point needs. Equal memory: rows at the same pool. Equal
/// accuracy: the pool each layout needs to reach a blotch target, interpolated
/// in log memory between sweep points.
#[must_use]
pub fn sweep(
    b: &Bench,
    k: Knobs,
    cells: &[f64],
    world_cells: f64,
    pools: &[f64],
) -> (String, Vec<SweepPoint>) {
    use crate::lattice::{Lattice, Layout};
    let mut out = String::new();
    let rec = k.stage_2b();
    let bytes = rec.engine_bytes_per_cell();
    let _ = writeln!(
        out,
        "## Layout and lighting memory sweep\n\n2b recommended, {} rays/cell/frame (rays scale with cells), history {}, pixels {:.3} m, reference {} spp, seeds {:?}. Pool MiB are **est**: {bytes} B/cell × world cells, world cells = {world_cells:.0} × (room cells / room cells at 25 cm cubic). Cell size is the cubic-equivalent density (BCC cube edge = h·2^(1/3)).\n",
        k.rays_per_cell, k.history_frames, b.pixel_size, b.ref_spp, b.seeds
    );
    let _ = writeln!(
        out,
        "Cubic-equivalent cell size per pool at {bytes} B/cell: "
    );
    for &mb in pools {
        let _ = write!(
            out,
            "{mb:.0} MiB → {:.3} m; ",
            cell_for_pool(mb, world_cells, bytes, 0.25)
        );
    }
    out.push_str("\n\n");
    let prepared: Vec<_> = SWEEP_SCENES.iter().filter_map(|n| b.prepare(n)).collect();
    let mut pts = Vec::new();
    let mut room_ref = [0.0f64; 2];
    for layout in [Layout::Cubic, Layout::Bcc] {
        for &h in cells {
            let cfg = CacheConfig {
                layout,
                cell_size: h,
                ..rec
            };
            let mut ms = [Metrics::default(), Metrics::default()];
            for (i, s) in prepared.iter().enumerate().take(2) {
                ms[i] = harness::run_static(b, s, cfg);
            }
            if layout == Layout::Cubic && (h - 0.25).abs() < 1e-9 {
                room_ref = [ms[0].cells, ms[1].cells];
            }
            pts.push(SweepPoint {
                layout,
                cell_size: h,
                pool_mb: 0.0,
                metrics: ms,
            });
        }
    }
    if room_ref[0] <= 0.0 {
        let c0 = pts.first().map_or(1.0, |p| p.metrics[0].cells);
        let h0 = pts.first().map_or(0.25, |p| p.cell_size);
        room_ref[0] = c0 * (h0 / 0.25) * (h0 / 0.25);
    }
    for p in &mut pts {
        p.pool_mb = pool_mb(p.metrics[0].cells, room_ref[0], world_cells, bytes);
    }
    let _ = writeln!(
        out,
        "| layout | cell (m) | pool MiB est | lookup fetches / ALU | contact energy | contact blotch | contact rel err | thin-wall energy | thin-wall blotch | thin-wall leak | room cells (contact) | rays/frame (contact) |\n|---|---|---|---|---|---|---|---|---|---|---|---|"
    );
    for p in &pts {
        let lc = Lattice::with_density_of_cubic(p.layout, p.cell_size).lookup_cost();
        let [c, t] = p.metrics;
        let _ = writeln!(
            out,
            "| {} | {:.4} | {:.1} | {} / {} | {:.3} | {:.4} | {:.3} | {:.3} | {:.4} | {:+.3} | {:.0} | {:.0} |",
            p.layout.name(),
            p.cell_size,
            p.pool_mb,
            lc.fetches,
            lc.alu,
            c.energy,
            c.blotch,
            c.mean_rel_error,
            t.energy,
            t.blotch,
            t.leak,
            c.cells,
            c.rays_per_frame
        );
    }
    out.push('\n');
    equal_accuracy(&mut out, &pts);
    (out, pts)
}

/// Equal-accuracy lines: pool each layout needs for cubic's best blotch + 10 %.
fn equal_accuracy(out: &mut String, pts: &[SweepPoint]) {
    use crate::lattice::Layout;
    let target = pts
        .iter()
        .filter(|p| p.layout == Layout::Cubic)
        .map(|p| p.metrics[0].blotch)
        .fold(f64::INFINITY, f64::min);
    let target = target * 1.1;
    let _ = writeln!(
        out,
        "Equal accuracy: pool each layout needs for contact blotch ≤ {target:.4} (110 % of cubic's best in this sweep), log-interpolated:\n"
    );
    for layout in [Layout::Cubic, Layout::Bcc] {
        let mut xs: Vec<(f64, f64)> = pts
            .iter()
            .filter(|p| p.layout == layout)
            .map(|p| (p.pool_mb, p.metrics[0].blotch))
            .collect();
        xs.sort_by(|a, c| a.0.total_cmp(&c.0));
        let mb = mb_to_reach(&xs, target);
        let _ = writeln!(
            out,
            "- {}: {}",
            layout.name(),
            mb.map_or_else(
                || "not reached in sweep".to_owned(),
                |v| format!("{v:.1} MiB est")
            )
        );
    }
    out.push('\n');
}

/// First memory at which blotch falls to `target`, interpolating in log MiB.
fn mb_to_reach(xs: &[(f64, f64)], target: f64) -> Option<f64> {
    let first = xs.first()?;
    if first.1 <= target {
        return Some(first.0);
    }
    xs.windows(2).find_map(|w| {
        let (a, c) = (w[0], w[1]);
        if c.1 > target || a.1 <= c.1 {
            return None;
        }
        let t = (a.1 - target) / (a.1 - c.1);
        Some((a.0.ln() + t * (c.0.ln() - a.0.ln())).exp())
    })
}

/// Stage 2c on the dolly: static 25 cm and 12.5 cm 2b against subdivision
/// with each seed rule, plus a skewed-origin pass and lamp-off timing.
#[must_use]
pub fn subdiv(b: &Bench, k: Knobs, frames: usize) -> String {
    use crate::cache::{Seed, Subdiv};
    let mut out = String::new();
    let rec = k.stage_2b();
    let fine = CacheConfig {
        cell_size: k.cell_size / 2.0,
        ..rec
    };
    let with = |seed: Seed| CacheConfig {
        subdiv: Some(Subdiv {
            seed,
            ..Subdiv::default()
        }),
        ..rec
    };
    let _ = writeln!(
        out,
        "## Stage 2c subdivision on the dolly\n\nParent cell {:.3} m, children {:.3} m, {} rays/cell/frame, 160 warm frames at the start pose, then {frames} frames (dolly 300, then held); pop max is over those {frames} frames, seeds {:?}. Subdiv defaults: {:?}.\n",
        k.cell_size,
        k.cell_size / 2.0,
        k.rays_per_cell,
        b.seeds,
        Subdiv::default()
    );
    let Some(s) = b.prepare("dolly") else {
        return out;
    };
    let runs = [
        ("static 2b, parent size", rec),
        ("static 2b, child size", fine),
        ("2c, zero-seeded children", with(Seed::Zero)),
        ("2c, parent-copy seed", with(Seed::Copy)),
        ("2c, 2b-model seed", with(Seed::Model)),
        (
            "2c, 2b-model seed worth 32 samples",
            CacheConfig {
                subdiv: Some(Subdiv {
                    seed_n: 32.0,
                    ..Subdiv::default()
                }),
                ..rec
            },
        ),
    ];
    let mut mask = Vec::new();
    let mut rows = Vec::new();
    for (label, cfg) in runs {
        let (r, m) = harness::run_dolly(b, &s, cfg, frames);
        if cfg.subdiv.is_some() && mask.is_empty() {
            mask = m;
        }
        rows.push((label, cfg, r));
    }
    let _ = writeln!(
        out,
        "| config | B/cell est | energy | blotch (split pixels) | pop max | pop steady | splits | merges | toggles after hold | catch-up rays | peak cells | rays |\n|---|---|---|---|---|---|---|---|---|---|---|---|"
    );
    for (label, cfg, r) in &rows {
        let blotch = if cfg.subdiv.is_some() {
            r.blotch_split
        } else {
            harness::static_blotch_in(b, &s, *cfg, &mask)
        };
        let _ = writeln!(
            out,
            "| {label} | {} | {:.3} | {:.4} | {:.4} | {:.4} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |",
            cfg.engine_bytes_per_cell(),
            r.energy,
            blotch,
            r.pop_max,
            r.pop_steady,
            r.splits,
            r.merges,
            r.toggles_after_hold,
            r.catch_up_rays,
            r.peak_cells,
            r.rays
        );
    }
    out.push('\n');
    subdiv_extras(&mut out, b, k, &s, frames);
    out
}

/// Skewed-origin and lamp-off passes for [`subdiv`].
fn subdiv_extras(out: &mut String, b: &Bench, k: Knobs, s: &harness::Prepared, frames: usize) {
    use crate::cache::{Seed, Subdiv};
    let rec = k.stage_2b();
    let with = |seed: Seed| CacheConfig {
        subdiv: Some(Subdiv {
            seed,
            ..Subdiv::default()
        }),
        ..rec
    };
    let _ = writeln!(
        out,
        "### Skewed origins (selection skew {}, 1/q weighted)\n\n| config | energy | blotch (split pixels) |\n|---|---|---|",
        rec.skew
    );
    for (label, seed) in [
        ("2c, parent-copy seed", Seed::Copy),
        ("2c, 2b-model seed", Seed::Model),
    ] {
        let cfg = CacheConfig {
            selection: Selection::SkewWeighted,
            ..with(seed)
        };
        let (r, _) = harness::run_dolly(b, s, cfg, frames);
        let _ = writeln!(out, "| {label} | {:.3} | {:.4} |", r.energy, r.blotch_split);
    }
    out.push('\n');
    if let Some(lo) = b.prepare("lamp-off") {
        let _ = writeln!(
            out,
            "### lamp-off with subdivision\n\n| config | lag | linger at +{} |\n|---|---|---|",
            k.linger_after
        );
        for (label, cfg) in [("static 2b", rec), ("2c, 2b-model seed", with(Seed::Model))] {
            let t = harness::run_lamp_off(b, &lo, cfg, k.linger_after);
            let _ = writeln!(out, "| {label} | {:.3} | {:.4} |", t.lag, t.linger);
        }
        out.push('\n');
    }
}
