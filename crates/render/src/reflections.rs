//! CPU picture for planar mirrors, portals, and flat water.
//!
//! The frame does not call this. Glossy screen marches, the world cache, and
//! `ray_query` wait on the lighting schedule. Low schedules zero reflection
//! rays. A refraction miss keeps the opaque sample, darkened by Beer–Lambert.
//!
//! The oblique row is Lengyel's clip with Vulkan clip Z in 0..1: the third
//! row is the scaled plane, not `plane - row3`. The shade rejects a point
//! behind the plane with [`genos_math::optics::plane_in_front`]. `portal_leak`
//! compares that predicate with [`genos_math::optics::oblique_draws`] on the
//! reflected camera's axis.

use genos_math::optics::{self, PortalLink, ReflectionPreset, MIRROR_FRESNEL_CUTOFF, WATER_IOR};
use genos_math::{Mat3, Vec3, Vec4};

/// Pixels on a side of one mirror, portal, or water tile.
pub const TILE: u32 = 8;

const FOV: f32 = 60.0_f32.to_radians();
const MIRROR_EYE: Vec3 = Vec3::new(0.0, 1.0, 4.0);
const WATER_EYE: Vec3 = Vec3::new(0.0, 3.0, 0.5);
const SIGMA: f32 = 0.35;
const HIDDEN: [f32; 3] = [1.0, 0.0, 1.0];
const WALL_POS: [f32; 3] = [0.85, 0.12, 0.08];
const WALL_NEG: [f32; 3] = [0.10, 0.55, 0.20];
const PORTAL_HIGH: [f32; 3] = [0.15, 0.35, 0.90];
const PORTAL_LOW: [f32; 3] = [0.90, 0.70, 0.20];
const FLOOR: [f32; 3] = [0.08, 0.22, 0.72];
const CEILING: [f32; 3] = [0.72, 0.68, 0.55];

/// A broken shade the checker uses to prove a gate can fail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PictureFault(u8);

impl PictureFault {
    pub const DRAW_BEHIND: u8 = 1;
    pub const BLINK: u8 = 2;
    pub const NEAR_BIAS: u8 = 4;
    pub const SKIP_SHORE: u8 = 8;
    pub const EXTRA_RAYS: u8 = 16;
    /// Apply the shore cap in deep water, where the crest is already under it.
    pub const FORCE_CAP: u8 = 32;

    #[must_use]
    pub const fn only(bit: u8) -> Self {
        Self(bit)
    }

    #[must_use]
    pub const fn all() -> Self {
        Self(
            Self::DRAW_BEHIND
                | Self::BLINK
                | Self::NEAR_BIAS
                | Self::SKIP_SHORE
                | Self::EXTRA_RAYS
                | Self::FORCE_CAP,
        )
    }

    fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }
}

/// Linear colour and primary-hit distance, three tiles stacked.
#[derive(Clone, Debug, PartialEq)]
pub struct ReflectionPicture {
    pub width: u32,
    pub height: u32,
    pub color: Vec<[f32; 3]>,
    pub depth: Vec<f32>,
}

/// Mirror, portal, and water tiles. `frame` is the still-camera index.
#[must_use]
pub fn shade_picture(frame: u32, fault: PictureFault) -> ReflectionPicture {
    let width = TILE;
    let height = TILE * 3;
    let mut color = vec![[0.0; 3]; idx(width * height)];
    let mut depth = vec![0.0; color.len()];
    for py in 0..TILE {
        for px in 0..TILE {
            let (mirror, mirror_depth) = mirror_pixel(px, py, fault);
            store(&mut color, &mut depth, 0, px, py, mirror, mirror_depth);
            let (portal, portal_depth) = portal_pixel(px, py, fault);
            store(&mut color, &mut depth, 1, px, py, portal, portal_depth);
            let (water, water_depth) = water_pixel(px, py);
            store(&mut color, &mut depth, 2, px, py, water, water_depth);
        }
    }
    if fault.has(PictureFault::BLINK) && frame % 2 == 1 {
        for pixel in &mut color {
            *pixel = scale(*pixel, 0.4);
        }
    }
    if fault.has(PictureFault::NEAR_BIAS) {
        for (pixel, distance) in color.iter_mut().zip(depth.iter()) {
            if *distance < 3.0 {
                *pixel = add(*pixel, 0.5);
            }
        }
    }
    ReflectionPicture {
        width,
        height,
        color,
        depth,
    }
}

