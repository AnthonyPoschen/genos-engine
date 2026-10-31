//! The live ReSTIR DI estimator (`restir_live.comp` + `lighting_direct.comp`,
//! flag on) is unbiased for still pixels with shadows off: the mean over many
//! frames of `lamp_light(chosen) * W` equals the exact unshadowed sum of
//! `lamp_light` over every point lamp.
//!
//! This is a line-for-line CPU mirror of the shader path: the tree proposal
//! (`LightTree`, the mirror of `restir_tree.glsl`, with the 16-bit
//! `next_f32`), `initial_at` (8 candidates, misses counted in M), `restir_phat`
//! (luma of `lamp_light` at `pos + n * 0.02`), the temporal `cap_history` +
//! `merge_pair` (M clamp 20, M cap 160) and the shade `lamp_light * W` with
//! `W = w_sum / (M * target)`. Spatial reuse is not mirrored here.
//!
//! Tolerance: 2 % of the exact sum per pixel, over 20000 frames x 8 seeds.

#![allow(
    clippy::pedantic,
    clippy::restriction,
    reason = "a test-only CPU mirror of shader arithmetic"
)]

use genos_math::Vec3;
use genos_restir_ref::rng::Rng;
use genos_restir_ref::shade::{Lamp, Receiver};
use genos_restir_ref::{LightTree, TreeFault, DEFAULT_M_CLAMP};

const LAMP_UNIT: f32 = 72.0;
const LAMP_RADIUS: f32 = 0.1;
const LIFT: f32 = 0.02;
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
const CANDIDATES: u32 = 8;
const M_CAP: u32 = 160;
const TOL: f32 = 0.02;

#[derive(Clone, Copy)]
struct LiveLamp {
    pos: Vec3,
    range: f32,
    color: Vec3,
}

#[derive(Clone, Copy)]
struct Pixel {
    pos: Vec3,
    n: Vec3,
    two_sided: bool,
}

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// `lamp_light` in scene_rays.glsl, point lamps only.
fn lamp_light(lamp: &LiveLamp, origin: Vec3, n: Vec3, two_sided: bool) -> Vec3 {
    let d = lamp.pos - origin;
    if lamp.range > 0.0 && d.dot(d) > lamp.range * lamp.range {
        return Vec3::ZERO;
    }
    let dist = d.length().max(1.0e-4);
    let toward = d * (1.0 / dist);
    let mut nd = toward.dot(n);
    if two_sided {
        nd = nd.abs();
    }
    if nd <= 0.0 {
        return Vec3::ZERO;
    }
    lamp.color * (nd * LAMP_UNIT / (dist * dist).max(LAMP_RADIUS * LAMP_RADIUS))
}

fn luma(c: Vec3) -> f32 {
    c.x * LUMA[0] + c.y * LUMA[1] + c.z * LUMA[2]
}

fn origin(p: &Pixel) -> Vec3 {
    p.pos + p.n * LIFT
}

/// `restir_phat`.
fn phat(p: &Pixel, lamp: &LiveLamp) -> f32 {
    luma(lamp_light(lamp, origin(p), p.n, p.two_sided))
}

#[derive(Clone, Copy, Default)]
struct Res {
    lamp: Option<usize>,
    w_sum: f32,
    m: u32,
    target: f32,
}

fn w_of(r: Res) -> f32 {
    if r.lamp.is_none() || r.m == 0 || r.target <= 0.0 {
        0.0
    } else {
        r.w_sum / (r.m as f32 * r.target)
    }
}

