//! Renderer metrics. A row stays `NOT-IMPLEMENTED` until that pass exists.
//! A zero is not substituted for a missing pass.

use genos_lighting_ref::{blur_relative, MetricParams, Picture};
use genos_render::LightingPreset;

use crate::device::fitted_cost;
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
const NO_TIME: &str = "no lavapipe window for a fog timestamp";
const LAVAPIPE_BYTES: &str = "lavapipe allocation, decimal MB";
const LAVAPIPE_MS: &str = "lavapipe timestamp, not an RTX 4070 measurement";

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
        vram("froxel_vram_potato_mb", LightingPreset::Potato, FROXEL_VRAM_POTATO_MB),
        vram("froxel_vram_low_mb", LightingPreset::Low, FROXEL_VRAM_LOW_MB),
        vram("froxel_vram_medium_mb", LightingPreset::Medium, FROXEL_VRAM_MEDIUM_MB),
        vram("froxel_vram_high_mb", LightingPreset::High, FROXEL_VRAM_HIGH_MB),
        milliseconds("froxel_ms_potato", LightingPreset::Potato, FROXEL_MS_POTATO),
        milliseconds("froxel_ms_low", LightingPreset::Low, FROXEL_MS_LOW),
        milliseconds("froxel_ms_medium", LightingPreset::Medium, FROXEL_MS_MEDIUM),
        milliseconds("froxel_ms_high", LightingPreset::High, FROXEL_MS_HIGH),
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

fn vram(name: &'static str, preset: LightingPreset, threshold: f64) -> Metric {
    noted(
        name,
        Some(genos_render::allocation_mb(preset)),
        threshold,
        LAVAPIPE_BYTES,
    )
}

fn milliseconds(name: &'static str, preset: LightingPreset, threshold: f64) -> Metric {
    match fitted_cost(preset) {
        Some(cost) => noted(name, Some(cost.milliseconds), threshold, LAVAPIPE_MS),
        None => absent(name, threshold, NO_TIME),
    }
}

fn present(name: &'static str, value: f64, threshold: f64) -> Metric {
    noted(name, Some(value), threshold, "")
}

fn noted(name: &'static str, value: Option<f64>, threshold: f64, note: &'static str) -> Metric {
    Metric {
        name,
        value,
        threshold,
        status: judge(value.is_some(), value, threshold),
        note,
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
