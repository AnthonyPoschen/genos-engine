//! One lavapipe timestamp span for a fog sample cap.
//!
//! A missing window leaves the millisecond rows unimplemented. The numbers are
//! lavapipe, not an RTX 4070.

use std::cell::RefCell;

use genos_render::{fit_pass, LightingPreset, PassCost, Renderer};
use genos_window::Window;

struct Session {
    renderer: Renderer,
    window: Window,
}

thread_local! {
    static GPU: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Fitted pass time. `None` when this process has no lavapipe window.
pub(crate) fn fitted_cost(preset: LightingPreset) -> Option<PassCost> {
    let mut missing = false;
    let cost = fit_pass(preset, |groups| {
        if let Some(milliseconds) = time_groups(groups) {
            milliseconds
        } else {
            missing = true;
            0.0
        }
    });
    if missing { None } else { Some(cost) }
}

fn time_groups(groups: u32) -> Option<f64> {
    GPU.with(|slot| {
        let mut guard = slot.borrow_mut();
        if guard.is_none() {
            *guard = Session::open();
        }
        let session = guard.as_mut()?;
        if session.window.display.is_null() {
            return None;
        }
        session.renderer.media_dispatch_ms(groups).ok()
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
