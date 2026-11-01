#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

//! Lavapipe froxel against the CPU column integral.
//!
//! `run_fog` is the inject and integrate dispatches. The frame test draws a
//! Low-preset picture with fog and without it. The composed bytes must differ,
//! and the frame's own column buffer must match the CPU integral.
//! Times are lavapipe timestamps, not RTX 4070 measurements.

use genos_render::{
    column_peak, composed_frame_camera, composed_frame_scene, inject, integrate_unshadowed,
    picture_mean_abs, picture_unchanged, DirectSun, FogDispatch, FogView, GridSize, Renderer, World,
};
use genos_scene::FogSettings;
use genos_window::Window;

const COLUMN_ABS: f32 = 1.0e-3;

fn open(width: u32, height: u32) -> Result<genos_window::Lifecycle<Renderer>, String> {
    let mut window = Window::open_proof(width, height)?;
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok(genos_window::Lifecycle::new(window, renderer, permit))
}

fn height_fog() -> FogSettings {
    FogSettings {
        enabled: true,
        density: 0.08,
        thickness: 40.0,
        albedo: 1.0,
        phase_g: 0.4,
        history: false,
        gi: false,
        ..FogSettings::default()
    }
}

fn job(settings: FogSettings, size: GridSize) -> FogDispatch {
    FogDispatch {
        settings,
        view: FogView::look([0.0, 1.6, 0.0], 0.0, 0.0, 0.5, 1.0),
        size,
        sun: DirectSun {
            toward: [0.0, 1.0, 0.0],
            color: [1.0, 0.95, 0.85],
        },
        lamps: Vec::new(),
        scene: None,
        flags: 0,
        stop_slices: 0,
        read_extinction: true,
        reference_far: 0.0,
    }
}

fn peak_column(gpu: &[genos_render::FogColumn], cpu: &[genos_render::FogColumn]) -> f32 {
    let mut peak = 0.0_f32;
    for (left, right) in gpu.iter().zip(cpu) {
        for channel in 0..3 {
            peak = peak.max((left.inscatter[channel] - right.inscatter[channel]).abs());
        }
        peak = peak.max((left.transmittance - right.transmittance).abs());
    }
    peak
}

#[test]
fn gpu_columns_match_the_unshadowed_integral() {
    let mut owner = open(64, 64).expect("lavapipe device");
    let renderer = &mut *owner.device;
    let settings = height_fog();
    let size = GridSize {
        width: 4,
        height: 2,
        slices: 4,
    };
    let view = FogView::look([0.0, 1.6, 0.0], 0.0, 0.0, 0.5, 1.0);
    let sun = DirectSun {
        toward: [0.0, 1.0, 0.0],
        color: [1.0, 0.95, 0.85],
    };
    let cpu = integrate_unshadowed(&settings, &view, size, sun);
    let extinction = inject(&settings, &view, size);
    let gpu = renderer
        .run_fog(&job(settings, size))
        .expect("froxel dispatch");
    assert_eq!(gpu.columns.len(), cpu.len());
    let err = peak_column(&gpu.columns, &cpu);
    assert!(
        err <= COLUMN_ABS,
        "column abs {err} limit {COLUMN_ABS} pass_ms {} vram {}",
        gpu.pass_ms,
        gpu.vram_bytes
    );
    assert_eq!(gpu.extinction.len(), extinction.extinction.len());
    let mut ext = 0.0_f32;
    for (left, right) in gpu.extinction.iter().zip(&extinction.extinction) {
        ext = ext.max((left - right).abs());
    }
    assert!(ext <= COLUMN_ABS, "extinction abs {ext} limit {COLUMN_ABS}");
    assert!(gpu.vram_bytes > 0, "froxel allocation was 0");
    println!(
        "froxel parity lavapipe pass_ms {} vram_bytes {} column_abs {err} extinction_abs {ext}",
        gpu.pass_ms, gpu.vram_bytes
    );
}

