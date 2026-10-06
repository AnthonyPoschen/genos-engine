//! Airborne Doppler, stereo balance, distance, and direct-path transmission.
//!
//! The mixer applies these numbers to PCM. The GPU pass uses the same
//! transmission rule on resident occluder bounds.

use genos_math::Vec3;

use crate::Barrier;

/// Speed of sound in still air, in meters per second.
pub const SPEED_OF_SOUND: f32 = 343.0;

/// Distance at which a positional source is full scale. Closer stays at this gain.
const REFERENCE_DISTANCE: f32 = 1.0;

/// Pitch ratio for still air.
///
/// Only the velocity component along the line from the source to the listener
/// counts. Motion toward the other raises pitch. Motion away lowers it.
pub(crate) fn doppler_ratio(
    source: Vec3,
    source_velocity: Vec3,
    listener: Vec3,
    listener_velocity: Vec3,
) -> f32 {
    let delta = listener - source;
    let distance = delta.length();
    if distance < 1.0e-4 {
        return 1.0;
    }
    let dir = delta / distance;
    let limit = SPEED_OF_SOUND * 0.99;
    let closing_source = source_velocity.dot(dir).clamp(-limit, limit);
    let closing_listener = (-listener_velocity.dot(dir)).clamp(-limit, limit);
    (SPEED_OF_SOUND + closing_listener) / (SPEED_OF_SOUND - closing_source)
}

/// Constant-power left and right gains from the horizontal bearing.
///
/// A source in front is equal on both channels. A source to one side, including
/// one behind the listener, favors that side. `facing` is a world direction.
pub(crate) fn pan_gains(source: Vec3, listener: Vec3, facing: Vec3) -> (f32, f32) {
    let mut forward = Vec3::new(facing.x, 0.0, facing.z);
    if forward.length_squared() < 1.0e-8 {
        forward = Vec3::new(0.0, 0.0, -1.0);
    }
    forward = forward.normalize();
    let mut right = forward.cross(Vec3::Y);
    if right.length_squared() < 1.0e-8 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }
    let mut to_source = Vec3::new(source.x - listener.x, 0.0, source.z - listener.z);
    if to_source.length_squared() < 1.0e-8 {
        let half = std::f32::consts::FRAC_1_SQRT_2;
        return (half, half);
    }
    to_source = to_source.normalize();
    let side = to_source.dot(right).clamp(-1.0, 1.0);
    let angle = (side + 1.0) * 0.25 * std::f32::consts::PI;
    (angle.cos(), angle.sin())
}

/// Inverse-distance pressure. Twice the distance is about half the amplitude.
pub(crate) fn distance_gain(source: Vec3, listener: Vec3) -> f32 {
    let distance = (listener - source).length().max(REFERENCE_DISTANCE);
    REFERENCE_DISTANCE / distance
}

/// Product of `exp(-absorption * thickness)` for every barrier the segment hits.
pub(crate) fn transmission_gain(source: Vec3, listener: Vec3, barriers: &[Barrier]) -> f32 {
    let mut gain = 1.0f32;
    for barrier in barriers {
        let thickness = segment_thickness(source, listener, barrier.min, barrier.max);
        let absorption = barrier.absorption.max(0.0);
        if thickness > 0.0 && absorption > 0.0 {
            gain *= (-absorption * thickness).exp();
        }
    }
    gain
}

/// Meters of the straight segment that lie inside the box.
pub(crate) fn segment_thickness(source: Vec3, listener: Vec3, min: Vec3, max: Vec3) -> f32 {
    let delta = listener - source;
    let distance = delta.length();
    if distance < 1.0e-5 {
        return 0.0;
    }
    let lo = Vec3::new(min.x.min(max.x), min.y.min(max.y), min.z.min(max.z));
    let hi = Vec3::new(min.x.max(max.x), min.y.max(max.y), min.z.max(max.z));
    let origin = [source.x, source.y, source.z];
    let step = [delta.x, delta.y, delta.z];
    let lower = [lo.x, lo.y, lo.z];
    let upper = [hi.x, hi.y, hi.z];
    let mut enter = 0.0f32;
    let mut exit = 1.0f32;
    for axis in 0..3 {
        if step[axis].abs() < 1.0e-8 {
            if origin[axis] < lower[axis] || origin[axis] > upper[axis] {
                return 0.0;
            }
            continue;
        }
        let inv = 1.0 / step[axis];
        let t1 = (lower[axis] - origin[axis]) * inv;
        let t2 = (upper[axis] - origin[axis]) * inv;
        enter = enter.max(t1.min(t2));
        exit = exit.min(t1.max(t2));
        if exit < enter {
            return 0.0;
        }
    }
    let start = enter.max(0.0);
    let end = exit.min(1.0);
    if end <= start {
        return 0.0;
    }
    (end - start) * distance
}
