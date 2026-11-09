//! Camera-turn history retention for the live temporal reprojection
//! (`restir_reproject` in `restir_live.comp`). Lavapipe's turn oracle grades
//! against a noisy low-spp reference, so this is the local, deterministic
//! gate: a CPU mirror of the shader's reprojection over an analytic hallway
//! (floor, ceiling, two walls, an end wall and a pillar), with ground truth
//! from ray casts.
//!
//! For every current pixel, history is *truly valid* when the previous
//! camera saw the same surface point (its ray to the point hits nothing
//! nearer). The live gate is the shader's combination: relative depth 0.02,
//! absolute 0.05 m, normal dot 0.9, one tap, and the candidate ray. On a yaw
//! turn that combination keeps at least 97 % of truly valid pixels and under
//! 1 % of disoccluded ones. The imported C5 policy (relative 0.1, five taps)
//! is printed as a historical diagnostic. It is not the live gate, and a miss
//! on the live policy is a failure of this test.

#![allow(
    clippy::pedantic,
    clippy::restriction,
    reason = "a test-only CPU mirror of shader arithmetic"
)]

type V = [f32; 3];
fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

const W: usize = 160;
const H: usize = 90;
const T: f32 = 0.577_35; // tan(30 deg)
const ASPECT: f32 = W as f32 / H as f32;

#[derive(Clone, Copy)]
struct Cam {
    eye: V,
    right: V,
    up: V,
    fwd: V,
}

fn cam(yaw_deg: f32, strafe: f32) -> Cam {
    let a = yaw_deg.to_radians();
    Cam {
        eye: [0.2 + strafe, 1.6, 0.0],
        right: [a.cos(), 0.0, -a.sin()],
        up: [0.0, 1.0, 0.0],
        fwd: [a.sin(), 0.0, a.cos()],
    }
}

fn ray(c: &Cam, px: f32, py: f32) -> V {
    let nx = px / (0.5 * W as f32) - 1.0;
    let ny = 1.0 - py / (0.5 * H as f32);
    add(c.fwd, add(mul(c.right, nx * T * ASPECT), mul(c.up, ny * T)))
}

/// Nearest hit: (t, normal). Hallway x in [-1.5, 1.5], y in [0, 2.6],
/// end wall z = 25, and a pillar box x [0.5, 0.9], z [4, 4.4].
fn cast(o: V, d: V) -> (f32, V) {
    let mut best = (f32::INFINITY, [0.0; 3]);
    let mut plane = |axis: usize, at: f32, n: V, lo: V, hi: V| {
        if d[axis].abs() < 1e-6 {
            return;
        }
        let t = (at - o[axis]) / d[axis];
        if t <= 1e-4 || t >= best.0 {
            return;
        }
        let p = add(o, mul(d, t));
        for k in 0..3 {
            if k != axis && (p[k] < lo[k] - 1e-4 || p[k] > hi[k] + 1e-4) {
                return;
            }
        }
        best = (t, n);
    };
    let room_lo = [-1.5, 0.0, -50.0];
    let room_hi = [1.5, 2.6, 25.0];
    plane(1, 0.0, [0.0, 1.0, 0.0], room_lo, room_hi);
    plane(1, 2.6, [0.0, -1.0, 0.0], room_lo, room_hi);
    plane(0, -1.5, [1.0, 0.0, 0.0], room_lo, room_hi);
    plane(0, 1.5, [-1.0, 0.0, 0.0], room_lo, room_hi);
    plane(2, 25.0, [0.0, 0.0, -1.0], room_lo, room_hi);
    let (blo, bhi) = ([0.5, 0.0, 4.0], [0.9, 2.6, 4.4]);
    plane(0, 0.5, [-1.0, 0.0, 0.0], blo, bhi);
    plane(0, 0.9, [1.0, 0.0, 0.0], blo, bhi);
    plane(2, 4.0, [0.0, 0.0, -1.0], blo, bhi);
    plane(2, 4.4, [0.0, 0.0, 1.0], blo, bhi);
    best
}

/// G-buffer at pixel centres: (pos, normal, fwd depth).
fn gbuffer(c: &Cam) -> Vec<(V, V, f32)> {
    let mut g = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let d = ray(c, x as f32 + 0.5, y as f32 + 0.5);
            let (t, n) = cast(c.eye, d);
            let p = add(c.eye, mul(d, t));
            g.push((p, n, dot(sub(p, c.eye), c.fwd)));
        }
    }
    g
}