#[test]
fn a_low_preset_frame_dispatches_the_froxel() {
    let mut owner = open(64, 64).expect("lavapipe device");
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let scene = composed_frame_scene(true);
    let world = World::from_scene(scene.clone());
    let camera = composed_frame_camera();
    let _ = window.pump();
    let on = renderer
        .draw(&world, &camera, true)
        .expect("draw")
        .expect("readback");
    let probe = renderer.fog_frame_probe(&scene.fog, scene.lighting.preset);
    assert_eq!(probe.dispatches, 1, "the frame did not dispatch froxel");
    assert_eq!(probe.history_accepts, 0);
    assert_eq!(probe.history_rejects, 0);
    assert!(probe.vram_bytes > 0, "frame vram was 0");
    assert!(probe.pass_ms.is_finite() && probe.pass_ms > 0.0);
    let columns = renderer.frame_fog_columns().expect("frame columns");
    let reference = renderer.frame_fog_reference(&scene);
    let err = column_peak(&columns, &reference);
    assert!(
        err <= f64::from(COLUMN_ABS),
        "frame column abs {err} limit {COLUMN_ABS}"
    );
    let off_scene = composed_frame_scene(false);
    let off_probe = renderer.fog_frame_probe(&off_scene.fog, off_scene.lighting.preset);
    assert_eq!(off_probe.dispatches, 0);
    assert_eq!(off_probe.pass_ms, 0.0);
    let off_world = World::from_scene(off_scene);
    let off = renderer
        .draw(&off_world, &camera, true)
        .expect("fog off draw")
        .expect("fog off readback");
    let unchanged = picture_unchanged(&on, &off);
    let mean = picture_mean_abs(&on, &off);
    println!(
        "froxel frame lavapipe pass_ms {} vram_bytes {} column_abs {err} picture_unchanged {unchanged} picture_mean_abs {mean}",
        probe.pass_ms, probe.vram_bytes
    );
    assert!(
        unchanged <= 0.0,
        "fog did not change the composed picture, mean abs {mean}"
    );
}

/// GPU froxel columns against `genos-fog-ref` on the Low grid (ADR 0026 F2).
///
/// Relative error is at most 1% where the reference in-scatter or transmittance
/// is above 1e-3. Below that the absolute error is at most 1e-5. The four
/// scenes are uniform density, height fog, a point lamp, and a sun shaft.
const F2_REL: f32 = 0.01;
const F2_ABS: f32 = 1.0e-5;
const F2_LUMA: f64 = 1.0e-3;

#[test]
fn gpu_froxel_matches_fog_ref_within_one_percent() {
    use genos_fog_ref::froxel::Preset;

    let mut owner = open(64, 64).expect("lavapipe device");
    let renderer = &mut *owner.device;
    let grid = Preset::Low.grid();
    let mut worst = Vec::new();
    for scene in f2_scenes() {
        let cam = f2_camera(&scene.ray);
        let frame = scene.scene.froxel_frame(grid, cam);
        let gpu = renderer
            .run_fog(&f2_job(&scene, cam, grid))
            .expect("froxel dispatch");
        let err = f2_column_error(&frame, cam, grid, &gpu.columns);
        println!(
            "f2 {name} rel_or_abs {err:.6} columns {}",
            name = scene.name,
            gpu.columns.len()
        );
        if err > F2_REL {
            worst.push(format!("{} {err:.6}", scene.name));
        }
    }
    assert!(worst.is_empty(), "fog-ref 1% gate: {worst:?}");
}

struct F2Scene {
    name: &'static str,
    scene: genos_fog_ref::Scene,
    ray: genos_fog_ref::Ray,
}

