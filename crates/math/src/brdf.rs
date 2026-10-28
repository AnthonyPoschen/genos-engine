//! CPU reference for the glTF 2.0 metallic-roughness BRDF (Appendix B).
//!
//! This is the oracle GPU shaders are compared against (ADR 0025). It is `f64`,
//! scalar, and has no lookup tables. Directions are given as cosines against
//! the shading normal, so callers do not need a frame.
//!
//! - Normal distribution: GGX (Trowbridge–Reitz), `alpha = roughness²`.
//! - Visibility: height-correlated Smith GGX, `V = G / (4 n·l n·v)`.
//! - Fresnel: Schlick, `F0 = 0.04` for dielectrics and base colour for metals.
//! - Diffuse: Lambert, `c_diff / π`, with `c_diff = base × (1 − metallic)`.
//! - Emission: `emissive factor × emissive strength`, added, not reflected.

use std::f64::consts::PI;

/// Dielectric reflectance at normal incidence (IOR 1.5), from glTF Appendix B.
pub const DIELECTRIC_F0: f64 = 0.04;

/// One glTF metallic-roughness material, linear values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub base_color: [f64; 3],
    pub metallic: f64,
    pub roughness: f64,
    pub emissive: [f64; 3],
    pub emissive_strength: f64,
}

impl Material {
    /// A dielectric with no emission.
    #[must_use]
    pub fn dielectric(base_color: [f64; 3], roughness: f64) -> Self {
        Self {
            base_color,
            metallic: 0.0,
            roughness,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
        }
    }

    /// A metal with no emission.
    #[must_use]
    pub fn metal(base_color: [f64; 3], roughness: f64) -> Self {
        Self {
            base_color,
            metallic: 1.0,
            roughness,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
        }
    }

    /// `alpha = roughness²`, the GGX width.
    #[must_use]
    pub fn alpha(&self) -> f64 {
        let r = self.roughness.clamp(0.0, 1.0);
        r * r
    }

    /// Emitted radiance, `emissive × KHR_materials_emissive_strength`.
    #[must_use]
    pub fn emission(&self) -> [f64; 3] {
        self.emissive.map(|e| e * self.emissive_strength)
    }

    /// Lambert albedo `base × (1 − metallic)` (ADR 0014).
    #[must_use]
    pub fn diffuse_albedo(&self) -> [f64; 3] {
        let m = self.metallic.clamp(0.0, 1.0);
        self.base_color.map(|c| c * (1.0 - m))
    }

    /// Reflectance at normal incidence, `mix(0.04, base, metallic)`.
    #[must_use]
    pub fn f0(&self) -> [f64; 3] {
        let m = self.metallic.clamp(0.0, 1.0);
        self.base_color.map(|c| DIELECTRIC_F0 * (1.0 - m) + c * m)
    }
}

/// GGX normal distribution `D(h)`. Its projected integral `∫ D cosθh dω` is 1.
#[must_use]
pub fn d_ggx(n_dot_h: f64, alpha: f64) -> f64 {
    if n_dot_h <= 0.0 {
        return 0.0;
    }
    let a2 = alpha * alpha;
    let den = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (PI * den * den)
}

/// Smith GGX masking `G1` for one direction.
#[must_use]
pub fn g1_smith_ggx(n_dot_x: f64, alpha: f64) -> f64 {
    if n_dot_x <= 0.0 {
        return 0.0;
    }
    let a2 = alpha * alpha;
    2.0 * n_dot_x / (n_dot_x + (a2 + (1.0 - a2) * n_dot_x * n_dot_x).sqrt())
}

/// Height-correlated Smith GGX visibility `V = G2 / (4 n·l n·v)` (glTF Appendix B).
#[must_use]
pub fn v_smith_ggx_correlated(n_dot_l: f64, n_dot_v: f64, alpha: f64) -> f64 {
    if n_dot_l <= 0.0 || n_dot_v <= 0.0 {
        return 0.0;
    }
    let a2 = alpha * alpha;
    let gv = n_dot_l * (n_dot_v * n_dot_v * (1.0 - a2) + a2).sqrt();
    let gl = n_dot_v * (n_dot_l * n_dot_l * (1.0 - a2) + a2).sqrt();
    let s = gv + gl;
    if s > 0.0 {
        0.5 / s
    } else {
        0.0
    }
}

