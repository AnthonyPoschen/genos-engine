mod aa;
mod budget;
mod field;
mod gltf_draw;
mod gpu;
mod lighting;
mod mesh;
mod occ_grid;
mod pack;
mod particles;
mod pool;
mod probe_tier;
mod probes;
mod scene_mesh;
mod shadow;
mod trace;
mod wire;
mod world;

pub use aa::Antialias;
pub use budget::{FrameMemory, ImageAdmit};
pub use field::{
    build, cascade_debug_lines, illuminate, illuminate_facing, probe_counts, sample, sample_world,
    Field,
};
pub use gpu::{
    Bounces, DebugView, DrawProfile, LampReport, LightBuildTimes, LightingConfig, ProbeValue,
    Renderer, ScreenLine, ScreenRect, ViewMode,
};
pub use lighting::Lighting;
pub use mesh::Vertex;
pub use occ_grid::{analytic_hit, walk_grid, GridHit};
pub use pack::{
    build_grid, lamp_range, pack_frame, probe_spacing, Pack, PackedDraw, SceneGrid, FIELD_PLACE,
    LAMP_CUTOFF,
};
pub use particles::{
    card_quad, compose_fog, fire_radiance, fog_along, frame_at, image_from_map, image_from_texture,
    medium, object_cards, shade_lit, Card, Emitter, FireLight, FogHit, FogLamp, Kind, LiveParticle,
    Puff, Simulation, FIRE_COLOR, PROOF_STEPS, SMOKE_ALBEDO, SMOKE_DENSITY, SMOKE_RADIUS, STEP_DT,
};
pub use probe_tier::{
    allocate, notice_band, probe_live, scene_boxes, set_notice_band, BrickReport, BrickSet,
    BrickState, SurfaceBox, TierBatch, TierItem, TierLayout, TierLight, TierState, TierStats,
    TierWeights, BRICK, PROBE_BYTES,
};
pub use scene_mesh::{SceneMesh, ShapeRef, SurfaceMaterial, CIRCLE_SEGMENTS};
pub use trace::{
    agreement, direct_light, direct_light_in, first_outgoing, linear_from_display, luminance, radiance,
    radiance_hits, ray_land, samples_toward, sees, Surfaces, trace as trace_ray, Hit as TraceHit,
};
pub use world::{
    fixed_bounds, identity_pose, in_view, solid_reach, Bounds, Displacement, DrawKind, FixedPart,
    Object, ParticleFrame, ParticleImage, ShaderSpace, World,
};
