//! Specular optics the reflection passes call.
//!
//! `crates/render/shaders/reflections.glsl` is the same formulas in GLSL.
//! Inputs are `f32`. Directions passed to [`refract_dir`] are unit length.
//! Portal bases are orthonormal, so the inverse basis is the transpose.

use crate::mat::{Mat3, Mat4};
use crate::vec::{Vec3, Vec4};

/// A further mirror or portal view is drawn while the running Fresnel product
/// stays at least this large and the view cap still has room.
pub const MIRROR_FRESNEL_CUTOFF: f32 = 0.02;

/// Wave height is clamped to this fraction of the water depth.
pub const SHORE_CAP: f32 = 0.75;

/// Index of refraction used for a water surface.
pub const WATER_IOR: f32 = 1.333;

/// Fallback, Low, Medium, or High. The ray and view caps are the preset table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReflectionPreset {
    Fallback = 0,
    Low = 1,
    Medium = 2,
    High = 3,
}

/// One Gerstner component. `direction` is the horizontal travel axis.
#[derive(Clone, Copy, Debug)]
pub struct GerstnerWave {
    pub direction: [f32; 2],
    pub amplitude: f32,
    pub wavelength: f32,
    pub speed: f32,
    pub steepness: f32,
}

/// One portal and the opening it draws.
#[derive(Clone, Copy, Debug)]
pub struct PortalLink {
    pub from_origin: Vec3,
    pub from_basis: Mat3,
    pub to_origin: Vec3,
    pub to_basis: Mat3,
}

/// `I - 2 dot(N, I) N`. `incident` points at the surface.
#[must_use]
pub fn mirror_dir(incident: Vec3, normal: Vec3) -> Vec3 {
    let along = normal.dot(incident);
    incident - normal * (2.0 * along)
}

/// Reflect `point` through a plane through the origin with unit `normal`.
#[must_use]
pub fn mirror_point(point: Vec3, normal: Vec3) -> Vec3 {
    let along = normal.dot(point);
    point - normal * (2.0 * along)
}

/// Unpolarized dielectric Fresnel.
///
/// `eta_t_over_eta_i` is `n_transmitted / n_incident`. From under water into
/// air that ratio is `1 / 1.333`. Total internal reflection returns 1.
#[must_use]
pub fn fresnel_dielectric(eta_t_over_eta_i: f32, cos_i: f32) -> f32 {
    let cos_i = cos_i.clamp(0.0, 1.0);
    let sin2_i = (1.0 - cos_i * cos_i).max(0.0);
    let sin2_t = sin2_i / (eta_t_over_eta_i * eta_t_over_eta_i);
    if sin2_t >= 1.0 {
        return 1.0;
    }
    let cos_t = (1.0 - sin2_t).sqrt();
    let eta = eta_t_over_eta_i;
    let rs = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    let rp = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    // The GLSL mirror is `0.5 * (rs * rs + rp * rp)`. `f32::midpoint` is a different expression.
    #[expect(
        clippy::manual_midpoint,
        reason = "keep the same product the GLSL Fresnel mirror uses"
    )]
    let fresnel = 0.5 * (rs * rs + rp * rp);
    fresnel
}

/// Snell direction. `eta` is `n_incident / n_transmitted`.
///
/// `None` is total internal reflection.
#[must_use]
pub fn refract_dir(incident: Vec3, normal: Vec3, eta: f32) -> Option<Vec3> {
    let along = normal.dot(incident);
    let k = 1.0 - eta * eta * (1.0 - along * along);
    if k < 0.0 {
        return None;
    }
    let scale = eta * along + k.sqrt();
    Some(Vec3::new(
        eta * incident.x - scale * normal.x,
        eta * incident.y - scale * normal.y,
        eta * incident.z - scale * normal.z,
    ))
}

/// GGX normal distribution. `alpha` is `roughness²`. The projected integral is 1.
#[must_use]
pub fn ggx_ndf(n_dot_h: f32, alpha: f32) -> f32 {
    let mu = n_dot_h.clamp(0.0, 1.0);
    let a2 = alpha * alpha;
    let den = mu * mu * (a2 - 1.0) + 1.0;
    a2 / (std::f32::consts::PI * den * den)
}

