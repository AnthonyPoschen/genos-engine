mod aa;
mod budget;
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
    aces2_rec709, agx_rec709, bloom_downsample, bloom_downsample_weight_sum, bloom_upsample,
    bloom_upsample_weight_sum, display_transform, ev100_from_luminance, exposure_from_camera,
    exposure_from_ev100, grade_peak, grade_relative, grade_weight, histogram_ev100,
    legacy_display, legacy_display_channel, near_grade, pick_hdr_output, pq_st2084, rec709_y,
    scene_luma, scene_peak_error, scene_read_from_rgba, scene_referred, scrgb_code, DisplayMode,
    HdrSwapchain, ListedFormat, SceneRead, PresentHost, COLOR_SPACE_HDR10, COLOR_SPACE_SCRGB,
    COLOR_SPACE_SRGB, FORMAT_A2B10,
    FORMAT_A2R10, FORMAT_BGRA8, FORMAT_R16F, FORMAT_RGBA8, GRAPHICS_WHITE_NITS,
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
