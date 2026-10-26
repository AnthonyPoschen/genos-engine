//! Renderer metrics. A row stays `NOT-IMPLEMENTED` until that pass exists.
//! A zero is not substituted for a missing pass.

use genos_lighting_ref::{blur_relative, MetricParams, Picture};

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
const NO_GI: &str =
    "world-cache in-scattering waits for lighting phase 3 (cursor/lighting-p3-capped-refresh)";
const NO_ALLOC: &str = "froxel buffers are not allocated";
const NO_TIME: &str = "froxel pass is not timed";

pub(crate) fn evaluate_feature() -> Vec<Metric> {
    vec![
        present(
            "froxel_temporal_smear_px",
            genos_render::smear_fixture(),
            FROXEL_TEMPORAL_SMEAR_PX,
        ),
        present(
            "fog_yaw_pop_rel",
            f64::from(genos_render::yaw_pop_fixture()),
            FOG_YAW_POP_REL,
        ),
        present(
            "local_volume_leak",
            f64::from(genos_render::leak_fixture()),
            LOCAL_VOLUME_LEAK,
        ),
        present(
            "shaft_shadow_blur_rel",
            shaft_grade(true),
            SHAFT_SHADOW_BLUR_REL,
        ),
        absent("gi_inscatter_blur_rel", GI_INSCATTER_BLUR_REL, NO_GI),
        absent("media_path_blur_rel", MEDIA_PATH_BLUR_REL, NO_MEDIA),
        absent("near_fog_blur_rel", NEAR_FOG_BLUR_REL, NO_FROXEL),
        present(
            "fog_still_flicker",
            f64::from(genos_render::still_flicker_fixture()),
            FOG_STILL_FLICKER,
        ),
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

fn shaft_grade(traced_shadows: bool) -> f64 {
    let reference = genos_render::shaft_image(true);
    let candidate = genos_render::shaft_image(traced_shadows);
    let Ok(reference) = Picture::new(reference.width, reference.height, reference.linear) else {
        return 1.0;
    };
    let Ok(candidate) = Picture::new(candidate.width, candidate.height, candidate.linear) else {
        return 1.0;
    };
    blur_relative(&reference, &candidate, &MetricParams::default()).unwrap_or(1.0)
}

fn present(name: &'static str, value: f64, threshold: f64) -> Metric {
    Metric {
        name,
        value: Some(value),
        threshold,
        status: judge(true, Some(value), threshold),
        note: "",
    }
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