/// `initial_at` with the tree proposal.
fn initial(p: &Pixel, lamps: &[LiveLamp], tree: &LightTree, rng: &mut Rng) -> Res {
    let recv = Receiver { position: p.pos, normal: p.n, albedo: v(1.0, 1.0, 1.0) };
    let mut r = Res::default();
    for _ in 0..CANDIDATES {
        let draw = tree.sample_sided(&recv, TreeFault::None, rng, p.two_sided);
        let unit = rng.next_f32();
        r.m += 1;
        if let Some((lamp, pdf)) = draw {
            let stream = if pdf > 0.0 { phat(p, &lamps[lamp]) / pdf } else { 0.0 };
            if stream > 0.0 {
                r.w_sum += stream;
                if unit * r.w_sum < stream {
                    r.lamp = Some(lamp);
                }
            }
        }
    }
    r.target = r.lamp.map_or(0.0, |l| phat(p, &lamps[l]));
    if r.lamp.is_none() || r.target <= 0.0 || r.w_sum <= 0.0 {
        return Res::default();
    }
    r
}

/// `cap_history` + `merge_pair` (restir_merge.glsl) for a still pixel.
fn temporal(p: &Pixel, lamps: &[LiveLamp], c: Res, h: Res, unit: f32) -> Res {
    if h.lamp.is_none() || h.m == 0 {
        return c;
    }
    let mut h = h;
    // An empty initial reservoir still drew CANDIDATES samples (stored as M 0).
    let m_now = if c.lamp.is_none() { CANDIDATES } else { c.m };
    let cap = DEFAULT_M_CLAMP * m_now;
    if m_now != 0 && h.m > cap {
        h.w_sum *= cap as f32 / h.m as f32;
        h.m = cap;
    }
    if h.m > M_CAP {
        h.w_sum *= M_CAP as f32 / h.m as f32;
        h.m = M_CAP;
    }
    if c.lamp.is_none() {
        // The empty current reservoir's M joins the count; W shrinks to match.
        let tq_h = phat(p, &lamps[h.lamp.unwrap_or(0)]);
        if tq_h <= 0.0 {
            return Res::default();
        }
        let m = h.m + m_now;
        return Res { lamp: h.lamp, w_sum: h.m as f32 * w_of(h) * tq_h, m, target: tq_h };
    }
    let tq_c = phat(p, &lamps[c.lamp.unwrap_or(0)]);
    let tq_h = phat(p, &lamps[h.lamp.unwrap_or(0)]);
    let mass_c = if tq_c > 0.0 { c.m as f32 * w_of(c) * tq_c } else { 0.0 };
    let mass_h = if tq_h > 0.0 { h.m as f32 * w_of(h) * tq_h } else { 0.0 };
    let area = mass_c + mass_h;
    if area <= 0.0 {
        return Res::default();
    }
    let pick_h = mass_h > 0.0 && (unit < mass_h / area || mass_c <= 0.0);
    let (sel, tq) = if pick_h { (h.lamp, tq_h) } else { (c.lamp, tq_c) };
    if tq <= 0.0 {
        return Res::default();
    }
    let count = c.m + h.m;
    let denom = count as f32 * tq;
    let w_new = area * tq / (tq * denom);
    Res { lamp: sel, w_sum: w_new * count as f32 * tq, m: count, target: tq }
}

fn scene() -> Vec<LiveLamp> {
    // A lamps-style room: a grid of dim lamps and a few bright ones low over the floor.
    let mut lamps = Vec::new();
    for i in 0..6 {
        for j in 0..6 {
            let x = -5.0 + 2.0 * i as f32;
            let z = -5.0 + 2.0 * j as f32;
            let hue = ((i * 7 + j * 3) % 5) as f32 / 5.0;
            lamps.push(LiveLamp { pos: v(x, 2.5, z), range: 9.0, color: v(0.3 + hue, 0.4, 1.0 - 0.5 * hue) * 0.4 });
        }
    }
    for k in 0..400 {
        let a = k as f32 * 0.157;
        let r = 12.0 + (k % 7) as f32;
        lamps.push(LiveLamp { pos: v(r * a.cos(), 0.4 + (k % 5) as f32, r * a.sin()), range: 0.0, color: v(0.05, 0.05, 0.06) });
    }
    lamps.push(LiveLamp { pos: v(0.3, 0.6, 0.2), range: 6.0, color: v(4.0, 3.0, 2.0) });
    lamps.push(LiveLamp { pos: v(-2.2, 0.35, 1.7), range: 5.0, color: v(1.0, 2.5, 4.0) });
    lamps.push(LiveLamp { pos: v(3.1, 1.0, -2.6), range: 4.0, color: v(3.0, 0.5, 0.5) });
    lamps
}