fn f2_scenes() -> Vec<F2Scene> {
    use genos_fog_ref::{Aabb, Light, Medium, Ray, Scene, V3};
    let sun = V3::new(0.5, 1.0, 0.0).norm();
    let homogeneous = Scene {
        medium: Medium::Homogeneous {
            sigma_t: 0.2,
            albedo: 0.8,
            top: 5.0,
        },
        g: 0.6,
        lights: vec![Light::Sun {
            to_light: sun,
            irradiance: 3.0,
        }],
        occluders: vec![],
        background: 0.5,
    };
    let yaw = 0.0_f64;
    let height = Scene {
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
    let lamp = Scene {
        medium: Medium::Homogeneous {
            sigma_t: 0.15,
            albedo: 0.8,
            top: f64::INFINITY,
        },
        g: 0.0,
        lights: vec![Light::Point {
            pos: V3::new(0.0, 2.0, 0.5),
            intensity: 8.0,
            on: true,
        }],
        occluders: vec![],
        background: 0.0,
    };
    let (ceil_lo, ceil_hi, half, big) = (3.0, 3.2, 0.5, 10.0);
    let slab = |min, max| Aabb { min, max };
    let shaft = Scene {
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
    };
    vec![
        F2Scene {
            name: "uniform density",
            scene: homogeneous,
            ray: Ray {
                o: V3::new(0.0, 1.0, 0.0),
                d: V3::new(yaw.cos(), 0.0, yaw.sin()),
                tmax: 10.0,
            },
        },
        F2Scene {
            name: "height fog",
            scene: height,
            ray: Ray {
                o: V3::new(0.0, 2.0, 0.0),
                d: down,
                tmax: 2.0 / -down.y,
            },
        },
        F2Scene {
            name: "lamp in fog",
            scene: lamp,
            ray: Ray {
                o: V3::new(-5.0, 1.0, 0.0),
                d: V3::new(1.0, 0.0, 0.0),
                tmax: 10.0,
            },
        },
        F2Scene {
            name: "sun shafts",
            scene: shaft,
            ray: Ray {
                o: V3::new(-6.0, 1.0, 0.0),
                d: V3::new(1.0, 0.0, 0.0),
                tmax: 12.0,
            },
        },
    ]
}

fn f2_camera(ray: &genos_fog_ref::Ray) -> genos_fog_ref::froxel::Camera {
    use genos_fog_ref::froxel::Camera;
    Camera {
        pos: ray.o,
        forward: ray.d,
        fov_y: 60_f64.to_radians(),
        aspect: 16.0 / 9.0,
        near: 0.1,
        far: ray.tmax,
    }
}

fn f2_basis(forward: genos_fog_ref::V3) -> (genos_fog_ref::V3, genos_fog_ref::V3) {
    use genos_fog_ref::V3;
    let up = if forward.y.abs() > 0.99 {
        V3::new(0.0, 0.0, 1.0)
    } else {
        V3::new(0.0, 1.0, 0.0)
    };
    let right = cross64(forward, up);
    let right = right * (1.0 / right.len());
    let up = cross64(right, forward);
    (right, up)
}

fn cross64(a: genos_fog_ref::V3, b: genos_fog_ref::V3) -> genos_fog_ref::V3 {
    use genos_fog_ref::V3;
    V3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

fn f32_of(value: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation, reason = "fog-ref f64 narrows to the GPU f32 grid")]
    {
        value as f32
    }
}

