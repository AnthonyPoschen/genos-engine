//! Specular optics the reflection passes call.
//!
//! `crates/render/shaders/reflections.glsl` is the same formulas in GLSL.
//! Inputs are `f32`. Directions passed to [`refract_dir`] are unit length.
//! Portal bases are orthonormal, so the inverse basis is the transpose.

use crate::mat::Mat3;
use crate::vec::Vec3;

/// A further mirror or portal view is drawn while the running Fresnel product
/// stays at least this large and the view cap still has room.
pub const MIRROR_FRESNEL_CUTOFF: f32 = 0.02;

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

fn transposed(basis: Mat3) -> Mat3 {
    let c = basis.cols;
    Mat3::from_cols([c[0], c[3], c[6], c[1], c[4], c[7], c[2], c[5], c[8]])
}

#[cfg(test)]
mod tests {
    use super::{
        flat_water_transmittance, fresnel_dielectric, mirror_views, portal_point, refract_dir,
        PortalLink, MIRROR_FRESNEL_CUTOFF,
    };
    use crate::{Mat3, Vec3};

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
}
