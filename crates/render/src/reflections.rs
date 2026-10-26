//! CPU picture for planar mirrors, portals, and flat water.
//!
//! The frame shades water with `reflection_water_shade` and rasters mirror and
//! portal views when the world has those surfaces. Glossy screen marches, the
//! world cache, and `ray_query` wait on the lighting schedule. Potato and Low
//! schedule zero reflection rays. A refraction miss keeps the opaque sample,
//! darkened by Beer–Lambert.
//!
//! The oblique row is Lengyel's clip with Vulkan clip Z in 0..1: the third
//! row is the scaled plane, not `plane - row3`. The shade rejects a point
//! behind the plane with [`genos_math::optics::plane_in_front`]. `portal_leak`
//! compares that predicate with [`genos_math::optics::oblique_draws`] on the
//! reflected camera's axis.

use genos_math::optics::{self, PortalLink, ReflectionPreset, MIRROR_FRESNEL_CUTOFF, WATER_IOR};
use genos_math::{Mat3, Mat4, Vec3, Vec4};
use genos_scene::{CAMERA_FAR, CAMERA_NEAR};

use crate::pack::GpuVertex;

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

    /// Fault bits the frame shader reads. Bit 4 is the near bias, bit 16 is extra rays.
    #[must_use]
    pub fn bits(self) -> u32 {
        u32::from(self.0)
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

/// Reflection rays a frame may cast. Potato (`0`) and Low (`1`) stay at zero.
#[must_use]
pub fn frame_ray_count(preset: u32) -> u32 {
    let preset = match preset {
        0 => ReflectionPreset::Potato,
        1 => ReflectionPreset::Low,
        2 => ReflectionPreset::Medium,
        _ => ReflectionPreset::High,
    };
    optics::reflection_ray_cap(preset)
}

/// The frame runs the reflection passes only when a surface asks for one.
#[must_use]
pub fn reflection_frame_active(mirrors: u32, portals: u32, water: u32) -> bool {
    mirrors.saturating_add(portals).saturating_add(water) > 0
}

/// Mirror camera, oblique near plane on the mirror. Vulkan clip Z is 0..1.
#[must_use]
pub fn mirror_view_proj(
    eye: Vec3,
    forward: Vec3,
    aspect: f32,
    origin: Vec3,
    normal: Vec3,
) -> [f32; 16] {
    let normal = unit_vec(normal);
    let forward = unit_vec(forward);
    let (right, up) = camera_axes(forward);
    let eye = optics::mirror_point(eye - origin, normal) + origin;
    oblique_camera_proj(
        eye,
        unit_vec(optics::mirror_dir(forward, normal)),
        unit_vec(optics::mirror_dir(right, normal)),
        unit_vec(optics::mirror_dir(up, normal)),
        aspect,
        origin,
        normal,
    )
}

/// Portal camera. The oblique plane sits on the destination opening.
#[must_use]
pub fn portal_view_proj(
    eye: Vec3,
    forward: Vec3,
    aspect: f32,
    origin: Vec3,
    normal: Vec3,
    portal_to: Vec3,
) -> [f32; 16] {
    let normal = unit_vec(normal);
    let forward = unit_vec(forward);
    let (right, up) = camera_axes(forward);
    let link = PortalLink {
        from_origin: origin,
        from_basis: Mat3::IDENTITY,
        to_origin: portal_to,
        to_basis: Mat3::IDENTITY,
    };
    oblique_camera_proj(
        optics::portal_point(link, eye),
        unit_vec(optics::portal_dir(link, forward)),
        unit_vec(optics::portal_dir(link, right)),
        unit_vec(optics::portal_dir(link, up)),
        aspect,
        portal_to,
        unit_vec(optics::portal_dir(link, normal)),
    )
}

/// One water quad, two triangles, on the plane. The albedo is the transmitted sample.
#[must_use]
pub fn water_quad(origin: Vec3, normal: Vec3) -> [GpuVertex; 6] {
    let normal = unit_vec(normal);
    let (right, tangent) = camera_axes(normal);
    let corner = |x: f32, z: f32| origin + right * x + tangent * z;
    let vert = |point: Vec3| GpuVertex {
        pos: [point.x, point.y, point.z],
        albedo: FLOOR,
        normal: [normal.x, normal.y, normal.z],
        shade: 1.0,
        uv: [-1.0, -1.0],
    };
    let a = corner(-1.0, -1.0);
    let b = corner(1.0, -1.0);
    let c = corner(1.0, 1.0);
    let d = corner(-1.0, 1.0);
    [vert(a), vert(b), vert(c), vert(a), vert(c), vert(d)]
}

fn oblique_camera_proj(
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    aspect: f32,
    origin: Vec3,
    normal: Vec3,
) -> [f32; 16] {
    let view = Mat4::from_cols([
        right.x,
        up.x,
        -forward.x,
        0.0,
        right.y,
        up.y,
        -forward.y,
        0.0,
        right.z,
        up.z,
        -forward.z,
        0.0,
        -right.dot(eye),
        -up.dot(eye),
        forward.dot(eye),
        1.0,
    ]);
    let rel = origin - eye;
    let origin_cam = Vec3::new(right.dot(rel), up.dot(rel), -forward.dot(rel));
    let normal_cam = Vec3::new(right.dot(normal), up.dot(normal), -forward.dot(normal));
    let plane = Vec4::new(
        normal_cam.x,
        normal_cam.y,
        normal_cam.z,
        -normal_cam.dot(origin_cam),
    );
    let fovy = 60.0_f32.to_radians();
    let h = 1.0 / (fovy * 0.5).tan();
    let w = h / aspect.max(0.01);
    let near = CAMERA_NEAR;
    let far = CAMERA_FAR;
    let proj = Mat4::from_cols([
        w,
        0.0,
        0.0,
        0.0,
        0.0,
        -h,
        0.0,
        0.0,
        0.0,
        0.0,
        far / (near - far),
        -1.0,
        0.0,
        0.0,
        (far * near) / (near - far),
        0.0,
    ]);
    (optics::oblique_projection(proj, plane) * view).cols
}

fn camera_axes(forward: Vec3) -> (Vec3, Vec3) {
    let helper = if forward.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let right = unit_vec(forward.cross(helper));
    let up = unit_vec(right.cross(forward));
    (right, up)
}

fn unit_vec(value: Vec3) -> Vec3 {
    let len = value.length();
    if len < 1.0e-8 {
        Vec3::Z
    } else {
        value / len
    }
}

/// Bytes the frame's grade buffer holds: 192 pixels, then four `u32`s.
pub const GRADE_BYTES: usize = 192 * 16 + 16;

/// What the reflection composite wrote. `ran` is false when the frame skipped it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReflectionGrade {
    pub ran: bool,
    pub color: Vec<[f32; 3]>,
    pub depth: Vec<f32>,
    pub rays: u32,
    pub views: u32,
}