fn pixels() -> Vec<(&'static str, Pixel)> {
    let up = v(0.0, 1.0, 0.0);
    vec![
        ("under bright lamp", Pixel { pos: v(0.3, 0.0, 0.2), n: up, two_sided: false }),
        ("near bright lamp", Pixel { pos: v(-2.0, 0.0, 1.5), n: up, two_sided: false }),
        ("beside red lamp", Pixel { pos: v(3.5, 0.0, -2.2), n: up, two_sided: false }),
        ("open floor", Pixel { pos: v(1.7, 0.0, 3.9), n: up, two_sided: false }),
        ("dark corner", Pixel { pos: v(6.5, 0.0, 6.5), n: up, two_sided: false }),
        ("wall facing bright lamp", Pixel { pos: v(1.3, 0.5, 0.2), n: v(-1.0, 0.0, 0.0), two_sided: false }),
        // Most candidates land on lamps behind this face (target 0), so many
        // frames' initial reservoirs are empty.
        ("outward wall past ring", Pixel { pos: v(17.0, 1.0, 0.0), n: v(1.0, 0.0, 0.0), two_sided: false }),
        ("ceiling over dark corner", Pixel { pos: v(6.5, 6.0, 6.5), n: v(0.0, 1.0, 0.0), two_sided: false }),
    ]
}

/// A two-sided face above the bright lamp: `lamp_light` lights it from below
/// (|n.l|), but the tree's cosine bound is one-sided, so the lamp under it is
/// never proposed: support loss (darkening).
fn two_sided_pixels() -> Vec<(&'static str, Pixel)> {
    vec![("two-sided leaf", Pixel { pos: v(0.3, 1.2, 0.2), n: v(0.0, 1.0, 0.0), two_sided: true })]
}

fn exact(p: &Pixel, lamps: &[LiveLamp]) -> Vec3 {
    lamps.iter().fold(Vec3::ZERO, |acc, l| acc + lamp_light(l, origin(p), p.n, p.two_sided))
}

/// A sphere occluder for the shadows-on case.
#[derive(Clone, Copy)]
struct Ball {
    center: Vec3,
    radius: f32,
}

/// `trace_occluded(origin, lamp)`: the segment from the lifted origin to the lamp.
fn occluded(balls: &[Ball], from: Vec3, to: Vec3) -> bool {
    let d = to - from;
    let len2 = d.dot(d);
    balls.iter().any(|b| {
        let t = ((b.center - from).dot(d) / len2).clamp(0.0, 1.0);
        let q = from + d * t - b.center;
        q.dot(q) < b.radius * b.radius
    })
}

/// Shadows on: the target stays unshadowed (`restir_phat`), the shade step
/// multiplies by visibility from the same lifted origin. The mean must equal
/// the exact shadowed sum.
fn mean_estimate_shadowed(p: &Pixel, lamps: &[LiveLamp], tree: &LightTree, balls: &[Ball], seed: u64, frames: u32) -> Vec3 {
    let mut rng = Rng::new(seed);
    let mut hist = Res::default();
    let mut sum = Vec3::ZERO;
    for _ in 0..frames {
        let c = initial(p, lamps, tree, &mut rng);
        let unit = rng.next_f32();
        let r = temporal(p, lamps, c, hist, unit);
        hist = r;
        if let Some(l) = r.lamp {
            if !occluded(balls, origin(p), lamps[l].pos) {
                sum += lamp_light(&lamps[l], origin(p), p.n, p.two_sided) * w_of(r);
            }
        }
    }
    sum * (1.0 / frames as f32)
}

