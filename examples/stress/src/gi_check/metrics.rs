//! Picture metrics for `gi-check`: a live picture against the CPU path-traced
//! reference, as luminance planes (docs/systems/debugging.md, "gi-check").
//!
//! Pictures are display values = tone(linear) with no gamma (`genos_debug`
//! reference.rs), so linear light is the tone curve undone ([`linear`]).

use std::path::Path;

use genos_debug::Image;

/// A W x H plane of f64, rows top to bottom.
#[derive(Clone)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub v: Vec<f64>,
}

impl Plane {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            v: vec![0.0; w * h],
        }
    }
    fn at(&self, x: usize, y: usize) -> f64 {
        self.v[y * self.w + x]
    }
    fn zip(&self, o: &Plane, f: impl Fn(f64, f64) -> f64) -> Self {
        Self {
            w: self.w,
            h: self.h,
            v: self.v.iter().zip(&o.v).map(|(&a, &b)| f(a, b)).collect(),
        }
    }
}

/// A W x H mask.
#[derive(Clone)]
pub struct Mask {
    w: usize,
    h: usize,
    v: Vec<bool>,
}

impl Mask {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            v: vec![false; w * h],
        }
    }
    fn count(&self) -> usize {
        self.v.iter().filter(|&&b| b).count()
    }
    fn and(&self, o: &Mask) -> Self {
        Self {
            w: self.w,
            h: self.h,
            v: self.v.iter().zip(&o.v).map(|(&a, &b)| a && b).collect(),
        }
    }
    fn or(&self, o: &Mask) -> Self {
        Self {
            w: self.w,
            h: self.h,
            v: self.v.iter().zip(&o.v).map(|(&a, &b)| a || b).collect(),
        }
    }
    fn not(&self) -> Self {
        Self {
            w: self.w,
            h: self.h,
            v: self.v.iter().map(|&a| !a).collect(),
        }
    }
    /// Grown by r pixels in a square (edges wrap, as the first Python tool did).
    fn dilate(&self, r: usize) -> Self {
        let mut out = self.clone();
        let (w, h) = (self.w as isize, self.h as isize);
        let r = r as isize;
        for y in 0..h {
            for x in 0..w {
                if !self.v[(y * w + x) as usize] {
                    continue;
                }
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (yy, xx) = ((y + dy).rem_euclid(h), (x + dx).rem_euclid(w));
                        out.v[(yy * w + xx) as usize] = true;
                    }
                }
            }
        }
        out
    }
}

/// Display value 0..1 to linear light: the engine's tone curve undone.
pub fn linear(d: f64) -> f64 {
    if d <= 0.64 {
        d.max(0.0)
    } else {
        let k = ((d - 0.64) / 0.14).clamp(0.0, 0.999);
        0.64 + 1.1 * k / (1.0 - k)
    }
}

fn rgb_plane(img: &Image, w: usize, h: usize, f: impl Fn([f64; 3]) -> f64) -> Plane {
    let mut p = Plane::new(w, h);
    let (iw, ih) = (img.width as usize, img.height as usize);
    // Same size, or an area average when the picture is a whole multiple larger,
    // or nearest otherwise.
    let (fx, fy) = (iw / w.max(1), ih / h.max(1));
    let whole = fx >= 1 && fy >= 1 && iw == fx * w && ih == fy * h;
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            let mut n = 0.0;
            let (bx, by, sx, sy) = if whole {
                (x * fx, y * fy, fx, fy)
            } else {
                (x * iw / w, y * ih / h, 1, 1)
            };
            for yy in by..by + sy {
                for xx in bx..bx + sx {
                    let i = (yy * iw + xx) * 3;
                    let c = [img.rgb[i], img.rgb[i + 1], img.rgb[i + 2]].map(|v| v as f64 / 255.0);
                    acc += f(c);
                    n += 1.0;
                }
            }
            p.v[y * w + x] = acc / n;
        }
    }
    p
}

