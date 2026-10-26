//! Renderer metrics. GPU readback is the candidate. The CPU model is the reference.
//!
//! Media and near references are a local closed form, not `genos_math`.
//! The shaft reference is the CPU shadowed integral: occlusion is not a closed form.
//! A row stays `NOT-IMPLEMENTED` until that pass exists. A zero is not a missing pass.

use std::cell::RefCell;

use genos_lighting_ref::{blur_relative, MetricParams, Picture};
use genos_render::{
    column_luma, flicker_dispatch, integrate_unshadowed, local_volume_leak, media_dispatch,
    near_dispatch, picture_from, point_at, ray_dir, shaft_dispatch, shaft_image, slice_edge,
    smear_dispatch, smear_tracking, sphere_dispatch, sphere_volume, still_flicker, FogColumn,
    FogDispatch, FogImage, FroxelGrid,
    LightingPreset, FOG_FLIP_G, FOG_IGNORE_SPHERE, FOG_LINEAR_SLICE, FOG_SKIP_EXTINCTION,
    FOG_SKIP_SHADOW, Z_FAR, Z_NEAR,
};

use crate::device::{allocation_mb, fitted_cost, run, unfitted_ms};
use crate::report::{judge, Metric};
use crate::Fixture;

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

const NO_GI: &str =
    "world-cache in-scattering waits for lighting phase 3 (cursor/lighting-p3-capped-refresh)";
const NO_TIME: &str = "no lavapipe window for a fog timestamp";
const LAVAPIPE_BYTES: &str = "lavapipe allocation, decimal MB";
const LAVAPIPE_MS: &str = "lavapipe timestamp, not an RTX 4070 measurement";
const SMEAR_NOTE: &str = "lavapipe GPU readback, High 160x90x48, all 320 yaw frames";
const SHAFT_NOTE: &str = "lavapipe GPU shadowed columns against the CPU shadowed integral";
const FLICKER_NOTE: &str = "lavapipe GPU frame N against frame N+1, camera still";
const MEDIA_NOTE: &str = "lavapipe GPU columns against a local closed form";
const SPHERE_NOTE: &str = "lavapipe extinction image, peak outside over peak inside";

/// One sample per yaw frame. A stride of 16 measures the same integral at 0.695 px
/// on the CPU model, above the frame-to-frame gate.
const SMEAR_STEP: u32 = 1;

#[derive(Clone, Copy)]
struct Good {
    smear: Option<f64>,
    shaft: Option<f64>,
    flicker: Option<f64>,
    media: Option<f64>,
    near: Option<f64>,
    sphere: Option<f64>,
    vram: [Option<f64>; 4],
    ms: [Option<f64>; 4],
}

thread_local! {
    static GOOD: RefCell<Option<Good>> = const { RefCell::new(None) };
}

pub(crate) fn evaluate_feature(fixture: Fixture) -> Vec<Metric> {
    match fixture {
        Fixture::Broken => broken_feature(),
        Fixture::Good => good_feature(),
    }
}

fn good_feature() -> Vec<Metric> {
    let good = cached_good();
    vec![
        optional("froxel_temporal_smear_px", good.smear, FROXEL_TEMPORAL_SMEAR_PX, SMEAR_NOTE),
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
        optional(
            "sphere_volume_leak",
            good.sphere,
            LOCAL_VOLUME_LEAK,
            SPHERE_NOTE,
        ),
        optional(
            "shaft_shadow_blur_rel",
            good.shaft,
            SHAFT_SHADOW_BLUR_REL,
            SHAFT_NOTE,
        ),
        absent("gi_inscatter_blur_rel", GI_INSCATTER_BLUR_REL, NO_GI),
        optional("media_path_blur_rel", good.media, MEDIA_PATH_BLUR_REL, MEDIA_NOTE),
        optional("near_fog_blur_rel", good.near, NEAR_FOG_BLUR_REL, MEDIA_NOTE),
        optional(
            "fog_still_flicker",
            good.flicker,
            FOG_STILL_FLICKER,
            FLICKER_NOTE,
        ),
        vram_row("froxel_vram_potato_mb", good.vram[0], FROXEL_VRAM_POTATO_MB),
        vram_row("froxel_vram_low_mb", good.vram[1], FROXEL_VRAM_LOW_MB),
        vram_row("froxel_vram_medium_mb", good.vram[2], FROXEL_VRAM_MEDIUM_MB),
        vram_row("froxel_vram_high_mb", good.vram[3], FROXEL_VRAM_HIGH_MB),
        ms_row("froxel_ms_potato", good.ms[0], FROXEL_MS_POTATO),
        ms_row("froxel_ms_low", good.ms[1], FROXEL_MS_LOW),
        ms_row("froxel_ms_medium", good.ms[2], FROXEL_MS_MEDIUM),
        ms_row("froxel_ms_high", good.ms[3], FROXEL_MS_HIGH),
    ]
}

