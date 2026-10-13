//! Scene, capsule camera, and script host.
//!
//! Lights and objects live here. The renderer decides which light reaches which surface.
//! Positions and directions use `genos-math`.

mod camera;
mod codimation;
mod debug_settings;
mod fps_counter;
mod lighting;
mod script;
mod settings;
mod types;

pub use camera::{
    look_direction, transform_point, update, view_proj, viewport_uv, Actions, Camera, CAMERA_FAR,
    CAMERA_HEIGHT, CAMERA_NEAR, PITCH_LIMIT,
};
pub use codimation::{Codimation, Easing};
pub use debug_settings::DebugSettings;
pub use fps_counter::{parse_enabled, parse_rate, FpsCli, FpsCounterSettings};
pub use genos_math::{Mat3, Mat4, Quat, Vec2, Vec3, Vec4};
pub use lighting::{
    check_lighting_arg, lighting_arg_time_of_day, lighting_flag_needs_value, AmbientMode,
    AmbientSettings, BounceSettings, DenoiserSettings, DirectSettings, GiSettings, GpuBuckets,
    LampPickMode, LampPickSettings, LightingPreset, LightingSettings, ProbeSettings, ShadowFilter,
    ShadowSettings, WorldCacheSettings, LIGHTING_BOUNCES_UNLIMITED,
};
pub use script::{load_path, load_str};
pub use settings::EngineSettings;
pub use types::{
    bounce_radiance, color_mix_of, diffuse_brdf, reflectance_of, Ceiling, Floor, Light, Scene,
    Shape, Sky, Solid, Wall, DEFAULT_BRDF, DEFAULT_REFLECTANCE, MAX_LAMPS, MAX_OCCLUDERS,
    PAINT_ALBEDO,
};
