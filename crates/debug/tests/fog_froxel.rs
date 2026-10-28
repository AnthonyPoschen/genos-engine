//! Stage F1 fog model (ADR 0026): the CPU froxel grid graded against the F0
//! path tracer on the five scenes of `docs/systems/rendering/fog/plan.md`, per
//! preset grid. CPU only and deterministic, so it runs headless anywhere.
//!
//! Prints one `fog_ref_err_near` row per scene and preset, the light-switch
//! `fog_switch_frames_to_ref`, and `fog_flicker_max` on a still camera.

use genos_fog_ref::froxel::{Camera, Preset};
use genos_fog_ref::{rel_err, Aabb, Light, Medium, Ray, Scene, V3};

/// The F1 gate on the model.
const GATE: f64 = 0.05;
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

/// Worst relative error of the composed grid against the reference over the
/// graded pixels of every ray in `case`.
fn grade(case: &Case, preset: Preset) -> f64 {
    let mut worst = 0.0_f64;
    for &ray in &case.rays {
        let cam = camera_for(ray);
        let frame = case.scene.froxel_frame(preset.grid(), cam);
        for (du, dv) in PIXELS {
            let d = cam.dir(0.5 + du, 0.5 + dv);
            let pix = Ray {
                o: ray.o,
                d,
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
        Case {
            name: "5 window shaft",
            scene: shaft_scene(),
            rays: vec![Ray {
                o: V3::new(-6.0, 1.0, 0.0),
                d: V3::new(1.0, 0.0, 0.0),
                tmax: 12.0,
            }],
            steps: 40 * STEPS,
        },
    ]
}

/// Every scene on every preset is within the near-error gate.
#[test]
fn froxel_model_matches_path_tracer_per_preset() {
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

/// The frame the lamp switches on is already within the gate; the frame
/// before is exactly black. Nothing carries over between frames.
#[test]
fn lamp_switch_frames_to_ref() {
    let ray = room_ray();
    let cam = camera_for(ray);
    for preset in Preset::ALL {
        let grid = preset.grid();
        let off = lamp_scene(false, room()).froxel_frame(grid, cam);
        let on_scene = lamp_scene(true, room());
        let on = on_scene.froxel_frame(grid, cam);
        let f0 = off.compose(ray, 0.0);
        let err = rel_err(on.compose(ray, 0.0), on_scene.reference(ray, STEPS));
        let frames = if err <= GATE { 1 } else { u32::MAX };
        println!(
            "fog_switch_frames_to_ref {:<7} {frames} (frame 0 {f0}, frame 1 err {err:.4})",
            preset.name()
        );
        assert!(f0 == 0.0, "{}: dark room not black", preset.name());
        assert!(frames <= 1, "{}: switch frame err {err}", preset.name());
        // Switching back off is black on the very next frame too.
        let back = lamp_scene(false, room()).froxel_frame(grid, cam);
        assert_eq!(back, off, "{}: off frame depends on history", preset.name());
    }
}

/// A still camera gives the identical frame every time: `fog_flicker_max` is 0.
#[test]
fn still_camera_does_not_flicker() {
    let case = &cases()[4];
    let cam = camera_for(case.rays[0]);
    let grid = Preset::Low.grid();
    let a = case.scene.froxel_frame(grid, cam);
    let b = case.scene.froxel_frame(grid, cam);
    let flicker = (a.compose(case.rays[0], 0.0) - b.compose(case.rays[0], 0.0)).abs();
    println!("fog_flicker_max {flicker}");
    assert_eq!(a, b);
}