/// Reflection rays the Low preset schedules. The picture does not trace any.
#[must_use]
pub fn scheduled_low_rays(fault: PictureFault) -> u32 {
    if fault.has(PictureFault::EXTRA_RAYS) {
        64
    } else {
        optics::reflection_ray_cap(ReflectionPreset::Low)
    }
}

/// Fraction of reflected-camera samples whose clip disagrees with the plane.
#[must_use]
pub fn portal_leak(fault: PictureFault) -> f64 {
    let projection = optics::parity_projection();
    let plane = Vec4::new(0.0, 0.0, 1.0, 4.0);
    let samples = [1.0_f32, 2.0, 6.0, -0.5, -1.0, -2.0];
    let mut wrong = 0u32;
    for world_z in samples {
        let geometric = optics::plane_in_front(Vec3::new(0.0, 0.0, world_z), Vec3::ZERO, Vec3::Z);
        let view = Vec3::new(0.0, 0.0, -world_z - 4.0);
        let drawn = if fault.has(PictureFault::DRAW_BEHIND) {
            true
        } else {
            optics::oblique_draws(projection, plane, view)
        };
        if drawn != geometric {
            wrong += 1;
        }
    }
    f64::from(wrong) / f64::from(u32::try_from(samples.len()).unwrap_or(6))
}