fn broken_feature() -> Vec<Metric> {
    let presets = [
        LightingPreset::Potato,
        LightingPreset::Low,
        LightingPreset::Medium,
        LightingPreset::High,
    ];
    let byte_ceilings = [
        FROXEL_VRAM_POTATO_MB,
        FROXEL_VRAM_LOW_MB,
        FROXEL_VRAM_MEDIUM_MB,
        FROXEL_VRAM_HIGH_MB,
    ];
    let time_ceilings = [
        FROXEL_MS_POTATO,
        FROXEL_MS_LOW,
        FROXEL_MS_MEDIUM,
        FROXEL_MS_HIGH,
    ];
    let mut rows = vec![
        present(
            "froxel_temporal_smear_px",
            genos_render::smear_broken(),
            FROXEL_TEMPORAL_SMEAR_PX,
        ),
        present(
            "fog_yaw_pop_rel",
            f64::from(genos_render::yaw_pop_broken()),
            FOG_YAW_POP_REL,
        ),
        present(
            "local_volume_leak",
            f64::from(genos_render::leak_broken()),
            LOCAL_VOLUME_LEAK,
        ),
        present(
            "sphere_volume_leak",
            f64::from(genos_render::sphere_leak_broken()),
            LOCAL_VOLUME_LEAK,
        ),
        present("shaft_shadow_blur_rel", shaft_grade_cpu(false), SHAFT_SHADOW_BLUR_REL),
        absent("gi_inscatter_blur_rel", GI_INSCATTER_BLUR_REL, NO_GI),
        present("media_path_blur_rel", scaled_media(false), MEDIA_PATH_BLUR_REL),
        present("near_fog_blur_rel", scaled_media(true), NEAR_FOG_BLUR_REL),
        present(
            "fog_still_flicker",
            f64::from(genos_render::still_flicker_broken()),
            FOG_STILL_FLICKER,
        ),
    ];
    for (preset, ceiling) in presets.into_iter().zip(byte_ceilings) {
        let name = vram_name(preset);
        rows.push(vram_row(name, allocation_mb(preset, true), ceiling));
    }
    for (preset, ceiling) in presets.into_iter().zip(time_ceilings) {
        let name = ms_name(preset);
        rows.push(ms_row(name, unfitted_ms(preset), ceiling));
    }
    rows
}

const fn vram_name(preset: LightingPreset) -> &'static str {
    match preset {
        LightingPreset::Potato | LightingPreset::DayCycle | LightingPreset::Gi1Bounce => {
            "froxel_vram_potato_mb"
        }
        LightingPreset::Low => "froxel_vram_low_mb",
        LightingPreset::Medium => "froxel_vram_medium_mb",
        LightingPreset::High => "froxel_vram_high_mb",
    }
}

const fn ms_name(preset: LightingPreset) -> &'static str {
    match preset {
        LightingPreset::Potato | LightingPreset::DayCycle | LightingPreset::Gi1Bounce => {
            "froxel_ms_potato"
        }
        LightingPreset::Low => "froxel_ms_low",
        LightingPreset::Medium => "froxel_ms_medium",
        LightingPreset::High => "froxel_ms_high",
    }
}

fn cached_good() -> Good {
    GOOD.with(|slot| {
        if let Some(good) = *slot.borrow() {
            return good;
        }
        let measured = measure_good();
        *slot.borrow_mut() = Some(measured);
        measured
    })
}

fn measure_good() -> Good {
    let presets = [
        LightingPreset::Potato,
        LightingPreset::Low,
        LightingPreset::Medium,
        LightingPreset::High,
    ];
    Good {
        smear: Some(smear_grade()),
        shaft: Some(shaft_grade(0)),
        flicker: Some(flicker_grade()),
        media: Some(media_grade(0, false)),
        near: Some(media_grade(0, true)),
        sphere: Some(sphere_grade(0)),
        vram: presets.map(|preset| allocation_mb(preset, false)),
        ms: presets.map(|preset| fitted_cost(preset).map(|cost| cost.milliseconds)),
    }
}

