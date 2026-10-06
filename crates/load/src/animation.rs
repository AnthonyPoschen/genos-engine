//! Text animation clip.
//!
//! ```text
//! key <time> <tx> <ty> <tz> <qx> <qy> <qz> <qw> <sx> <sy> <sz>
//! ```
//!
//! Rotation is a quaternion in x, y, z, w order. Sampling uses linear
//! translation and scale. Rotation uses [`genos_math::Quat::slerp`], which
//! flips one side when the dot product is negative.

use genos_math::{Quat, Vec3};

use crate::error::LoadError;

/// Translation, rotation, and scale at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: [f32; 3],
    /// Quaternion x, y, z, w.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// One pose at `time_seconds`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransformKey {
    pub time_seconds: f32,
    pub pose: Transform,
}

/// Timed transform keys.
#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub keys: Vec<TransformKey>,
}

impl Animation {
    /// Pose at `time_seconds`.
    ///
    /// Time before the first key returns that key. Time after the last key
    /// returns that key. Between keys, translation and scale are linear.
    /// Rotation is the short-arc slerp.
    pub fn sample(&self, time_seconds: f32) -> Result<Transform, LoadError> {
        if !time_seconds.is_finite() || self.keys.is_empty() {
            return Err(LoadError::Unrecognized);
        }
        let first = &self.keys[0];
        if time_seconds <= first.time_seconds {
            return Ok(first.pose);
        }
        let last = &self.keys[self.keys.len() - 1];
        if time_seconds >= last.time_seconds {
            return Ok(last.pose);
        }
        let mut upper = 1usize;
        while upper < self.keys.len() && self.keys[upper].time_seconds < time_seconds {
            upper += 1;
        }
        let right = &self.keys[upper];
        let left = &self.keys[upper - 1];
        let span = right.time_seconds - left.time_seconds;
        if span <= 0.0 {
            return Err(LoadError::Unrecognized);
        }
        let t = (time_seconds - left.time_seconds) / span;
        Ok(Transform {
            translation: array3(vec3(left.pose.translation).lerp(vec3(right.pose.translation), t)),
            rotation: array4(quat(left.pose.rotation).slerp(quat(right.pose.rotation), t)),
            scale: array3(vec3(left.pose.scale).lerp(vec3(right.pose.scale), t)),
        })
    }
}

pub(crate) fn decode_animation(bytes: &[u8]) -> Result<Animation, LoadError> {
    let text = std::str::from_utf8(bytes).map_err(|_| LoadError::Unrecognized)?;
    let mut keys = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let tag = parts.next().ok_or(LoadError::Unrecognized)?;
        if tag != "key" {
            return Err(LoadError::Unrecognized);
        }
        let time_seconds = number(parts.next())?;
        let translation = [
            number(parts.next())?,
            number(parts.next())?,
            number(parts.next())?,
        ];
        let rotation = quat([
            number(parts.next())?,
            number(parts.next())?,
            number(parts.next())?,
            number(parts.next())?,
        ]);
        if rotation.length() < 1.0e-8 {
            return Err(LoadError::Unrecognized);
        }
        let rotation = rotation.normalize();
        let scale = [
            number(parts.next())?,
            number(parts.next())?,
            number(parts.next())?,
        ];
        if parts.next().is_some() {
            return Err(LoadError::Unrecognized);
        }
        keys.push(TransformKey {
            time_seconds,
            pose: Transform {
                translation,
                rotation: array4(rotation),
                scale,
            },
        });
    }
    if keys.is_empty() {
        return Err(LoadError::Unrecognized);
    }
    keys.sort_by(|a, b| a.time_seconds.total_cmp(&b.time_seconds));
    for pair in keys.windows(2) {
        if pair[1].time_seconds <= pair[0].time_seconds {
            return Err(LoadError::Unrecognized);
        }
    }
    Ok(Animation { keys })
}

fn number(token: Option<&str>) -> Result<f32, LoadError> {
    let token = token.ok_or(LoadError::Unrecognized)?;
    let value: f32 = token.parse().map_err(|_| LoadError::Unrecognized)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(LoadError::Unrecognized)
    }
}

fn vec3(value: [f32; 3]) -> Vec3 {
    Vec3::new(value[0], value[1], value[2])
}

fn array3(value: Vec3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

fn quat(value: [f32; 4]) -> Quat {
    Quat::new(value[0], value[1], value[2], value[3])
}

fn array4(value: Quat) -> [f32; 4] {
    [value.x, value.y, value.z, value.w]
}
