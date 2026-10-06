//! Positions, directions, and rotations for movement.
//!
//! Components are `f32`. Matrices are column-major. `A * B` applies `B` first.
//! Y is up. The values are plain `Copy` values, and the operations do not allocate.

mod mat;
mod quat;
mod vec;

pub use mat::{Mat3, Mat4};
pub use quat::Quat;
pub use vec::{Vec2, Vec3, Vec4};