/// One planted bug against the row it is supposed to fail.
#[must_use]
pub fn planted_scores() -> Vec<(&'static str, f64, f64)> {
    vec![
        (
            "media_path_blur_rel",
            media_grade(FOG_FLIP_G, false),
            MEDIA_PATH_BLUR_REL,
        ),
        (
            "near_fog_blur_rel",
            extinction_grade(),
            NEAR_FOG_BLUR_REL,
        ),
        (
            "shaft_shadow_blur_rel",
            shaft_grade(FOG_SKIP_SHADOW),
            SHAFT_SHADOW_BLUR_REL,
        ),
        (
            "sphere_volume_leak",
            sphere_grade(FOG_IGNORE_SPHERE),
            LOCAL_VOLUME_LEAK,
        ),
        (
            "media_path_blur_rel",
            linear_slice_grade(),
            MEDIA_PATH_BLUR_REL,
        ),
        (
            "froxel_vram_high_mb",
            allocation_mb(LightingPreset::High, true).unwrap_or(0.0),
            FROXEL_VRAM_HIGH_MB,
        ),
    ]
}

fn smear_grade() -> f64 {
    smear_tracking(SMEAR_STEP, |frame| {
        let job = smear_dispatch(frame);
        let Some(gpu) = run(&job) else {
            return Vec::new();
        };
        column_luma(&gpu.columns)
    })
}

fn flicker_grade() -> f64 {
    let job = flicker_dispatch();
    let Some(first) = run(&job) else {
        return 1.0;
    };
    let Some(second) = run(&job) else {
        return 1.0;
    };
    f64::from(still_flicker(
        &column_luma(&first.columns),
        &column_luma(&second.columns),
    ))
}

fn sphere_grade(flags: u32) -> f64 {
    let job = sphere_dispatch(flags);
    let Some(gpu) = run(&job) else {
        return 1.0;
    };
    let grid = FroxelGrid {
        size: job.size,
        extinction: gpu.extinction,
    };
    f64::from(local_volume_leak(&grid, &sphere_volume(), &job.view))
}

fn shaft_grade(flags: u32) -> f64 {
    let job = shaft_dispatch(flags);
    let Some(gpu) = run(&job) else {
        return 1.0;
    };
    image_grade(shaft_image(true), picture_from(job.size, &gpu.columns))
}

fn shaft_grade_cpu(traced: bool) -> f64 {
    image_grade(shaft_image(true), shaft_image(traced))
}

/// Dense near segment. Skipping per-slice extinction is visible once optical depth is not tiny.
fn extinction_grade() -> f64 {
    let mut job = near_dispatch(FOG_SKIP_EXTINCTION);
    job.settings.density = 1.0;
    let Some(gpu) = run(&job) else {
        return 1.0;
    };
    image_grade(
        closed_picture(&job, true),
        picture_from(job.size, &gpu.columns),
    )
}

/// Local sphere on linear slice edges, against the exponential CPU integral.
fn linear_slice_grade() -> f64 {
    let mut job = sphere_dispatch(FOG_LINEAR_SLICE);
    job.read_extinction = false;
    let Some(gpu) = run(&job) else {
        return 1.0;
    };
    let cpu = integrate_unshadowed(&job.settings, &job.view, job.size, job.sun);
    image_grade(
        picture_from(job.size, &cpu),
        picture_from(job.size, &gpu.columns),
    )
}

fn media_grade(flags: u32, near: bool) -> f64 {
    let job = if near {
        near_dispatch(flags)
    } else {
        media_dispatch(flags, 0)
    };
    let Some(gpu) = run(&job) else {
        return 1.0;
    };
    let reference = closed_picture(&job, near);
    image_grade(reference, picture_from(job.size, &gpu.columns))
}

fn scaled_media(near: bool) -> f64 {
    let (mut candidate, reference) = if near {
        genos_render::near_images()
    } else {
        genos_render::media_images()
    };
    for value in &mut candidate.linear {
        *value *= 1.4;
    }
    image_grade(reference, candidate)
}

fn closed_picture(job: &FogDispatch, near: bool) -> FogImage {
    let mut columns = Vec::new();
    let stop = if job.stop_slices == 0 {
        job.size.slices
    } else {
        job.stop_slices.min(job.size.slices)
    };
    let z1 = if near {
        slice_edge(stop, job.size.slices)
    } else {
        Z_FAR
    };
    for y in 0..job.size.height {
        for x in 0..job.size.width {
            columns.push(closed_column(job, x, y, z1, near));
        }
    }
    picture_from(job.size, &columns)
}