/// Linear luminance of a picture, resampled to w x h.
pub fn luminance(img: &Image, w: usize, h: usize) -> Plane {
    rgb_plane(img, w, h, |c| {
        let c = c.map(linear);
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    })
}

pub fn load(path: &Path) -> Result<Image, String> {
    Image::load(path)
}

fn kernel(s: f64) -> Vec<f64> {
    let r = (3.0 * s + 0.5) as isize;
    let k: Vec<f64> = (-r..=r)
        .map(|x| (-(x * x) as f64 / (2.0 * s * s)).exp())
        .collect();
    let sum: f64 = k.iter().sum();
    k.into_iter().map(|v| v / sum).collect()
}

/// Separable Gaussian, edges repeated.
fn blur(a: &Plane, s: f64) -> Plane {
    let k = kernel(s);
    let r = (k.len() / 2) as isize;
    let (w, h) = (a.w as isize, a.h as isize);
    let mut t = Plane::new(a.w, a.h);
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let xx = (x + i as isize - r).clamp(0, w - 1);
                acc += kv * a.v[(y * w + xx) as usize];
            }
            t.v[(y * w + x) as usize] = acc;
        }
    }
    let mut out = Plane::new(a.w, a.h);
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let yy = (y + i as isize - r).clamp(0, h - 1);
                acc += kv * t.v[(yy * w + x) as usize];
            }
            out.v[(y * w + x) as usize] = acc;
        }
    }
    out
}

/// Blur of a over mask m, normalised by the blurred mask; and that weight.
fn masked_blur(a: &Plane, m: &Plane, s: f64) -> (Plane, Plane) {
    let num = blur(&a.zip(m, |x, y| x * y), s);
    let den = blur(m, s);
    (num.zip(&den, |n, d| n / d.max(1.0e-6)), den)
}

fn grad(a: &Plane) -> (Plane, Plane) {
    let mut gx = Plane::new(a.w, a.h);
    let mut gy = Plane::new(a.w, a.h);
    for y in 0..a.h {
        for x in 0..a.w {
            if x > 0 && x + 1 < a.w {
                gx.v[y * a.w + x] = (a.at(x + 1, y) - a.at(x - 1, y)) * 0.5;
            }
            if y > 0 && y + 1 < a.h {
                gy.v[y * a.w + x] = (a.at(x, y + 1) - a.at(x, y - 1)) * 0.5;
            }
        }
    }
    (gx, gy)
}

fn sum_where(a: &Plane, m: &Mask) -> f64 {
    a.v.iter()
        .zip(&m.v)
        .filter(|(_, &b)| b)
        .map(|(&x, _)| x)
        .sum()
}

