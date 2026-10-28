//! Stage F0 fog reference (ADR 0026): the CPU path tracer against closed forms
//! on the five scenes of `docs/systems/rendering/fog/plan.md`.
//!
//! CPU only and seeded, so it runs headless anywhere. Each scene prints
//! `fog_ref_err_near` (worst relative error against the closed form) and the
//! renderer row stays `NOT-IMPLEMENTED` until stage F2.

use std::f64::consts::PI;

use genos_fog_ref::{hg, rel_err, Aabb, Light, Medium, Ray, Scene, V3};

/// The F0 gate: path tracer within 1% of the closed form.
const GATE: f64 = 0.01;
const N: u32 = 400_000;
const STEPS: u32 = 4_000;

fn report(scene: &str, err: f64) {
    println!("fog_ref_err_near {scene:<28} {err:.5}  renderer NOT-IMPLEMENTED");
    assert!(err <= GATE, "{scene}: fog_ref_err_near {err} > {GATE}");
}

fn check(name: &str, got: f64, stderr: f64, want: f64) -> f64 {
    let err = rel_err(got, want);
    println!("  {name:<40} got {got:.6} ± {stderr:.6} want {want:.6} err {err:.5}");
    err
}

fn horizontal(o: V3, yaw_deg: f64, tmax: f64) -> Ray {
    let a = yaw_deg.to_radians();
    Ray {
        o,
        d: V3::new(a.cos(), 0.0, a.sin()),
        tmax,
    }
}

#[test]
fn phase_function_integrates_to_one() {
    for g in [-0.5, 0.0, 0.3, 0.6, 0.9] {
        let steps = 200_000_u32;
        let dc = 2.0 / f64::from(steps);
        let total: f64 = (0..steps)
            .map(|i| 2.0 * PI * hg(g, -1.0 + dc * (f64::from(i) + 0.5)) * dc)
            .sum();
        assert!((total - 1.0).abs() < 1e-3, "HG g={g} integrates to {total}");
    }
}

/// Scene 1: homogeneous fog slab, slanted sun, horizontal rays. The sun leg is
/// constant, so `L = Lbg e^{-σt D} + σs p(θ) E e^{-σt Ls} (1 - e^{-σt D}) / σt`.
#[test]
fn scene1_homogeneous_sun() {
    let (sigma_t, albedo, top, g, e, bg) = (0.2, 0.8, 5.0, 0.6, 3.0, 0.5);
    let sun = V3::new(0.5, 1.0, 0.0).norm();
    let scene = Scene {
        medium: Medium::Homogeneous {
            sigma_t,
            albedo,
            top,
        },
        g,
        lights: vec![Light::Sun {
            to_light: sun,
            irradiance: e,
        }],
        occluders: vec![],
        background: bg,
    };
    let mut worst = 0.0_f64;
    for (i, yaw) in [0.0, 60.0, 180.0].into_iter().enumerate() {
        let ray = horizontal(V3::new(0.0, 1.0, 0.0), yaw, 10.0);
        let d = ray.tmax;
        let ls = (top - ray.o.y) / sun.y;
        let want = bg * (-sigma_t * d).exp()
            + albedo
                * sigma_t
                * hg(g, sun.dot(ray.d))
                * e
                * (-sigma_t * ls).exp()
                * (1.0 - (-sigma_t * d).exp())
                / sigma_t;
        let tr = scene.trace_transmittance(ray.o, ray.d, d, N, 11 + seed(i));
        worst = worst.max(check(
            "Beer-Lambert T",
            tr.mean,
            tr.stderr,
            (-sigma_t * d).exp(),
        ));
        let pt = scene.trace_delta(ray, N, 100 + seed(i));
        worst = worst.max(check(
            &format!("delta tracking yaw {yaw}"),
            pt.mean,
            pt.stderr,
            want,
        ));
        worst = worst.max(check("quadrature", scene.reference(ray, STEPS), 0.0, want));
    }
    report("1 homogeneous sun", worst);
}

