//! CPU fog reference for stage F0 of ADR 0026.
//!
//! A single-scattering path tracer for participating media (homogeneous slab and
//! exponential height fog, Henyey-Greenstein phase, sun and point lamps, box
//! occluders) plus the closed forms it is graded against. Everything here is
//! stateless and seeded, so the same call returns the same number on every run
//! and a light change shows up on the very next frame.
//!
//! See `docs/systems/rendering/fog/plan.md`.

pub mod froxel;

use std::f64::consts::PI;
use std::ops::{Add, Mul, Sub};

/// A 3-vector in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V3 {
    /// X.
    pub x: f64,
    /// Y, up.
    pub y: f64,
    /// Z.
    pub z: f64,
}

impl V3 {
    /// Builds a vector.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Dot product.
    #[must_use]
    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Length.
    #[must_use]
    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// Unit vector in the same direction.
    #[must_use]
    pub fn norm(self) -> Self {
        self * (1.0 / self.len())
    }
}

impl Add for V3 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for V3 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for V3 {
    type Output = Self;
    fn mul(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

/// The participating medium.
#[derive(Clone, Copy, Debug)]
pub enum Medium {
    /// Constant extinction below `top` (use `f64::INFINITY` for unbounded).
    Homogeneous {
        /// Extinction per metre.
        sigma_t: f64,
        /// Scattering over extinction.
        albedo: f64,
        /// Height of the fog top.
        top: f64,
    },
    /// Extinction `sigma0 * exp(-y / scale_height)`.
    Height {
        /// Extinction per metre at y = 0.
        sigma0: f64,
        /// Scale height in metres.
        scale_height: f64,
        /// Scattering over extinction.
        albedo: f64,
    },
}

impl Medium {
    /// Extinction at `p`.
    #[must_use]
    pub fn sigma_t(&self, p: V3) -> f64 {
        match *self {
            Self::Homogeneous { sigma_t, top, .. } => {
                if p.y < top {
                    sigma_t
                } else {
                    0.0
                }
            }
            Self::Height {
                sigma0,
                scale_height,
                ..
            } => sigma0 * (-p.y / scale_height).exp(),
        }
    }

    /// Scattering over extinction.
    #[must_use]
    pub fn albedo(&self) -> f64 {
        match *self {
            Self::Homogeneous { albedo, .. } | Self::Height { albedo, .. } => albedo,
        }
    }

    /// Closed-form optical depth from `o` along unit `d` for distance `t`.
    #[must_use]
    pub fn optical_depth(&self, o: V3, d: V3, t: f64) -> f64 {
        // Rounding can hand in -1e-16 at the start of a ray.
        let t = t.max(0.0);
        match *self {
            Self::Homogeneous { sigma_t, top, .. } => {
                let inside = if d.y.abs() < 1e-12 {
                    if o.y < top {
                        t
                    } else {
                        0.0
                    }
                } else {
                    let cross = ((top - o.y) / d.y).clamp(0.0, t);
                    if d.y > 0.0 {
                        cross
                    } else {
                        t - cross
                    }
                };
                sigma_t * inside
            }
            Self::Height {
                sigma0,
                scale_height: h,
                ..
            } => {
                if d.y.abs() < 1e-12 {
                    sigma0 * (-o.y / h).exp() * t
                } else {
                    sigma0 * h / d.y * ((-o.y / h).exp() - (-(o.y + t * d.y) / h).exp())
                }
            }
        }
    }

    /// Beer-Lambert transmittance from the closed-form optical depth.
    #[must_use]
    pub fn transmittance(&self, o: V3, d: V3, t: f64) -> f64 {
        (-self.optical_depth(o, d, t)).exp()
    }

    /// An upper bound on extinction along the segment.
    fn majorant(&self, o: V3, d: V3, t: f64) -> f64 {
        match *self {
            Self::Homogeneous { sigma_t, .. } => sigma_t,
            Self::Height { .. } => {
                let low = if (o + d * t).y < o.y { o + d * t } else { o };
                self.sigma_t(low)
            }
        }
    }

    /// How far a shadow ray towards `to_light` has to run before the medium ends.
    fn leg(&self, p: V3, to_light: V3) -> f64 {
        let top = match *self {
            Self::Homogeneous { top, .. } => top,
            Self::Height { scale_height, .. } => p.y + 40.0 * scale_height,
        };
        if to_light.y > 1e-6 && top.is_finite() {
            ((top - p.y) / to_light.y).max(0.0)
        } else {
            1e4
        }
    }
}

/// Henyey-Greenstein phase function, normalised over the sphere.
#[must_use]
pub fn hg(g: f64, cos_theta: f64) -> f64 {
    let denom = 1.0 + g * g - 2.0 * g * cos_theta;
    (1.0 - g * g) / (4.0 * PI * denom * denom.sqrt())
}

/// A light.
#[derive(Clone, Copy, Debug)]
pub enum Light {
    /// Directional light.
    Sun {
        /// Unit direction towards the sun.
        to_light: V3,
        /// Irradiance at the top of the fog.
        irradiance: f64,
    },
    /// Point lamp.
    Point {
        /// Position.
        pos: V3,
        /// Radiant intensity.
        intensity: f64,
        /// Switch state.
        on: bool,
    },
}

/// An opaque, black axis-aligned box.
#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    /// Minimum corner.
    pub min: V3,
    /// Maximum corner.
    pub max: V3,
}

impl Aabb {
    /// True when the ray from `o` along `d` enters the box before `dist`.
    #[must_use]
    pub fn blocks(&self, o: V3, d: V3, dist: f64) -> bool {
        let mut near = 0.0_f64;
        let mut far = dist;
        for (oa, da, lo, hi) in [
            (o.x, d.x, self.min.x, self.max.x),
            (o.y, d.y, self.min.y, self.max.y),
            (o.z, d.z, self.min.z, self.max.z),
        ] {
            if da.abs() < 1e-15 {
                if oa < lo || oa > hi {
                    return false;
                }
            } else {
                let (a, b) = ((lo - oa) / da, (hi - oa) / da);
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if near > far {
                    return false;
                }
            }
        }
        true
    }
}

/// A camera ray that ends on a surface of radiance `background` at `tmax`.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    /// Origin.
    pub o: V3,
    /// Unit direction.
    pub d: V3,
    /// Distance to the background surface.
    pub tmax: f64,
}

