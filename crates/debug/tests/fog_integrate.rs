//! Stage F2 fog integration (ADR 0026): inject extinction and in-scattering,
//! skip temporal reprojection, integrate front to back, and grade the composed
//! column against the F0 ray march (`Scene::reference`) on the five scenes of
//! `docs/systems/rendering/fog/plan.md`.
//!
//! CPU only and deterministic. Prints one `fog_ref_err_near` row per scene and
//! preset, plus the Low window-shaft margin against the 0.05 gate.

use genos_fog_ref::froxel::{integrate_front_to_back, reproject, Camera, Preset};
use genos_fog_ref::{rel_err, Aabb, Light, Medium, Ray, Scene, V3};

/// Near-error gate from ADR 0026.
const GATE: f64 = 0.05;
/// F1's printed Low window-shaft error (2 shadow rays). The margin under the
/// gate was 0.0017. The froxel integration path reports its own margin.
const F1_LOW_SHAFT: f64 = 0.0483;
/// Reference quadrature steps.
const STEPS: u32 = 4_000;
/// Screen offsets graded around each scene ray (the near-camera pixels).
const PIXELS: [(f64, f64); 5] = [
    (0.0, 0.0),
    (0.12, 0.0),
    (-0.12, 0.0),
    (0.0, 0.1),
    (0.0, -0.1),
];

struct Case {
    name: &'static str,
    scene: Scene,
    rays: Vec<Ray>,
    /// Extra quadrature resolution for thin features.
    steps: u32,
}

fn camera_for(ray: Ray) -> Camera {
    Camera {
        pos: ray.o,
        forward: ray.d,
        fov_y: 60_f64.to_radians(),
        aspect: 16.0 / 9.0,
        near: 0.1,
        far: ray.tmax,
    }
}

/// Worst relative error of the integrated grid against the ray march.
fn grade(case: &Case, preset: Preset) -> f64 {
    let mut worst = 0.0_f64;
    for &ray in &case.rays {
        let cam = camera_for(ray);
        let injected = case.scene.inject_froxels(preset.grid(), cam);
        let temporal = reproject(injected, None);
        assert_eq!(
            temporal.history_accepts,
            0,
            "{} {}: history accept",
            case.name,
            preset.name()
        );
        assert_eq!(temporal.history_rejects, 0);
        let frame = integrate_front_to_back(&temporal.volume);
        for (du, dv) in PIXELS {
            let pix = Ray {
                o: ray.o,
                d: cam.dir(0.5 + du, 0.5 + dv),
                tmax: ray.tmax,
            };
            let want = case.scene.reference(pix, case.steps);
            let got = frame.compose(pix, case.scene.background);
            worst = worst.max(rel_err(got, want));
        }
    }
    worst
}