/// Deterministic standard normal noise.
fn gaussian_noise(n: usize) -> Vec<f64> {
    let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        ((s >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    };
    (0..n)
        .map(|_| {
            let (u, v) = (next(), next());
            (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
        })
        .collect()
}

/// One view's scores. Errors are relative (share of the light); lower is better.
#[derive(Clone, Debug, Default)]
pub struct ViewScore {
    pub view: String,
    /// sum |live - ref| / sum ref on 2 x 2 blocks (as `compare`).
    pub mean_rel: f64,
    pub bias: f64,
    /// Band-pass (sigma 2 - 10 px) residual RMS on flat evenly lit surfaces, the
    /// reference's own noise through the same band, and the excess over it.
    pub blob: f64,
    pub blob_floor: f64,
    pub blob_excess: f64,
    /// Relative error (sigma 1 px) within 2 px of lines where surfaces meet, and
    /// more than 20 px from them (open wall).
    pub contact: f64,
    pub open: f64,
    /// Gradient error along geometry and shadow edges.
    pub edge_err: f64,
}

/// Scores of one view: `dir/<view>-live.png` and the G-buffer captures
/// (`-normal`, `-depth`, `-albedo`, `-direct`) against `ref_dir/<view>-reference.png`.
/// `ref_noise` is the reference's per-pixel relative noise (its floor).
pub fn score_view(
    dir: &Path,
    ref_dir: &Path,
    view: &str,
    ref_noise: f64,
) -> Result<ViewScore, String> {
    let rimg = load(&ref_dir.join(format!("{view}-reference.png")))?;
    let (w, h) = (rimg.width as usize, rimg.height as usize);
    let reference = luminance(&rimg, w, h);
    let live = luminance(&load(&dir.join(format!("{view}-live.png")))?, w, h);
    let nimg = load(&dir.join(format!("{view}-normal.png")))?;
    let normal: Vec<Plane> = (0..3)
        .map(|c| rgb_plane(&nimg, w, h, move |p| p[c] * 2.0 - 1.0))
        .collect();
    let depth = rgb_plane(&load(&dir.join(format!("{view}-depth.png")))?, w, h, |p| {
        p[0]
    });
    let aimg = load(&dir.join(format!("{view}-albedo.png")))?;
    let albedo: Vec<Plane> = (0..3)
        .map(|c| rgb_plane(&aimg, w, h, move |p| p[c]))
        .collect();
    let direct = luminance(&load(&dir.join(format!("{view}-direct.png")))?, w, h);
    let mut hit = Mask::new(w, h);
    for i in 0..w * h {
        hit.v[i] = depth.v[i] > 0.0;
    }
    // Geometry edges (normal or depth breaks) and albedo edges, against the
    // neighbour above and to the left (wrapping, as numpy roll).
    let mut nb = Mask::new(w, h);
    let mut db = Mask::new(w, h);
    let mut ab = Mask::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            for j in [((y + h - 1) % h) * w + x, y * w + (x + w - 1) % w] {
                let dot: f64 = (0..3).map(|c| normal[c].v[i] * normal[c].v[j]).sum();
                nb.v[i] |= dot < 0.9;
                db.v[i] |= (depth.v[i] - depth.v[j]).abs() > 0.02 + 0.03 * (1.0 - depth.v[i]);
                let da = (0..3)
                    .map(|c| (albedo[c].v[i] - albedo[c].v[j]).abs())
                    .fold(0.0, f64::max);
                ab.v[i] |= da > 0.06;
            }
        }
    }
    let geo = nb.or(&db);
    let contact = nb.and(&db.dilate(1).not());
    // Shadow edges from the engine's exact direct view.
    let (gdx, gdy) = grad(&blur(&direct, 1.0));
    let mut lit: Vec<f64> = direct
        .v
        .iter()
        .zip(&hit.v)
        .filter(|(_, &b)| b)
        .map(|(&x, _)| x)
        .collect();
    lit.sort_by(|a, b| a.total_cmp(b));
    let p95 = lit
        .get(((lit.len() as f64 - 1.0) * 0.95).round() as usize)
        .copied()
        .unwrap_or(0.0);
    let thr = 0.08 * p95.max(1.0e-3);
    let mut shadow = Mask::new(w, h);
    let geo1 = geo.dilate(1);
    for i in 0..w * h {
        shadow.v[i] = gdx.v[i].hypot(gdy.v[i]) > thr && !geo1.v[i];
    }
    let edgeband = geo.or(&shadow).dilate(2).and(&hit);
    let flat = hit
        .and(&geo.or(&ab).dilate(4).not())
        .and(&shadow.dilate(4).not());
    // Edge gradient error.
    let (lx, ly) = grad(&blur(&live, 1.0));
    let (rx, ry) = grad(&blur(&reference, 1.0));
    let ge = Plane {
        w,
        h,
        v: (0..w * h)
            .map(|i| (lx.v[i] - rx.v[i]).hypot(ly.v[i] - ry.v[i]))
            .collect(),
    };
    let gr = Plane {
        w,
        h,
        v: (0..w * h).map(|i| rx.v[i].hypot(ry.v[i])).collect(),
    };
    let edge_err = sum_where(&ge, &edgeband) / sum_where(&gr, &edgeband).max(1.0e-9);
    // Blob: band-pass sigma 2 - 10 on the flat mask.
    let f = Plane {
        w,
        h,
        v: flat.v.iter().map(|&b| if b { 1.0 } else { 0.0 }).collect(),
    };
    let band = |a: &Plane| {
        let (a0, _) = masked_blur(a, &f, 2.0);
        let (a1, den) = masked_blur(a, &f, 10.0);
        (a0.zip(&a1, |x, y| x - y), a1, den)
    };
    let (lb, _, den8) = band(&live);
    let (rb, r8, _) = band(&reference);
    let mut ok = Mask::new(w, h);
    for i in 0..w * h {
        ok.v[i] = flat.v[i] && r8.v[i] > 0.01 && den8.v[i] > 0.2;
    }
    let rms = |p: &Plane| {
        let n = ok.count().max(1) as f64;
        let s: f64 = (0..w * h)
            .filter(|&i| ok.v[i])
            .map(|i| (p.v[i] / r8.v[i].max(0.01)).powi(2))
            .sum();
        (s / n).sqrt()
    };
    let blob = rms(&lb.zip(&rb, |a, b| a - b));
    let noise = gaussian_noise(w * h);
    let nz = Plane {
        w,
        h,
        v: (0..w * h)
            .map(|i| noise[i] * reference.v[i].max(0.0) * ref_noise)
            .collect(),
    };
    let (nb_band, _, _) = band(&nz);
    let blob_floor = rms(&nb_band);
    // Mean error on 2 x 2 blocks.
    let (mut diff, mut bias, mut total) = (0.0, 0.0, 0.0);
    for by in 0..h / 2 {
        for bx in 0..w / 2 {
            let idx = [
                (2 * by) * w + 2 * bx,
                (2 * by) * w + 2 * bx + 1,
                (2 * by + 1) * w + 2 * bx,
                (2 * by + 1) * w + 2 * bx + 1,
            ];
            let hits = idx.iter().filter(|&&i| hit.v[i]).count();
            if hits < 2 {
                continue;
            }
            let l: f64 = idx.iter().map(|&i| live.v[i]).sum::<f64>() / 4.0;
            let r: f64 = idx.iter().map(|&i| reference.v[i]).sum::<f64>() / 4.0;
            diff += (l - r).abs();
            bias += l - r;
            total += r;
        }
    }
    // Error by distance from contact lines, away from silhouettes, shadow edges and
    // albedo edges.
    let mut dist = vec![99u32; w * h];
    let mut cur = contact.clone();
    for r in 0..33 {
        for i in 0..w * h {
            if cur.v[i] && dist[i] == 99 {
                dist[i] = r;
            }
        }
        cur = cur.dilate(1);
    }
    let clean = hit.and(&db.or(&shadow).or(&ab).dilate(2).not());
    let gl = blur(&live, 1.0);
    let grf = blur(&reference, 1.0);
    let ring = |a: u32, b: u32| {
        let (mut e, mut t, mut n) = (0.0, 0.0, 0usize);
        for i in 0..w * h {
            if clean.v[i] && dist[i] >= a && dist[i] <= b {
                e += (gl.v[i] - grf.v[i]).abs();
                t += grf.v[i];
                n += 1;
            }
        }
        if n > 30 {
            e / t.max(1.0e-9)
        } else {
            f64::NAN
        }
    };
    Ok(ViewScore {
        view: view.to_string(),
        mean_rel: diff / total.max(1.0e-9),
        bias: bias / total.max(1.0e-9),
        blob,
        blob_floor,
        blob_excess: (blob * blob - blob_floor * blob_floor).max(0.0).sqrt(),
        contact: ring(0, 2),
        open: ring(21, 99),
        edge_err,
    })
}

