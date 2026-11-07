#![expect(
    clippy::many_single_char_names,
    reason = "reference maths keeps textbook names (p, n, d, t, h) so it reads against the papers"
)]

//! CPU reference for the ADR 0039 world cache: plain SH-L1 cells (stage 2),
//! in-cell position data (stage 2b), one-level subdivision (stage 2c), and the
//! cubic vs body-centred cubic (BCC) cell layout. No window and no Vulkan
//! device: a path tracer gives the ground truth and the cache runs frame by
//! frame on the CPU.
//!
//! Sources:
//! - Ramamoorthi and Hanrahan, "An Efficient Representation for Irradiance
//!   Environment Maps", SIGGRAPH 2001 (SH-L1 irradiance).
//! - Müller, Gross and Novák, "Practical Path Guiding for Efficient
//!   Light-Transport Simulation", EGSR 2017 (sum-based spatial subdivision).
//! - Entezari, Van De Ville and Möller, "Practical Box Splines for
//!   Reconstruction on the Body Centered Cubic Lattice", TVCG 14(2) 2008
//!   (BCC reconstruction), and Theußl, Möller and Gröller, "Optimal Regular
//!   Volume Sampling", IEEE Visualization 2001 (BCC needs 29.3 % fewer
//!   samples than cubic for isotropic band-limited volumes).
//!
//! Departures from the engine and those papers:
//! - Monochrome radiance. The engine is RGB; byte counts are given for RGB.
//! - Boxes, point lamps and one Lambertian albedo; receivers are world-space
//!   pixels on every surface facing into the room.
//! - Blotch is computed on surface pixels in world units, not screen pixels.
//! - A lamp change cuts every cell's history to `clear_keep_frames`, and the
//!   cap stays there for `recover_frames` so stale bounces flush. This stands
//!   in for the stage 1 convergence fix; the engine does not do it yet.
//! - Irradiance normalisation (`irr_norm`) and the Wiener gradient shrink
//!   (`wiener`) are additions found here, not in the ADR's first 2b list.
//! - Read visibility (`vis`) casts a real occlusion ray per neighbour on the
//!   CPU; its GPU cost is reported as a counter, not measured.
//! - BCC reads interpolate over the containing Delaunay tetrahedron (4 points)
//!   instead of a BCC box spline.
//!
//! Replicated state: none; the cache is client-side lighting.
//! Authority: the checker process.
//! Prediction / rollback: none.
//! Determinism: fixed seeds; one PCG32 stream per fixed work chunk, so results
//! do not depend on the thread count.
//! Headless server: this crate is headless.

pub mod cache;
pub mod geom;
pub mod harness;
pub mod lattice;
pub mod metrics;
pub mod reference;
pub mod report;
pub mod rng;
pub mod scene;
pub mod sh;
