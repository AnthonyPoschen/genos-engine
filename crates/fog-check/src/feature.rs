//! Renderer metrics. The froxel pass and the media path tracer are not in
//! this tree, so every row is `NOT-IMPLEMENTED`. A zero is not substituted.

use crate::report::{judge, Metric};

const FROXEL_TEMPORAL_SMEAR_PX: f64 = 0.50;
const FOG_YAW_POP_REL: f64 = 0.05;
const LOCAL_VOLUME_LEAK: f64 = 1.0e-4;
const SHAFT_SHADOW_BLUR_REL: f64 = 0.12;
const GI_INSCATTER_BLUR_REL: f64 = 0.12;
const MEDIA_PATH_BLUR_REL: f64 = 0.12;
const NEAR_FOG_BLUR_REL: f64 = 0.08;
const FOG_STILL_FLICKER: f64 = 0.02;
const FROXEL_VRAM_POTATO_MB: f64 = 0.05;
const FROXEL_VRAM_LOW_MB: f64 = 1.5;
const FROXEL_VRAM_MEDIUM_MB: f64 = 4.0;
const FROXEL_VRAM_HIGH_MB: f64 = 12.0;
const FROXEL_MS_POTATO: f64 = 0.05;
const FROXEL_MS_LOW: f64 = 0.40;
const FROXEL_MS_MEDIUM: f64 = 0.80;
const FROXEL_MS_HIGH: f64 = 1.20;

const NO_FROXEL: &str = "froxel pass is not in the tree";
const NO_MEDIA: &str = "path tracer does not trace participating media";
const NO_GI: &str = "fog does not read the world cache";
const NO_ALLOC: &str = "froxel buffers are not allocated";
const NO_TIME: &str = "froxel pass is not timed";

pub(crate) fn evaluate_feature() -> Vec<Metric> {
    vec![
        absent(
            "froxel_temporal_smear_px",
            FROXEL_TEMPORAL_SMEAR_PX,
            NO_FROXEL,
        ),
        absent("fog_yaw_pop_rel", FOG_YAW_POP_REL, NO_FROXEL),
        absent("local_volume_leak", LOCAL_VOLUME_LEAK, NO_FROXEL),
        absent("shaft_shadow_blur_rel", SHAFT_SHADOW_BLUR_REL, NO_FROXEL),
        absent("gi_inscatter_blur_rel", GI_INSCATTER_BLUR_REL, NO_GI),
        absent("media_path_blur_rel", MEDIA_PATH_BLUR_REL, NO_MEDIA),
        absent("near_fog_blur_rel", NEAR_FOG_BLUR_REL, NO_FROXEL),
        absent("fog_still_flicker", FOG_STILL_FLICKER, NO_FROXEL),
        absent("froxel_vram_potato_mb", FROXEL_VRAM_POTATO_MB, NO_ALLOC),
        absent("froxel_vram_low_mb", FROXEL_VRAM_LOW_MB, NO_ALLOC),
        absent("froxel_vram_medium_mb", FROXEL_VRAM_MEDIUM_MB, NO_ALLOC),
        absent("froxel_vram_high_mb", FROXEL_VRAM_HIGH_MB, NO_ALLOC),
        absent("froxel_ms_potato", FROXEL_MS_POTATO, NO_TIME),
        absent("froxel_ms_low", FROXEL_MS_LOW, NO_TIME),
        absent("froxel_ms_medium", FROXEL_MS_MEDIUM, NO_TIME),
        absent("froxel_ms_high", FROXEL_MS_HIGH, NO_TIME),
    ]
}

fn absent(name: &'static str, threshold: f64, note: &'static str) -> Metric {
    // `Some(0.0)` must not become a pass while the probe is off.
    Metric {
        name,
        value: None,
        threshold,
        status: judge(false, Some(0.0), threshold),
        note,
    }
}