/// Cold-versus-warm gap of one frame of a walk, both resampled to 320 x 180:
/// the whole-frame mean gap and the worst 10 x 10 px tile (relative).
pub fn walk_gap(cold: &Path, warm: &Path) -> Result<(f64, f64), String> {
    let (w, h) = (320, 180);
    let c = luminance(&load(cold)?, w, h);
    let m = luminance(&load(warm)?, w, h);
    let mean_w: f64 = m.v.iter().sum::<f64>() / (w * h) as f64;
    let gap: f64 =
        c.v.iter()
            .zip(&m.v)
            .map(|(a, b)| (a - b).abs())
            .sum::<f64>()
            / (w * h) as f64
            / mean_w.max(1.0e-4);
    let mut worst: f64 = 0.0;
    for ty in 0..h / 10 {
        for tx in 0..w / 10 {
            let (mut sc, mut sw) = (0.0, 0.0);
            for y in ty * 10..ty * 10 + 10 {
                for x in tx * 10..tx * 10 + 10 {
                    sc += c.at(x, y);
                    sw += m.at(x, y);
                }
            }
            let (sc, sw) = (sc / 100.0, sw / 100.0);
            worst = worst.max((sc - sw).abs() / sw.max(0.01));
        }
    }
    Ok((gap, worst))
}

