mod aa;
mod budget;
mod gltf_draw;
mod gpu;
mod mesh;
mod mesh_field;
mod occ_grid;
mod pack;
mod particles;
mod pool;
mod scene_mesh;
mod trace;
mod wire;
mod world;

pub use aa::Antialias;
pub use budget::{FrameMemory, ImageAdmit};
pub use genos_bake as bake;
pub use gpu::{
    DebugView, DrawProfile, LampReport, LightingAtrousStats, Renderer, ScreenLine, ScreenRect,
    SettleStats, ViewMode,
};
pub use genos_scene::{LightingPreset, LightingSettings};
pub use mesh_field::{field_words, instance_distance, FieldInstance, FIELD_STEPS};
pub use occ_grid::{analytic_hit, walk_grid, GridHit};
pub use pack::{build_grid, lamp_range, pack_frame, Pack, PackedDraw, SceneGrid, LAMP_CUTOFF};
pub use particles::{
    card_quad, compose_fog, fire_radiance, fog_along, frame_at, image_from_map, image_from_texture,
    medium, object_cards, shade_lit, Card, Emitter, FireLight, FogHit, FogLamp, Kind, LiveParticle,
    Puff, Simulation, FIRE_COLOR, PROOF_STEPS, SMOKE_ALBEDO, SMOKE_DENSITY, SMOKE_RADIUS, STEP_DT,
};
pub use scene_mesh::{SceneMesh, ShapeRef, SurfaceMaterial, CIRCLE_SEGMENTS};
pub use trace::{
    agreement, direct_light, direct_light_in, first_outgoing, linear_from_display, luminance,
    radiance, radiance_hits, ray_land, samples_toward, sees, trace as trace_ray, Hit as TraceHit,
    Surfaces,
};
pub use world::{
    fixed_bounds, identity_pose, in_view, solid_reach, Bounds, Displacement, DrawKind, FixedPart,
    Object, ParticleFrame, ParticleImage, ShaderSpace, World,
};

/// `GENOS_SCENE_DELTA=1`: print each frame's resident scene update (S70C).
pub(crate) fn scene_delta_trace() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("GENOS_SCENE_DELTA").is_some())
}
