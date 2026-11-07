//! Vectors, axis-aligned boxes, and ray casts against them.

use crate::rng::Rng;

/// Three `f64` components. World units are metres; `y` is up.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[expect(
    clippy::should_implement_trait,
    reason = "by-value helpers named like the shader code they mirror; no operator overloading in the reference"
)]
impl V3 {
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    #[must_use]
    pub fn splat(v: f64) -> Self {
        Self::new(v, v, v)
    }

    #[must_use]
    pub fn axis(self, i: usize) -> f64 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    pub fn set_axis(&mut self, i: usize, v: f64) {
        match i {
            0 => self.x = v,
            1 => self.y = v,
            _ => self.z = v,
        }
    }

    #[must_use]
    pub fn unit_axis(i: usize, sign: f64) -> Self {
        let mut v = Self::default();
        v.set_axis(i, sign);
        v
    }

    #[must_use]
    pub fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    #[must_use]
    pub fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    #[must_use]
    pub fn scale(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }

    #[must_use]
    pub fn mul(self, o: Self) -> Self {
        Self::new(self.x * o.x, self.y * o.y, self.z * o.z)
    }

    #[must_use]
    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    #[must_use]
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    #[must_use]
    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    #[must_use]
    pub fn normalize(self) -> Self {
        let l = self.length();
        if l > 0.0 {
            self.scale(1.0 / l)
        } else {
            self
        }
    }

    /// Index of the component with the largest magnitude.
    #[must_use]
    pub fn dominant_axis(self) -> usize {
        let (ax, ay, az) = (self.x.abs(), self.y.abs(), self.z.abs());
        if ax >= ay && ax >= az {
            0
        } else if ay >= az {
            1
        } else {
            2
        }
    }
}

/// Axis-aligned box. Opaque, Lambertian, one albedo for the scene.
#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: V3,
    pub max: V3,
}

impl Aabb {
    #[must_use]
    pub const fn new(min: V3, max: V3) -> Self {
        Self { min, max }
    }

    /// Strictly inside, `eps` away from every face.
    #[must_use]
    pub fn contains(&self, p: V3, eps: f64) -> bool {
        (0..3).all(|i| p.axis(i) > self.min.axis(i) + eps && p.axis(i) < self.max.axis(i) - eps)
    }

    /// Entry distance and the face normal there, for `t` in `(t_min, t_max)`.
    #[must_use]
    pub fn hit(&self, o: V3, d: V3, t_min: f64, t_max: f64) -> Option<(f64, V3)> {
        let mut t0 = t_min;
        let mut t1 = t_max;
        let mut normal = V3::default();
        for i in 0..3 {
            let (oi, di) = (o.axis(i), d.axis(i));
            let (lo, hi) = (self.min.axis(i), self.max.axis(i));
            if di.abs() < 1e-12 {
                if oi < lo || oi > hi {
                    return None;
                }
                continue;
            }
            let inv = 1.0 / di;
            let (mut a, mut b) = ((lo - oi) * inv, (hi - oi) * inv);
            let mut sign = -1.0;
            if a > b {
                std::mem::swap(&mut a, &mut b);
                sign = 1.0;
            }
            if a > t0 {
                t0 = a;
                normal = V3::unit_axis(i, sign);
            }
            t1 = t1.min(b);
            if t0 > t1 {
                return None;
            }
        }
        if t0 > t_min && normal != V3::default() {
            Some((t0, normal))
        } else {
            None
        }
    }
}

/// A ray hit: distance, point and outward normal.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub t: f64,
    pub p: V3,
    pub n: V3,
}

/// Point lamp: position and radiant intensity (W/sr, monochrome).
#[derive(Clone, Copy, Debug)]
pub struct Lamp {
    pub pos: V3,
    pub intensity: f64,
}

/// Boxes and lamps. Every surface has `albedo`.
#[derive(Clone, Debug)]
pub struct World {
    pub boxes: Vec<Aabb>,
    pub lamps: Vec<Lamp>,
    pub albedo: f64,
}

/// Offset used to leave a surface before casting.
pub const EPS: f64 = 1e-4;