/// Beer–Lambert transmittance `exp(-sigma * distance)`.
#[must_use]
pub fn beer(sigma: f32, distance: f32) -> f32 {
    (-sigma * distance).exp()
}

/// Flat-water transmittance at normal incidence, `1 - F`, exact Fresnel.
#[must_use]
pub fn flat_water_transmittance(ior: f32) -> f32 {
    1.0 - fresnel_dielectric(ior, 1.0)
}

/// `to_origin + to_basis * flip_z * transpose(from_basis) * (point - from_origin)`.
#[must_use]
pub fn portal_point(link: PortalLink, point: Vec3) -> Vec3 {
    let local = transposed(link.from_basis).transform(point - link.from_origin);
    let flipped = Vec3::new(local.x, local.y, -local.z);
    link.to_origin + link.to_basis.transform(flipped)
}

/// How many recursive views the Fresnel product and the cap allow.
///
/// Each entry of `fresnel` is the reflectance of the next surface. The product
/// starts at 1. A view is emitted when the product stays at least `cutoff` and
/// the count is still under `cap`.
#[must_use]
pub fn mirror_views(fresnel: &[f32], cap: u32, cutoff: f32) -> u32 {
    let mut weight: f32 = 1.0;
    let mut emitted: u32 = 0;
    for reflectance in fresnel {
        if emitted >= cap {
            break;
        }
        weight *= *reflectance;
        if weight < cutoff {
            break;
        }
        emitted += 1;
    }
    emitted
}

/// Reflection rays the preset schedules. Fallback and Low stay at zero.
#[must_use]
pub fn reflection_ray_cap(preset: ReflectionPreset) -> u32 {
    match preset {
        ReflectionPreset::Fallback | ReflectionPreset::Low => 0,
        ReflectionPreset::Medium => 32_000,
        ReflectionPreset::High => 128_000,
    }
}

/// Extra mirror and portal views the preset schedules.
#[must_use]
pub fn reflection_view_cap(preset: ReflectionPreset) -> u32 {
    match preset {
        ReflectionPreset::Fallback => 0,
        ReflectionPreset::Low => 1,
        ReflectionPreset::Medium => 2,
        ReflectionPreset::High => 3,
    }
}

/// `true` when `point` is on the camera side of a plane through `origin`.
#[must_use]
pub fn plane_in_front(point: Vec3, origin: Vec3, normal: Vec3) -> bool {
    normal.dot(point - origin) >= 0.0
}

/// Move the near plane of a Vulkan projection onto `plane`.
///
/// `plane` is camera space, `nx x + ny y + nz z + d = 0`, with the camera on
/// the positive side. Clip Z is 0..1, so the third row is `c` and the scale
/// is 1. Lengyel's listing subtracts the fourth row and uses 2, because that
/// lands the plane on the OpenGL near plane `z/w = -1`. `sgn(0)` is +1.
#[must_use]
pub fn oblique_projection(projection: Mat4, plane: Vec4) -> Mat4 {
    let inverse = inverse_mat4(projection);
    let corner = Vec4::new(sgn(plane.x), sgn(plane.y), 1.0, 1.0);
    let q = mul_mat_vec(inverse, corner);
    let denom = plane.dot(q);
    let scale = if denom.abs() < 1.0e-12 {
        0.0
    } else {
        1.0 / denom
    };
    let mut out = projection;
    set_row(&mut out, 2, plane * scale);
    out
}

/// The third row of [`oblique_projection`], column-major storage.
#[must_use]
pub fn oblique_clip_row(projection: Mat4, plane: Vec4) -> [f32; 4] {
    let row = mat_row(oblique_projection(projection, plane), 2);
    [row.x, row.y, row.z, row.w]
}