/// Scene 2: height fog over a plane, low sun. Optical depth along a slanted ray
/// is `σ0 H / d_y (e^{-y0/H} - e^{-y1/H})`; the in-scatter integral uses it inside
/// a fine quadrature.
#[test]
fn scene2_height_fog_low_sun() {
    let (sigma0, h) = (0.3, 1.5);
    let scene = Scene {
        medium: Medium::Height {
            sigma0,
            scale_height: h,
            albedo: 0.9,
        },
        g: 0.3,
        lights: vec![Light::Sun {
            to_light: V3::new(1.0, 0.15, 0.3).norm(),
            irradiance: 2.0,
        }],
        occluders: vec![],
        background: 0.0,
    };
    let down = V3::new(1.0, -0.2, 0.0).norm();
    let o = V3::new(0.0, 2.0, 0.0);
    let rays = [
        Ray {
            o,
            d: down,
            tmax: o.y / -down.y,
        },
        Ray {
            o,
            d: V3::new(0.0, 0.3, 1.0).norm(),
            tmax: 20.0,
        },
        Ray {
            o,
            d: V3::new(-1.0, -0.05, 0.2).norm(),
            tmax: 15.0,
        },
    ];
    let mut worst = 0.0_f64;
    for (i, ray) in rays.into_iter().enumerate() {
        let tau = sigma0 * h / ray.d.y
            * ((-ray.o.y / h).exp() - (-(ray.o.y + ray.tmax * ray.d.y) / h).exp());
        let tr = scene.trace_transmittance(ray.o, ray.d, ray.tmax, N, 21 + seed(i));
        worst = worst.max(check(
            "slanted Beer-Lambert T",
            tr.mean,
            tr.stderr,
            (-tau).exp(),
        ));
        let want = scene.reference(ray, STEPS);
        let pt = scene.trace_delta(ray, N, 200 + seed(i));
        worst = worst.max(check(
            &format!("delta tracking ray {i}"),
            pt.mean,
            pt.stderr,
            want,
        ));
    }
    report("2 height fog low sun", worst);
}

fn lamp_scene(on: bool, occluders: Vec<Aabb>) -> Scene {
    Scene {
        medium: Medium::Homogeneous {
            sigma_t: 0.15,
            albedo: 0.8,
            top: f64::INFINITY,
        },
        g: 0.0,
        lights: vec![Light::Point {
            pos: V3::new(0.0, 2.0, 0.5),
            intensity: 8.0,
            on,
        }],
        occluders,
        background: 0.0,
    }
}

/// Scene 3: point lamp in homogeneous fog. Without extinction the in-scatter is
/// the closed form `σs I (θb - θa) / (4π h)`; with extinction, equiangular sampling
/// and delta tracking must both match the quadrature.
#[test]
fn scene3_point_lamp() {
    let ray = Ray {
        o: V3::new(-5.0, 1.0, 0.0),
        d: V3::new(1.0, 0.0, 0.0),
        tmax: 10.0,
    };
    let mut worst = 0.0_f64;

    // Zero-extinction limit: compare per unit σs against the exact closed form.
    let tiny = 1e-9;
    let mut thin = lamp_scene(true, vec![]);
    thin.medium = Medium::Homogeneous {
        sigma_t: tiny,
        albedo: 1.0,
        top: f64::INFINITY,
    };
    let (h, delta) = ((1.0_f64 * 1.0 + 0.5 * 0.5).sqrt(), 5.0);
    let closed = 8.0 * ((ray.tmax - delta).atan2(h) - (-delta).atan2(h)) / (4.0 * PI * h);
    worst = worst.max(check(
        "quadrature, no extinction",
        thin.reference(ray, STEPS) / tiny,
        0.0,
        closed,
    ));
    let eq = thin.trace_equiangular(ray, 0, 1_000, 3);
    worst = worst.max(check(
        "equiangular, no extinction",
        eq.mean / tiny,
        eq.stderr / tiny,
        closed,
    ));

    let scene = lamp_scene(true, vec![]);
    let want = scene.reference(ray, STEPS);
    let eq = scene.trace_equiangular(ray, 0, N / 4, 31);
    worst = worst.max(check("equiangular sampling", eq.mean, eq.stderr, want));
    let pt = scene.trace_delta(ray, 4 * N, 32);
    worst = worst.max(check("delta tracking", pt.mean, pt.stderr, want));
    report("3 point lamp", worst);
}

fn room() -> Vec<Aabb> {
    let (lo, hi, w) = (V3::new(-3.0, 0.0, -3.0), V3::new(3.0, 3.0, 3.0), 0.1);
    vec![
        Aabb {
            min: V3::new(lo.x - w, lo.y, lo.z),
            max: V3::new(lo.x, hi.y, hi.z),
        },
        Aabb {
            min: V3::new(hi.x, lo.y, lo.z),
            max: V3::new(hi.x + w, hi.y, hi.z),
        },
        Aabb {
            min: V3::new(lo.x, lo.y - w, lo.z),
            max: V3::new(hi.x, lo.y, hi.z),
        },
        Aabb {
            min: V3::new(lo.x, hi.y, lo.z),
            max: V3::new(hi.x, hi.y + w, hi.z),
        },
        Aabb {
            min: V3::new(lo.x, lo.y, lo.z - w),
            max: V3::new(hi.x, hi.y, lo.z),
        },
        Aabb {
            min: V3::new(lo.x, lo.y, hi.z),
            max: V3::new(hi.x, hi.y, hi.z + w),
        },
    ]
}

