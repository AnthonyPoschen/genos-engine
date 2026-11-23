#![cfg_attr(
    test,
    expect(clippy::panic, reason = "a test aborts when a fixture fails")
)]

//! The CPU reference traces Sponza's triangles. No window and no GPU.
//!
//! `cargo test -p genos-debug --release --test gltf_reference -- --nocapture`.
//! The test skips with a message when the Sponza bytes are not in the checkout.

use std::path::Path;
use std::sync::Arc;

use genos_debug::reference::{render, RefSetup};
use genos_load::{shade_uv, transform_point, AlphaMode, GltfScene, LoadError};
use genos_render::{identity_pose, GltfSurface, Surfaces};
use genos_scene::{Floor, Light, Scene, Sky, Vec3};

const SPONZA: &str = "../../assets/built/sponza/sponza.gltf";

/// `debug/scenes/sponza.scene`: the sun, the sky and the three lamps, with the
/// 1 cm floor the lighting binary parks at x = 1000.
fn sponza_scene() -> Scene {
    let mut scene = Scene {
        floor: Floor {
            position: Vec3::new(1000.0, 0.0, 0.0),
            half_x: 0.01,
            half_z: 0.01,
            color: [0.2, 0.2, 0.2],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: Vec::new(),
        ceiling: None,
        sky: Some(Sky {
            color: [0.12, 0.15, 0.20],
        }),
        lighting: genos_scene::LightingSettings::default(),
        debug: genos_scene::DebugSettings::default(),
        fps_counter: genos_scene::FpsCounterSettings::default(),
        event_loop: genos_scene::EventLoopSettings::default(),
        reflections: genos_scene::ReflectionSettings::default(),
        hdr: genos_scene::HdrSettings::legacy_bytes(),
        fog: genos_scene::FogSettings::default(),
        particles: genos_scene::ParticleSettings::default(),
        menu: genos_scene::MenuSettings::default(),
    };
    let sun = [0.25f32, -1.0, 0.15];
    let len = sun.iter().map(|c| c * c).sum::<f32>().sqrt();
    scene.lights.push(Light {
        position: Vec3::new(0.0, 7.0, 0.0),
        color: [2.0, 1.9, 1.7],
        direction: Vec3::new(sun[0] / len, sun[1] / len, sun[2] / len),
    });
    let lamps = [
        ([-6.0, 6.0, -0.4], [6.0, 4.5, 3.0]),
        ([-6.0, 3.2, 4.2], [3.0, 2.5, 2.0]),
        ([5.0, 4.0, -0.4], [2.0, 1.5, 1.2]),
    ];
    for (at, color) in lamps {
        scene.lights.push(Light {
            position: Vec3::new(at[0], at[1], at[2]),
            color,
            direction: Vec3::ZERO,
        });
    }
    scene
}

/// Points on MASK triangles with the face normal there: (clear texels, opaque texels).
type Point = ([f32; 3], [f32; 3]);

fn mask_points(model: &GltfScene) -> (Vec<Point>, Vec<Point>) {
    let (mut clear, mut opaque) = (Vec::new(), Vec::new());
    for instance in &model.instances {
        let Some(mesh) = model.meshes.get(instance.mesh) else {
            continue;
        };
        for primitive in &mesh.primitives {
            let Some(material) = primitive.material.and_then(|m| model.materials.get(m)) else {
                continue;
            };
            let AlphaMode::Mask(cutoff) = material.alpha_mode else {
                continue;
            };
            // Every 40th triangle is enough, and keeps the test short.
            for t in primitive.indices.as_chunks::<3>().0.iter().step_by(40) {
                let corner = |k: usize| usize::try_from(t[k]).unwrap_or(usize::MAX);
                let at = [corner(0), corner(1), corner(2)];
                if at.iter().any(|i| *i >= primitive.positions.len())
                    || primitive.texcoords.len() != primitive.positions.len()
                {
                    continue;
                }
                let p = at.map(|i| transform_point(&instance.transform, primitive.positions[i]));
                let uv = at.map(|i| primitive.texcoords[i]);
                let centre: [f32; 3] = std::array::from_fn(|a| (p[0][a] + p[1][a] + p[2][a]) / 3.0);
                let centre_uv: [f32; 2] =
                    std::array::from_fn(|a| (uv[0][a] + uv[1][a] + uv[2][a]) / 3.0);
                let e1: [f32; 3] = std::array::from_fn(|a| p[1][a] - p[0][a]);
                let e2: [f32; 3] = std::array::from_fn(|a| p[2][a] - p[0][a]);
                let n = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let len = n.iter().map(|c| c * c).sum::<f32>().sqrt();
                if len < 1.0e-9 {
                    continue;
                }
                let alpha = shade_uv(material, &model.images, centre_uv).texel_alpha;
                let point = (centre, n.map(|c| c / len));
                if alpha < cutoff * 0.25 {
                    clear.push(point);
                } else if alpha > 0.9 {
                    opaque.push(point);
                }
            }
        }
    }
    (clear, opaque)
}

/// True when the 8 mm segment through `point` along its normal is blocked.
fn blocked_through(surface: &GltfSurface, (p, n): Point) -> bool {
    let from: [f32; 3] = std::array::from_fn(|a| p[a] + n[a] * 0.004);
    let to: [f32; 3] = std::array::from_fn(|a| p[a] - n[a] * 0.004);
    surface.blocked(from, to)
}

/// Sponza, or None (with a SKIP line) when its bytes are not in the checkout.
fn sponza(test: &str) -> Option<GltfScene> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SPONZA);
    match genos_load::load_gltf_file(&path) {
        Ok(model) => Some(model),
        Err(err @ (LoadError::LfsPointer { .. } | LoadError::NotFound { .. })) => {
            println!("SKIP {test}: {err}");
            None
        }
        Err(err) => panic!("{}: {err}", path.display()),
    }
}

