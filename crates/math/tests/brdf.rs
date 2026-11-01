//! Oracle checks for the CPU reference BRDF (ADR 0025): analytic values and
//! white-furnace energy integrals.

#![allow(
    clippy::many_single_char_names,
    clippy::manual_midpoint,
    clippy::needless_range_loop
)] // Quadrature reads closest to the textbook with n, v, l, h and indexed channels.

use genos_math::brdf::{
    d_ggx, directional_albedo, eval, fresnel_schlick, g1_smith_ggx, lambert, shade,
    v_smith_ggx_correlated, Cosines, Material, DIELECTRIC_F0,
};
use std::f64::consts::PI;

const ALPHAS: [f64; 5] = [0.05, 0.1, 0.3, 0.6, 1.0];

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// `∫ D(h) cosθh dω = 1`, over θh with a fine midpoint rule in `cos²`-space.
#[test]
fn ggx_projected_area_is_one() {
    for alpha in ALPHAS {
        let n = 200_000;
        let mut sum = 0.0;
        // Substitute u = cos θ, dω = 2π du (isotropic).
        for i in 0..n {
            let u = (f64::from(i) + 0.5) / f64::from(n);
            sum += d_ggx(u, alpha) * u * 2.0 * PI / f64::from(n);
        }
        assert!(close(sum, 1.0, 2e-3), "alpha {alpha}: {sum}");
    }
}

#[test]
fn ggx_peak_is_one_over_pi_alpha_squared() {
    for alpha in ALPHAS {
        assert!(close(d_ggx(1.0, alpha), 1.0 / (PI * alpha * alpha), 1e-9));
    }
    // alpha = 1 is uniform: 1/π everywhere on the hemisphere.
    for mu in [0.1, 0.5, 0.9] {
        assert!(close(d_ggx(mu, 1.0), 1.0 / PI, 1e-12));
    }
    assert_eq!(d_ggx(-0.2, 0.3), 0.0);
}

#[test]
fn schlick_endpoints() {
    assert!(close(fresnel_schlick(DIELECTRIC_F0, 1.0), 0.04, 1e-12));
    assert!(close(fresnel_schlick(DIELECTRIC_F0, 0.0), 1.0, 1e-12));
    assert!(close(fresnel_schlick(0.5, 0.5), 0.5 + 0.5 / 32.0, 1e-12));
}

#[test]
fn smith_known_values() {
    // Normal incidence: G1 = 1, V = 1/4 for any alpha.
    for alpha in ALPHAS {
        assert!(close(g1_smith_ggx(1.0, alpha), 1.0, 1e-12));
        assert!(close(v_smith_ggx_correlated(1.0, 1.0, alpha), 0.25, 1e-12));
    }
    // alpha = 1: G1(μ) = 2μ / (μ + 1).
    assert!(close(g1_smith_ggx(0.5, 1.0), 2.0 / 3.0, 1e-12));
    // alpha = 1, n·l = n·v = 0.5: V = 0.5 / (2 · 0.5 · 1) = 0.5.
    assert!(close(v_smith_ggx_correlated(0.5, 0.5, 1.0), 0.5, 1e-12));
    assert_eq!(v_smith_ggx_correlated(0.0, 0.5, 0.3), 0.0);
}

#[test]
fn lambert_white_furnace_is_one() {
    let m = Material::dielectric([1.0; 3], 1.0);
    assert!(close(lambert(1.0), 1.0 / PI, 1e-15));
    // Pure Lambert part: integrate albedo/π cos over the hemisphere.
    let n = 400;
    let mut sum = 0.0;
    for i in 0..n {
        let t = (f64::from(i) + 0.5) * 0.5 * PI / f64::from(n);
        sum += lambert(1.0) * t.cos() * t.sin() * 2.0 * PI * 0.5 * PI / f64::from(n);
    }
    assert!(close(sum, 1.0, 1e-5), "{sum}");
    assert_eq!(m.diffuse_albedo(), [1.0; 3]);
}