/// Scene 4: lamp switching on in a dark foggy room. Frame 0 (off) is exactly
/// black; frame 1 (on) is at the converged value straight away, because nothing
/// carries state between frames.
#[test]
fn scene4_lamp_switch_in_dark_room() {
    let ray = Ray {
        o: V3::new(-2.9, 1.0, 0.0),
        d: V3::new(1.0, 0.0, 0.0),
        tmax: 5.9,
    };
    let off = lamp_scene(false, room());
    let on = lamp_scene(true, room());
    let f0 = off.trace_delta(ray, N, 41);
    let f0_eq = off.trace_equiangular(ray, 0, N, 41);
    println!(
        "  frame 0 (off): delta {} equiangular {}",
        f0.mean, f0_eq.mean
    );
    assert!(
        f0.mean == 0.0 && f0_eq.mean == 0.0,
        "dark room is not black with the lamp off"
    );
    let want = on.reference(ray, STEPS);
    assert!(want > 0.0);
    let f1 = on.trace_delta(ray, 4 * N, 42);
    let f1_eq = on.trace_equiangular(ray, 0, N / 4, 42);
    let mut worst = check("frame 1 (on) delta tracking", f1.mean, f1.stderr, want);
    worst = worst.max(check(
        "frame 1 (on) equiangular",
        f1_eq.mean,
        f1_eq.stderr,
        want,
    ));
    let open = lamp_scene(true, vec![]).reference(ray, STEPS);
    worst = worst.max(check("walls do not shadow the inside", want, 0.0, open));
    println!("  fog_switch_frames_to_ref 1");
    report("4 lamp switch dark room", worst);
}

/// Scene 5: a sun shaft through a ceiling window. The lit stretch of a
/// horizontal ray is the interval whose sun rays clear both faces of the window,
/// so `L = σs p E e^{-σt Ls} (e^{-σt ta} - e^{-σt tb}) / σt`.
#[test]
fn scene5_window_shaft() {
    let (sigma_t, albedo, g, e) = (0.1, 0.9, 0.2, 5.0);
    let (ceil_lo, ceil_hi, half) = (3.0, 3.2, 0.5);
    let sun = V3::new(0.3, 1.0, 0.0).norm();
    let big = 10.0;
    let slab = |min: V3, max: V3| Aabb { min, max };
    let occluders = vec![
        slab(V3::new(-big, ceil_lo, -big), V3::new(-half, ceil_hi, big)),
        slab(V3::new(half, ceil_lo, -big), V3::new(big, ceil_hi, big)),
        slab(V3::new(-half, ceil_lo, -big), V3::new(half, ceil_hi, -half)),
        slab(V3::new(-half, ceil_lo, half), V3::new(half, ceil_hi, big)),
    ];
    let scene = Scene {
        medium: Medium::Homogeneous {
            sigma_t,
            albedo,
            top: ceil_lo,
        },
        g,
        lights: vec![Light::Sun {
            to_light: sun,
            irradiance: e,
        }],
        occluders,
        background: 0.0,
    };
    let ray = Ray {
        o: V3::new(-6.0, 1.0, 0.0),
        d: V3::new(1.0, 0.0, 0.0),
        tmax: 12.0,
    };
    // A point at x is lit when x + (y - y0) * sun.x / sun.y is inside the window at both faces.
    let shift = |y: f64| (y - ray.o.y) * sun.x / sun.y;
    let x_lo = (-half - shift(ceil_lo)).max(-half - shift(ceil_hi));
    let x_hi = (half - shift(ceil_lo)).min(half - shift(ceil_hi));
    let (ta, tb) = (x_lo - ray.o.x, x_hi - ray.o.x);
    let ls = (ceil_lo - ray.o.y) / sun.y;
    let want = albedo
        * sigma_t
        * hg(g, sun.dot(ray.d))
        * e
        * (-sigma_t * ls).exp()
        * ((-sigma_t * ta).exp() - (-sigma_t * tb).exp())
        / sigma_t;
    let mut worst = check("quadrature", scene.reference(ray, 40 * STEPS), 0.0, want);
    let pt = scene.trace_delta(ray, 4 * N, 51);
    worst = worst.max(check("delta tracking", pt.mean, pt.stderr, want));
    report("5 window shaft", worst);
}

fn seed(i: usize) -> u64 {
    u64::try_from(i).unwrap_or(0)
}