/// Linear luminance of a picture at 640 x 360 (the turn check).
pub fn lum640(path: &Path) -> Result<Plane, String> {
    Ok(luminance(&load(path)?, 640, 360))
}

/// Low-frequency difference of a against b: both blurred (8 px at 640 x 360, the
/// size of the blotches a turn leaves), mean |a - b| over b's mean light.
pub fn blob(a: &Plane, b: &Plane) -> f64 {
    let (a, b) = (blur(a, 8.0), blur(b, 8.0));
    let sum_b: f64 = b.v.iter().sum();
    a.v.iter().zip(&b.v).map(|(x, y)| (x - y).abs()).sum::<f64>() / sum_b.max(1.0e-6)
}

/// Error of a moving frame against the settled picture at its pose, low frequency
/// and relative to the light where it is (so a dim wall lit only by bounce counts
/// as much as a sunlit one): a - b with pixels off by more than 0.1 (a box edge a
/// pixel out of place, dilated 2 px) dropped, blurred 8 px.
pub fn rel_error(a: &Plane, b: &Plane) -> Plane {
    let (w, h) = (a.w, a.h);
    let mut off = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if (a.at(x, y) - b.at(x, y)).abs() > 0.1 {
                for yy in y.saturating_sub(2)..(y + 3).min(h) {
                    for xx in x.saturating_sub(2)..(x + 3).min(w) {
                        off[yy * w + xx] = true;
                    }
                }
            }
        }
    }
    let mut e = Plane::new(w, h);
    for i in 0..w * h {
        e.v[i] = if off[i] { 0.0 } else { a.v[i] - b.v[i] };
    }
    blur(&e, 8.0)
}

/// The light it is relative to: b blurred 8 px plus 5 % of b's mean.
pub fn rel_base(b: &Plane) -> Plane {
    let mean = b.v.iter().sum::<f64>() / b.v.len().max(1) as f64;
    let mut out = blur(b, 8.0);
    for v in &mut out.v {
        *v += 0.05 * mean;
    }
    out
}

/// Mean of |e| / base (rel_error, rel_base).
pub fn rel_mean(e: &Plane, base: &Plane) -> f64 {
    e.v.iter().zip(&base.v).map(|(x, y)| x.abs() / y).sum::<f64>() / e.v.len().max(1) as f64
}

/// Mean of |e1 - e0| / base: how much the error moved from one frame to the next
/// (the swim).
pub fn rel_swim(e0: &Plane, e1: &Plane, base: &Plane) -> f64 {
    e0.v.iter()
        .zip(&e1.v)
        .zip(&base.v)
        .map(|((a, b), y)| (b - a).abs() / y)
        .sum::<f64>()
        / e0.v.len().max(1) as f64
}

/// Linear luminance of a picture at 320 x 180.
pub fn lum320(path: &Path) -> Result<Plane, String> {
    Ok(luminance(&load(path)?, 320, 180))
}

