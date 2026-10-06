//! Positions and directions. `f32` components. The values are `Copy`.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

macro_rules! vec_arith {
    ($ty:ident { $($field:ident),+ }) => {
        impl Add for $ty {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self { $($field: self.$field + rhs.$field),+ }
            }
        }

        impl Sub for $ty {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                Self { $($field: self.$field - rhs.$field),+ }
            }
        }

        impl Neg for $ty {
            type Output = Self;
            fn neg(self) -> Self {
                Self { $($field: -self.$field),+ }
            }
        }

        impl Mul<f32> for $ty {
            type Output = Self;
            fn mul(self, scale: f32) -> Self {
                Self { $($field: self.$field * scale),+ }
            }
        }

        impl Mul<$ty> for f32 {
            type Output = $ty;
            fn mul(self, rhs: $ty) -> $ty {
                rhs * self
            }
        }

        impl Div<f32> for $ty {
            type Output = Self;
            fn div(self, scale: f32) -> Self {
                Self { $($field: self.$field / scale),+ }
            }
        }

        impl AddAssign for $ty {
            fn add_assign(&mut self, rhs: Self) {
                *self = *self + rhs;
            }
        }

        impl SubAssign for $ty {
            fn sub_assign(&mut self, rhs: Self) {
                *self = *self - rhs;
            }
        }
    };
}

macro_rules! vec_metrics {
    ($ty:ident { $($field:ident),+ }) => {
        impl $ty {
            pub fn dot(self, rhs: Self) -> f32 {
                0.0 $(+ self.$field * rhs.$field)+
            }

            pub fn length_squared(self) -> f32 {
                self.dot(self)
            }

            pub fn length(self) -> f32 {
                self.length_squared().sqrt()
            }

            /// Unit length. A zero vector stays zero.
            pub fn normalize(self) -> Self {
                let length_sq = self.length_squared();
                if length_sq <= f32::MIN_POSITIVE {
                    return Self::ZERO;
                }
                self / length_sq.sqrt()
            }

            pub fn lerp(self, rhs: Self, t: f32) -> Self {
                self + (rhs - self) * t
            }
        }
    };
}

/// A 2-component position or direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const X: Self = Self { x: 1.0, y: 0.0 };
    pub const Y: Self = Self { x: 0.0, y: 1.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

vec_arith!(Vec2 { x, y });
vec_metrics!(Vec2 { x, y });

/// A 3-component position or direction. Y is up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub const X: Self = Self {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    pub const Y: Self = Self {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    pub const Z: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - self.z * rhs.y,
            y: self.z * rhs.x - self.x * rhs.z,
            z: self.x * rhs.y - self.y * rhs.x,
        }
    }
}

vec_arith!(Vec3 { x, y, z });
vec_metrics!(Vec3 { x, y, z });

/// A 4-component vector. `w` is the homogeneous coordinate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Vec4 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }
}

vec_arith!(Vec4 { x, y, z, w });
vec_metrics!(Vec4 { x, y, z, w });