struct Params {
    depth_rel: f32,
    depth_abs: f32,
    normal_min: f32,
    search: bool,
    candidate_ray: bool,
}
/// View-depth mirror. Not the live shader: `restir_reproject` compares the
/// candidate ray, not this frame's view depth.
const VIEW_DEPTH: Params = Params {
    depth_rel: 0.02,
    depth_abs: 0.05,
    normal_min: 0.9,
    search: false,
    candidate_ray: false,
};
/// Live `restir_reproject`: relative 0.02, absolute 0.05 m, normal 0.9, one
/// tap, candidate ray. Same numbers as `restir_live.comp`.
const LIVE: Params = Params {
    depth_rel: 0.02,
    depth_abs: 0.05,
    normal_min: 0.9,
    search: false,
    candidate_ray: true,
};
/// Imported C5 head (Bitterli 2020 §6 relative 0.1, five taps). Historical
/// diagnostic only. Not an authorization of the live constants.
const C5_HISTORICAL: Params = Params {
    depth_rel: 0.1,
    depth_abs: 0.05,
    normal_min: 0.9,
    search: true,
    candidate_ray: true,
};

fn depth_normal(p: &Params, z_ref: f32, z_other: f32, n_ref: V, n_other: V) -> bool {
    if z_ref.is_nan() || z_other.is_nan() || z_ref <= 0.0 || z_other <= 0.0 {
        return false;
    }
    let gap = (z_ref - z_other).abs();
    if gap / z_ref > p.depth_rel && gap > p.depth_abs {
        return false;
    }
    dot(n_ref, n_other) >= p.normal_min
}

/// Pixel `floor(px) + offset`, or `None` off the grid. Lossless: the floor is
/// found by search, not an `f32` to integer cast.
fn tap(px: [f32; 2], ox: isize, oy: isize) -> Option<(usize, usize)> {
    let floor = |v: f32, n: usize| (0..n).find(|&i| (i + 1) as f32 > v);
    let qx = floor(px[0], W)?.checked_add_signed(ox).filter(|&q| q < W)?;
    let qy = floor(px[1], H)?.checked_add_signed(oy).filter(|&q| q < H)?;
    Some((qx, qy))
}