/// Relative error of `a` against `b` over the whole picture, and the worst
/// 10 x 10 px tile.
pub fn rel_err(a: &Plane, b: &Plane) -> (f64, f64) {
    let (w, h) = (a.w, a.h);
    let sum_b: f64 = b.v.iter().sum();
    let err: f64 =
        a.v.iter()
            .zip(&b.v)
            .map(|(x, y)| (x - y).abs())
            .sum::<f64>()
            / sum_b.max(1.0e-6);
    let mut worst: f64 = 0.0;
    let tile_mean = sum_b / b.v.len().max(1) as f64 * 100.0;
    for ty in 0..h / 10 {
        for tx in 0..w / 10 {
            let (mut sa, mut sb) = (0.0, 0.0);
            for y in ty * 10..ty * 10 + 10 {
                for x in tx * 10..tx * 10 + 10 {
                    sa += a.at(x, y);
                    sb += b.at(x, y);
                }
            }
            // Over the tile's own light, or the frame's mean tile where the tile is
            // darker (a near-black tile would turn any noise into a huge ratio).
            worst = worst.max((sa - sb).abs() / sb.max(tile_mean).max(1.0));
        }
    }
    (err, worst)
}

/// Frame-to-frame change of a beyond b's own change, over b's light.
pub fn excess_change(a0: &Plane, a1: &Plane, b0: &Plane, b1: &Plane) -> f64 {
    let sum_b: f64 = b1.v.iter().sum();
    let e: f64 = (0..a1.v.len())
        .map(|i| ((a1.v[i] - a0.v[i]) - (b1.v[i] - b0.v[i])).abs())
        .sum();
    e / sum_b.max(1.0e-6)
}

/// Depth view at 320 x 180 (near is bright, 0 where nothing).
pub fn depth320(path: &Path) -> Result<Plane, String> {
    Ok(rgb_plane(&load(path)?, 320, 180, |p| p[0]))
}

/// The space a box left: pixels some earlier pose had nearer (the box stood
/// there) that this pose does not, grown by `grow` px (its contact shadow and
/// bounce), minus where anything is nearer now than before (the box now).
pub fn vacated(now: &Plane, before: &[&Plane], grow: usize) -> Vec<bool> {
    let n = now.v.len();
    let mut left = Mask::new(now.w, now.h);
    let mut here = Mask::new(now.w, now.h);
    for b in before {
        for i in 0..n {
            left.v[i] |= b.v[i] > now.v[i] + 0.02;
            here.v[i] |= now.v[i] > b.v[i] + 0.02;
        }
    }
    let here = here.dilate(1);
    let grown = left.dilate(grow);
    (0..n)
        .map(|i| grown.v[i] && !here.v[i] && now.v[i] > 0.0)
        .collect()
}

/// Relative error of `a` against `b` inside `mask`.
pub fn masked_err(a: &Plane, b: &Plane, mask: &[bool]) -> f64 {
    let (mut e, mut t) = (0.0, 0.0);
    for i in 0..a.v.len() {
        if mask[i] {
            e += (a.v[i] - b.v[i]).abs();
            t += b.v[i];
        }
    }
    e / t.max(1.0e-6)
}

/// The light a relative score is taken against: b blurred s px plus 5 % of b's mean.
fn rel_base_s(b: &Plane, s: f64) -> Plane {
    let mean = b.v.iter().sum::<f64>() / b.v.len().max(1) as f64;
    let mut out = blur(b, s);
    for v in &mut out.v {
        *v += 0.05 * mean;
    }
    out
}

