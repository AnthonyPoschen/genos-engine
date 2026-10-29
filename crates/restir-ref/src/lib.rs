//! CPU reference for `ReSTIR` direct lighting (ADR 0022 stage C).
//!
//! Initial candidates are resampled importance sampling (Talbot, Cline, and
//! Egbert, EGSR 2005). Temporal and spatial reuse follow Bitterli, Wyman,
//! et al., "Spatiotemporal reservoir resampling for real-time ray tracing
//! with dynamic direct lighting", TOG 2020. The weight is the balance
//! heuristic (Bitterli et al., "Generalized Resampled Importance Sampling",
//! SIGGRAPH 2022). Two reservoirs use the same threshold as
//! `genos_render::lamp_pick::pairwise_merge`. The renderer is unchanged.
//!
//! Departures from those papers:
//! - Proposal is uniform or an exact discrete target CDF, not a light tree.
//! - `p̂` is unshadowed Rec. 709 luminance. One shadow ray multiplies the shade.
//! - Point lamps and a Lambertian BRDF only. No sun, no BSDF sample.
//! - Receivers are a grid. Temporal history is the same index, not a motion vector.
//! - Occluders are boxes, not mesh SDFs.
//! - One spatial pass. History is the post-temporal reservoir, not the spatial one.
//! - `M` is not clamped. The shader's 24-bit field is wider than these scenes.
//! - An empty canonical sample returns the history record unchanged, matching
//!   `pairwise_merge`. A fresh candidate set is the normal frame.
//! - The four gates run on an unoccluded grid (`Scene::gate`). `Scene::shadow`
//!   adds one box and is the bias check only. One-sample visibility does not
//!   settle with `M`, and a multi-seed mean of it stays spatially white.
//!
//! Replicated state: none. The reservoirs stay in the checker.
//! Authority: the checker process.
//! Prediction / rollback: none.
//! Determinism: one PCG32 stream and the draw order in `restir`.
//! Headless server: this crate is headless. A game server does not run it.

pub mod dump;
pub mod harness;
pub mod metrics;
pub mod reservoir;
pub mod restir;
pub mod rng;
pub mod scene;
pub mod shade;

pub use dump::{diff_dumps, encode_dump, parse_dump, Diff, Dump};
pub use harness::{encode_report, evaluate, write_run, EvalSpec, Report};
pub use metrics::{frames_to_ref, grid_worst, ref_err_near, GateLimits};
pub use restir::{Config, Counters, History, Proposal, Session};
pub use shade::Scene;
