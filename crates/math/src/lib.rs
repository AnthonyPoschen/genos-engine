//! Positions, directions, and rotations for movement, and the closed forms
//! for a participating medium and for emissive triangles.
//!
//! Vector components are `f32`. Matrices are column-major. `A * B` applies `B` first.
//! Y is up. The values are plain `Copy` values, and the operations do not allocate.
//! The medium closed forms and the emissive-triangle reference are `f64`. The
//! medium forms have `f32` wrappers for the renderer. The emissive reference
//! does not, until a renderer calls it.

pub mod emissive;
mod mat;
mod media;
pub mod optics;
mod quat;
pub mod tonemap;
mod vec;

pub use mat::{Mat3, Mat4};
pub use media::{
    height_fog_density, height_fog_density_f32, height_optical_depth, height_optical_depth_f32,
    homogeneous_inscatter, homogeneous_inscatter_f32, phase_hg, phase_hg_f32, transmittance,
    transmittance_f32, HEIGHT_FOG_HORIZONTAL_COS,
};
pub use quat::Quat;
pub use vec::{Vec2, Vec3, Vec4};