/// The atrium checkpoint of `debug/scenes/sponza.scene`.
fn atrium(surface: Arc<GltfSurface>, width: u32, height: u32, spp: u32, bounces: u32) -> RefSetup {
    RefSetup {
        scene: sponza_scene(),
        eye: [-13.0, 1.8, 0.0],
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        width,
        height,
        spp,
        noise_target: 0.0,
        max_spp: spp,
        seconds: f32::INFINITY,
        max_bounces: bounces,
        triangles: false,
        materials: Vec::new(),
        specular: false,
        meshes: Some(surface),
    }
}

/// The atrium at the `reference` command's defaults (320×180, 64 paths, no
/// bounce limit), saved to look at and to time:
/// `cargo test -p genos-debug --release --test gltf_reference -- --ignored --nocapture`.
/// The picture goes to `target/reference-known/sponza-atrium.png`.
#[test]
#[ignore = "a full trace to look at; it takes about a minute"]
fn sponza_atrium_picture() {
    let Some(model) = sponza("sponza_atrium_picture") else {
        return;
    };
    let surface = Arc::new(GltfSurface::from_gltf(&model, &identity_pose()));
    let picture = render(&atrium(surface, 320, 180, 64, 64));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reference-known");
    std::fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("{}: {err}", dir.display()));
    let out = dir.join("sponza-atrium.png");
    picture
        .image
        .save(&out)
        .unwrap_or_else(|err| panic!("{}: {err}", out.display()));
    println!(
        "{}: {} spp, noise {:.4}, {:.1} s",
        out.display(),
        picture.spp,
        picture.noise,
        picture.seconds
    );
}

#[test]
fn the_reference_traces_sponza() {
    let Some(model) = sponza("the_reference_traces_sponza") else {
        return;
    };
    let started = std::time::Instant::now();
    let surface = Arc::new(GltfSurface::from_gltf(&model, &identity_pose()));
    println!(
        "sponza: {} triangles, bounds {:?}, built in {:.2} s",
        surface.triangle_count(),
        surface.bounds(),
        started.elapsed().as_secs_f32()
    );
    assert!(surface.triangle_count() > 200_000);

    // Straight down from the middle of the atrium: the floor, facing up.
    let floor = surface
        .first_hit([0.0, 1.0, 0.0], [0.0, -1.0, 0.0], 50.0)
        .unwrap_or_else(|| panic!("no floor under (0, 1, 0)"));
    assert!(floor.point[1].abs() < 0.1, "floor at {:?}", floor.point);
    assert!(floor.normal[1] > 0.9, "floor normal {:?}", floor.normal);
    assert!(floor.albedo.iter().all(|c| *c > 0.0 && *c <= 1.0));

    // A wall blocks a shadow ray; a clear leaf texel does not, an opaque one does.
    assert!(surface.blocked([0.0, 1.0, 0.0], [0.0, 1.0, 40.0]));
    let (clear, opaque) = mask_points(&model);
    assert!(
        clear.len() > 20 && opaque.len() > 20,
        "mask samples: {} clear, {} opaque",
        clear.len(),
        opaque.len()
    );
    let clear_blocked = clear
        .iter()
        .filter(|p| blocked_through(&surface, **p))
        .count();
    let opaque_blocked = opaque
        .iter()
        .filter(|p| blocked_through(&surface, **p))
        .count();
    println!(
        "mask: {clear_blocked}/{} clear texels blocked, {opaque_blocked}/{} opaque texels blocked",
        clear.len(),
        opaque.len()
    );
    // A few clear points have another leaf inside the 8 mm segment.
    assert!(clear_blocked * 10 <= clear.len());
    assert_eq!(opaque_blocked, opaque.len());

    let setup = atrium(surface, 64, 36, 16, 2);
    assert_eq!(setup.geometry(), "shapes+gltf");
    let picture = render(&setup);
    let pixels = picture.hit.len();
    let covered = picture.hit.iter().filter(|h| **h).count();
    let luminance = |c: &[f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let lit = picture
        .hit
        .iter()
        .filter(|h| **h)
        .map(|_| 1.0f32)
        .sum::<f32>();
    let mean = picture.linear.iter().map(luminance).sum::<f32>() / lit.max(1.0);
    println!(
        "atrium: coverage {covered}/{pixels}, mean luminance {mean:.4}, noise {:.3}, {:.2} s",
        picture.noise, picture.seconds
    );
    assert!(covered * 10 > pixels * 9, "coverage {covered}/{pixels}");
    assert!(mean > 0.0 && mean.is_finite(), "mean luminance {mean}");
    assert!(picture.linear.iter().flatten().all(|c| c.is_finite()));

    // Without the model the same pose sees nothing: the old `coverage: 0`.
    let bare = render(&RefSetup {
        meshes: None,
        spp: 1,
        max_spp: 1,
        ..setup
    });
    assert!(bare.hit.iter().all(|h| !*h));
}
