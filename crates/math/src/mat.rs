//! Column-major matrices. `A * B` applies `B` first. A matrix multiplies a column vector.

use crate::quat::Quat;
use crate::vec::{Vec3, Vec4};
use std::ops::Mul;

/// A 3×3 matrix. Column-major. Index is `column * 3 + row`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3 {
    pub cols: [f32; 9],
}

impl Mat3 {
    pub const IDENTITY: Self = Self {
        cols: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
    };

    pub const fn from_cols(cols: [f32; 9]) -> Self {
        Self { cols }
    }

    pub fn from_scale(scale: Vec3) -> Self {
        Self {
            cols: [scale.x, 0.0, 0.0, 0.0, scale.y, 0.0, 0.0, 0.0, scale.z],
        }
    }

    /// Rotation that matches `Quat::rotate` for a unit quaternion.
    pub fn from_quat(rotation: Quat) -> Self {
        let (x, y, z, w) = (rotation.x, rotation.y, rotation.z, rotation.w);
        let xx = x * x;
        let yy = y * y;
        let zz = z * z;
        let xy = x * y;
        let xz = x * z;
        let yz = y * z;
        let wx = w * x;
        let wy = w * y;
        let wz = w * z;
        Self {
            cols: [
                1.0 - 2.0 * (yy + zz),
                2.0 * (xy + wz),
                2.0 * (xz - wy),
                2.0 * (xy - wz),
                1.0 - 2.0 * (xx + zz),
                2.0 * (yz + wx),
                2.0 * (xz + wy),
                2.0 * (yz - wx),
                1.0 - 2.0 * (xx + yy),
            ],
        }
    }

    pub fn from_axis_angle(axis: Vec3, radians: f32) -> Self {
        Self::from_quat(Quat::from_axis_angle(axis, radians))
    }

    pub fn transform(self, direction: Vec3) -> Vec3 {
        let m = &self.cols;
        Vec3::new(
            m[0] * direction.x + m[3] * direction.y + m[6] * direction.z,
            m[1] * direction.x + m[4] * direction.y + m[7] * direction.z,
            m[2] * direction.x + m[5] * direction.y + m[8] * direction.z,
        )
    }
}

impl Mul for Mat3 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        let mut cols = [0.0; 9];
        for col in 0..3 {
            for row in 0..3 {
                cols[col * 3 + row] = self.cols[row] * rhs.cols[col * 3]
                    + self.cols[3 + row] * rhs.cols[col * 3 + 1]
                    + self.cols[6 + row] * rhs.cols[col * 3 + 2];
            }
        }
        Self { cols }
    }
}

/// A 4×4 matrix. Column-major. Index is `column * 4 + row`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    pub cols: [f32; 16],
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        cols: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
    };

    pub const fn from_cols(cols: [f32; 16]) -> Self {
        Self { cols }
    }

    pub fn from_scale(scale: Vec3) -> Self {
        Self {
            cols: [
                scale.x, 0.0, 0.0, 0.0, 0.0, scale.y, 0.0, 0.0, 0.0, 0.0, scale.z, 0.0, 0.0, 0.0,
                0.0, 1.0,
            ],
        }
    }

    pub fn from_translation(offset: Vec3) -> Self {
        Self {
            cols: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, offset.x, offset.y,
                offset.z, 1.0,
            ],
        }
    }

    pub fn from_quat(rotation: Quat) -> Self {
        let linear = Mat3::from_quat(rotation);
        let m = linear.cols;
        Self {
            cols: [
                m[0], m[1], m[2], 0.0, m[3], m[4], m[5], 0.0, m[6], m[7], m[8], 0.0, 0.0, 0.0, 0.0,
                1.0,
            ],
        }
    }

    pub fn from_axis_angle(axis: Vec3, radians: f32) -> Self {
        Self::from_quat(Quat::from_axis_angle(axis, radians))
    }

    /// Affine point. `w` is treated as 1 and is not divided out.
    pub fn transform_point(self, point: Vec3) -> Vec3 {
        let m = &self.cols;
        Vec3::new(
            m[0] * point.x + m[4] * point.y + m[8] * point.z + m[12],
            m[1] * point.x + m[5] * point.y + m[9] * point.z + m[13],
            m[2] * point.x + m[6] * point.y + m[10] * point.z + m[14],
        )
    }

    /// Direction. Translation is ignored.
    pub fn transform_vector(self, direction: Vec3) -> Vec3 {
        let m = &self.cols;
        Vec3::new(
            m[0] * direction.x + m[4] * direction.y + m[8] * direction.z,
            m[1] * direction.x + m[5] * direction.y + m[9] * direction.z,
            m[2] * direction.x + m[6] * direction.y + m[10] * direction.z,
        )
    }

    /// Homogeneous point. The caller divides by `w` when it needs NDC.
    pub fn transform_homogeneous(self, point: Vec3) -> Vec4 {
        let m = &self.cols;
        Vec4::new(
            m[0] * point.x + m[4] * point.y + m[8] * point.z + m[12],
            m[1] * point.x + m[5] * point.y + m[9] * point.z + m[13],
            m[2] * point.x + m[6] * point.y + m[10] * point.z + m[14],
            m[3] * point.x + m[7] * point.y + m[11] * point.z + m[15],
        )
    }
}

impl Mul for Mat4 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        let mut cols = [0.0; 16];
        for col in 0..4 {
            for row in 0..4 {
                cols[col * 4 + row] = self.cols[row] * rhs.cols[col * 4]
                    + self.cols[4 + row] * rhs.cols[col * 4 + 1]
                    + self.cols[8 + row] * rhs.cols[col * 4 + 2]
                    + self.cols[12 + row] * rhs.cols[col * 4 + 3];
            }
        }
        Self { cols }
    }
}

impl Mul<Vec4> for Mat4 {
    type Output = Vec4;
    fn mul(self, rhs: Vec4) -> Vec4 {
        let m = &self.cols;
        Vec4::new(
            m[0] * rhs.x + m[4] * rhs.y + m[8] * rhs.z + m[12] * rhs.w,
            m[1] * rhs.x + m[5] * rhs.y + m[9] * rhs.z + m[13] * rhs.w,
            m[2] * rhs.x + m[6] * rhs.y + m[10] * rhs.z + m[14] * rhs.w,
            m[3] * rhs.x + m[7] * rhs.y + m[11] * rhs.z + m[15] * rhs.w,
        )
    }
}
