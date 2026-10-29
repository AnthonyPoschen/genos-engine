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
//! - Proposal is uniform, an exact discrete target CDF, or a light-tree bound
//!   (Conty Estevez and Kulla, HPG 2018). The tree draws one leaf per candidate.
//!   It is not the flat target CDF, and it does not split a node into several
//!   samples. Departures are in `tree`.
//! - `p̂` is unshadowed Rec. 709 luminance. One shadow ray multiplies the shade.
//! - Point lamps and a Lambertian BRDF only. No sun, no BSDF sample.
//! - Receivers are a grid. Temporal history is the same index, not a motion vector.
//! - Occluders are boxes, not mesh SDFs.
//! - One spatial pass. History is the post-temporal reservoir, not the spatial one.
//! - History `M` is clamped to `m_clamp` times the current frame's candidate
//!   count before the temporal merge (default 20, Bitterli et al. 2020).
//!   `m_clamp = 0` leaves `M` unclamped. The output of the merge can be one
//!   candidate above the cap.
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
pub mod tree;

pub use dump::{diff_dumps, encode_dump, parse_dump, Diff, Dump};
pub use harness::{encode_report, evaluate, write_run, EvalSpec, Report};
pub use metrics::{frames_to_ref, grid_worst, ref_err_near, GateLimits};
pub use restir::{Config, Counters, History, Proposal, Session, DEFAULT_M_CLAMP};
pub use tree::{LightTree, TreeFault};
pub use shade::Scene;