/// Weak white furnace (Heitz 2014): `∫ D(h) G1(v) max(v·h,0) / n·v dω_h = 1`.
#[test]
fn smith_weak_white_furnace_is_one() {
    for alpha in ALPHAS {
        for n_dot_v in [0.2, 0.5, 0.9, 1.0] {
            let sv = (1.0f64 - n_dot_v * n_dot_v).sqrt();
            let v = [sv, 0.0, n_dot_v];
            let (nt, np) = (4000, 256);
            let dt = 0.5 * PI / f64::from(nt);
            let mut sum = 0.0;
            for i in 0..nt {
                // Uniform in θh so narrow lobes are resolved; dω = sinθ dθ dφ.
                let t = (f64::from(i) + 0.5) * dt;
                let (s, u) = t.sin_cos();
                for j in 0..np {
                    let phi = (f64::from(j) + 0.5) * 2.0 * PI / f64::from(np);
                    let h = [s * phi.cos(), s * phi.sin(), u];
                    let vh = (v[0] * h[0] + v[1] * h[1] + v[2] * h[2]).max(0.0);
                    sum += d_ggx(u, alpha) * vh * s;
                }
            }
            sum *= g1_smith_ggx(n_dot_v, alpha) / n_dot_v * (2.0 * PI / f64::from(np)) * dt;
            assert!(close(sum, 1.0, 1e-2), "alpha {alpha} n·v {n_dot_v}: {sum}");
        }
    }
}

#[test]
fn normal_incidence_hand_value() {
    // l = v = n: D = 1/(π α²), V = 1/4, F = F0, so spec = F0 / (4 π α²).
    let c = Cosines {
        n_dot_l: 1.0,
        n_dot_v: 1.0,
        n_dot_h: 1.0,
        v_dot_h: 1.0,
    };
    let gold = Material::metal([1.0, 0.766, 0.336], 0.5);
    let a = gold.alpha();
    let f = eval(&gold, c);
    for k in 0..3 {
        assert!(close(f[k], gold.base_color[k] / (4.0 * PI * a * a), 1e-12));
    }
    let plastic = Material::dielectric([0.5, 0.2, 0.1], 0.5);
    let f = eval(&plastic, c);
    for k in 0..3 {
        let want = 0.96 * plastic.base_color[k] / PI + 0.04 / (4.0 * PI * a * a);
        assert!(close(f[k], want, 1e-12));
    }
}

#[test]
fn reciprocity() {
    let m = Material {
        base_color: [0.8, 0.5, 0.2],
        metallic: 0.4,
        roughness: 0.35,
        emissive: [0.0; 3],
        emissive_strength: 1.0,
    };
    let n = [0.0, 0.0, 1.0];
    let l = [0.6f64, 0.0, 0.8];
    let v = [-0.3f64, 0.4, (1.0f64 - 0.25).sqrt()];
    let a = eval(&m, Cosines::from_dirs(n, l, v));
    let b = eval(&m, Cosines::from_dirs(n, v, l));
    for k in 0..3 {
        assert!(close(a[k], b[k], 1e-12));
    }
}

/// White furnace for full materials. Metals never exceed 1. Dielectrics stay
/// at or under 1 at normal view; off normal the Appendix B `fresnel_mix` gains
/// up to ~30 %, which is bounded here and pinned below rather than hidden
/// (ADR 0025).
#[test]
fn material_white_furnace_conserves_energy() {
    for roughness in [0.3, 0.6, 1.0] {
        for n_dot_v in [0.1, 0.5, 1.0] {
            let a = directional_albedo(&Material::metal([1.0; 3], roughness), n_dot_v, 512, 256)[0];
            assert!(
                a <= 1.0 + 5e-3 && a > 0.25,
                "metal r {roughness} nv {n_dot_v}: {a}"
            );
        }
        for metallic in [0.0, 0.5] {
            for n_dot_v in [0.1, 0.5, 1.0] {
                let mut m = Material::dielectric([1.0; 3], roughness);
                m.metallic = metallic;
                let a = directional_albedo(&m, n_dot_v, 512, 256)[0];
                let cap = if n_dot_v >= 1.0 { 1.0 + 5e-3 } else { 1.3 };
                assert!(
                    a <= cap && a > 0.5,
                    "m {metallic} r {roughness} nv {n_dot_v}: {a}"
                );
            }
        }
    }
    // Single-scatter GGX loses energy only at high roughness.
    let smooth = directional_albedo(&Material::metal([1.0; 3], 0.3), 1.0, 512, 256)[0];
    let rough = directional_albedo(&Material::metal([1.0; 3], 1.0), 1.0, 512, 256)[0];
    assert!(smooth > 0.95, "{smooth}");
    assert!(rough < smooth, "{rough} vs {smooth}");
}