/// Mirror of `restir_reproject`.
fn reproject(p: &Params, prev: &Cam, hist: &[(V, V, f32)], pos: V, nrm: V) -> bool {
    let rel = sub(pos, prev.eye);
    let z = dot(rel, prev.fwd);
    if z <= 1e-3 {
        return false;
    }
    let ndc = [
        dot(rel, prev.right) / (z * T * ASPECT),
        dot(rel, prev.up) / (z * T),
    ];
    let px = [
        (ndc[0] + 1.0) * 0.5 * W as f32,
        (1.0 - ndc[1]) * 0.5 * H as f32,
    ];
    if px[0] < 0.0 || px[1] < 0.0 || px[0] >= W as f32 || px[1] >= H as f32 {
        return false;
    }
    let offs: &[(isize, isize)] = if p.search {
        &[(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
    } else {
        &[(0, 0)]
    };
    for &(ox, oy) in offs {
        let Some((qx, qy)) = tap(px, ox, oy) else {
            continue;
        };
        let (_, n_prev, z_prev) = hist[qy * W + qx];
        let z_cmp = if p.candidate_ray {
            let d = ray(prev, qx as f32 + 0.5, qy as f32 + 0.5);
            let den = dot(d, nrm);
            if den.abs() > 1e-3 {
                dot(rel, nrm) / den
            } else {
                z
            }
        } else {
            z
        };
        if depth_normal(p, z_cmp, z_prev, nrm, n_prev) {
            return true;
        }
    }
    false
}

/// Whether any of the five history taps around `pos`'s reprojection shows
/// the same plane as (`pos`, `n`).
fn same_plane_nearby(prev: &Cam, hist: &[(V, V, f32)], pos: V, n: V) -> bool {
    let rel = sub(pos, prev.eye);
    let z = dot(rel, prev.fwd);
    let px = [
        (dot(rel, prev.right) / (z * T * ASPECT) + 1.0) * 0.5 * W as f32,
        (1.0 - dot(rel, prev.up) / (z * T)) * 0.5 * H as f32,
    ];
    [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .any(|&(ox, oy)| {
            let Some((qx, qy)) = tap(px, ox, oy) else {
                return false;
            };
            let (hp, hn, _) = hist[qy * W + qx];
            dot(hn, n) > 0.999 && dot(sub(hp, pos), n).abs() < 1e-3
        })
}

struct Retention {
    valid: u32,
    kept: u32,
    occl: u32,
    false_keep: u32,
}

impl Retention {
    fn keep(&self) -> f32 {
        self.kept as f32 / self.valid.max(1) as f32
    }

    fn false_rate(&self) -> f32 {
        self.false_keep as f32 / self.occl.max(1) as f32
    }

    fn rejected(&self) -> u32 {
        self.valid.saturating_sub(self.kept)
    }
}

/// Counts over a yaw turn. `kept / valid` is retained truly-visible history.
/// `valid - kept` is the rejection of that history. `false_keep / occl` is
/// history kept on a true disocclusion.
fn retention(p: &Params, yaw_step: f32, strafe_step: f32, frames: usize) -> Retention {
    let (mut valid, mut kept, mut occl, mut false_keep) = (0u32, 0u32, 0u32, 0u32);
    for f in 0..frames {
        let (pc, cc) = (
            cam(f as f32 * yaw_step, f as f32 * strafe_step),
            cam((f + 1) as f32 * yaw_step, (f + 1) as f32 * strafe_step),
        );
        let hist = gbuffer(&pc);
        for &(pos, n, _) in &gbuffer(&cc) {
            let rel = sub(pos, pc.eye);
            let z = dot(rel, pc.fwd);
            let ndc = [
                dot(rel, pc.right) / (z * T * ASPECT),
                dot(rel, pc.up) / (z * T),
            ];
            if z <= 1e-3 || ndc[0].abs() >= 1.0 || ndc[1].abs() >= 1.0 {
                continue;
            } // off-screen: not a disocclusion
            let len = dot(rel, rel).sqrt();
            let (t, _) = cast(pc.eye, mul(rel, 1.0 / len));
            let seen = (t - len).abs() < 1e-3 * len.max(1.0);
            let keep = reproject(p, &pc, &hist, pos, n);
            if seen {
                valid += 1;
                kept += keep as u32;
            } else if !same_plane_nearby(&pc, &hist, pos, n) {
                // A hidden point whose history taps all show another surface:
                // a true disocclusion. (A hidden point next to history of its
                // own plane, at a silhouette, may legitimately reuse it.)
                occl += 1;
                false_keep += keep as u32;
            }
        }
    }
    Retention {
        valid,
        kept,
        occl,
        false_keep,
    }
}

#[test]
fn a_camera_turn_keeps_temporal_history_except_on_disocclusion() {
    // Pure yaw (no disocclusion possible: occlusion does not depend on
    // orientation), then yaw while strafing past the pillar.
    // 97 % retained and 1 % false-keep are the established expectation.
    // They are not lowered when the live policy misses them.
    for &(step, strafe) in &[(1.0f32, 0.0f32), (3.0, 0.0), (2.0, -0.05)] {
        let view = retention(&VIEW_DEPTH, step, strafe, 8);
        let live = retention(&LIVE, step, strafe, 8);
        let c5 = retention(&C5_HISTORICAL, step, strafe, 8);
        eprintln!(
            "yaw {step} deg strafe {strafe} m/frame: \
             view-depth kept {kept_v}/{valid_v} ({keep_v:.4}) rejected {rej_v} false {fk_v}/{oc_v} ({fr_v:.4}) | \
             live kept {kept}/{valid} ({keep:.4}) rejected {rej} false {fk}/{oc} ({fr:.4}) | \
             c5-historical kept {kept_c}/{valid_c} ({keep_c:.4}) rejected {rej_c} false {fk_c}/{oc_c} ({fr_c:.4})",
            kept_v = view.kept,
            valid_v = view.valid,
            keep_v = view.keep(),
            rej_v = view.rejected(),
            fk_v = view.false_keep,
            oc_v = view.occl,
            fr_v = view.false_rate(),
            kept = live.kept,
            valid = live.valid,
            keep = live.keep(),
            rej = live.rejected(),
            fk = live.false_keep,
            oc = live.occl,
            fr = live.false_rate(),
            kept_c = c5.kept,
            valid_c = c5.valid,
            keep_c = c5.keep(),
            rej_c = c5.rejected(),
            fk_c = c5.false_keep,
            oc_c = c5.occl,
            fr_c = c5.false_rate(),
        );
        assert!(
            live.keep() >= 0.97,
            "live turn {step}: kept {}/{} ({:.4}) of valid history, rejected {}",
            live.kept,
            live.valid,
            live.keep(),
            live.rejected(),
        );
        assert!(
            live.false_rate() <= 0.01,
            "live turn {step}: kept {}/{} ({:.4}) of disoccluded history",
            live.false_keep,
            live.occl,
            live.false_rate(),
        );
        assert!(
            c5.keep() >= 0.97 && c5.false_rate() <= 0.01,
            "c5 historical diagnostic moved: keep {:.4} false {:.4}",
            c5.keep(),
            c5.false_rate(),
        );
    }
}
