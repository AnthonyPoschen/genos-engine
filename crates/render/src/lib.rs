mod aa;
mod budget;
mod fog;
mod fog_budget;
mod fog_pass;
mod gltf_draw;
mod gpu;
mod hdr_output;
#[cfg(test)]
mod lamp_pick;
mod mesh;
mod mesh_field;
mod occ_grid;
mod pack;
mod particles;
mod pool;
mod scene_mesh;
mod trace;
mod unfilled;
mod wire;
mod world;

pub use aa::Antialias;
pub use fog_pass::{
    box_dispatch, box_volume, column_luma, column_peak, composed_frame_camera, composed_frame_scene,
    flicker_dispatch, integrate_traced, media_dispatch, media_images, near_dispatch, near_images,
    picture_from, picture_mean_abs, picture_unchanged, shaft_dispatch, shaft_image, smear_broken,
    smear_dispatch, smear_fixture, smear_tracking, sphere_dispatch, sphere_volume, yaw_pop_broken,
    yaw_pop_fixture, yaw_pop_rel, FogDispatch, YAW_FRAMES,
    FogGpu, FogImage, FogPointLamp, FOG_FLIP_G, FOG_IGNORE_SPHERE, FOG_LINEAR_SLICE,
    FOG_SKIP_EXTINCTION, FOG_SKIP_SHADOW,
};
pub use fog_budget::{
    allocation_bytes, allocation_mb, allocation_mb_doubled, fit_pass, fog_probe,
    millisecond_ceiling, sample_cap, unfitted_milliseconds, FogProbe, PassCost,
};
pub use fog::{
    apply_fog, compose_camera_medium, compose_pixel_medium, default_frame_sun, dispatched_grid,
    frame_fog_view, grid_size, inject, point_at, ray_dir,
    integrate_samples, integrate_unshadowed,
    leak_broken,
    leak_fixture, local_volume_leak, puff_volume, sample_depth_aware, slice_edge, sphere_leak_broken,
    still_flicker,
    still_flicker_broken, still_flicker_fixture, uniform_delta_transmittance, world_cache_inscatter,
    DepthTap, DirectSun,
    FogColumn, FogView, FroxelGrid, GridSize, PixelRay, SliceSample, Z_FAR, Z_NEAR,
};
pub use budget::{FrameMemory, ImageAdmit};
pub use genos_bake as bake;
pub use genos_scene::{EngineSettings, LightingPreset, LightingSettings};
pub use gpu::{
    check_gi_metrics_arg, check_gpu_times_arg, check_lighting_sun_arg, gi_metrics_flag_needs_value,
    gpu_times_flag_needs_value, lighting_sun_flag_needs_value, DebugView, DrawProfile, GiSample,
    LampPickCounts, LampReport, LightingAtrousStats, LightingSunStats, Renderer, ScreenLine,
    ScreenRect, SettleStats, UnfilledControl, UnfilledStats, ViewMode,
};
pub use hdr_output::{
    aces2_rec709, agx_rec709, bloom_commute_error, bloom_downsample, bloom_downsample_weight_sum,
    bloom_pyramid, bloom_upsample,
    bloom_upsample_weight_sum, camera_ev100, display_transform, ev100_from_luminance,
    exposure_from_camera,
    exposure_from_ev100, exposure_adapt_outside, exposure_overshoot, exposure_still_change,
    exposure_with_compensation, grade_peak, grade_relative, grade_weight, histogram_ev100,
    histogram_ev100_bins, histogram_ev100_from_counts, karis_downsample_rgb,
    karis_firefly_response, karis_weight,
    step_exposure, agx_display_linear, graphics_white_linear, hdr10_pq, nits_from_display,
    rec2020_from_rec709, scrgb_from_display,
    legacy_display, legacy_display_channel, near_grade, pack_f32, pick_hdr_output, present_host, pq_st2084,
    rec709_y, resolve_swapchain, surface_choice, swapchain_format_code, swapchain_nits,
    swapchain_pair,
    scene_luma, scene_peak_error, scene_read_from_rgba, scene_referred, scrgb_code, still_pop_share,
    DisplayMode,
    HdrSwapchain, Image, ListedFormat, SceneRead, PresentHost, COLOR_SPACE_HDR10, COLOR_SPACE_SCRGB,
    COLOR_SPACE_SRGB, FORMAT_A2B10,
    FORMAT_A2R10, FORMAT_BGRA8, FORMAT_R16F, FORMAT_RGBA8, GRAPHICS_WHITE_NITS, HDR_PEAK_NITS,
    SCRGB_NITS_PER_CODE,
};
pub use mesh::tone;
pub use mesh_field::{field_words, instance_distance, FieldInstance, FIELD_STEPS};
pub use occ_grid::{analytic_hit, walk_grid, GridHit};
pub use pack::{build_grid, lamp_range, pack_frame, Pack, PackedDraw, SceneGrid, LAMP_CUTOFF};
pub use particles::{
    card_quad, compose_fog, fire_radiance, fog_along, frame_at, height_fog_along, image_from_map,
    image_from_texture, medium, object_cards, phase_weight, shade_lit, Card, Emitter, FireLight,
    FogHit, FogLamp, Kind, LiveParticle, Puff, Simulation, FIRE_COLOR, PROOF_STEPS, SMOKE_ALBEDO,
    SMOKE_DENSITY, SMOKE_RADIUS, STEP_DT,
};
pub use scene_mesh::{SceneMesh, ShapeRef, SurfaceMaterial, CIRCLE_SEGMENTS};
pub use trace::{
    agreement, direct_light, direct_light_in, first_outgoing, linear_from_display, luminance,
    radiance, radiance_hits, ray_land, samples_toward, sees, trace as trace_ray, Hit as TraceHit,
    Surfaces,
};
pub use unfilled::{check_unfilled_arg, unfilled_flag_needs_value};
pub use world::{
    fixed_bounds, identity_pose, in_view, solid_reach, Bounds, Displacement, DrawKind, FixedPart,
    MaterialAtlas, Object, ParticleFrame, ParticleImage, ShaderSpace, Surface, World,
};

/// `GENOS_SCENE_DELTA=1`: print each frame's resident scene update (S70C).
#[allow(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): read the config struct or a CLI flag, not the environment. docs/LINT_RULES.md"
)]
pub(crate) fn scene_delta_trace() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("GENOS_SCENE_DELTA").is_some())
}