fn horizontal(o: V3, yaw_deg: f64, tmax: f64) -> Ray {
    let a = yaw_deg.to_radians();
    Ray {
        o,
        d: V3::new(a.cos(), 0.0, a.sin()),
        tmax,
    }
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

fn room() -> Vec<Aabb> {
    let (lo, hi, w) = (V3::new(-3.0, 0.0, -3.0), V3::new(3.0, 3.0, 3.0), 0.1);
    let b = |min, max| Aabb { min, max };
    vec![
        b(V3::new(lo.x - w, lo.y, lo.z), V3::new(lo.x, hi.y, hi.z)),
        b(V3::new(hi.x, lo.y, lo.z), V3::new(hi.x + w, hi.y, hi.z)),
        b(V3::new(lo.x, lo.y - w, lo.z), V3::new(hi.x, lo.y, hi.z)),
        b(V3::new(lo.x, hi.y, lo.z), V3::new(hi.x, hi.y + w, hi.z)),
        b(V3::new(lo.x, lo.y, lo.z - w), V3::new(hi.x, hi.y, lo.z)),
        b(V3::new(lo.x, lo.y, hi.z), V3::new(hi.x, hi.y, hi.z + w)),
    ]
}

fn room_ray() -> Ray {
    Ray {
        o: V3::new(-2.9, 1.0, 0.0),
        d: V3::new(1.0, 0.0, 0.0),
        tmax: 5.9,
    }
}

fn shaft_scene() -> Scene {
    let (ceil_lo, ceil_hi, half, big) = (3.0, 3.2, 0.5, 10.0);
    let slab = |min, max| Aabb { min, max };
    Scene {
        medium: Medium::Homogeneous {
            sigma_t: 0.1,
            albedo: 0.9,
            top: ceil_lo,
        },
        g: 0.2,
        lights: vec![Light::Sun {
            to_light: V3::new(0.3, 1.0, 0.0).norm(),
            irradiance: 5.0,
        }],
        occluders: vec![
            slab(V3::new(-big, ceil_lo, -big), V3::new(-half, ceil_hi, big)),
            slab(V3::new(half, ceil_lo, -big), V3::new(big, ceil_hi, big)),
            slab(V3::new(-half, ceil_lo, -big), V3::new(half, ceil_hi, -half)),
            slab(V3::new(-half, ceil_lo, half), V3::new(half, ceil_hi, big)),
        ],
        background: 0.0,
    }
}

fn shaft_case() -> Case {
    Case {
        name: "5 window shaft",
        scene: shaft_scene(),
        rays: vec![Ray {
            o: V3::new(-6.0, 1.0, 0.0),
            d: V3::new(1.0, 0.0, 0.0),
            tmax: 12.0,
        }],
        steps: 40 * STEPS,
    }
}

fn cases() -> Vec<Case> {
    let sun1 = V3::new(0.5, 1.0, 0.0).norm();
    let s1 = Scene {
        medium: Medium::Homogeneous {
            sigma_t: 0.2,
            albedo: 0.8,
            top: 5.0,
        },
        g: 0.6,
        lights: vec![Light::Sun {
            to_light: sun1,
            irradiance: 3.0,
        }],
        occluders: vec![],
        background: 0.5,
    };
    let o1 = V3::new(0.0, 1.0, 0.0);
    let s2 = Scene {
        medium: Medium::Height {
            sigma0: 0.3,
            scale_height: 1.5,
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
    let o2 = V3::new(0.0, 2.0, 0.0);
    vec![
        Case {
            name: "1 homogeneous sun",
            scene: s1,
            rays: vec![
                horizontal(o1, 0.0, 10.0),
                horizontal(o1, 60.0, 10.0),
                horizontal(o1, 180.0, 10.0),
            ],
            steps: STEPS,
        },
        Case {
            name: "2 height fog low sun",
            scene: s2,
            rays: vec![
                Ray {
                    o: o2,
                    d: down,
                    tmax: o2.y / -down.y,
                },
                Ray {
                    o: o2,
                    d: V3::new(0.0, 0.3, 1.0).norm(),
                    tmax: 20.0,
                },
                Ray {
                    o: o2,
                    d: V3::new(-1.0, -0.05, 0.2).norm(),
                    tmax: 15.0,
                },
            ],
            steps: STEPS,
        },
        Case {
            name: "3 point lamp",
            scene: lamp_scene(true, vec![]),
            rays: vec![Ray {
                o: V3::new(-5.0, 1.0, 0.0),
                d: V3::new(1.0, 0.0, 0.0),
                tmax: 10.0,
            }],
            steps: STEPS,
        },
        Case {
            name: "4 lamp switch dark room",
            scene: lamp_scene(true, room()),
            rays: vec![room_ray()],
            steps: STEPS,
        },
        shaft_case(),
    ]
}

/// Every scene on every preset is within the near-error gate.
#[test]
fn integrated_froxel_matches_ray_march_per_scene() {
    let mut failures = Vec::new();
    println!(
        "{:<26} {:>8} {:>8} {:>8} {:>8}",
        "fog_ref_err_near", "Low", "Medium", "High", "Ultra"
    );
    for case in cases() {
        let errs: Vec<f64> = Preset::ALL.iter().map(|&p| grade(&case, p)).collect();
        println!(
            "{:<26} {:>8.4} {:>8.4} {:>8.4} {:>8.4}",
            case.name, errs[0], errs[1], errs[2], errs[3]
        );
        for (p, e) in Preset::ALL.iter().zip(&errs) {
            if *e > GATE {
                failures.push(format!("{} {}: {e:.4}", case.name, p.name()));
            }
        }
    }
    assert!(failures.is_empty(), "over {GATE}: {failures:?}");
}

/// Low's 2-shadow-ray shaft stays inside the gate. Prints the margin under 0.05
/// next to the F1 record of 0.0483 (margin 0.0017).
#[test]
fn low_preset_shaft_margin() {
    let err = grade(&shaft_case(), Preset::Low);
    let margin = GATE - err;
    let recorded_margin = GATE - F1_LOW_SHAFT;
    println!(
        "low_shaft fog_ref_err_near {err:.4} limit {GATE:.2} margin {margin:.4} f1_recorded {F1_LOW_SHAFT:.4} f1_margin {recorded_margin:.4}"
    );
    assert!(
        err <= GATE,
        "Low shaft fog_ref_err_near {err:.4} limit {GATE:.2} margin {margin:.4}"
    );
    assert!(
        err <= F1_LOW_SHAFT + 5e-5,
        "Low shaft {err:.6} slipped past the F1 record {F1_LOW_SHAFT}"
    );
}

/// A lit previous volume does not light the next frame. Reprojection is off.
#[test]
fn temporal_reprojection_stays_off() {
    let ray = room_ray();
    let cam = camera_for(ray);
    let grid = Preset::Low.grid();
    let dark_scene = lamp_scene(false, room());
    let lit_scene = lamp_scene(true, room());
    let dark = dark_scene.inject_froxels(grid, cam);
    let lit = lit_scene.inject_froxels(grid, cam);
    let mut poisoned = lit.clone();
    poisoned.scattering.fill(50.0);
    poisoned.optical_depth.fill(4.0);
    let temporal = reproject(dark.clone(), Some(&poisoned));
    println!(
        "history_accepts {} history_rejects {}",
        temporal.history_accepts, temporal.history_rejects
    );
    assert_eq!(temporal.history_accepts, 0);
    assert_eq!(temporal.history_rejects, 0);
    assert_eq!(temporal.volume, dark);
    let composed = integrate_front_to_back(&temporal.volume).compose(ray, 0.0);
    println!("reprojected_dark_compose {composed}");
    assert_eq!(composed, 0.0);

    let moved = Camera {
        forward: V3::new(0.0, 0.0, 1.0),
        ..cam
    };
    let moved_volume = lit_scene.inject_froxels(grid, moved);
    let again = reproject(moved_volume.clone(), Some(&dark));
    assert_eq!(again.history_accepts, 0);
    assert_eq!(again.volume, moved_volume);
    assert_ne!(moved_volume.scattering, dark.scattering);
}
