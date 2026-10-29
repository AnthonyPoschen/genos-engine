//! CPU reference for one-sided emissive triangles (ADR 0030).
//!
//! glTF emission is radiance: `emissiveFactor × emissiveStrength`. A triangle
//! is sampled uniformly in area. The density the shading integral uses is the
//! solid-angle pdf `pdf_A * r² / cos θ_light` (Pharr, Jakob, and Humphreys,
//! *Physically Based Rendering*, area lights). The estimate is unshadowed
//! irradiance, `L_e * cos θ_receiver / pdf_ω`.
//!
//! These functions are `f64` and do not allocate. World up is Y; positions are
//! whatever frame the caller passes. There is no shader mirror and no `f32`
//! wrapper until a renderer calls this.

/// Emitted radiance of a glTF material, `emissiveFactor × emissiveStrength`.
///
/// Strength is 1 when `KHR_materials_emissive_strength` is absent. The product
/// is the same line as the metallic-roughness reference's emission, when that
/// reference is present. Channels are not clamped.
#[must_use]
pub fn emitted_radiance(emissive_factor: [f64; 3], emissive_strength: f64) -> [f64; 3] {
    emissive_factor.map(|channel| channel * emissive_strength)
}

/// One one-sided emissive triangle. `radiance` is [`emitted_radiance`].
///
/// The front face is the right-handed winding `v0 → v1 → v2`. The back face
/// emits nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub v0: [f64; 3],
    pub v1: [f64; 3],
    pub v2: [f64; 3],
    pub radiance: [f64; 3],
}

/// A point drawn uniformly in area, and `1 / area`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaSample {
    pub point: [f64; 3],
    pub pdf_area: f64,
}

/// Extents of a rectangle parallel to the receiver, in the receiver frame.
///
/// The receiver is at the local origin. Its normal is local +Z. The rectangle
/// lies in the plane `z = height` and spans `[min_x, max_x] × [min_y, max_y]`.
/// `height > 0` is in front of the receiver. This frame is not world space:
/// world up is Y, and the caller maps a ceiling panel into this frame (or
/// compares the scalar integral, which depends only on the extents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParallelRectangle {
    pub height: f64,
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
    pub radiance: [f64; 3],
}

impl Triangle {
    /// Triangle area. Zero when the vertices are colinear.
    #[must_use]
    pub fn area(self) -> f64 {
        0.5 * length(self.cross_edge())
    }

    /// Unit front-face normal. The zero vector when the area is zero.
    #[must_use]
    pub fn normal(self) -> [f64; 3] {
        let crossed = self.cross_edge();
        let len = length(crossed);
        if len <= f64::MIN_POSITIVE {
            [0.0; 3]
        } else {
            scale(crossed, 1.0 / len)
        }
    }

    /// Uniform area sample. `u` and `v` are in `[0, 1]`.
    ///
    /// Weights are `b0 = 1 - sqrt(u)`, `b1 = v * sqrt(u)`, `b2 = 1 - b0 - b1`
    /// (Shirley). Returns `None` for a degenerate triangle or for parameters
    /// outside `[0, 1]`.
    #[must_use]
    pub fn sample_area(self, u: f64, v: f64) -> Option<AreaSample> {
        let area = self.area();
        if area <= f64::MIN_POSITIVE || !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return None;
        }
        let along = u.sqrt();
        let b0 = 1.0 - along;
        let b1 = v * along;
        let b2 = 1.0 - b0 - b1;
        Some(AreaSample {
            point: [
                b0 * self.v0[0] + b1 * self.v1[0] + b2 * self.v2[0],
                b0 * self.v0[1] + b1 * self.v1[1] + b2 * self.v2[1],
                b0 * self.v0[2] + b1 * self.v1[2] + b2 * self.v2[2],
            ],
            pdf_area: 1.0 / area,
        })
    }

    fn cross_edge(self) -> [f64; 3] {
        cross(sub(self.v1, self.v0), sub(self.v2, self.v0))
    }
}

