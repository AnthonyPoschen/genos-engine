//! One lavapipe device for froxel readback.
//!
//! A missing window leaves the GPU rows unimplemented. Times and bytes are
//! lavapipe, not an RTX 4070.

use std::cell::RefCell;

use genos_render::{
    column_peak, composed_frame_camera, composed_frame_scene, fit_pass, picture_mean_abs,
    picture_unchanged, FogDispatch, FogGpu, LightingPreset, PassCost, Renderer, World,
};
use genos_window::Window;

struct Session {
    renderer: Renderer,
    window: Window,
}

thread_local! {
    static GPU: RefCell<Option<Session>> = const { RefCell::new(None) };
    static FITS: RefCell<Vec<PassCost>> = const { RefCell::new(Vec::new()) };
}

/// Fitted pass time from froxel timestamps. `None` without a lavapipe window.
pub(crate) fn fitted_cost(preset: LightingPreset) -> Option<PassCost> {
    if !open() {
        return None;
    }
    let mut missing = false;
    let cost = fit_pass(preset, |samples| {
        if let Some(milliseconds) = sample_ms(preset, samples) {
            milliseconds
        } else {
            missing = true;
            0.0
        }
    });
    if missing {
        None
    } else {
        FITS.with(|slot| slot.borrow_mut().push(cost));
        Some(cost)
    }
}

/// Fitted passes recorded by the last good feature grade, in preset order.
#[must_use]
pub fn recorded_fits() -> Vec<PassCost> {
    FITS.with(|slot| slot.borrow().clone())
}

/// Full sample cap, not reduced. The broken millisecond row uses this.
pub(crate) fn unfitted_ms(preset: LightingPreset) -> Option<f64> {
    if !open() {
        return None;
    }
    let samples = genos_render::sample_cap(preset);
    sample_ms(preset, samples)
}

pub(crate) fn allocation_mb(preset: LightingPreset, doubled: bool) -> Option<f64> {
    if !open() {
        return None;
    }
    with_renderer(|renderer| {
        renderer.froxel_allocation(preset, doubled).map(|bytes| {
            let bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
            f64::from(bytes) / 1_000_000.0
        })
    })
}

/// Column error and picture-unchanged score for one Low frame.
/// `None` without a lavapipe window.
pub(crate) fn frame_grade() -> Option<(f64, f64)> {
    if !open() {
        return None;
    }
    with_renderer(grade_frame)
}

fn grade_frame(renderer: &mut Renderer) -> Result<(f64, f64), String> {
    let camera = composed_frame_camera();
    let on_world = World::from_scene(composed_frame_scene(true));
    let on = renderer
        .draw(&on_world, &camera, true)?
        .unwrap_or_default();
    let columns = renderer.frame_fog_columns()?;
    let scene = composed_frame_scene(true);
    let reference = renderer.frame_fog_reference(&scene);
    let column = column_peak(&columns, &reference);
    let off_world = World::from_scene(composed_frame_scene(false));
    let off = renderer
        .draw(&off_world, &camera, true)?
        .unwrap_or_default();
    let picture = picture_unchanged(&on, &off);
    println!(
        "frame grade column_abs {column:.6e} picture_unchanged {picture} picture_mean_abs {:.6e}",
        picture_mean_abs(&on, &off)
    );
    Ok((column, picture))
}

pub(crate) fn run(job: &FogDispatch) -> Option<FogGpu> {
    if !open() {
        return None;
    }
    with_renderer(|renderer| renderer.run_fog(job))
}

fn sample_ms(preset: LightingPreset, samples: u32) -> Option<f64> {
    with_renderer(|renderer| renderer.froxel_sample_ms(preset, samples))
}

fn open() -> bool {
    GPU.with(|slot| {
        let mut guard = slot.borrow_mut();
        if guard.is_none() {
            *guard = Session::open();
        }
        guard.as_ref().is_some_and(|session| !session.window.display.is_null())
    })
}

fn with_renderer<T>(f: impl FnOnce(&mut Renderer) -> Result<T, String>) -> Option<T> {
    GPU.with(|slot| {
        let mut guard = slot.borrow_mut();
        let session = guard.as_mut()?;
        f(&mut session.renderer).ok()
    })
}

impl Session {
    fn open() -> Option<Self> {
        let mut window = Window::open_proof(64, 64).ok()?;
        let frame = window.pump();
        let renderer =
            Renderer::open(window.display, window.surface, frame.width, frame.height).ok()?;
        Some(Self { renderer, window })
    }
}