/// `true` when `view_point` survives the oblique near plane.
#[must_use]
pub fn oblique_draws(projection: Mat4, plane: Vec4, view_point: Vec3) -> bool {
    let clip = mul_mat_vec(
        oblique_projection(projection, plane),
        Vec4::new(view_point.x, view_point.y, view_point.z, 1.0),
    );
    if clip.w <= 1.0e-6 {
        return false;
    }
    let z = clip.z / clip.w;
    (0.0..=1.0).contains(&z)
}

/// Vertical displacement and the unit normal of a Gerstner sum.
///
/// The returned point is the displacement added to `(xz.0, 0, xz.1)`.
#[must_use]
pub fn gerstner(waves: &[GerstnerWave], xz: [f32; 2], time: f32) -> (Vec3, Vec3) {
    let mut point = Vec3::new(xz[0], 0.0, xz[1]);
    let mut normal = Vec3::Y;
    for wave in waves {
        let axis = wave_axis(*wave);
        let wave_number = std::f32::consts::TAU / wave.wavelength.max(1.0e-4);
        let phase = wave_number * (axis[0] * xz[0] + axis[1] * xz[1] - wave.speed * time);
        let sine = phase.sin();
        let cosine = phase.cos();
        let steep = wave.steepness;
        let amplitude = wave.amplitude;
        point.x += steep * amplitude * axis[0] * cosine;
        point.y += amplitude * sine;
        point.z += steep * amplitude * axis[1] * cosine;
        let slope = wave_number * amplitude;
        normal.x -= axis[0] * slope * cosine;
        normal.y -= steep * slope * sine;
        normal.z -= axis[1] * slope * cosine;
    }
    (point, normal.normalize())
}

/// `min(|deep|, cap * depth)`, with the sign of `deep`.
#[must_use]
pub fn shoreline_height(deep: f32, depth: f32, cap: f32) -> f32 {
    let limit = (cap * depth).max(0.0);
    deep.clamp(-limit, limit)
}

/// `F * reflected + (1 - F) * transmitted * exp(-sigma * distance)`.
#[must_use]
pub fn water_shade(
    fresnel: f32,
    reflected: Vec3,
    transmitted: Vec3,
    sigma: f32,
    distance: f32,
) -> Vec3 {
    let weight = fresnel.clamp(0.0, 1.0);
    let absorb = beer(sigma, distance);
    reflected * weight + transmitted * ((1.0 - weight) * absorb)
}

/// Direction through a portal. The translation of the link cancels.
#[must_use]
pub fn portal_dir(link: PortalLink, direction: Vec3) -> Vec3 {
    portal_point(link, direction) - portal_point(link, Vec3::ZERO)
}

fn wave_axis(wave: GerstnerWave) -> [f32; 2] {
    let len =
        (wave.direction[0] * wave.direction[0] + wave.direction[1] * wave.direction[1]).sqrt();
    if len <= 1.0e-8 {
        return [1.0, 0.0];
    }
    [wave.direction[0] / len, wave.direction[1] / len]
}