/// A fog scene.
#[derive(Clone, Debug)]
pub struct Scene {
    /// Medium.
    pub medium: Medium,
    /// Henyey-Greenstein asymmetry.
    pub g: f64,
    /// Lights.
    pub lights: Vec<Light>,
    /// Occluders.
    pub occluders: Vec<Aabb>,
    /// Radiance of the surface every camera ray ends on.
    pub background: f64,
}

/// A Monte Carlo result.
#[derive(Clone, Copy, Debug)]
pub struct Estimate {
    /// Sample mean.
    pub mean: f64,
    /// Standard error of the mean.
    pub stderr: f64,
}

/// Deterministic splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        f64::from(u32::try_from(z >> 32).unwrap_or(0)) / 4_294_967_296.0
    }

    /// Exponential free path for rate `rate`.
    fn free_path(&mut self, rate: f64) -> f64 {
        -(1.0 - self.next()).ln() / rate
    }
}

struct Leg {
    dir: V3,
    occlude: f64,
    medium: f64,
    radiance: f64,
}

impl Scene {
    fn leg(&self, p: V3, light: &Light) -> Option<Leg> {
        match *light {
            Light::Sun {
                to_light,
                irradiance,
            } => Some(Leg {
                dir: to_light,
                occlude: f64::INFINITY,
                medium: self.medium.leg(p, to_light),
                radiance: irradiance,
            }),
            Light::Point { pos, intensity, on } => {
                if !on {
                    return None;
                }
                let v = pos - p;
                let r = v.len();
                Some(Leg {
                    dir: v * (1.0 / r),
                    occlude: r,
                    medium: r,
                    radiance: intensity / (r * r),
                })
            }
        }
    }

    fn visible(&self, p: V3, d: V3, dist: f64) -> bool {
        !self.occluders.iter().any(|b| b.blocks(p, d, dist))
    }

    /// Direct in-scattered radiance towards the camera at `p` per unit scattering,
    /// with light transmittance from `transmit`.
    fn direct(&self, p: V3, view: V3, mut transmit: impl FnMut(V3, V3, f64) -> f64) -> f64 {
        self.lights
            .iter()
            .filter_map(|l| self.leg(p, l))
            .filter(|leg| self.visible(p, leg.dir, leg.occlude))
            .map(|leg| {
                hg(self.g, leg.dir.dot(view)) * leg.radiance * transmit(p, leg.dir, leg.medium)
            })
            .sum()
    }

    /// Ratio-tracking transmittance estimate (unbiased, path-traced).
    fn ratio_tracking(&self, o: V3, d: V3, dist: f64, rng: &mut Rng) -> f64 {
        // Twice the bound: with the exact bound a homogeneous medium gives 0-or-1
        // weights, while 2x gives Poisson-distributed halvings and far less noise.
        let maj = 2.0 * self.medium.majorant(o, d, dist);
        if maj <= 0.0 {
            return 1.0;
        }
        let mut t = 0.0;
        let mut tr = 1.0;
        loop {
            t += rng.free_path(maj);
            if t >= dist {
                return tr;
            }
            tr *= 1.0 - self.medium.sigma_t(o + d * t) / maj;
        }
    }

    /// Path-traced transmittance along a ray, by ratio tracking.
    #[must_use]
    pub fn trace_transmittance(&self, o: V3, d: V3, dist: f64, n: u32, seed: u64) -> Estimate {
        let mut rng = Rng(seed);
        stats((0..n).map(|_| self.ratio_tracking(o, d, dist, &mut rng)), n)
    }

