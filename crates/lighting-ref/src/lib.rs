//! Reference-image checks for lighting.
//!
//! The metric core stays free of the window and of Vulkan so a defect injected
//! into a picture can be caught without a GPU. `genos-stress` owns the scenes
//! and is the driver. Reports and reference pictures are written under
//! `./genos-debug/` in the current directory.

pub mod baseline;
pub mod catalog;
pub mod config;
pub mod flip;
pub mod metrics;
pub mod paths;
pub mod picture;
pub mod reflections;
pub mod policy;
pub mod report;
pub mod selftest;
pub mod store;
pub mod verdict;

pub use metrics::{
    blur_radius, blur_relative, mean_luma_ratio, mean_relative, measure, settle_line, FrameMetrics,
    MetricParams,
};

pub use catalog::{available_checkpoints, checkpoints, scene, scenes, CheckpointDesc, SceneDesc};
pub use paths::{refcheck_dir, reference_dir, utc_stamp, DEBUG_DIR};
pub use picture::Picture;
pub use store::{write_manifest, write_reference, Manifest, ReferenceRecord};