/// Picture `a`, taken looking along (yaw_a, pitch), seen from (yaw_b, pitch) at the
/// same eye: a turn on the spot moves the picture exactly by this (fovy 60 degrees,
/// look = (sin y cos p, sin p, -cos y cos p)). Also whether each pixel came from
/// inside `a`.
pub fn warp_yaw(a: &Plane, yaw_a: f64, yaw_b: f64, pitch: f64) -> (Plane, Vec<bool>) {
    let basis = |yaw: f64| {
        let f = [yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos()];
        let r = {
            let r = [-f[2], 0.0, f[0]];
            let l = (r[0] * r[0] + r[2] * r[2]).sqrt();
            [r[0] / l, 0.0, r[2] / l]
        };
        let u = [
            r[1] * f[2] - r[2] * f[1],
            r[2] * f[0] - r[0] * f[2],
            r[0] * f[1] - r[1] * f[0],
        ];
        (f, r, u)
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (fb, rb, ub) = basis(yaw_b);
    let (fa, ra, ua) = basis(yaw_a);
    let (w, h) = (a.w, a.h);
    let t = (30.0f64).to_radians().tan();
    let aspect = w as f64 / h as f64;
    let mut out = Plane::new(w, h);
    let mut ok = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let nx = ((x as f64 + 0.5) / w as f64 * 2.0 - 1.0) * t * aspect;
            let ny = (1.0 - (y as f64 + 0.5) / h as f64 * 2.0) * t;
            let d = [0, 1, 2].map(|i| fb[i] + nx * rb[i] + ny * ub[i]);
            let z = dot(d, fa);
            if z <= 0.0 {
                continue;
            }
            let px = dot(d, ra) / z / (t * aspect);
            let py = dot(d, ua) / z / t;
            let sx = (px + 1.0) / 2.0 * w as f64 - 0.5;
            let sy = (1.0 - py) / 2.0 * h as f64 - 0.5;
            if sx < 0.0 || sy < 0.0 || sx >= (w - 1) as f64 || sy >= (h - 1) as f64 {
                continue;
            }
            let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
            let (fx, fy) = (sx - x0 as f64, sy - y0 as f64);
            out.v[y * w + x] = a.at(x0, y0) * (1.0 - fx) * (1.0 - fy)
                + a.at(x0 + 1, y0) * fx * (1.0 - fy)
                + a.at(x0, y0 + 1) * (1.0 - fx) * fy
                + a.at(x0 + 1, y0 + 1) * fx * fy;
            ok[y * w + x] = true;
        }
    }
    (out, ok)
}

/// World swim of a turn on the spot: frame a (yaw_a) moved onto frame b (yaw_b)
/// by the turn, then the low-frequency change (blurred s px, pixels off by more
/// than 0.1 dropped) relative to the light there (blur of b plus 5 % of its
/// mean), so a dim wall lit only by bounce counts as much as a lit one. A still
/// world and a perfect solution give 0; light patterns redrawn by the turn
/// (blotches that move or change as the probe grid slides) do not.
pub fn world_swim(a: &Plane, b: &Plane, yaw_a: f64, yaw_b: f64, pitch: f64, s: f64) -> f64 {
    let (wa, ok) = warp_yaw(a, yaw_a, yaw_b, pitch);
    let mut e = Plane::new(b.w, b.h);
    let mut m = Plane::new(b.w, b.h);
    for i in 0..b.v.len() {
        if ok[i] {
            m.v[i] = 1.0;
            let d = wa.v[i] - b.v[i];
            if d.abs() <= 0.1 {
                e.v[i] = d;
            }
        }
    }
    let (e, m) = (blur(&e, s), blur(&m, s));
    let base = rel_base_s(b, s);
    let (mut sum, mut n) = (0.0, 0usize);
    for i in 0..b.v.len() {
        if m.v[i] > 0.99 {
            sum += e.v[i].abs() / base.v[i];
            n += 1;
        }
    }
    sum / n.max(1) as f64
}

/// Still-frame blotch: the band of the error of `live` against a path-traced
/// `reference` that blotches live in (blurred s px, minus the error blurred 6 s
/// px, so an even bias over a wall does not count), relative to the light there.
pub fn blotch(live: &Plane, reference: &Plane, s: f64) -> f64 {
    let mut e = Plane::new(live.w, live.h);
    for i in 0..e.v.len() {
        e.v[i] = live.v[i] - reference.v[i];
    }
    let (fine, coarse) = (blur(&e, s), blur(&e, 6.0 * s));
    let base = rel_base_s(reference, s);
    fine.v
        .iter()
        .zip(&coarse.v)
        .zip(&base.v)
        .map(|((f, c), b)| (f - c).abs() / b)
        .sum::<f64>()
        / e.v.len().max(1) as f64
}