fn closed_column(job: &FogDispatch, x: u32, y: u32, z1: f32, near: bool) -> FogColumn {
    let dir = ray_dir(&job.view, job.size, x, y);
    let p0 = point_at(&job.view, job.size, x, y, Z_NEAR);
    let p1 = point_at(&job.view, job.size, x, y, z1);
    let delta = sub(p1, p0);
    let distance = length(delta);
    let tau = if near {
        if distance <= 1.0e-8 || job.settings.density <= 0.0 {
            0.0
        } else {
            height_optical_depth(
                job.settings.density,
                job.settings.thickness,
                job.settings.height,
                p0[1],
                delta[1],
                dir[1],
                distance,
            )
        }
    } else {
        tail_optical_depth(job, p0[1], dir[1])
    };
    let mu = dot(dir, job.sun.toward).clamp(-1.0, 1.0);
    let phase = phase_hg(job.settings.phase_g, mu);
    let albedo = job.settings.albedo.clamp(0.0, 1.0);
    let ratio = scale_color(job.sun.color, albedo * phase);
    let (absorb, transmittance) = if !tau.is_finite() || tau >= 1.0e6 {
        (1.0, 0.0)
    } else {
        let through = transmittance_of(tau);
        (1.0 - through, through)
    };
    FogColumn {
        inscatter: scale_color(ratio, absorb),
        transmittance,
    }
}

fn tail_optical_depth(job: &FogDispatch, y0: f32, cos_theta: f32) -> f32 {
    if job.settings.density <= 0.0 {
        return 0.0;
    }
    let sigma = height_density(
        job.settings.density,
        job.settings.thickness,
        y0,
        job.settings.height,
    );
    if sigma <= 0.0 {
        return 0.0;
    }
    if cos_theta.abs() < 1.0e-4 || job.settings.thickness <= 0.0 || cos_theta <= 0.0 {
        return f32::INFINITY;
    }
    sigma * job.settings.thickness / cos_theta
}

fn height_density(sigma0: f32, thickness: f32, y: f32, base: f32) -> f32 {
    if thickness <= 0.0 {
        sigma0
    } else {
        sigma0 * (-(y - base) / thickness).exp()
    }
}

fn height_optical_depth(
    sigma0: f32,
    thickness: f32,
    base: f32,
    y0: f32,
    dy: f32,
    cos_theta: f32,
    distance: f32,
) -> f32 {
    let sigma_y0 = height_density(sigma0, thickness, y0, base);
    if cos_theta.abs() < 1.0e-4 || thickness <= 0.0 {
        sigma_y0 * distance.abs()
    } else {
        sigma_y0 * thickness * (1.0 - (-dy / thickness).exp()) / cos_theta
    }
}

fn phase_hg(g: f32, mu: f32) -> f32 {
    let denom = 1.0 + g * g - 2.0 * g * mu;
    if denom <= 0.0 {
        0.0
    } else {
        (1.0 - g * g) / (4.0 * std::f32::consts::PI * denom.powf(1.5))
    }
}

fn transmittance_of(optical_depth: f32) -> f32 {
    (-optical_depth).exp()
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(v: [f32; 3]) -> f32 {
    dot(v, v).sqrt()
}

fn scale_color(color: [f32; 3], factor: f32) -> [f32; 3] {
    [color[0] * factor, color[1] * factor, color[2] * factor]
}

fn image_grade(reference: FogImage, candidate: FogImage) -> f64 {
    let Ok(reference) = Picture::new(reference.width, reference.height, reference.linear) else {
        return 1.0;
    };
    let Ok(candidate) = Picture::new(candidate.width, candidate.height, candidate.linear) else {
        return 1.0;
    };
    blur_relative(&reference, &candidate, &MetricParams::default()).unwrap_or(1.0)
}

fn vram_row(name: &'static str, value: Option<f64>, threshold: f64) -> Metric {
    optional(name, value, threshold, LAVAPIPE_BYTES)
}

fn ms_row(name: &'static str, value: Option<f64>, threshold: f64) -> Metric {
    optional(name, value, threshold, LAVAPIPE_MS)
}

fn optional(name: &'static str, value: Option<f64>, threshold: f64, note: &'static str) -> Metric {
    match value {
        Some(value) => noted(name, Some(value), threshold, note),
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
    Metric {
        name,
        value: None,
        threshold,
        status: judge(false, Some(0.0), threshold),
        note,
    }
}