/// Known property of the glTF Appendix B dielectric mix: a white dielectric at
/// grazing view reflects more than it receives (converged value 1.2837 at
/// roughness 0.3, n·v 0.1). GPU shaders match the formula, including this.
#[test]
fn appendix_b_dielectric_grazing_gain_is_pinned() {
    let a = directional_albedo(&Material::dielectric([1.0; 3], 0.3), 0.1, 1024, 1024)[0];
    assert!(close(a, 1.2837, 2e-3), "{a}");
}

#[test]
fn black_metal_and_metal_diffuse_are_dark() {
    let m = Material::metal([0.9, 0.6, 0.3], 0.5);
    assert_eq!(m.diffuse_albedo(), [0.0; 3]);
    assert_eq!(m.f0(), [0.9, 0.6, 0.3]);
    let black = Material::metal([0.0; 3], 0.5);
    // F0 = 0 still reflects at grazing via Schlick, so albedo is small, not zero.
    let a = directional_albedo(&black, 1.0, 256, 128)[0];
    assert!(a < 0.05, "{a}");
}

#[test]
fn emission_adds_and_is_not_reflected() {
    let mut m = Material::dielectric([0.0; 3], 1.0);
    m.emissive = [1.0, 0.5, 0.25];
    m.emissive_strength = 4.0;
    let c = Cosines {
        n_dot_l: -0.5,
        n_dot_v: 1.0,
        n_dot_h: 0.2,
        v_dot_h: 0.6,
    };
    assert_eq!(shade(&m, c, [10.0; 3]), [4.0, 2.0, 1.0]);
}

/// f32 Filament / glTF shade, the same arithmetic as `pbr_brdf.glsl`:
/// GGX `D` from `|n × h|²` (the form fixed in #107) and height-correlated
/// Smith `V` (`0.5 / (gv + gl)`, Filament `V_SmithGGXCorrelated`).
fn emulate_shader(
    base: [f32; 3],
    metallic: f32,
    roughness: f32,
    n: [f32; 3],
    l: [f32; 3],
    v: [f32; 3],
) -> [f32; 3] {
    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }
    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    fn normalize(a: [f32; 3]) -> [f32; 3] {
        let len = dot(a, a).sqrt();
        [a[0] / len, a[1] / len, a[2] / len]
    }
    let n_dot_l = dot(n, l);
    let n_dot_v = dot(n, v);
    if n_dot_l <= 0.0 || n_dot_v <= 0.0 {
        return [0.0; 3];
    }
    let h = normalize(add(l, v));
    let n_dot_h = dot(n, h);
    let v_dot_h = dot(v, h);
    let cx = cross(n, h);
    let n_cross_h_sq = dot(cx, cx);
    let alpha = roughness.clamp(0.0, 1.0);
    let alpha = alpha * alpha;
    let a2 = alpha * alpha;
    let d = if n_dot_h <= 0.0 {
        0.0
    } else {
        let den = n_dot_h * n_dot_h * a2 + n_cross_h_sq;
        a2 / (std::f32::consts::PI * den * den)
    };
    let gv = n_dot_l * (n_dot_v * n_dot_v * (1.0 - a2) + a2).sqrt();
    let gl = n_dot_v * (n_dot_l * n_dot_l * (1.0 - a2) + a2).sqrt();
    let s = gv + gl;
    let vis = if s > 0.0 { 0.5 / s } else { 0.0 };
    let dv = d * vis;
    let m = metallic.clamp(0.0, 1.0);
    let fresnel = |f0: f32| {
        let k = 1.0 - v_dot_h.clamp(0.0, 1.0);
        let k2 = k * k;
        f0 + (1.0 - f0) * (k2 * k2 * k)
    };
    let fd = fresnel(0.04);
    std::array::from_fn(|channel| {
        let base = base[channel];
        let dielectric = (1.0 - fd) * (base / std::f32::consts::PI) + fd * dv;
        let metal = fresnel(base) * dv;
        dielectric * (1.0 - m) + metal * m
    })
}