fn gerstner_crest() -> f32 {
    optics::gerstner(
        &[optics::GerstnerWave {
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
    .y
}

/// Max Gerstner crest / depth after the shore clamp. The shallow sample is the boundary.
#[must_use]
pub fn shoreline_ratio(fault: PictureFault) -> f64 {
    let crest = gerstner_crest();
    let depths = [0.0625_f32, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0];
    let mut max_ratio: f64 = 0.0;
    for depth in depths {
        let height = if fault.has(PictureFault::SKIP_SHORE) {
            crest
        } else {
            optics::shoreline_height(crest, depth, optics::SHORE_CAP)
        };
        max_ratio = max_ratio.max(f64::from(height) / f64::from(depth));
    }
    max_ratio
}

/// Crest / depth at 4 m. The crest is under the cap there, so the ratio stays below 0.75.
#[must_use]
pub fn shoreline_deep_ratio(fault: PictureFault) -> f64 {
    let crest = gerstner_crest();
    let depth = 4.0_f32;
    let height = if fault.has(PictureFault::FORCE_CAP) {
        optics::SHORE_CAP * depth
    } else {
        optics::shoreline_height(crest, depth, optics::SHORE_CAP)
    };
    f64::from(height) / f64::from(depth)
}

fn mirror_pixel(px: u32, py: u32, fault: PictureFault) -> ([f32; 3], f32) {
    let (eye, dir) = primary_ray(
        px,
        py,
        MIRROR_EYE,
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::X,
        Vec3::Y,
    );
    let Some((_, distance)) = hit_plane(eye, dir, 0.0, 2) else {
        return ([0.0; 3], 0.0);
    };
    let views = optics::mirror_views(
        &[1.0],
        optics::reflection_view_cap(ReflectionPreset::Low),
        MIRROR_FRESNEL_CUTOFF,
    );
    if views == 0 {
        return ([0.0; 3], distance);
    }
    let origin = reflect_point(eye);
    let direction = optics::mirror_dir(dir, Vec3::Z);
    let color = first_in_front(
        origin,
        direction,
        &[
            Surface {
                axis: 2,
                place: -1.0,
                color: hidden_color,
            },
            Surface {
                axis: 2,
                place: 2.0,
                color: split_wall,
            },
        ],
        Vec3::ZERO,
        Vec3::Z,
        fault.has(PictureFault::DRAW_BEHIND),
    );
    (color, distance)
}

fn portal_pixel(px: u32, py: u32, fault: PictureFault) -> ([f32; 3], f32) {
    let (eye, dir) = primary_ray(
        px,
        py,
        MIRROR_EYE,
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::X,
        Vec3::Y,
    );
    let Some((hit, distance)) = hit_plane(eye, dir, 0.0, 2) else {
        return ([0.0; 3], 0.0);
    };
    let link = axis_portal();
    let direction = optics::portal_dir(link, dir);
    let origin = optics::portal_point(link, hit) + direction * 1.0e-3;
    let color = first_in_front(
        origin,
        direction,
        &[
            Surface {
                axis: 2,
                place: -1.0,
                color: hidden_color,
            },
            Surface {
                axis: 2,
                place: 2.0,
                color: split_portal,
            },
        ],
        Vec3::ZERO,
        Vec3::Z,
        fault.has(PictureFault::DRAW_BEHIND),
    );
    (color, distance)
}

fn water_pixel(px: u32, py: u32) -> ([f32; 3], f32) {
    let (eye, dir) = primary_ray(
        px,
        py,
        WATER_EYE,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::X,
        Vec3::new(0.0, 0.0, -1.0),
    );
    let Some((hit, distance)) = hit_plane(eye, dir, 1.0, 1) else {
        return ([0.0; 3], 0.0);
    };
    let opaque = floor_along(eye, dir);
    (water_color(dir, hit, opaque, true), distance)
}

fn water_color(dir: Vec3, hit: Vec3, opaque: [f32; 3], floor_exists: bool) -> [f32; 3] {
    let cos_i = Vec3::Y.dot(-dir).clamp(0.0, 1.0);
    let fresnel = optics::fresnel_dielectric(WATER_IOR, cos_i);
    let reflected_dir = optics::mirror_dir(dir, Vec3::Y);
    let reflected = match hit_plane(hit, reflected_dir, 4.0, 1) {
        Some(_) => CEILING,
        None => opaque,
    };
    let eta = 1.0 / WATER_IOR;
    let Some(transmitted_dir) = optics::refract_dir(dir, Vec3::Y, eta) else {
        return CEILING;
    };
    let transmitted = if floor_exists {
        hit_plane(hit, transmitted_dir, 0.0, 1).map(|(place, _)| {
            let travel = (place - hit).length();
            (FLOOR, travel)
        })
    } else {
        None
    };
    let (sample, travel) = transmitted.unwrap_or((opaque, (hit.y - 0.0).max(0.0)));
    let shaded = optics::water_shade(
        fresnel,
        Vec3::new(reflected[0], reflected[1], reflected[2]),
        Vec3::new(sample[0], sample[1], sample[2]),
        SIGMA,
        travel,
    );
    [shaded.x, shaded.y, shaded.z]
}

fn floor_along(eye: Vec3, dir: Vec3) -> [f32; 3] {
    if hit_plane(eye, dir, 0.0, 1).is_some() {
        FLOOR
    } else {
        [0.0; 3]
    }
}

struct Surface {
    axis: usize,
    place: f32,
    color: fn(Vec3) -> [f32; 3],
}

fn first_in_front(
    origin: Vec3,
    direction: Vec3,
    surfaces: &[Surface],
    plane_origin: Vec3,
    plane_normal: Vec3,
    draw_behind: bool,
) -> [f32; 3] {
    let mut best_t = f32::MAX;
    let mut best: Option<[f32; 3]> = None;
    for surface in surfaces {
        let Some((hit, travel)) = hit_plane(origin, direction, surface.place, surface.axis) else {
            continue;
        };
        if travel <= 1.0e-3 || travel >= best_t {
            continue;
        }
        if !draw_behind && !optics::plane_in_front(hit, plane_origin, plane_normal) {
            continue;
        }
        best_t = travel;
        best = Some((surface.color)(hit));
    }
    best.unwrap_or([0.0; 3])
}

fn hidden_color(_hit: Vec3) -> [f32; 3] {
    HIDDEN
}

fn split_wall(hit: Vec3) -> [f32; 3] {
    if hit.x >= 0.0 {
        WALL_POS
    } else {
        WALL_NEG
    }
}

fn split_portal(hit: Vec3) -> [f32; 3] {
    if hit.y >= 1.0 {
        PORTAL_HIGH
    } else {
        PORTAL_LOW
    }
}

fn reflect_point(point: Vec3) -> Vec3 {
    optics::mirror_point(point, Vec3::Z)
}

fn axis_portal() -> PortalLink {
    PortalLink {
        from_origin: Vec3::ZERO,
        from_basis: Mat3::IDENTITY,
        to_origin: Vec3::new(10.0, 0.0, 0.0),
        to_basis: Mat3::IDENTITY,
    }
}

fn primary_ray(px: u32, py: u32, eye: Vec3, forward: Vec3, right: Vec3, up: Vec3) -> (Vec3, Vec3) {
    let scale = (FOV * 0.5).tan();
    let u = (unit(px) + 0.5) / unit(TILE) * 2.0 - 1.0;
    let v = 1.0 - (unit(py) + 0.5) / unit(TILE) * 2.0;
    let dir = (forward + right * (u * scale) + up * (v * scale)).normalize();
    (eye, dir)
}

/// `axis` 0 is X, 1 is Y, 2 is Z. Returns the hit and the positive travel.
fn hit_plane(origin: Vec3, direction: Vec3, place: f32, axis: usize) -> Option<(Vec3, f32)> {
    let (start, slope) = match axis {
        0 => (origin.x, direction.x),
        1 => (origin.y, direction.y),
        _ => (origin.z, direction.z),
    };
    if slope.abs() < 1.0e-8 {
        return None;
    }
    let travel = (place - start) / slope;
    if travel <= 1.0e-4 {
        return None;
    }
    Some((origin + direction * travel, travel))
}

fn store(
    color: &mut [[f32; 3]],
    depth: &mut [f32],
    tile: u32,
    px: u32,
    py: u32,
    pixel: [f32; 3],
    distance: f32,
) {
    let index = idx(tile * TILE + py) * idx(TILE) + idx(px);
    color[index] = pixel;
    depth[index] = distance;
}

fn idx(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
}

fn unit(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(0))
}

fn scale(color: [f32; 3], factor: f32) -> [f32; 3] {
    [color[0] * factor, color[1] * factor, color[2] * factor]
}

fn add(color: [f32; 3], extra: f32) -> [f32; 3] {
    [color[0] + extra, color[1] + extra, color[2] + extra]
}

#[cfg(test)]
mod tests {
    use super::{
        idx, portal_leak, scheduled_low_rays, shade_picture, shoreline_deep_ratio, shoreline_ratio,
        water_color, PictureFault, HIDDEN, TILE,
    };
    use genos_math::Vec3;

    #[test]
    fn low_schedules_zero_rays_and_the_broken_scheduler_does_not() {
        assert_eq!(scheduled_low_rays(PictureFault::default()), 0);
        assert_eq!(
            scheduled_low_rays(PictureFault::only(PictureFault::EXTRA_RAYS)),
            64
        );
    }

    #[test]
    fn the_clip_matches_the_plane_and_drawing_behind_leaks() {
        assert!(portal_leak(PictureFault::default()) <= f64::EPSILON);
        assert!(portal_leak(PictureFault::only(PictureFault::DRAW_BEHIND)) > 0.0);
    }

    #[test]
    fn the_shore_clamp_holds_and_the_unclamped_crest_does_not() {
        assert!(shoreline_ratio(PictureFault::default()) <= 0.75);
        assert!(shoreline_ratio(PictureFault::only(PictureFault::SKIP_SHORE)) > 0.75);
        let deep = shoreline_deep_ratio(PictureFault::default());
        assert!(deep < 0.75, "{deep}");
        assert!(shoreline_deep_ratio(PictureFault::only(PictureFault::FORCE_CAP)) > 0.5);
    }

    #[test]
    fn mirrors_hide_the_back_side_and_a_miss_keeps_the_opaque_sample() {
        let picture = shade_picture(0, PictureFault::default());
        assert_eq!(picture.width, TILE);
        assert_eq!(picture.height, TILE * 3);
        for pixel in picture.color.iter().take(idx(TILE * TILE)) {
            assert_ne!(*pixel, HIDDEN);
        }
        let broken = shade_picture(0, PictureFault::only(PictureFault::DRAW_BEHIND));
        assert!(broken
            .color
            .iter()
            .take(idx(TILE * TILE))
            .any(|pixel| *pixel == HIDDEN));
        let miss = water_color(
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            [0.2, 0.3, 0.4],
            false,
        );
        let hit = water_color(
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            [0.2, 0.3, 0.4],
            true,
        );
        assert_ne!(miss, hit);
        let again = shade_picture(1, PictureFault::default());
        assert_eq!(picture.color, again.color);
    }
}
