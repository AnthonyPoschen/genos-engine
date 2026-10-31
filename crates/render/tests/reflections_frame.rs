//! The presented frame composites the mirror and the portal and counts rays.

#![expect(clippy::as_conversions, reason = "the readback index is a byte offset")]
#![expect(clippy::expect_used, reason = "a failed fixture aborts the test")]
#![expect(
    clippy::print_stderr,
    reason = "a mismatch prints the sample that failed"
)]

use genos_math::optics::ReflectionPreset;
use genos_render::{grade_camera, grade_world, Renderer, GRADE_HEIGHT, GRADE_WIDTH};
use genos_window::Window;

fn open() -> (std::sync::MutexGuard<'static, ()>, Window, Renderer) {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut window = Window::open_proof(GRADE_WIDTH, GRADE_HEIGHT).expect("proof window");
    let frame = window.pump();
    let renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)
        .expect("renderer");
    (guard, window, renderer)
}

fn sample(pixels: &[u8], width: u32, x: u32, y: u32) -> [f32; 3] {
    let index = ((y * width + x) * 4) as usize;
    [
        f32::from(pixels[index + 2]) / 255.0,
        f32::from(pixels[index + 1]) / 255.0,
        f32::from(pixels[index]) / 255.0,
    ]
}

#[test]
fn the_presented_frame_shows_the_mirror_the_portal_and_the_ray_cap() {
    let (_guard, mut window, mut renderer) = open();
    let camera = grade_camera();
    let scene = grade_world(ReflectionPreset::Medium);
    let _ = window.pump();
    let pixels = renderer
        .draw(&scene, &camera, true)
        .expect("draw")
        .expect("readback");
    let grade = renderer.reflection_grade().expect("grade");
    let width = renderer.width();
    let height = renderer.height();
    eprintln!(
        "extent {width}x{height} rays {} views {}",
        grade.rays, grade.views
    );
    for (name, x, y) in [
        ("water", 40u32, 30u32),
        ("below", 120, 30),
        ("mirror", 40, 90),
        ("portal", 100, 90),
        ("extra", 140, 90),
        ("ggx", 2, 2),
    ] {
        if x < width && y < height {
            eprintln!("{name} {:?}", sample(&pixels, width, x, y));
        }
    }
    assert!(grade.ran);
    assert_eq!(
        grade.rays, 32_000,
        "medium must count the rays it dispatched"
    );
    assert_eq!(grade.views, 2, "medium keeps the mirror and the portal");
    let mirror = sample(&pixels, width, 40, 90);
    assert!(mirror[0] > 0.6, "mirror {mirror:?}");
    assert!(
        mirror[0] > mirror[2],
        "mirror should be the red wall, got {mirror:?}"
    );
    let portal = sample(&pixels, width, 100, 90);
    assert!(
        portal[2] > portal[0],
        "portal should be the blue wall, got {portal:?}"
    );
    let extra = sample(&pixels, width, 140, 90);
    assert!(
        (extra[0] - 0.28).abs() < 0.08 && (extra[2] - 0.34).abs() < 0.08,
        "refused view keeps the albedo, got {extra:?}"
    );

    for (preset, rays, views) in [
        (ReflectionPreset::Potato, 0, 0),
        (ReflectionPreset::Low, 0, 1),
        (ReflectionPreset::High, 128_000, 3),
    ] {
        let scene = grade_world(preset);
        let _ = window.pump();
        renderer.draw(&scene, &camera, true).expect("preset draw");
        let grade = renderer.reflection_grade().expect("preset grade");
        eprintln!("{:?} rays {} views {}", preset, grade.rays, grade.views);
        assert_eq!(grade.rays, rays);
        assert_eq!(grade.views, views);
    }
}