fn rel_err(got: f64, want: f64) -> f64 {
    let scale = want.abs().max(1.0e-6);
    (got - want).abs() / scale
}

/// Stage 3 gate, before the direct pass: the shader's f32 lobe against the
/// f64 reference on full normal / view / light vectors.
#[test]
fn shader_form_matches_the_reference_on_vector_cases() {
    let normals = [
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        normalize64([0.2, 0.9, 0.3]),
        normalize64([-0.4, 0.4, 0.8]),
    ];
    let views = [
        [0.0, 1.0, 0.0],
        [0.0, 0.6, 0.8],
        normalize64([0.3, 0.2, 0.9]),
        normalize64([-0.5, 0.7, 0.2]),
    ];
    let lights = [
        [0.0, 1.0, 0.0],
        [0.6, 0.8, 0.0],
        normalize64([0.2, 0.5, 0.8]),
        normalize64([-0.7, 0.3, 0.4]),
    ];
    let roughness = [0.15_f32, 0.3, 0.55, 1.0];
    let metallic = [0.0_f32, 0.35, 1.0];
    let bases = [[1.0_f32, 1.0, 1.0], [0.8, 0.1, 0.05], [0.04, 0.6, 0.3]];
    let mut worst = 0.0_f64;
    let mut seen = 0u32;
    for n in normals {
        for v in views {
            for l in lights {
                let ndl = dot64(n, l);
                let ndv = dot64(n, v);
                if ndl <= 0.05 || ndv <= 0.05 {
                    continue;
                }
                for &rough in &roughness {
                    for &metal in &metallic {
                        for base in bases {
                            let emu = emulate_shader(
                                base,
                                metal,
                                rough,
                                n.map(|c| c as f32),
                                l.map(|c| c as f32),
                                v.map(|c| c as f32),
                            );
                            let reference = eval(
                                &Material {
                                    base_color: base.map(f64::from),
                                    metallic: f64::from(metal),
                                    roughness: f64::from(rough),
                                    emissive: [0.0; 3],
                                    emissive_strength: 1.0,
                                },
                                Cosines::from_dirs(n, l, v),
                            );
                            for channel in 0..3 {
                                worst =
                                    worst.max(rel_err(f64::from(emu[channel]), reference[channel]));
                            }
                            seen += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(seen > 100, "the case grid collapsed: {seen}");
    assert!(
        worst <= 1.0e-4,
        "shader form vs reference relative error {worst} over {seen} cases"
    );
}

fn dot64(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize64(a: [f64; 3]) -> [f64; 3] {
    let len = dot64(a, a).sqrt();
    [a[0] / len, a[1] / len, a[2] / len]
}

/// Closed form: a white metal at roughness 1 seen head-on has D = 1/π and
/// V = 1/(2(1 + n·l)), so its albedo is `∫₀¹ u/(1+u) du = 1 − ln 2`.
#[test]
fn rough_white_metal_albedo_is_one_minus_ln2() {
    let a = directional_albedo(&Material::metal([1.0; 3], 1.0), 1.0, 1024, 64)[0];
    assert!(close(a, 1.0 - std::f64::consts::LN_2, 1e-5), "{a}");
}