    /// Path-traced radiance along `ray`: delta tracking to a real collision, then
    /// next-event estimation to every light with ratio-tracked transmittance.
    #[must_use]
    pub fn trace_delta(&self, ray: Ray, n: u32, seed: u64) -> Estimate {
        let mut rng = Rng(seed);
        let maj = self.medium.majorant(ray.o, ray.d, ray.tmax);
        let albedo = self.medium.albedo();
        let samples = (0..n).map(|_| {
            if maj <= 0.0 {
                return self.background;
            }
            let mut t = 0.0;
            loop {
                t += rng.free_path(maj);
                if t >= ray.tmax {
                    return self.background;
                }
                let p = ray.o + ray.d * t;
                if rng.next() * maj < self.medium.sigma_t(p) {
                    return albedo
                        * self.direct(p, ray.d, |q, dir, dist| {
                            self.ratio_tracking(q, dir, dist, &mut rng)
                        });
                }
            }
        });
        stats(samples, n)
    }

    /// Path-traced radiance along `ray` with equiangular sampling about the point
    /// lamp at index `lamp` (Kulla and Fajardo 2012) and closed-form transmittance.
    #[must_use]
    pub fn trace_equiangular(&self, ray: Ray, lamp: usize, n: u32, seed: u64) -> Estimate {
        let mut rng = Rng(seed);
        let tail = self.background * self.medium.transmittance(ray.o, ray.d, ray.tmax);
        let Some(&Light::Point { pos, on: true, .. }) = self.lights.get(lamp) else {
            return Estimate {
                mean: tail,
                stderr: 0.0,
            };
        };
        let delta = (pos - ray.o).dot(ray.d);
        let h = (ray.o + ray.d * delta - pos).len().max(1e-9);
        let th_a = (-delta).atan2(h);
        let th_b = (ray.tmax - delta).atan2(h);
        let samples = (0..n).map(|_| {
            let th = th_a + rng.next() * (th_b - th_a);
            let t = delta + h * th.tan();
            let pdf = h / ((th_b - th_a) * (h * h + (t - delta) * (t - delta)));
            let p = ray.o + ray.d * t;
            let sigma_s = self.medium.albedo() * self.medium.sigma_t(p);
            let cam = self.medium.transmittance(ray.o, ray.d, t);
            tail + sigma_s
                * cam
                * self.direct(p, ray.d, |q, dir, dist| {
                    self.medium.transmittance(q, dir, dist)
                })
                / pdf
        });
        stats(samples, n)
    }

    /// Deterministic reference: closed-form optical depths and visibility,
    /// integrated along the ray by composite Simpson on `steps` intervals
    /// (equiangular substitution about point lamps keeps it smooth).
    #[must_use]
    pub fn reference(&self, ray: Ray, steps: u32) -> f64 {
        let tail = self.background * self.medium.transmittance(ray.o, ray.d, ray.tmax);
        let albedo = self.medium.albedo();
        let integrand = |t: f64| {
            let p = ray.o + ray.d * t;
            albedo
                * self.medium.sigma_t(p)
                * self.medium.transmittance(ray.o, ray.d, t)
                * self.direct(p, ray.d, |q, dir, dist| {
                    self.medium.transmittance(q, dir, dist)
                })
        };
        let steps = steps + steps % 2;
        let lamp = self.lights.iter().find_map(|l| match *l {
            Light::Point { pos, on: true, .. } => Some(pos),
            Light::Point { .. } | Light::Sun { .. } => None,
        });
        let body = if let Some(pos) = lamp {
            let delta = (pos - ray.o).dot(ray.d);
            let h = (ray.o + ray.d * delta - pos).len().max(1e-9);
            let (a, b) = ((-delta).atan2(h), (ray.tmax - delta).atan2(h));
            simpson(a, b, steps, |th| {
                let t = delta + h * th.tan();
                integrand(t) * (h * h + (t - delta) * (t - delta)) / h
            })
        } else {
            simpson(0.0, ray.tmax, steps, integrand)
        };
        tail + body
    }
}

fn simpson(lo: f64, hi: f64, steps: u32, func: impl Fn(f64) -> f64) -> f64 {
    let step = (hi - lo) / f64::from(steps);
    let mut sum = func(lo) + func(hi);
    for i in 1..steps {
        let weight = if i % 2 == 1 { 4.0 } else { 2.0 };
        sum += weight * func(lo + step * f64::from(i));
    }
    sum * step / 3.0
}

fn stats(samples: impl Iterator<Item = f64>, n: u32) -> Estimate {
    let (mut s, mut s2) = (0.0, 0.0);
    for x in samples {
        s += x;
        s2 += x * x;
    }
    let nf = f64::from(n.max(1));
    let mean = s / nf;
    let var = (s2 / nf - mean * mean).max(0.0);
    Estimate {
        mean,
        stderr: (var / nf).sqrt(),
    }
}

/// Relative error with a floor so near-black values do not blow up.
#[must_use]
pub fn rel_err(got: f64, want: f64) -> f64 {
    (got - want).abs() / want.abs().max(1e-6)
}
