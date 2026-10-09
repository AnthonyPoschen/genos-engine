//! Triangle meshes on the CPU: the triangle soup every tracer and baker shares, and
//! an in-repo bounding volume hierarchy (ADR 0007) for ray and closest-point queries.
//!
//! The GPU builds its own acceleration structures. This BVH serves the CPU reference
//! path tracer, the import-time bakers (distance fields, cards) and tests.

mod bvh;
mod triangle;

pub use bvh::{Bvh, RayHit};
pub use triangle::{Aabb, MeshTriangle};
