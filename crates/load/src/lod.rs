//! CPU selection for a cooked discrete LOD chain.
//!
//! Geometry LOD stage 1 ([`docs/systems/geometry-lod.md`](../../../docs/systems/geometry-lod.md)).
//! The cook writes the chain. This module only projects the stored object-space
//! error and picks a level. It does not simplify.
//!
//! The projected error is `e * H / (2 d tan(fov/2))`. A level is eligible when
//! that value is under `τ` (`LodView::error_px`). The pick is the coarsest
//! eligible level, then shifted by `LodView::bias` (Low is +1).
//!
//! A camera sitting on the switch distance would flip if the comparison were
//! stateless. [`select_lod`] keeps `current` until the projected error has
//! crossed `τ` by [`LOD_HYSTERESIS`] (10%). The design's 0.25 s dither fade is
//! the renderer blend and is not this function.
//!
//! Networking, from the geometry LOD design. Render-only.
//! - Replicated state: none. LOD choice is never replicated.
//! - Authority: the client that draws. The server does not pick a level.
//! - Prediction / rollback: none.
//! - Determinism: the same chain, distance, and settings pick the same level.
//! - Headless server: loads the sim subset and never LODs or this selector.

#![allow(
    clippy::module_name_repetitions,
    reason = "LodView and LOD_HYSTERESIS are the public geometry-lod names"
)]

/// Fraction of `τ` that the projected error must clear before the level changes.
pub const LOD_HYSTERESIS: f32 = 0.1;

/// High preset `lod_error_px`, pixels.
pub const LOD_ERROR_PX_HIGH: f32 = 1.0;
/// Medium preset `lod_error_px`, pixels.
pub const LOD_ERROR_PX_MED: f32 = 1.25;
/// Low preset `lod_error_px`, pixels.
pub const LOD_ERROR_PX_LOW: f32 = 2.0;
/// Low preset `lod_bias`: one level coarser than the error pick.
pub const LOD_BIAS_LOW: i32 = 1;

/// One cooked level. `error_m` is the object-space error against level 0, metres.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshLod {
    pub error_m: f32,
    pub primitives: Vec<crate::Primitive>,
}

/// The view the screen-error metric reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodView {
    /// Distance from the camera to the object, metres (`d`).
    pub distance_m: f32,
    /// Viewport height, pixels (`H`).
    pub height_px: f32,
    /// Vertical field of view, radians.
    pub fov_y_rad: f32,
    /// Screen-error budget, pixels (`τ`, `lod_error_px`).
    pub error_px: f32,
    /// Levels toward coarser after the error pick. Low is [`LOD_BIAS_LOW`].
    pub bias: i32,
}

/// `e * H / (2 d tan(fov/2))`, in pixels.
///
/// A non-positive error projects to 0. A non-positive distance or field of
/// view projects a positive error to infinity, so only an error-free level is
/// eligible.
#[must_use]
pub fn screen_error_px(error_m: f32, distance_m: f32, height_px: f32, fov_y_rad: f32) -> f32 {
    if error_m <= 0.0 {
        return 0.0;
    }
    let denom = 2.0 * distance_m * (fov_y_rad * 0.5).tan();
    if !error_m.is_finite() || !height_px.is_finite() || !denom.is_finite() || denom <= 0.0 {
        return f32::INFINITY;
    }
    error_m * height_px / denom
}

/// Coarsest level whose projected error is under `τ`, with hysteresis.
///
/// `errors_m` is the chain's object-space error, level 0 first. `current` is
/// the level already on screen. `None` picks with no hold.
#[must_use]
pub fn select_lod(errors_m: &[f32], view: LodView, current: Option<usize>) -> usize {
    if errors_m.is_empty() {
        return 0;
    }
    let nominal = pick(errors_m, view, view.error_px);
    let Some(held) = current else {
        return nominal;
    };
    let held = held.min(errors_m.len() - 1);
    if nominal == held {
        return held;
    }
    if nominal > held {
        let clear = pick(errors_m, view, view.error_px * (1.0 - LOD_HYSTERESIS));
        if clear > held {
            clear
        } else {
            held
        }
    } else {
        let error = errors_m.get(held).copied().unwrap_or(0.0);
        let screen = screen_error_px(error, view.distance_m, view.height_px, view.fov_y_rad);
        if screen >= view.error_px * (1.0 + LOD_HYSTERESIS) {
            nominal
        } else {
            held
        }
    }
}

fn pick(errors_m: &[f32], view: LodView, tau: f32) -> usize {
    let mut best = 0;
    for (index, error) in errors_m.iter().copied().enumerate() {
        if screen_error_px(error, view.distance_m, view.height_px, view.fov_y_rad) < tau {
            best = index;
        }
    }
    apply_bias(best, errors_m.len(), view.bias)
}

fn apply_bias(index: usize, len: usize, bias: i32) -> usize {
    let last = len.saturating_sub(1);
    let index = index.min(last);
    match bias.cmp(&0) {
        std::cmp::Ordering::Greater => {
            let shift = usize::try_from(bias).unwrap_or(last);
            index.saturating_add(shift).min(last)
        }
        std::cmp::Ordering::Less => {
            let shift = usize::try_from(bias.unsigned_abs()).unwrap_or(last);
            index.saturating_sub(shift)
        }
        std::cmp::Ordering::Equal => index,
    }
}