fn sgn(value: f32) -> f32 {
    if value < 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn mat_row(m: Mat4, row: usize) -> Vec4 {
    let c = &m.cols;
    Vec4::new(c[row], c[4 + row], c[8 + row], c[12 + row])
}

fn set_row(m: &mut Mat4, row: usize, value: Vec4) {
    m.cols[row] = value.x;
    m.cols[4 + row] = value.y;
    m.cols[8 + row] = value.z;
    m.cols[12 + row] = value.w;
}

fn mul_mat_vec(m: Mat4, v: Vec4) -> Vec4 {
    let c = &m.cols;
    Vec4::new(
        c[0] * v.x + c[4] * v.y + c[8] * v.z + c[12] * v.w,
        c[1] * v.x + c[5] * v.y + c[9] * v.z + c[13] * v.w,
        c[2] * v.x + c[6] * v.y + c[10] * v.z + c[14] * v.w,
        c[3] * v.x + c[7] * v.y + c[11] * v.z + c[15] * v.w,
    )
}

/// Gauss–Jordan inverse. A singular matrix returns [`Mat4::IDENTITY`].
fn inverse_mat4(m: Mat4) -> Mat4 {
    let mut aug = [[0.0_f32; 8]; 4];
    for (row, line) in aug.iter_mut().enumerate() {
        for (col, cell) in line.iter_mut().enumerate().take(4) {
            *cell = m.cols[col * 4 + row];
        }
        line[4 + row] = 1.0;
    }
    for col in 0..4 {
        let mut pivot = col;
        let mut best = aug[col][col].abs();
        for (row, line) in aug.iter().enumerate().skip(col + 1) {
            let value = line[col].abs();
            if value > best {
                best = value;
                pivot = row;
            }
        }
        if best < 1.0e-12 {
            return Mat4::IDENTITY;
        }
        aug.swap(col, pivot);
        let div = aug[col][col];
        for cell in &mut aug[col] {
            *cell /= div;
        }
        let pivot_row = aug[col];
        for (row, line) in aug.iter_mut().enumerate() {
            if row == col {
                continue;
            }
            let factor = line[col];
            for (index, cell) in line.iter_mut().enumerate() {
                *cell -= factor * pivot_row[index];
            }
        }
    }
    let mut cols = [0.0; 16];
    for (row, line) in aug.iter().enumerate() {
        for col in 0..4 {
            cols[col * 4 + row] = line[4 + col];
        }
    }
    Mat4::from_cols(cols)
}

/// Vulkan perspective used by the oblique-clip parity row. Y is flipped.
#[must_use]
pub fn parity_projection() -> Mat4 {
    let h = 1.0 / (60.0_f32.to_radians() * 0.5).tan();
    let near = 0.05_f32;
    let far = 200.0_f32;
    let mut cols = [0.0; 16];
    cols[0] = h;
    cols[5] = -h;
    cols[10] = far / (near - far);
    cols[11] = -1.0;
    cols[14] = (far * near) / (near - far);
    Mat4::from_cols(cols)
}

fn transposed(basis: Mat3) -> Mat3 {
    let c = basis.cols;
    Mat3::from_cols([c[0], c[3], c[6], c[1], c[4], c[7], c[2], c[5], c[8]])
}

#[cfg(test)]
mod tests {
    use super::{
        flat_water_transmittance, fresnel_dielectric, gerstner, mirror_views, oblique_draws,
        parity_projection, plane_in_front, portal_point, reflection_ray_cap, reflection_view_cap,
        refract_dir, shoreline_height, water_shade, GerstnerWave, PortalLink, ReflectionPreset,
        MIRROR_FRESNEL_CUTOFF, SHORE_CAP, WATER_IOR,
    };
    use crate::{Mat3, Mat4, Vec3, Vec4};

    #[test]
    fn below_water_fresnel_is_total_internal_reflection() {
        let eta = 1.0_f32 / 1.333;
        let got = fresnel_dielectric(eta, 0.5);
        assert!((got - 1.0).abs() < 1.0e-6, "{got}");
        assert!(refract_dir(Vec3::new(0.95, -0.05, 0.0).normalize(), Vec3::Y, 1.333).is_none());
    }

    #[test]
    fn flat_water_matches_the_baked_transmittance() {
        let got = flat_water_transmittance(1.333);
        assert!((got - 0.979_626_8).abs() < 1.0e-5, "{got}");
    }

    #[test]
    fn fresnel_recursion_stops_on_weight_and_on_the_cap() {
        let water = [0.0204_f32, 0.0597];
        assert_eq!(mirror_views(&water, 0, MIRROR_FRESNEL_CUTOFF), 0);
        assert_eq!(mirror_views(&water, 3, MIRROR_FRESNEL_CUTOFF), 1);
        let bright = [0.5_f32; 5];
        assert_eq!(mirror_views(&bright, 3, MIRROR_FRESNEL_CUTOFF), 3);
    }

    #[test]
    fn an_axis_aligned_portal_flips_forward_and_translates() {
        let link = PortalLink {
            from_origin: Vec3::ZERO,
            from_basis: Mat3::IDENTITY,
            to_origin: Vec3::new(10.0, 0.0, 0.0),
            to_basis: Mat3::IDENTITY,
        };
        let got = portal_point(link, Vec3::new(1.0, 2.0, 3.0));
        assert!((got.x - 11.0).abs() < 1.0e-6);
        assert!((got.y - 2.0).abs() < 1.0e-6);
        assert!((got.z - -3.0).abs() < 1.0e-6);
    }

    #[test]
    fn low_schedules_no_reflection_rays_and_one_view() {
        assert_eq!(reflection_ray_cap(ReflectionPreset::Low), 0);
        assert_eq!(reflection_ray_cap(ReflectionPreset::Fallback), 0);
        assert_eq!(reflection_ray_cap(ReflectionPreset::Medium), 32_000);
        assert_eq!(reflection_view_cap(ReflectionPreset::Low), 1);
        assert_eq!(reflection_view_cap(ReflectionPreset::High), 3);
    }

    #[test]
    fn oblique_clip_rejects_the_back_of_the_plane() {
        // Reflected camera at z = -4 looking toward +Z. The mirror is z = 0.
        // View Z is negative in front of that camera, so world z maps to -z - 4.
        let projection = parity_projection();
        let plane = Vec4::new(0.0, 0.0, 1.0, 4.0);
        let front = [1.0_f32, 2.0, 6.0];
        let behind = [-0.5_f32, -1.0, -2.0];
        for world_z in front {
            let point = Vec3::new(0.0, 0.0, -world_z - 4.0);
            assert!(plane_in_front(
                Vec3::new(0.0, 0.0, world_z),
                Vec3::ZERO,
                Vec3::Z
            ));
            assert!(
                oblique_draws(projection, plane, point),
                "mirror side clipped world z={world_z} view={point:?}"
            );
        }
        for world_z in behind {
            let point = Vec3::new(0.0, 0.0, -world_z - 4.0);
            assert!(!plane_in_front(
                Vec3::new(0.0, 0.0, world_z),
                Vec3::ZERO,
                Vec3::Z
            ));
            assert!(
                !oblique_draws(projection, plane, point),
                "back side drawn world z={world_z} view={point:?}"
            );
        }
        let product = mul_check(projection, inverse_of(projection));
        for value in product {
            assert!(value.abs() < 1.0e-4, "inverse {product:?}");
        }
    }

    #[test]
    fn shoreline_clamp_holds_the_crest_to_the_cap() {
        let crest = gerstner(
            &[GerstnerWave {
                direction: [1.0, 0.0],
                amplitude: 0.4,
                wavelength: 8.0,
                speed: 0.0,
                steepness: 0.5,
            }],
            [2.0, 0.0],
            0.0,
        )
        .0
        .y;
        assert!((crest - 0.4).abs() < 1.0e-5, "{crest}");
        let shallow = shoreline_height(crest, 0.0625, SHORE_CAP);
        assert!((shallow / 0.0625 - SHORE_CAP).abs() < 1.0e-5, "{shallow}");
        assert!(shoreline_height(crest, 4.0, SHORE_CAP) < crest + 1.0e-6);
    }

    #[test]
    fn water_shade_darkens_a_miss_with_beer() {
        let fresnel = fresnel_dielectric(WATER_IOR, 1.0);
        let opaque = Vec3::new(0.2, 0.4, 0.6);
        let reflected = Vec3::new(0.9, 0.8, 0.7);
        let got = water_shade(fresnel, reflected, opaque, 0.5, 2.0);
        let absorb = (-0.5_f32 * 2.0).exp();
        let expect = reflected * fresnel + opaque * ((1.0 - fresnel) * absorb);
        assert!((got.x - expect.x).abs() < 1.0e-6);
        assert!((got.y - expect.y).abs() < 1.0e-6);
        assert!((got.z - expect.z).abs() < 1.0e-6);
    }

    fn inverse_of(m: Mat4) -> Mat4 {
        super::inverse_mat4(m)
    }

    fn mul_check(a: Mat4, b: Mat4) -> [f32; 16] {
        let product = a * b;
        let mut err = [0.0; 16];
        for (index, value) in product.cols.into_iter().enumerate() {
            let identity = if index % 5 == 0 { 1.0 } else { 0.0 };
            err[index] = value - identity;
        }
        err
    }
}