/// Read the grade buffer the composite stores. `None` when the bytes are short.
#[must_use]
pub fn reflection_grade_from_bytes(bytes: &[u8]) -> Option<ReflectionGrade> {
    if bytes.len() < GRADE_BYTES {
        return None;
    }
    let mut color = Vec::with_capacity(192);
    let mut depth = Vec::with_capacity(192);
    for index in 0..192 {
        let at = index * 16;
        let mut word = [0u8; 4];
        word.copy_from_slice(&bytes[at..at + 4]);
        let r = f32::from_le_bytes(word);
        word.copy_from_slice(&bytes[at + 4..at + 8]);
        let g = f32::from_le_bytes(word);
        word.copy_from_slice(&bytes[at + 8..at + 12]);
        let b = f32::from_le_bytes(word);
        word.copy_from_slice(&bytes[at + 12..at + 16]);
        let distance = f32::from_le_bytes(word);
        color.push([r, g, b]);
        depth.push(distance);
    }
    let trailer = 192 * 16;
    Some(ReflectionGrade {
        ran: true,
        color,
        depth,
        rays: read_u32(&bytes[trailer..trailer + 4]),
        views: read_u32(&bytes[trailer + 4..trailer + 8]),
    })
}

fn read_u32(bytes: &[u8]) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(bytes);
    u32::from_le_bytes(word)
}

/// Water quads for the translucent pass. At most four surfaces.
#[must_use]
pub fn water_vertices(reflectors: &[crate::world::Reflector]) -> Vec<GpuVertex> {
    let mut out = Vec::new();
    for reflector in reflectors
        .iter()
        .filter(|item| item.kind == crate::world::ReflectorKind::Water)
        .take(4)
    {
        let quad = water_quad(
            Vec3::new(reflector.origin[0], reflector.origin[1], reflector.origin[2]),
            Vec3::new(reflector.normal[0], reflector.normal[1], reflector.normal[2]),
        );
        out.extend(quad);
    }
    out
}

/// Lighting preset index the frame pushes. Potato is 0, Low is 1, Medium is 2.
#[must_use]
pub fn lighting_reflection_preset(preset: genos_scene::LightingPreset) -> u32 {
    match preset {
        genos_scene::LightingPreset::Potato => 0,
        genos_scene::LightingPreset::Low => 1,
        genos_scene::LightingPreset::Medium => 2,
        genos_scene::LightingPreset::DayCycle
        | genos_scene::LightingPreset::Gi1Bounce
        | genos_scene::LightingPreset::High => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        frame_ray_count, idx, mirror_view_proj, portal_leak, reflection_frame_active,
        scheduled_low_rays, shade_picture, shoreline_deep_ratio, shoreline_ratio, water_color,
        water_quad, PictureFault, HIDDEN, TILE,
    };
    use genos_math::{Mat4, Vec3};

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

    #[test]
    fn potato_and_low_cast_no_rays_and_an_empty_scene_skips_the_frame() {
        assert_eq!(frame_ray_count(0), 0);
        assert_eq!(frame_ray_count(1), 0);
        assert!(frame_ray_count(2) > 0);
        assert!(!reflection_frame_active(0, 0, 0));
        assert!(reflection_frame_active(1, 0, 0));
        assert!(reflection_frame_active(0, 0, 1));
    }

    #[test]
    fn the_oblique_mirror_keeps_the_front_and_clips_the_back() {
        let matrix = mirror_view_proj(
            Vec3::new(0.0, 1.0, 4.0),
            Vec3::new(0.0, 0.0, -1.0),
            1.0,
            Vec3::ZERO,
            Vec3::Z,
        );
        let front = clip_z(&matrix, Vec3::new(0.0, 1.0, 1.0));
        let back = clip_z(&matrix, Vec3::new(0.0, 1.0, -1.0));
        assert!(front >= -1.0e-4, "{front}");
        assert!(back < 0.0, "{back}");
        let quad = water_quad(Vec3::Y, Vec3::Y);
        assert_eq!(quad.len(), 6);
        assert!((quad[0].normal[1] - 1.0).abs() < 1.0e-5);
    }

    fn clip_z(matrix: &[f32; 16], point: Vec3) -> f32 {
        let clip = Mat4::from_cols(*matrix).transform_homogeneous(point);
        clip.z / clip.w
    }
}
