//! Rotations. A unit quaternion rotates a direction. `q` and `-q` are the same turn.

use crate::vec::Vec3;
use std::ops::{Add, Mul, Neg};

/// A rotation. `(x, y, z)` is the vector part. `w` is the scalar part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z + self.w * rhs.w
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// Unit length. A zero quaternion stays zero.
    pub fn normalize(self) -> Self {
        let length_sq = self.dot(self);
        if length_sq <= f32::MIN_POSITIVE {
            return Self {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            };
        }
        let inv = 1.0 / length_sq.sqrt();
        self * inv
    }

    /// Right-handed rotation around `axis` by `radians`.
    pub fn from_axis_angle(axis: Vec3, radians: f32) -> Self {
        let axis = axis.normalize();
        if axis.length_squared() == 0.0 {
            return Self::IDENTITY;
        }
        let (sin_half, cos_half) = (radians * 0.5).sin_cos();
        Self {
            x: axis.x * sin_half,
            y: axis.y * sin_half,
            z: axis.z * sin_half,
            w: cos_half,
        }
    }

    /// Rotate a direction. `self` must be unit length.
    pub fn rotate(self, direction: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let t = q.cross(direction) * 2.0;
        direction + t * self.w + q.cross(t)
    }

    /// Short-arc blend. A negative dot product flips one side before the blend.
    pub fn slerp(self, mut rhs: Self, t: f32) -> Self {
        let mut dot = self.dot(rhs);
        if dot < 0.0 {
            rhs = -rhs;
            dot = -dot;
        }
        if dot > 0.9995 {
            return (self * (1.0 - t) + rhs * t).normalize();
        }
        let theta = dot.clamp(-1.0, 1.0).acos();
        let sin_theta = theta.sin();
        let scale0 = ((1.0 - t) * theta).sin() / sin_theta;
        let scale1 = (t * theta).sin() / sin_theta;
        (self * scale0 + rhs * scale1).normalize()
    }
}

impl Add for Quat {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
            w: self.w + rhs.w,
        }
    }
}

impl Neg for Quat {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
            w: -self.w,
        }
    }
}

impl Mul<f32> for Quat {
    type Output = Self;
    fn mul(self, scale: f32) -> Self {
        Self {
            x: self.x * scale,
            y: self.y * scale,
            z: self.z * scale,
            w: self.w * scale,
        }
    }
}
