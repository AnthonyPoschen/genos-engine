//! Path-traced reference for the planar mirror, the portal, and flat water.
//!
//! This file does not call [`genos_render::shade_picture`] or
//! [`genos_math::optics::water_shade`]. It rebuilds the three tiles with
//! Whitted rays. The mirror reflects from the hit. The portal continues from
//! [`genos_math::optics::portal_point`]. Water is
//! `F * reflected + (1 - F) * transmitted * beer`. The Low view cap is the
//! Fresnel budget. A refused view keeps black, which is the opaque miss.

use genos_math::optics::{self, PortalLink, ReflectionPreset, MIRROR_FRESNEL_CUTOFF, WATER_IOR};
use genos_math::{Mat3, Vec3};

/// Pixels on a side of one tile. Matches the engine picture.
pub const TILE: u32 = 8;

const FOV: f32 = 60.0_f32.to_radians();
const MIRROR_EYE: Vec3 = Vec3::new(0.0, 1.0, 4.0);
const WATER_EYE: Vec3 = Vec3::new(0.0, 3.0, 0.5);
const SIGMA: f32 = 0.35;
const WALL_POS: [f32; 3] = [0.85, 0.12, 0.08];
const WALL_NEG: [f32; 3] = [0.10, 0.55, 0.20];
const PORTAL_HIGH: [f32; 3] = [0.15, 0.35, 0.90];
const PORTAL_LOW: [f32; 3] = [0.90, 0.70, 0.20];
const FLOOR: [f32; 3] = [0.08, 0.22, 0.72];
const CEILING: [f32; 3] = [0.72, 0.68, 0.55];

/// Linear colour and primary-hit distance, three tiles stacked.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub color: Vec<[f32; 3]>,
    pub depth: Vec<f32>,
}

/// One still frame. The camera does not move, so every frame is this image.
#[must_use]
pub fn trace() -> Reference {
    let width = TILE;
    let height = TILE * 3;
    let count = idx(width * height);
    let mut color = vec![[0.0; 3]; count];
    let mut depth = vec![0.0; count];
    for py in 0..TILE {
        for px in 0..TILE {
            store(&mut color, &mut depth, 0, px, py, mirror_pixel(px, py));
            store(&mut color, &mut depth, 1, px, py, portal_pixel(px, py));
            store(&mut color, &mut depth, 2, px, py, water_pixel(px, py));
        }
    }
    Reference { color, depth }
}

/// Mean channel of one lit pixel. The reference is still, so the series is flat.
#[must_use]
pub fn still_pixel(index: usize) -> f64 {
    let image = trace();
    let pixel = image.color.get(index).copied().unwrap_or([0.0; 3]);
    f64::from(pixel[0] + pixel[1] + pixel[2]) / 3.0
}

fn mirror_pixel(px: u32, py: u32) -> ([f32; 3], f32) {
    let (eye, ray) = primary(
        px,
        py,
        MIRROR_EYE,
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::X,
        Vec3::Y,
    );
    let Some((hit, distance)) = hit_plane(eye, ray, 0.0, 2) else {
        return ([0.0; 3], 0.0);
    };
    (mirror_color(hit, ray), distance)
}

fn portal_pixel(px: u32, py: u32) -> ([f32; 3], f32) {
    let (eye, ray) = primary(
        px,
        py,
        MIRROR_EYE,
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::X,
        Vec3::Y,
    );
    let Some((hit, distance)) = hit_plane(eye, ray, 0.0, 2) else {
        return ([0.0; 3], 0.0);
    };
    (portal_color(hit, ray), distance)
}

fn water_pixel(px: u32, py: u32) -> ([f32; 3], f32) {
    let (eye, ray) = primary(
        px,
        py,
        WATER_EYE,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::X,
        Vec3::new(0.0, 0.0, -1.0),
    );
    let Some((hit, distance)) = hit_plane(eye, ray, 1.0, 1) else {
        return ([0.0; 3], 0.0);
    };
    let opaque = if hit_plane(eye, ray, 0.0, 1).is_some() {
        FLOOR
    } else {
        [0.0; 3]
    };
    (water_color(ray, hit, opaque), distance)
}

