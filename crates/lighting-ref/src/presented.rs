//! One presented grade frame, plus the ray dispatch count of each preset.
//!
//! The pixels are the swapchain readback (`draw` with readback). The ray count
//! is the atomic the frame shader increments, not a stored cap.

use std::sync::OnceLock;

use genos_math::optics::ReflectionPreset;
use genos_render::{grade_camera, grade_world, Renderer, GRADE_HEIGHT, GRADE_WIDTH};
use genos_window::Window;

/// BGRA readback of the Medium grade scene, and the dispatch counts.
pub struct Shot {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Potato, Low, Medium, High.
    pub rays: [u32; 4],
    pub views: [u32; 4],
    /// Max mean-channel change between still Medium frames.
    pub flicker: f64,
    pub gpu_ns: u64,
    pub vram_bytes: u64,
}

/// The grade frame. The first call opens the proof window.
///
/// # Errors
///
/// Returns an error when the compositor or the device refuses the window.
pub fn shot() -> Result<&'static Shot, String> {
    static SHOT: OnceLock<Result<Shot, String>> = OnceLock::new();
    match SHOT.get_or_init(capture) {
        Ok(shot) => Ok(shot),
        Err(err) => Err(err.clone()),
    }
}

fn capture() -> Result<Shot, String> {
    let mut window = Window::open_proof(GRADE_WIDTH, GRADE_HEIGHT)?;
    let frame = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    let camera = grade_camera();
    let mut frames = Vec::new();
    let mut medium_grade = None;
    for _ in 0..3 {
        let _ = window.pump();
        let pixels = renderer
            .draw(&grade_world(ReflectionPreset::Medium), &camera, true)?
            .ok_or("the grade frame did not read back")?;
        medium_grade = Some(renderer.reflection_grade()?);
        frames.push(pixels);
    }
    let flicker = match (frames.first(), frames.get(1), frames.get(2)) {
        (Some(first), Some(second), Some(third)) => {
            frame_delta(first, second).max(frame_delta(second, third))
        }
        _ => 0.0,
    };
    let pixels = frames.pop().ok_or("the grade frame did not read back")?;
    let medium = medium_grade.ok_or("the grade frame did not record a ray count")?;
    let mut rays = [0; 4];
    let mut views = [0; 4];
    let presets = [
        ReflectionPreset::Potato,
        ReflectionPreset::Low,
        ReflectionPreset::Medium,
        ReflectionPreset::High,
    ];
    for (index, preset) in presets.into_iter().enumerate() {
        if preset == ReflectionPreset::Medium {
            rays[index] = medium.rays;
            views[index] = medium.views;
            continue;
        }
        let _ = window.pump();
        renderer.draw(&grade_world(preset), &camera, true)?;
        let grade = renderer.reflection_grade()?;
        rays[index] = grade.rays;
        views[index] = grade.views;
    }
    Ok(Shot {
        pixels,
        width: renderer.width(),
        height: renderer.height(),
        rays,
        views,
        flicker,
        gpu_ns: medium.gpu_ns,
        vram_bytes: medium.vram_bytes,
    })
}

fn frame_delta(a: &[u8], b: &[u8]) -> f64 {
    let count = a.len().min(b.len()) / 4;
    let mut max = 0.0_f64;
    for index in 0..count {
        let at = index * 4;
        let luma = |bytes: &[u8]| {
            (f64::from(bytes[at + 2]) + f64::from(bytes[at + 1]) + f64::from(bytes[at]))
                / (3.0 * 255.0)
        };
        max = max.max((luma(a) - luma(b)).abs());
    }
    max
}

/// One pixel of a BGRA readback, linear 0..1. Out of range is black.
#[must_use]
pub fn sample(shot: &Shot, x: u32, y: u32) -> [f64; 3] {
    if x >= shot.width || y >= shot.height {
        return [0.0; 3];
    }
    let index = usize::try_from((y * shot.width + x) * 4).unwrap_or(0);
    if index + 2 >= shot.pixels.len() {
        return [0.0; 3];
    }
    [
        f64::from(shot.pixels[index + 2]) / 255.0,
        f64::from(shot.pixels[index + 1]) / 255.0,
        f64::from(shot.pixels[index]) / 255.0,
    ]
}