fn mean_estimate(p: &Pixel, lamps: &[LiveLamp], tree: &LightTree, seed: u64, frames: u32) -> Vec3 {
    let mut rng = Rng::new(seed);
    let mut hist = Res::default();
    let mut sum = Vec3::ZERO;
    for _ in 0..frames {
        let c = initial(p, lamps, tree, &mut rng);
        let unit = rng.next_f32();
        let r = temporal(p, lamps, c, hist, unit);
        hist = r;
        if let Some(l) = r.lamp {
            sum += lamp_light(&lamps[l], origin(p), p.n, p.two_sided) * w_of(r);
        }
    }
    sum * (1.0 / frames as f32)
}

fn check(set: &[(&'static str, Pixel)]) {
    let lamps = scene();
    let refs: Vec<Lamp> = lamps.iter().map(|l| Lamp { position: l.pos, intensity: l.color }).collect();
    let tree = LightTree::build(&refs);
    let mut failures = Vec::new();
    for (name, p) in set {
        let want = exact(p, &lamps);
        let mut got = Vec3::ZERO;
        let seeds = [11u64, 23, 37, 41, 53, 67, 71, 89];
        for seed in seeds {
            got += mean_estimate(p, &lamps, &tree, seed, 20000);
        }
        got = got * (1.0 / seeds.len() as f32);
        let rel = (luma(got) - luma(want)) / luma(want).max(1.0e-6);
        eprintln!("{name:>24}: exact {:.4} mean {:.4} rel {:+.4}", luma(want), luma(got), rel);
        if rel.abs() > TOL {
            failures.push(format!("{name}: rel {rel:+.4}"));
        }
    }
    assert!(failures.is_empty(), "biased pixels: {failures:?}");
}

#[test]
fn live_estimator_mean_matches_the_exact_unshadowed_lamp_sum() {
    check(&pixels());
}

#[test]
fn live_estimator_two_sided_matches_the_exact_sum() {
    check(&two_sided_pixels());
}

#[test]
fn live_estimator_shadowed_mean_matches_the_exact_shadowed_sum() {
    let lamps = scene();
    let refs: Vec<Lamp> = lamps.iter().map(|l| Lamp { position: l.pos, intensity: l.color }).collect();
    let tree = LightTree::build(&refs);
    // One ball hides the bright warm lamp from the floor beside it; one sits
    // just above the floor so the 0.02 lift decides grazing visibility.
    let balls = [
        Ball { center: v(-0.6, 0.35, 0.8), radius: 0.3 },
        Ball { center: v(1.0, 0.03, 3.9), radius: 0.025 },
    ];
    let mut failures = Vec::new();
    for (name, p) in pixels() {
        let want = lamps.iter().fold(Vec3::ZERO, |acc, l| {
            if occluded(&balls, origin(&p), l.pos) {
                acc
            } else {
                acc + lamp_light(l, origin(&p), p.n, p.two_sided)
            }
        });
        let mut got = Vec3::ZERO;
        let seeds = [11u64, 23, 37, 41, 53, 67, 71, 89, 97, 101, 103, 107, 109, 113, 127, 131];
        for seed in seeds {
            got += mean_estimate_shadowed(&p, &lamps, &tree, &balls, seed, 20000);
        }
        got = got * (1.0 / seeds.len() as f32);
        let rel = (luma(got) - luma(want)) / luma(want).max(1.0e-6);
        eprintln!("{name:>24}: shadowed exact {:.4} mean {:.4} rel {:+.4}", luma(want), luma(got), rel);
        if rel.abs() > TOL {
            failures.push(format!("{name}: rel {rel:+.4}"));
        }
    }
    assert!(failures.is_empty(), "biased pixels: {failures:?}");
}

/// Temporal reuse while the bright lamp orbits (lamps-spin): the history
/// sample is re-scored at this frame's lamp position (`tq_h`), and when any
/// lamp changed the history keeps at most the current M (header word 39). The
/// tree is rebuilt each frame like the live prefix. Over many frames the
/// mean estimate must equal the mean of the per-frame exact sums. A near
/// lamp is heavy-tailed, so a pixel fails only when its bias is both over
/// `TOL` and over 3 standard errors of the 16 per-seed estimates.
#[test]
fn live_estimator_moving_lamp_temporal_matches_the_exact_sum() {
    let base = scene();
    let spin = base.len() - 3;
    let mut failures = Vec::new();
    for (name, p) in pixels() {
        let mut got = 0.0f64;
        let mut want = 0.0f64;
        let frames = 10000u32;
        let mut per = Vec::new();
        for seed in [11u64, 23, 37, 41, 53, 67, 71, 89, 97, 101, 103, 107, 109, 113, 127, 131] {
            let (g0, w0) = (got, want);
            let mut rng = Rng::new(seed);
            let mut hist = Res::default();
            for f in 0..frames {
                let a = f as f32 * 0.07;
                let mut lamps = base.clone();
                lamps[spin].pos = v(0.3 + 0.8 * a.cos(), 0.6, 0.2 + 0.8 * a.sin());
                let refs: Vec<Lamp> = lamps.iter().map(|l| Lamp { position: l.pos, intensity: l.color }).collect();
                let tree = LightTree::build(&refs);
                let c = initial(&p, &lamps, &tree, &mut rng);
                let m_now = if c.lamp.is_none() { CANDIDATES } else { c.m };
                // Any lamp changed this frame (the orbiting one always has):
                // every history sample gets the same clamp, whatever it holds.
                if hist.m > m_now {
                    hist.w_sum *= m_now as f32 / hist.m as f32;
                    hist.m = m_now;
                }
                let unit = rng.next_f32();
                let r = temporal(&p, &lamps, c, hist, unit);
                hist = r;
                if let Some(l) = r.lamp {
                    got += f64::from(luma(lamp_light(&lamps[l], origin(&p), p.n, p.two_sided) * w_of(r)));
                }
                want += f64::from(luma(exact(&p, &lamps)));
            }
            per.push((got - g0) / (want - w0).max(1.0e-9) - 1.0);
        }
        let k = per.len() as f64;
        let mu = per.iter().sum::<f64>() / k;
        let se = (per.iter().map(|x| (x - mu) * (x - mu)).sum::<f64>() / (k - 1.0)).sqrt() / k.sqrt();
        let rel = ((got - want) / want.max(1.0e-6)) as f32;
        eprintln!("{name:>24}: moving exact {want:.1} mean {got:.1} rel {rel:+.4} (seeds {mu:+.4} se {se:.4})");
        if want > 0.0 && rel.abs() > TOL && f64::from(rel.abs()) > 3.0 * se {
            failures.push(format!("{name}: rel {rel:+.4}"));
        }
    }
    assert!(failures.is_empty(), "biased pixels: {failures:?}");
}

/// `balance_list` (restir_spatial_ops.glsl) over the current pixel and one
/// neighbour: empty slots carry no M and drop out of the MIS denominator.
fn spatial(ps: &[Pixel], lamps: &[LiveLamp], slots: &[Res], unit: f32) -> Res {
    let empty = |r: &Res| r.lamp.is_none() || r.m == 0;
    if slots.len() == 2 && empty(&slots[0]) {
        return if empty(&slots[1]) { Res::default() } else { slots[1] };
    }
    let here = &ps[0];
    let tq: Vec<f32> = slots.iter().map(|r| r.lamp.map_or(0.0, |l| phat(here, &lamps[l]))).collect();
    let mass: Vec<f32> = slots
        .iter()
        .zip(&tq)
        .map(|(r, &t)| if !empty(r) && t > 0.0 { r.m as f32 * w_of(*r) * t } else { 0.0 })
        .collect();
    let area: f32 = mass.iter().sum();
    if area <= 0.0 {
        return Res::default();
    }
    let mark = unit * area;
    let mut acc = 0.0;
    let mut sel = None;
    for (i, &m) in mass.iter().enumerate() {
        if m <= 0.0 {
            continue;
        }
        sel = Some(i);
        acc += m;
        if mark < acc {
            break;
        }
    }
    let Some(sel) = sel else { return Res::default() };
    let lamp = slots[sel].lamp.unwrap_or(0);
    let t_src = phat(&ps[sel], &lamps[lamp]);
    if tq[sel] <= 0.0 || t_src <= 0.0 {
        return Res::default();
    }
    let mut denom = 0.0;
    let mut count = 0;
    for (r, p) in slots.iter().zip(ps) {
        if empty(r) {
            continue;
        }
        count += r.m;
        denom += r.m as f32 * phat(p, &lamps[lamp]);
    }
    if denom <= 0.0 || count == 0 {
        return Res::default();
    }
    let w_new = area * t_src / (tq[sel] * denom);
    Res { lamp: Some(lamp), w_sum: w_new * count as f32 * tq[sel], m: count, target: tq[sel] }
}

/// Spatial reuse alone (no history): a miss-heavy pixel and its neighbour on
/// the same face. The mean at the first pixel must equal its exact sum.
#[test]
fn live_estimator_spatial_matches_the_exact_sum() {
    let lamps = scene();
    let refs: Vec<Lamp> = lamps.iter().map(|l| Lamp { position: l.pos, intensity: l.color }).collect();
    let tree = LightTree::build(&refs);
    let pairs = [
        ("outward wall past ring", v(17.0, 1.0, 0.0), v(17.0, 1.0, 0.3), v(1.0, 0.0, 0.0)),
        ("open floor", v(1.7, 0.0, 3.9), v(1.9, 0.0, 3.9), v(0.0, 1.0, 0.0)),
        ("wall facing bright lamp", v(1.3, 0.5, 0.2), v(1.3, 0.5, 0.4), v(-1.0, 0.0, 0.0)),
    ];
    let mut failures = Vec::new();
    for (name, a, b, n) in pairs {
        let ps = [Pixel { pos: a, n, two_sided: false }, Pixel { pos: b, n, two_sided: false }];
        let want = luma(exact(&ps[0], &lamps));
        let mut per_seed = Vec::new();
        for seed in 1u64..=16 {
            let mut rng = Rng::new(seed);
            let mut sum = Vec3::ZERO;
            let frames = 150_000;
            for _ in 0..frames {
                let slots = [initial(&ps[0], &lamps, &tree, &mut rng), initial(&ps[1], &lamps, &tree, &mut rng)];
                let r = spatial(&ps, &lamps, &slots, rng.next_f32());
                if let Some(l) = r.lamp {
                    sum += lamp_light(&lamps[l], origin(&ps[0]), n, false) * w_of(r);
                }
            }
            per_seed.push(luma(sum) / frames as f32);
        }
        let k = per_seed.len() as f32;
        let mean = per_seed.iter().sum::<f32>() / k;
        let var = per_seed.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / (k - 1.0);
        let rel = (mean - want) / want.max(1.0e-6);
        let se = (var / k).sqrt() / want.max(1.0e-6);
        eprintln!("{name:>24}: exact {want:.4} mean {mean:.4} rel {rel:+.4} se {se:.4}");
        if rel.abs() > TOL && rel.abs() > 3.0 * se {
            failures.push(format!("{name}: rel {rel:+.4} se {se:.4}"));
        }
    }
    assert!(failures.is_empty(), "biased pixels: {failures:?}");
}