/// Solid-angle density of an area sample seen from `origin`.
///
/// `pdf_ω = pdf_area * r² / cos θ_light`, with `cos θ_light` the cosine on the
/// front face. `None` on the back face, at grazing (`cos θ_light ≤ 0`), or
/// when `point` is `origin`.
#[must_use]
pub fn solid_angle_pdf(
    origin: [f64; 3],
    point: [f64; 3],
    light_normal: [f64; 3],
    pdf_area: f64,
) -> Option<f64> {
    let toward_light = sub(point, origin);
    let distance_sq = dot(toward_light, toward_light);
    if distance_sq <= f64::MIN_POSITIVE || pdf_area <= 0.0 {
        return None;
    }
    let toward_unit = scale(toward_light, 1.0 / distance_sq.sqrt());
    let cos_light = -dot(light_normal, toward_unit);
    if cos_light <= 0.0 {
        None
    } else {
        Some(pdf_area * distance_sq / cos_light)
    }
}

/// Unshadowed irradiance at a receiver from one area sample.
///
/// `L_e * max(n · ω_i, 0) / pdf_ω`. Zero when the sample faces away, the
/// receiver faces away, or the solid-angle pdf is absent. `receiver_normal`
/// and `light_normal` are unit vectors. No visibility ray.
#[must_use]
pub fn irradiance_from_area_sample(
    receiver_position: [f64; 3],
    receiver_normal: [f64; 3],
    light_normal: [f64; 3],
    radiance: [f64; 3],
    point: [f64; 3],
    pdf_area: f64,
) -> [f64; 3] {
    let Some(pdf_omega) = solid_angle_pdf(receiver_position, point, light_normal, pdf_area) else {
        return [0.0; 3];
    };
    let toward_light = sub(point, receiver_position);
    let distance_sq = dot(toward_light, toward_light);
    if distance_sq <= f64::MIN_POSITIVE {
        return [0.0; 3];
    }
    let cos_receiver = dot(
        receiver_normal,
        scale(toward_light, 1.0 / distance_sq.sqrt()),
    );
    if cos_receiver <= 0.0 {
        return [0.0; 3];
    }
    radiance.map(|channel| channel * cos_receiver / pdf_omega)
}

/// Analytic unshadowed irradiance of a parallel Lambertian rectangle.
///
/// The geometric factor is the projected solid angle. For the corner
/// rectangle `(0, 0)`–`(x, y)` at distance `h` it is
///
/// ```text
/// ½ ( (x / √(x²+h²)) arctan(y / √(x²+h²)) + (y / √(y²+h²)) arctan(x / √(y²+h²)) )
/// ```
///
/// which is `π` times the view factor from a differential area to a parallel
/// rectangle. A plane that fills the front hemisphere tends to `π`. `height ≤ 0`
/// returns zero.
#[must_use]
pub fn parallel_rectangle_irradiance(rect: ParallelRectangle) -> [f64; 3] {
    if rect.height <= 0.0 {
        return [0.0; 3];
    }
    let factor = projected_corner(rect.max_x, rect.max_y, rect.height)
        - projected_corner(rect.min_x, rect.max_y, rect.height)
        - projected_corner(rect.max_x, rect.min_y, rect.height)
        + projected_corner(rect.min_x, rect.min_y, rect.height);
    rect.radiance.map(|channel| channel * factor)
}

/// Projected solid angle of the corner rectangle `(0, 0)`–`(x, y)`.
fn projected_corner(x: f64, y: f64, height: f64) -> f64 {
    let height_sq = height * height;
    let x_span = (x * x + height_sq).sqrt();
    let y_span = (y * y + height_sq).sqrt();
    // Average of the two view-factor terms. `midpoint` is that average.
    f64::midpoint(
        (x / x_span) * (y / x_span).atan(),
        (y / y_span) * (x / y_span).atan(),
    )
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], factor: f64) -> [f64; 3] {
    [a[0] * factor, a[1] * factor, a[2] * factor]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
