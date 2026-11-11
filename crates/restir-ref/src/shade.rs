//! Unshadowed point-lamp shade, one shadow ray, and the brute-force sum.
//!
//! Lambertian BRDF `albedo / π`. The target `p̂` is the Rec. 709 luminance of
//! the unshadowed RGB. Visibility is not in `p̂`. The shade multiplies it once,
//! on the selected lamp.

use genos_math::Vec3;

use crate::reservoir::{reservoir_w, Reservoir};

/// Rec. 709 luminance weights. The same triple the lighting record uses.
pub const LUMA: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

const RAY_EPS: f32 = 1.0e-3;

/// A point lamp. `intensity` is RGB radiant intensity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    pub position: Vec3,
    pub intensity: Vec3,
}

/// A diffuse receiver. `normal` is unit length. Y is up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Receiver {
    pub position: Vec3,
    pub normal: Vec3,
    pub albedo: Vec3,
}

/// An axis-aligned occluder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

/// Receivers, lamps, and boxes. `width` is the row stride of `receivers`.
/// `camera` is the point `ref_err_near` measures distance from.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub lamps: Vec<Lamp>,
    pub receivers: Vec<Receiver>,
    pub occluders: Vec<Aabb>,
    pub width: usize,
    pub camera: Vec3,
}

/// Rec. 709 luminance.
#[must_use]
pub fn luminance(rgb: Vec3) -> f32 {
    rgb.dot(LUMA)
}

/// Unshadowed reflected radiance. Zero when the lamp is behind the normal.
#[must_use]
pub fn unshadowed(receiver: &Receiver, lamp: &Lamp) -> Vec3 {
    let delta = lamp.position - receiver.position;
    let dist2 = delta.length_squared();
    if dist2 <= 1.0e-8 {
        return Vec3::ZERO;
    }
    let dist = dist2.sqrt();
    let cos = receiver.normal.dot(delta / dist);
    if cos <= 0.0 {
        return Vec3::ZERO;
    }
    let scale = cos / (dist2 * std::f32::consts::PI);
    Vec3::new(
        lamp.intensity.x * receiver.albedo.x * scale,
        lamp.intensity.y * receiver.albedo.y * scale,
        lamp.intensity.z * receiver.albedo.z * scale,
    )
}

/// Unshadowed luminance. This is `p̂`.
#[must_use]
pub fn target(receiver: &Receiver, lamp: &Lamp) -> f32 {
    luminance(unshadowed(receiver, lamp))
}

/// True when the open segment from the receiver to the lamp misses every box.
#[must_use]
pub fn visible(receiver: &Receiver, lamp: &Lamp, occluders: &[Aabb]) -> bool {
    let delta = lamp.position - receiver.position;
    let dist2 = delta.length_squared();
    if dist2 <= 1.0e-8 {
        return false;
    }
    let dist = dist2.sqrt();
    if dist <= RAY_EPS * 2.0 {
        return true;
    }
    let dir = delta / dist;
    let origin = receiver.position + receiver.normal * RAY_EPS;
    let t_max = dist - RAY_EPS;
    for box_ in occluders {
        if ray_hits_aabb(origin, dir, box_, RAY_EPS, t_max) {
            return false;
        }
    }
    true
}

/// Sum of every lamp, with visibility. The reference the reservoirs estimate.
#[must_use]
pub fn brute(receiver: &Receiver, scene: &Scene) -> Vec3 {
    let mut sum = Vec3::ZERO;
    for lamp in &scene.lamps {
        let rgb = unshadowed(receiver, lamp);
        if luminance(rgb) <= 0.0 {
            continue;
        }
        if visible(receiver, lamp, &scene.occluders) {
            sum += rgb;
        }
    }
    sum
}

/// `unshadowed * visibility * W` for the selected lamp.
#[must_use]
pub fn shade(receiver: &Receiver, reservoir: Reservoir, scene: &Scene) -> Vec3 {
    if reservoir.is_empty() || reservoir_w(reservoir) <= 0.0 {
        return Vec3::ZERO;
    }
    let Some(index) = usize::try_from(reservoir.lamp).ok() else {
        return Vec3::ZERO;
    };
    let Some(lamp) = scene.lamps.get(index) else {
        return Vec3::ZERO;
    };
    let rgb = unshadowed(receiver, lamp);
    if !visible(receiver, lamp, &scene.occluders) {
        return Vec3::ZERO;
    }
    rgb * reservoir_w(reservoir)
}

fn ray_hits_aabb(origin: Vec3, dir: Vec3, box_: &Aabb, t_min: f32, t_max: f32) -> bool {
    let mut near = t_min;
    let mut far = t_max;
    if !slab(origin.x, dir.x, box_.min.x, box_.max.x, &mut near, &mut far) {
        return false;
    }
    if !slab(origin.y, dir.y, box_.min.y, box_.max.y, &mut near, &mut far) {
        return false;
    }
    slab(origin.z, dir.z, box_.min.z, box_.max.z, &mut near, &mut far)
}

fn slab(origin: f32, dir: f32, min: f32, max: f32, near: &mut f32, far: &mut f32) -> bool {
    if dir.abs() <= 1.0e-8 {
        return origin >= min && origin <= max;
    }
    let inv = 1.0 / dir;
    let mut t0 = (min - origin) * inv;
    let mut t1 = (max - origin) * inv;
    if t0 > t1 {
        std::mem::swap(&mut t0, &mut t1);
    }
    *near = near.max(t0);
    *far = far.min(t1);
    near <= far
}

#[cfg(test)]
mod tests {
    use super::{brute, visible, Aabb, Lamp, Receiver, Scene};
    use genos_math::Vec3;

    fn receiver_at(position: Vec3) -> Receiver {
        Receiver {
            position,
            normal: Vec3::Y,
            albedo: Vec3::new(1.0, 1.0, 1.0),
        }
    }

    #[test]
    fn a_wall_blocks_one_lamp_and_not_the_other() {
        let receiver = receiver_at(Vec3::new(1.0, 0.0, 0.0));
        let blocked = Lamp {
            position: Vec3::new(-1.0, 0.5, 0.0),
            intensity: Vec3::new(1.0, 1.0, 1.0),
        };
        let clear = Lamp {
            position: Vec3::new(1.0, 2.0, 0.0),
            intensity: Vec3::new(1.0, 1.0, 1.0),
        };
        let wall = Aabb {
            min: Vec3::new(-0.2, 0.0, -1.0),
            max: Vec3::new(0.2, 2.0, 1.0),
        };
        assert!(!visible(&receiver, &blocked, &[wall]));
        assert!(visible(&receiver, &clear, &[wall]));
        let scene = Scene {
            lamps: vec![blocked, clear],
            receivers: vec![receiver],
            occluders: vec![wall],
            width: 1,
            camera: Vec3::new(0.0, 1.5, -2.0),
        };
        let open = Scene {
            occluders: Vec::new(),
            ..scene.clone()
        };
        let with_wall = super::luminance(brute(&receiver, &scene));
        let no_wall = super::luminance(brute(&receiver, &open));
        assert!(with_wall < no_wall, "wall {with_wall} open {no_wall}");
        assert!(with_wall > 0.0, "the high lamp still reaches");
    }
}