fn f2_job(
    scene: &F2Scene,
    cam: genos_fog_ref::froxel::Camera,
    grid: genos_fog_ref::froxel::Grid,
) -> FogDispatch {
    use genos_fog_ref::{Light, Medium};
    use genos_render::{DirectSun, FogPointLamp, FogView, GridSize, FOG_F2_REFERENCE};
    use genos_scene::{FogSettings, FogShape, FogVolume};
    let (right, up) = f2_basis(cam.forward);
    let ty = f32_of((cam.fov_y * 0.5).tan());
    let view = FogView {
        origin: [f32_of(cam.pos.x), f32_of(cam.pos.y), f32_of(cam.pos.z)],
        forward: [f32_of(cam.forward.x), f32_of(cam.forward.y), f32_of(cam.forward.z)],
        right: [f32_of(right.x), f32_of(right.y), f32_of(right.z)],
        up: [f32_of(up.x), f32_of(up.y), f32_of(up.z)],
        tan_half_x: ty * f32_of(cam.aspect),
        tan_half_y: ty,
    };
    let (density, height, thickness, albedo) = match scene.scene.medium {
        Medium::Homogeneous { sigma_t, albedo, top } => (
            f32_of(sigma_t),
            if top.is_finite() { f32_of(top) } else { 1.0e20 },
            0.0,
            f32_of(albedo),
        ),
        Medium::Height { sigma0, scale_height, albedo } => {
            (f32_of(sigma0), 0.0, f32_of(scale_height), f32_of(albedo))
        }
    };
    let mut sun = DirectSun {
        toward: [0.0, 1.0, 0.0],
        color: [0.0, 0.0, 0.0],
    };
    let mut lamps = Vec::new();
    for light in &scene.scene.lights {
        match *light {
            Light::Sun { to_light, irradiance } => {
                let e = f32_of(irradiance);
                sun = DirectSun {
                    toward: [f32_of(to_light.x), f32_of(to_light.y), f32_of(to_light.z)],
                    color: [e, e, e],
                };
            }
            Light::Point { pos, intensity, on } if on => {
                let e = f32_of(intensity);
                lamps.push(FogPointLamp {
                    position: [f32_of(pos.x), f32_of(pos.y), f32_of(pos.z)],
                    color: [e, e, e],
                });
            }
            Light::Point { .. } => {}
        }
    }
    let volumes = scene
        .scene
        .occluders
        .iter()
        .map(|box_| FogVolume {
            shape: FogShape::Box,
            position: [f32_of(box_.min.x), f32_of(box_.min.y), f32_of(box_.min.z)],
            extents: [f32_of(box_.max.x), f32_of(box_.max.y), f32_of(box_.max.z)],
            density: 0.0,
            albedo: 0.0,
            phase_g: 0.0,
            falloff: 0.0,
        })
        .collect();
    FogDispatch {
        settings: FogSettings {
            enabled: true,
            density,
            height,
            thickness,
            albedo,
            phase_g: f32_of(scene.scene.g),
            volumes,
            ..FogSettings::default()
        },
        view,
        size: GridSize {
            width: grid.w,
            height: grid.h,
            slices: grid.d,
        },
        sun,
        lamps,
        scene: None,
        flags: FOG_F2_REFERENCE,
        stop_slices: 0,
        read_extinction: false,
        reference_far: f32_of(cam.far),
    }
}

fn f2_column_error(
    frame: &genos_fog_ref::froxel::Frame,
    cam: genos_fog_ref::froxel::Camera,
    grid: genos_fog_ref::froxel::Grid,
    gpu: &[genos_render::FogColumn],
) -> f32 {
    use genos_fog_ref::Ray;
    let mut worst = 0.0_f32;
    for row in 0..grid.h {
        for col in 0..grid.w {
            let u = (f64::from(col) + 0.5) / f64::from(grid.w);
            let v = (f64::from(row) + 0.5) / f64::from(grid.h);
            let dir = cam.dir(u, v);
            let cos = dir.dot(cam.forward).max(1.0e-6);
            let ray = Ray {
                o: cam.pos,
                d: dir,
                tmax: cam.far / cos,
            };
            let lit = frame.compose(ray, 0.0);
            let trans = frame.compose(ray, 1.0) - lit;
            let index = usize::try_from(row * grid.w + col).unwrap_or(0);
            let Some(column) = gpu.get(index) else {
                return f32::MAX;
            };
            for channel in column.inscatter {
                worst = worst.max(f2_gate(f32_of(lit), channel));
            }
            worst = worst.max(f2_gate(f32_of(trans), column.transmittance));
        }
    }
    worst
}

fn f2_gate(reference: f32, got: f32) -> f32 {
    let gap = (reference - got).abs();
    if f64::from(reference.abs()) > F2_LUMA {
        gap / reference.abs()
    } else if gap > F2_ABS {
        F2_REL + gap
    } else {
        0.0
    }
}