/// Schlick Fresnel `F0 + (1 − F0)(1 − v·h)⁵`.
#[must_use]
pub fn fresnel_schlick(f0: f64, v_dot_h: f64) -> f64 {
    let k = (1.0 - v_dot_h.clamp(0.0, 1.0)).powi(5);
    f0 + (1.0 - f0) * k
}

/// Lambert BRDF `albedo / π`.
#[must_use]
pub fn lambert(albedo: f64) -> f64 {
    albedo / PI
}

/// The cosines one BRDF evaluation needs, all against the shading normal
/// except `v_dot_h` (view against half vector).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cosines {
    pub n_dot_l: f64,
    pub n_dot_v: f64,
    pub n_dot_h: f64,
    pub v_dot_h: f64,
}

impl Cosines {
    /// Cosines from unit normal, light and view vectors.
    #[must_use]
    pub fn from_dirs(n: [f64; 3], l: [f64; 3], v: [f64; 3]) -> Self {
        let h = normalize(add(l, v));
        Self {
            n_dot_l: dot(n, l),
            n_dot_v: dot(n, v),
            n_dot_h: dot(n, h),
            v_dot_h: dot(v, h),
        }
    }
}

/// The glTF Appendix B BRDF `f(l, v)` per channel, without the `n·l` factor.
///
/// Dielectric: `mix(diffuse, specular, F(0.04))`, i.e. `(1 − F)·c/π + F·D·V`.
/// Metal: `F(base)·D·V`. Mixed by `metallic`.
#[must_use]
pub fn eval(m: &Material, c: Cosines) -> [f64; 3] {
    if c.n_dot_l <= 0.0 || c.n_dot_v <= 0.0 {
        return [0.0; 3];
    }
    let alpha = m.alpha();
    let dv = d_ggx(c.n_dot_h, alpha) * v_smith_ggx_correlated(c.n_dot_l, c.n_dot_v, alpha);
    let metallic = m.metallic.clamp(0.0, 1.0);
    let fd = fresnel_schlick(DIELECTRIC_F0, c.v_dot_h);
    m.base_color.map(|base| {
        let dielectric = (1.0 - fd) * lambert(base) + fd * dv;
        let metal = fresnel_schlick(base, c.v_dot_h) * dv;
        dielectric * (1.0 - metallic) + metal * metallic
    })
}

/// Outgoing radiance from one light of radiance `li` arriving along `l`:
/// `emission + f(l, v) · li · n·l`.
#[must_use]
pub fn shade(m: &Material, c: Cosines, li: [f64; 3]) -> [f64; 3] {
    let f = eval(m, c);
    let e = m.emission();
    let nl = c.n_dot_l.max(0.0);
    [
        e[0] + f[0] * li[0] * nl,
        e[1] + f[1] * li[1] * nl,
        e[2] + f[2] * li[2] * nl,
    ]
}

/// Directional albedo `∫ f(l, v) n·l dω_l` for a view at `n_dot_v`, by midpoint
/// quadrature over the hemisphere (`n_theta × n_phi` cells). This is the white
/// furnace: lit by uniform radiance 1, the reflected radiance equals this value.
#[must_use]
#[allow(clippy::many_single_char_names)] // n, v, l, f are the textbook names.
pub fn directional_albedo(m: &Material, n_dot_v: f64, n_theta: u32, n_phi: u32) -> [f64; 3] {
    let n = [0.0, 0.0, 1.0];
    let sin_v = (1.0 - n_dot_v * n_dot_v).max(0.0).sqrt();
    let v = [sin_v, 0.0, n_dot_v];
    let d_theta = 0.5 * PI / f64::from(n_theta);
    let d_phi = 2.0 * PI / f64::from(n_phi);
    let mut sum = [0.0; 3];
    for i in 0..n_theta {
        let theta = (f64::from(i) + 0.5) * d_theta;
        let (st, ct) = theta.sin_cos();
        for j in 0..n_phi {
            let phi = (f64::from(j) + 0.5) * d_phi;
            let l = [st * phi.cos(), st * phi.sin(), ct];
            let f = eval(m, Cosines::from_dirs(n, l, v));
            let w = ct * st * d_theta * d_phi;
            for k in 0..3 {
                sum[k] += f[k] * w;
            }
        }
    }
    sum
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn normalize(a: [f64; 3]) -> [f64; 3] {
    let len = dot(a, a).sqrt();
    if len > 0.0 {
        a.map(|x| x / len)
    } else {
        [0.0, 0.0, 1.0]
    }
}
