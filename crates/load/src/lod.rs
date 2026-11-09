//! CPU selection for a cooked discrete LOD chain.
//!
//! Geometry LOD stage 1 ([`docs/systems/geometry-lod.md`](../../../docs/systems/geometry-lod.md)).
//! The cook writes the chain. This module only projects the stored object-space
//! error and picks a level. It does not simplify.
//!
//! The projected error is `e * s * H / (2 d tan(fov/2))`. `s` is an upper bound
//! on the instance transform's operator norm (the largest singular value of
//! the 3×3). `d` is the view-space depth of the nearest
//! point of the instance bounds (`forward · (corner - eye)`), the same depth
//! the perspective divide uses, not the Euclidean distance to the centre.
//! A level is eligible when that value is under `τ` (`LodView::error_px`).
//! The pick is the coarsest eligible level, then shifted by `LodView::bias`
//! (Low is +1).
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

/// Object-space bounds and the column-major object-to-world transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodInstance {
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub transform: [f32; 16],
}

/// The view the screen-error metric reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodView {
    /// View-space depth of the nearest bounds point, metres (`d`).
    /// Positive in front of the camera. Not the distance to the centre.
    pub depth_m: f32,
    /// Upper bound on the operator norm of the instance 3×3. Multiplies `error_m`.
    pub scale: f32,
    /// Viewport height, pixels (`H`).
    pub height_px: f32,
    /// Vertical field of view, radians.
    pub fov_y_rad: f32,
    /// Screen-error budget, pixels (`τ`, `lod_error_px`).
    pub error_px: f32,
    /// Levels toward coarser after the error pick. Low is [`LOD_BIAS_LOW`].
    pub bias: i32,
}

/// Upper bound on the operator norm of the column-major 3×3.
///
/// The value is the largest singular value, `sqrt` of the largest eigenvalue
/// of `MᵀM`. A column length is that norm for a pure scale or reflection, and
/// it is short of the norm once a column is sheared into another.
#[must_use]
pub fn max_axis_scale(transform: &[f32; 16]) -> f32 {
    let c0 = column3(transform, 0);
    let c1 = column3(transform, 4);
    let c2 = column3(transform, 8);
    if [c0, c1, c2]
        .iter()
        .any(|column| column.iter().any(|axis| !axis.is_finite()))
    {
        return f32::INFINITY;
    }
    let lambda = largest_eigenvalue(
        dot3(c0, c0),
        dot3(c0, c1),
        dot3(c0, c2),
        dot3(c1, c1),
        dot3(c1, c2),
        dot3(c2, c2),
    );
    if !lambda.is_finite() {
        return f32::INFINITY;
    }
    lambda.max(0.0).sqrt()
}