fn mirror_color(hit: Vec3, ray: Vec3) -> [f32; 3] {
    let views = optics::mirror_views(
        &[1.0],
        optics::reflection_view_cap(ReflectionPreset::Low),
        MIRROR_FRESNEL_CUTOFF,
    );
    if views == 0 {
        return [0.0; 3];
    }
    let bounce = optics::mirror_dir(ray, Vec3::Z);
    let Some((place, _)) = hit_plane(hit, bounce, 2.0, 2) else {
        return [0.0; 3];
    };
    if place.x >= 0.0 {
        WALL_POS
    } else {
        WALL_NEG
    }
}

fn portal_color(hit: Vec3, ray: Vec3) -> [f32; 3] {
    let link = axis_portal();
    let bounce = optics::portal_dir(link, ray);
    let origin = optics::portal_point(link, hit) + bounce * 1.0e-3;
    let Some((place, _)) = hit_plane(origin, bounce, 2.0, 2) else {
        return [0.0; 3];
    };
    if place.y >= 1.0 {
        PORTAL_HIGH
    } else {
        PORTAL_LOW
    }
}

/// `F * reflected + (1 - F) * transmitted * exp(-sigma * distance)`.
///
/// A refraction miss keeps `opaque`. Total internal reflection returns the ceiling.
fn water_color(ray: Vec3, hit: Vec3, opaque: [f32; 3]) -> [f32; 3] {
    let cos_i = Vec3::Y.dot(-ray).clamp(0.0, 1.0);
    let fresnel = optics::fresnel_dielectric(WATER_IOR, cos_i);
    let bounce = optics::mirror_dir(ray, Vec3::Y);
    let reflected = if hit_plane(hit, bounce, 4.0, 1).is_some() {
        CEILING
    } else {
        opaque
    };
    let eta = 1.0 / WATER_IOR;
    let Some(bent) = optics::refract_dir(ray, Vec3::Y, eta) else {
        return CEILING;
    };
    let (sample, travel) = match hit_plane(hit, bent, 0.0, 1) {
        Some((place, _)) => (FLOOR, (place - hit).length()),
        None => (opaque, hit.y.max(0.0)),
    };
    let weight = fresnel.clamp(0.0, 1.0);
    let absorb = optics::beer(SIGMA, travel);
    mix(reflected, sample, weight, (1.0 - weight) * absorb)
}

fn axis_portal() -> PortalLink {
    PortalLink {
        from_origin: Vec3::ZERO,
        from_basis: Mat3::IDENTITY,
        to_origin: Vec3::new(10.0, 0.0, 0.0),
        to_basis: Mat3::IDENTITY,
    }
}

fn primary(px: u32, py: u32, eye: Vec3, forward: Vec3, right: Vec3, up: Vec3) -> (Vec3, Vec3) {
    let scale = (FOV * 0.5).tan();
    let u = (unit(px) + 0.5) / unit(TILE) * 2.0 - 1.0;
    let v = 1.0 - (unit(py) + 0.5) / unit(TILE) * 2.0;
    let ray = (forward + right * (u * scale) + up * (v * scale)).normalize();
    (eye, ray)
}

/// `axis` 0 is X, 1 is Y, 2 is Z.
fn hit_plane(origin: Vec3, ray: Vec3, place: f32, axis: usize) -> Option<(Vec3, f32)> {
    let (start, slope) = match axis {
        0 => (origin.x, ray.x),
        1 => (origin.y, ray.y),
        _ => (origin.z, ray.z),
    };
    if slope.abs() < 1.0e-8 {
        return None;
    }
    let travel = (place - start) / slope;
    if travel <= 1.0e-4 {
        return None;
    }
    Some((origin + ray * travel, travel))
}

fn store(
    color: &mut [[f32; 3]],
    depth: &mut [f32],
    tile: u32,
    px: u32,
    py: u32,
    pixel: ([f32; 3], f32),
) {
    let index = idx(tile * TILE + py) * idx(TILE) + idx(px);
    color[index] = pixel.0;
    depth[index] = pixel.1;
}

fn mix(a: [f32; 3], b: [f32; 3], wa: f32, wb: f32) -> [f32; 3] {
    [
        a[0] * wa + b[0] * wb,
        a[1] * wa + b[1] * wb,
        a[2] * wa + b[2] * wb,
    ]
}

fn idx(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
}

fn unit(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(0))
}