impl World {
    /// Nearest hit along `d` (unit), or `None`. Counts one cast.
    #[must_use]
    pub fn trace(&self, o: V3, d: V3, t_max: f64) -> Option<Hit> {
        let mut best: Option<(f64, V3)> = None;
        let mut limit = t_max;
        for b in &self.boxes {
            if let Some((t, n)) = b.hit(o, d, EPS, limit) {
                limit = t;
                best = Some((t, n));
            }
        }
        best.map(|(t, n)| Hit {
            t,
            p: o.add(d.scale(t)),
            n,
        })
    }

    /// Whether `a` sees `b` (both points; `b` excluded).
    #[must_use]
    pub fn visible(&self, a: V3, b: V3) -> bool {
        let d = b.sub(a);
        let len = d.length();
        if len < EPS {
            return true;
        }
        let dir = d.scale(1.0 / len);
        self.boxes
            .iter()
            .all(|bx| bx.hit(a, dir, EPS, len - EPS).is_none())
    }

    /// Direct irradiance at `p` with normal `n` from every lamp that is on.
    #[must_use]
    pub fn direct(&self, p: V3, n: V3, lamps_on: bool) -> f64 {
        if !lamps_on {
            return 0.0;
        }
        let start = p.add(n.scale(EPS * 4.0));
        let mut e = 0.0;
        for lamp in &self.lamps {
            let to = lamp.pos.sub(p);
            let d2 = to.dot(to).max(1e-4);
            let l = to.scale(1.0 / d2.sqrt());
            let cos = n.dot(l);
            if cos > 0.0 && self.visible(start, lamp.pos) {
                e += lamp.intensity * cos / d2;
            }
        }
        e
    }
}

/// Orthonormal basis `(t, b)` around unit `n`.
#[must_use]
pub fn basis(n: V3) -> (V3, V3) {
    let helper = if n.x.abs() > 0.9 {
        V3::new(0.0, 1.0, 0.0)
    } else {
        V3::new(1.0, 0.0, 0.0)
    };
    let t = helper.cross(n).normalize();
    let b = n.cross(t);
    (t, b)
}

/// Cosine-weighted direction about `n`; pdf `cos / pi`.
pub fn cosine_dir(n: V3, rng: &mut Rng) -> V3 {
    let (u1, u2) = (rng.next_f64(), rng.next_f64());
    let r = u1.sqrt();
    let phi = std::f64::consts::TAU * u2;
    let (t, b) = basis(n);
    t.scale(r * phi.cos())
        .add(b.scale(r * phi.sin()))
        .add(n.scale((1.0 - u1).max(0.0).sqrt()))
        .normalize()
}

/// Uniform direction on the hemisphere about `n`; pdf `1 / (2 pi)`.
pub fn uniform_hemi(n: V3, rng: &mut Rng) -> V3 {
    let (u1, u2) = (rng.next_f64(), rng.next_f64());
    let z = u1;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = std::f64::consts::TAU * u2;
    let (t, b) = basis(n);
    t.scale(r * phi.cos())
        .add(b.scale(r * phi.sin()))
        .add(n.scale(z))
        .normalize()
}

/// `floor(x)` as an index, clamped to `±2^29`. Bisects on exact `f64::from(i32)`
/// values so the crate needs no `as` cast.
#[must_use]
pub fn floor_i32(x: f64) -> i32 {
    const LIM: i32 = 1 << 29;
    if x.is_nan() || x < f64::from(-LIM) {
        return -LIM;
    }
    if x >= f64::from(LIM) {
        return LIM;
    }
    // Invariant: f64(lo) <= x < f64(hi); the span stays below 2^31.
    let (mut lo, mut hi) = (-LIM, LIM);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if f64::from(mid) <= x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// `round(x)` as an index.
#[must_use]
pub fn round_i32(x: f64) -> i32 {
    floor_i32(x + 0.5)
}

/// `floor(x)` as a count, 0 below zero.
#[must_use]
pub fn floor_usize(x: f64) -> usize {
    usize::try_from(floor_i32(x).max(0)).unwrap_or(0)
}

/// `usize` as `f64` without the precision lint: counts here stay below 2^32.
#[must_use]
pub fn fl(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}