fn column3(transform: &[f32; 16], base: usize) -> [f32; 3] {
    [
        transform.get(base).copied().unwrap_or(0.0),
        transform.get(base + 1).copied().unwrap_or(0.0),
        transform.get(base + 2).copied().unwrap_or(0.0),
    ]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Largest eigenvalue of a symmetric 3×3, via the trigonometric form of the
/// cubic (Kopp, "Efficient numerical diagonalization of hermitian 3×3 matrices").
/// Off-diagonals are `ab`, `ac`, `bc`.
fn largest_eigenvalue(aa: f32, ab: f32, ac: f32, bb: f32, bc: f32, cc: f32) -> f32 {
    let off = ab * ab + ac * ac + bc * bc;
    if off <= 0.0 {
        return aa.max(bb).max(cc).max(0.0);
    }
    let mean = (aa + bb + cc) * (1.0 / 3.0);
    let p2 = (aa - mean) * (aa - mean)
        + (bb - mean) * (bb - mean)
        + (cc - mean) * (cc - mean)
        + 2.0 * off;
    if p2 <= 0.0 || !p2.is_finite() {
        return mean.max(0.0);
    }
    let scale = (p2 * (1.0 / 6.0)).sqrt();
    if scale <= 0.0 || !scale.is_finite() {
        return mean.max(0.0);
    }
    let b00 = (aa - mean) / scale;
    let b11 = (bb - mean) / scale;
    let b22 = (cc - mean) / scale;
    let b01 = ab / scale;
    let b02 = ac / scale;
    let b12 = bc / scale;
    let det = b00 * (b11 * b22 - b12 * b12) - b01 * (b01 * b22 - b12 * b02)
        + b02 * (b01 * b12 - b11 * b02);
    let half = (det * 0.5).clamp(-1.0, 1.0);
    let phi = half.acos() * (1.0 / 3.0);
    let two_p = 2.0 * scale;
    let third = std::f32::consts::TAU / 3.0;
    let e0 = mean + two_p * phi.cos();
    let e1 = mean + two_p * (phi + third).cos();
    let e2 = mean + two_p * (phi + 2.0 * third).cos();
    e0.max(e1).max(e2).max(0.0)
}

/// Minimum `forward · (world - eye)` over the transformed bounds corners.
///
/// `forward` is the camera's look direction, unit length, matching the view
/// matrix in `camera.rs` where view-space depth is `-view_z`. A bounds that
/// crosses the camera plane has depth 0. A non-finite corner has depth 0, so
/// the projection stays on the finest level.
#[must_use]
pub fn nearest_view_depth(eye: [f32; 3], forward: [f32; 3], instance: LodInstance) -> f32 {
    let mut min_d = f32::INFINITY;
    let mut max_d = f32::NEG_INFINITY;
    let xs = [instance.bounds_min[0], instance.bounds_max[0]];
    let ys = [instance.bounds_min[1], instance.bounds_max[1]];
    let zs = [instance.bounds_min[2], instance.bounds_max[2]];
    for z in zs {
        for y in ys {
            for x in xs {
                let world = crate::transform_point(&instance.transform, [x, y, z]);
                if world.iter().any(|axis| !axis.is_finite()) {
                    return 0.0;
                }
                let depth = forward[0] * (world[0] - eye[0])
                    + forward[1] * (world[1] - eye[1])
                    + forward[2] * (world[2] - eye[2]);
                min_d = min_d.min(depth);
                max_d = max_d.max(depth);
            }
        }
    }
    if !min_d.is_finite() || !max_d.is_finite() {
        return 0.0;
    }
    if min_d <= 0.0 && max_d > 0.0 {
        return 0.0;
    }
    min_d
}

/// Depth and scale for one instance. `forward` is a unit look direction.
#[must_use]
pub fn view_for_instance(
    eye: [f32; 3],
    forward: [f32; 3],
    height_px: f32,
    fov_y_rad: f32,
    error_px: f32,
    bias: i32,
    instance: LodInstance,
) -> LodView {
    LodView {
        depth_m: nearest_view_depth(eye, forward, instance),
        scale: max_axis_scale(&instance.transform),
        height_px,
        fov_y_rad,
        error_px,
        bias,
    }
}

/// `e * H / (2 d tan(fov/2))`, in pixels. `d` is view-space depth.
///
/// A non-positive error projects to 0. A non-positive depth or field of
/// view projects a positive error to infinity, so only an error-free level is
/// eligible. Scale the object-space error by [`max_axis_scale`] before calling.
#[must_use]
pub fn screen_error_px(error_m: f32, depth_m: f32, height_px: f32, fov_y_rad: f32) -> f32 {
    if error_m <= 0.0 {
        return 0.0;
    }
    let denom = 2.0 * depth_m * (fov_y_rad * 0.5).tan();
    if !error_m.is_finite() || !height_px.is_finite() || !denom.is_finite() || denom <= 0.0 {
        return f32::INFINITY;
    }
    error_m * height_px / denom
}

fn world_error(error_m: f32, scale: f32) -> f32 {
    if error_m <= 0.0 {
        return 0.0;
    }
    if !error_m.is_finite() || !scale.is_finite() || scale < 0.0 {
        return f32::INFINITY;
    }
    error_m * scale
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
        let screen = screen_error_px(
            world_error(error, view.scale),
            view.depth_m,
            view.height_px,
            view.fov_y_rad,
        );
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
        if screen_error_px(
            world_error(error, view.scale),
            view.depth_m,
            view.height_px,
            view.fov_y_rad,
        ) < tau
        {
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

#[cfg(test)]
mod tests {
    use super::{
        max_axis_scale, nearest_view_depth, screen_error_px, select_lod, view_for_instance,
        LodInstance, LodView,
    };

    const EYE: [f32; 3] = [0.0, 0.0, 0.0];
    const FORWARD: [f32; 3] = [0.0, 0.0, -1.0];
    const FOV_60: f32 = std::f32::consts::FRAC_PI_3;

    fn translate_scale(z: f32, scale: f32) -> [f32; 16] {
        [
            scale, 0.0, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, z, 1.0,
        ]
    }

    fn identity() -> [f32; 16] {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    #[test]
    fn off_axis_at_60_deg_uses_nearest_depth_not_the_centre() {
        let instance = LodInstance {
            bounds_min: [1700.0, -1.0, -800.0],
            bounds_max: [1900.0, 1.0, -600.0],
            transform: identity(),
        };
        let view = view_for_instance(EYE, FORWARD, 1000.0, FOV_60, 1.0, 0, instance);
        let centre = [1800.0_f32, 0.0, -700.0];
        let centre_distance = (centre[0] * centre[0] + centre[2] * centre[2]).sqrt();
        assert!((view.depth_m - 600.0).abs() < 1.0e-3);
        assert!(centre_distance > view.depth_m * 2.0);
        let errors = [0.0, 0.5, 2.0];
        let by_depth = select_lod(&errors, view, None);
        let by_centre = select_lod(
            &errors,
            LodView {
                depth_m: centre_distance,
                ..view
            },
            None,
        );
        assert_eq!(by_depth, 1);
        assert_eq!(by_centre, 2);
        let tan_half = (FOV_60 * 0.5).tan();
        let at_depth = 2.0 * 1000.0 / (2.0 * view.depth_m * tan_half);
        let at_centre = 2.0 * 1000.0 / (2.0 * centre_distance * tan_half);
        assert!((at_depth - screen_error_px(2.0, view.depth_m, 1000.0, FOV_60)).abs() < 1.0e-3);
        assert!(at_depth > 1.0);
        assert!(at_centre < 1.0);
    }

    #[test]
    fn a_scaled_instance_multiplies_the_object_space_error() {
        let bounds_min = [-0.01, -0.01, -0.01];
        let bounds_max = [0.01, 0.01, 0.01];
        let unit_xform = translate_scale(-3000.0, 1.0);
        let grown_xform = translate_scale(-3000.0, 4.0);
        let unit = view_for_instance(
            EYE,
            FORWARD,
            1000.0,
            FOV_60,
            1.0,
            0,
            LodInstance {
                bounds_min,
                bounds_max,
                transform: unit_xform,
            },
        );
        let grown = view_for_instance(
            EYE,
            FORWARD,
            1000.0,
            FOV_60,
            1.0,
            0,
            LodInstance {
                bounds_min,
                bounds_max,
                transform: grown_xform,
            },
        );
        assert!((max_axis_scale(&grown_xform) - 4.0).abs() < 1.0e-5);
        assert!((unit.depth_m - grown.depth_m).abs() < 0.1);
        assert!(
            (nearest_view_depth(
                EYE,
                FORWARD,
                LodInstance {
                    bounds_min,
                    bounds_max,
                    transform: grown_xform,
                },
            ) - grown.depth_m)
                .abs()
                < 1.0e-4
        );
        assert!((unit.scale - 1.0).abs() < 1.0e-5);
        assert!((grown.scale - 4.0).abs() < 1.0e-5);
        let errors = [0.0, 2.0];
        assert_eq!(select_lod(&errors, unit, None), 1);
        assert_eq!(select_lod(&errors, grown, None), 0);
    }

    fn column_length(transform: &[f32; 16]) -> f32 {
        let column = |base: usize| {
            let x = transform[base];
            let y = transform[base + 1];
            let z = transform[base + 2];
            (x * x + y * y + z * z).sqrt()
        };
        column(0).max(column(4)).max(column(8))
    }

    #[test]
    fn a_shear_uses_the_operator_norm_not_the_longest_column() {
        // Columns (1,0,0), (1,1,0), (0,0,1). Longest column is √2.
        // Singular value is √((3+√5)/2) = φ ≈ 1.618, which is larger.
        let z = -1400.0;
        let shear = [
            1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, z, 1.0,
        ];
        let scale = max_axis_scale(&shear);
        let column = column_length(&shear);
        let expected = f32::midpoint(3.0, 5.0_f32.sqrt()).sqrt();
        assert!(
            (scale - expected).abs() < 1.0e-4,
            "operator norm {scale}, expected {expected}"
        );
        assert!(
            scale > column + 0.1,
            "operator norm {scale} must clear the longest column {column}"
        );
        let tan_half = (FOV_60 * 0.5).tan();
        let depth = f32::midpoint(column, scale) * 1000.0 / (2.0 * tan_half);
        let view = view_for_instance(
            EYE,
            FORWARD,
            1000.0,
            FOV_60,
            1.0,
            0,
            LodInstance {
                bounds_min: [0.0, 0.0, 0.0],
                bounds_max: [0.0, 0.0, 0.0],
                transform: [
                    1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -depth,
                    1.0,
                ],
            },
        );
        assert!((view.scale - scale).abs() < 1.0e-4);
        let errors = [0.0, 1.0];
        assert_eq!(select_lod(&errors, view, None), 0);
        assert_eq!(
            select_lod(
                &errors,
                LodView {
                    scale: column,
                    ..view
                },
                None,
            ),
            1
        );
    }

    #[test]
    fn depth_at_or_behind_the_camera_stays_on_the_finest_level() {
        let errors = [0.0, 0.1, 1.0, 10.0];
        let behind = LodInstance {
            bounds_min: [0.0, 0.0, 1.0],
            bounds_max: [1.0, 1.0, 2.0],
            transform: identity(),
        };
        let behind_view = view_for_instance(EYE, FORWARD, 1000.0, FOV_60, 1.0, 0, behind);
        assert!(behind_view.depth_m <= 0.0);
        assert_eq!(select_lod(&errors, behind_view, None), 0);
        assert_eq!(select_lod(&[1.0, 10.0], behind_view, None), 0);

        let crossing = LodInstance {
            bounds_min: [-1.0, -1.0, -1.0],
            bounds_max: [1.0, 1.0, 1.0],
            transform: identity(),
        };
        let crossing_view = view_for_instance(EYE, FORWARD, 1000.0, FOV_60, 1.0, 0, crossing);
        assert_eq!(crossing_view.depth_m, 0.0);
        assert_eq!(select_lod(&errors, crossing_view, None), 0);
    }
}
