#![expect(
    clippy::as_conversions,
    reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_lossless,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_precision_loss,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_sign_loss,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::chunks_exact_to_as_chunks,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "TODO(lint-2026-10-14): publish intent with the CLI argument, not set_var. docs/LINT_RULES.md"
)]
#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]
#![cfg_attr(
    test,
    expect(clippy::panic, reason = "a test aborts when a fixture fails")
)]
#![expect(
    clippy::large_enum_variant,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::map_unwrap_or,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::struct_excessive_bools,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::too_many_lines,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::unnecessary_wraps,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::unwrap_used,
    reason = "TODO(lint-2026-10-14): pre-existing. docs/LINT_RULES.md"
)]
#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "TODO(lint-2026-10-14): pre-existing wildcard match. docs/LINT_RULES.md"
)]

//! Lighting debug binary. Scenes come from `debug/scenes/`. Arguments choose the
//! scene, the checkpoint, the lighting preset, and where pictures land.
//! `./genos-debug/<kind>/` is the default output. `--out` overrides it.
//! `GENOS_HDR_OUT` applies only when `--out` was omitted.
//! An environment variable applies only when the matching argument was omitted.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use genos_debug::{png, script::ScriptRun, CheckpointPose, Host, Knob, LoadPace};
use genos_input::{character_controller, debug_keys, InputCode, InputSystem, KeyBindings};
use genos_render::{identity_pose, DrawKind, FixedPart, Renderer, ScreenRect, ViewMode, World};
use genos_scene::{
    look_direction, parse_enabled, parse_menu_visible, parse_rate, update, Actions, Camera,
    DebugSettings, EventLoopSettings, Floor, FocusPolicy, FogCli, FogEnv, FogSettings,
    FpsCounterSettings, Light, MenuSettings, ParticleCli, ParticleEnv, ParticlePreset,
    ParticleSettings, ReflectionSettings, Scene, Sky, Vec3, PITCH_LIMIT,
};
use genos_scene_lib::{Entry, Kind, Lamp, Sun};
use genos_stress::building::{Layout, LightMix};
use genos_stress::knobs::{
    apply_fog_knob, apply_lighting_knob, apply_particle_knob, fog_knobs, lighting_knobs,
    particle_knobs,
};
use genos_stress::stage::{Scale, Stage};
use genos_ui::{
    button_panel_blocks, show_menu_tab, top_bar, Action, FpsOverlay, InputPolicy, PanelBlock, PanelCheck,
    PanelDropdown, PanelRow, PanelSection, PanelSlider, PanelTab, PanelTabs, Pointer, SliderStep,
    State, MENU_DOCK, MENU_HIDE, MENU_SHOW, PANEL_ID, TOP_BAR_HEIGHT,
};
use genos_window::{extent_changed, FocusGate, Window};

const USAGE: &str = "\
genos-lighting — lighting scenes, checkpoints, and the debug picture

  --scene NAME           scene file name (debug/scenes). Falls back to GENOS_SCENE,
                         then lighting-stress (genos-lighting) or mesh
                         (genos-editor). stress is the old name of
                         lighting-stress and still opens it. genos-lighting
                         does not list mesh scenes
  --scenes DIR           also list the .scene files in DIR (a game's own
                         scenes). Paths inside a scene file are under the
                         repository. GENOS_SCENES applies only when this is
                         omitted
  --mesh-workspace DIR   mesh-workbench workspace a mesh scene opens, in place
                         of the one its scene file names. GENOS_MESH_WORKSPACE
                         applies only when this is omitted
  --checkpoint NAME      jump here at launch
  --eye X,Y,Z            camera position, metres. --eye=X,Y,Z is the same flag.
                         Keeps the checkpoint aim unless --look is set
  --look X,Y,Z           world point to aim at. --look=X,Y,Z is the same flag.
                         Yaw 0 looks down -Z
  --preset NAME          fallback (alias: potato), low, medium, or high. Sets GENOS_LIGHTING_PRESET
  --exposure-mode auto|camera|fixed
                         meter, physical camera, or a fixed EV100.
                         GENOS_HDR_EXPOSURE_MODE applies only when this is omitted
  --ev100 F              fixed EV100. GENOS_HDR_EV100 applies only when this is omitted
  --compensation F       stops added to exposure. GENOS_HDR_COMPENSATION applies
                         only when this is omitted
  --iso F                camera ISO. GENOS_HDR_ISO applies only when this is omitted
  --aperture F           f-number. GENOS_HDR_APERTURE applies only when this is omitted
  --shutter F            shutter seconds. GENOS_HDR_SHUTTER applies only when this is omitted
  --adapt-down F         seconds for exposure to fall. GENOS_HDR_ADAPT_DOWN applies
                         only when this is omitted
  --adapt-up F           seconds for exposure to rise. GENOS_HDR_ADAPT_UP applies
                         only when this is omitted
  --adapt-strength F     0..1, how far auto exposure follows the metered mean away
                         from its anchor (1 fully).
                         GENOS_HDR_ADAPT_STRENGTH applies only when this is omitted
  --adapt-anchor-seconds F
                         seconds auto exposure's anchor takes to settle where the
                         eye stays (0 follows the meter at once).
                         GENOS_HDR_ADAPT_ANCHOR_SECONDS applies only when this is omitted
  --adapt-knee-stops F   stops from the anchor at which exposure follows half of
                         the strength: under it little, past it most (0: flat).
                         GENOS_HDR_ADAPT_KNEE_STOPS applies only when this is omitted
  --highlight-percentile F
                         0.5..1, the luminance share that sets the highlight key.
                         GENOS_HDR_HIGHLIGHT_PERCENTILE applies only when this is omitted
  --highlight-stops F    0.5..10, stops above mid-grey the highlight key places that
                         luminance. GENOS_HDR_HIGHLIGHT_STOPS applies only when this
                         is omitted
  --hdr-display agx|aces|legacy
                         display transform. GENOS_HDR_DISPLAY applies only when this
                         is omitted. --display is the Wayland name; a value of
                         agx, aces, or legacy also selects the transform
  --bloom on|off         scene-referred bloom. GENOS_HDR_BLOOM applies only when this is omitted
  --bloom-intensity F    bloom scale. GENOS_HDR_BLOOM_INTENSITY applies only when this is omitted
  --bloom-levels N       pyramid levels, 0 to 6. GENOS_HDR_BLOOM_LEVELS applies
                         only when this is omitted
  --output auto|sdr|hdr10|scrgb
                         swapchain choice. GENOS_HDR_OUTPUT applies only when this is omitted
  --hdr-transition F     seconds a change of EV100, compensation, or bloom intensity
                         takes to reach the picture. 0 applies it on the next frame.
                         GENOS_HDR_TRANSITION applies only when this is omitted
  --fog on|off           height fog and local volumes (GENOS_FOG)
  --fog-density N        extinction at the fog height (GENOS_FOG_DENSITY)
  --fog-height N         world height of that extinction, metres (GENOS_FOG_HEIGHT)
  --fog-thickness N      scale height, metres (GENOS_FOG_THICKNESS)
  --fog-albedo N         single-scatter albedo (GENOS_FOG_ALBEDO)
  --fog-phase N          Henyey-Greenstein g (GENOS_FOG_PHASE)
  --fog-gi on|off        world-cache read (GENOS_FOG_GI)
  --fog-history on|off   stored only; the pass does not blend (GENOS_FOG_HISTORY)
  --fog-grid WxHxS       froxel columns, rows, and slices. 0 on an axis keeps
                         the preset (GENOS_FOG_GRID)
  --fog-samples N        cell budget. 0 dispatches the whole grid (GENOS_FOG_SAMPLES)
  --fx-preset low|medium|high
                         particle quality knobs (GENOS_FX_PRESET)
  --fx-max N             particle buffer budget (GENOS_FX_MAX)
  --fx-count N           live particle cap (GENOS_FX_COUNT)
  --fx-freeze            hold positions; skip integrate and emit (GENOS_FX_FREEZE)
  --fx-spawn X,Y,Z       place the stage-1 emitter and turn it on
  --lighting-direct-specular on|off
                         Appendix B shade on pixel direct (default off)
                         (GENOS_LIGHTING_DIRECT_SPECULAR; the argument wins)
  --surface-cache-attach-movers on|off
                         mover cache entries follow the object
                         (GENOS_LIGHTING_SURFACE_CACHE_ATTACH_MOVERS)
  --surface-cache-blend msme|hold
                         published blend (GENOS_LIGHTING_SURFACE_CACHE_BLEND)
  --surface-cache-firefly-sigma F
                         MSME firefly clamp, default 8, biased
                         (GENOS_LIGHTING_SURFACE_CACHE_FIREFLY_SIGMA)
  --surface-cache-ray-budget N
                         cache-round ray cap; 0 keeps the existing lists
                         (GENOS_LIGHTING_SURFACE_CACHE_RAY_BUDGET).
                         World mode 3 no longer spends a fill allocator
                         budget. Cascades trace level-0 every frame and
                         refresh upper cells on a fixed rolling stride.
  --surface-cache-dormant-rays N
                         fresh rays per frame for a dormant cache entry, 0..16.
                         0 keeps one sweep in eight. 1..16 traces one new
                         16-ray entry every ceil(16 / N) frames
                         (GENOS_LIGHTING_SURFACE_CACHE_DORMANT_RAYS)
  --gpu-times N          print GPU_MS every N frames (GENOS_GPU_TIMES)
  --gi-metrics [on|off]  record GI pass times and work counters.
                         A bare flag turns recording on. GENOS_GI_METRICS
                         applies only when this argument is omitted.
                         Files go to <out>/gi
  --script FILE          run a rhai script on this binary, then quit.
                         The scene step is 1/60. genos-stress is not this path
  --report DIR           script report (default <out>/gi/<script name>)
  --lighting-debug       print the lighting probe line (GENOS_LIGHTING_DEBUG)
  --cooked DIR           load a glTF scene from a genos-cook directory (ADR 0035).
                         GENOS_COOKED applies only when this is omitted.
                         Unset keeps the glTF and PNG path
  --unfilled             show the texels view (red own mean, green seed, blue set count)
  --full-effort          preset high, unless --preset is set, and compose every frame
                         (GENOS_LIGHTING_COMPOSE_ALL)
  --denoise-launch hash|screen
                         picture wavelet (GENOS_LIGHTING_DENOISE_LAUNCH). Preset high
                         uses screen. The other presets keep hash. This flag and the
                         variable override the preset
  --denoise-screen on|off alias of --denoise-launch (GENOS_LIGHTING_DENOISE_SCREEN)
  --headless             proof window, no panel
  --display NAME         Wayland display name. Omit it to use the session compositor.
  --lifecycle-block-ms N stall shutdown once, for the quit-gap test.
                         GENOS_LIFECYCLE_BLOCK_MS applies only when this is omitted
  --lifecycle-close-early quit before the first frame. GENOS_LIFECYCLE_CLOSE_EARLY
                         applies only when this is omitted
  --frames N             quit after N frames. The last frame PNG leaves out
                         the debug menu, the menu tab, the frame-rate plate,
                         and tool overlays
  --ui                   keep those overlays in that PNG
  --reflections on|off   mirrors, portals, and water (default on).
                         GENOS_REFLECTIONS applies only when this is omitted
  --reflection-rays N    ray cap for this run, 0 to 128000.
                         GENOS_REFLECTION_RAYS applies only when this is omitted
  --mirror-views N       extra mirror and portal views, 0 to 3.
                         GENOS_MIRROR_VIEWS applies only when this is omitted
  --water on|off         water pass (default on).
                         GENOS_WATER applies only when this is omitted
  --caustics off|image|cache
                         caustic mode. GENOS_CAUSTICS applies only when this is omitted
  --reflection-tracer off|software|ray_query
                         tracer. GENOS_REFLECTION_TRACER applies only when this is omitted
  --fps-counter [on|off] frame-rate plate (default off). A bare flag turns it on.
                         GENOS_FPS_COUNTER applies only when this is omitted
  --fps-hz N             plate update rate in Hz (default 4, range 0.25 to 30).
                         GENOS_FPS_HZ applies only when this is omitted
  --menu shown|hidden    debug menu (default shown). GENOS_MENU applies only
                         when this is omitted
  --mcp-port N           loopback MCP port (default 8766 for genos-lighting,
                         8765 for genos-editor). GENOS_MCP_PORT applies
                         only when this is omitted. genos-stress stays on 8765
  --out DIR              screenshots, dumps, and logs (default ./genos-debug).
                         GENOS_HDR_OUT applies only when this is omitted
  --size WxH             window size (default 1280x720; 320x180 for --check)
  --check                every scene at its check checkpoint, four frames,
                         mean luma against that scene's own floor

Page Up and Page Down switch scenes. Keys 1-9 jump checkpoints. Home and 0 reset
to the active checkpoint. The wheel scrolls the menu while the pointer is over
it. Perf can set that wheel policy to hover, modal, click, or passive. A click
gives the menu the keys. Arrow keys then move the focused control,
and a focused slider steps. Enter picks the highlighted choice or presses the
focused control. Escape closes an open list, then releases the keys. A click
outside releases them too. Grave releases the keys on the first press and
leaves the menu up. The next grave hides it. F1 hides it either way. Perf sets
Wheel focus
and Key focus to hover, modal, click, or passive. The title bar hide button
hides it. A menu tab in the top-left corner shows it. --menu hidden starts it
hidden. GENOS_MENU applies only when that argument was omitted.
get_menu_settings and set_menu_settings read and write shown or hidden, the
page, the folds, and those two focus policies. The Perf page shows the longest
event-loop stall. get_event_loop and set_event_loop read it and can write 0 to
clear it. ] and [ change the page while
the menu has the keys. Tab and shift-Tab do that too. F toggles the frame-rate
plate. F3 is the same key. A section title folds that group.
The fold is remembered in <out>/menu/collapsed.txt. The open page is remembered
in <out>/menu/tab.txt.
The groups are crates/scene-program/menu.txt. A section key of - has no fold.
GENOS_MENU_LAYOUT points at another file. The panel stays within a third of
the window. The pages are Scene, Pipeline, Lamps, GI, Denoise,
Shadows, Perf, Output, and Info. Lamps and GI keep folds because those pages
are tall.
WASD fly, Space
up, C or Ctrl down, Shift is faster. The plate shows FPS and frame ms, averaged
over each 1/rate window (default 4 Hz). It is off until the checkbox, F or F3, MCP,
or --fps-counter turns it on. GENOS_DEBUG_KEYS replaces those keys.

Screenshots go to <out>/shots, the log to <out>/logs, lighting dumps to <out>/dumps.
GI metrics go to <out>/gi. frames.csv is the live window. gi_metrics write NAME
saves NAME.csv (p50 and p99 per pass and counter) and NAME-frames.csv.
The binary does not write under $HOME.
";

const FLY_SPEED: f32 = 5.0;
const STICK_LOOK: f32 = 2.0;
const MAX_FRAME_SECONDS: f32 = 0.25;
const STEP: f32 = 1.0 / 60.0;
const ID_SCENE: u32 = 2000;
const ID_CP: u32 = 2200;
const ID_VIEW: u32 = 2400;
const ID_DROP: u32 = 2600;
const ID_FPS: u32 = 2800;
const ID_FPS_HZ: u32 = 2801;
const ID_REFL_ENABLED: u32 = 2900;
const ID_REFL_WATER: u32 = 2901;
const ID_REFL_ROUGH: u32 = 2902;
const ID_REFL_MIRROR: u32 = 2903;
const ID_REFL_VIEWS: u32 = 2904;
const ID_REFL_RAYS: u32 = 2905;
const ID_REFL_SHORE: u32 = 2906;
const ID_REFL_CAUSTICS: u32 = 2907;
const ID_REFL_TRACER: u32 = 2908;
const ID_MENU_WHEEL: u32 = 2820;
const ID_MENU_KEYS: u32 = 2821;
const ID_KNOB: u32 = 3000;
/// The check boxes of the mesh viewer's Objects panel, then its Replay panel.
#[cfg(feature = "mesh")]
const ID_MESH_OBJECT: u32 = 6000;
#[cfg(feature = "mesh")]
const ID_MESH_EVENT: u32 = 6500;
/// The "open" box of a collection's row in the Objects panel.
#[cfg(feature = "mesh")]
const ID_MESH_FOLD: u32 = 7000;
const ID_SWITCH: u32 = 4000;
const ID_HDR_EXPOSURE: u32 = 5000;
const ID_HDR_DISPLAY: u32 = 5001;
const ID_HDR_OUTPUT: u32 = 5002;
const ID_HDR_EV: u32 = 5100;
const ID_HDR_COMP: u32 = 5101;
const ID_HDR_ISO: u32 = 5102;
const ID_HDR_APERTURE: u32 = 5103;
const ID_HDR_SHUTTER: u32 = 5104;
const ID_HDR_LEVELS: u32 = 5105;
const ID_HDR_BLOOM_I: u32 = 5106;
const ID_HDR_ADAPT_DOWN: u32 = 5107;
const ID_HDR_ADAPT_UP: u32 = 5108;
const ID_HDR_TRANSITION: u32 = 5109;
const ID_HDR_ADAPT_STRENGTH: u32 = 5110;
const ID_HDR_HIGHLIGHT_PERCENTILE: u32 = 5111;
const ID_HDR_HIGHLIGHT_STOPS: u32 = 5112;
const ID_HDR_ADAPT_ANCHOR: u32 = 5113;
const ID_HDR_ADAPT_KNEE: u32 = 5114;
const ID_HDR_BLOOM: u32 = 5200;
/// The "Save look to scene" button of the Output page.
const ID_HDR_SAVE: u32 = 5300;

mod menu_layout;
#[cfg(feature = "mesh")]
mod mesh_view;
#[cfg(feature = "mesh")]
pub use mesh_view::{read_catalogue, set_out_dir as set_mesh_out_dir, CommandDoc, MeshView};

use menu_layout::{load_menu_layout, menu_folds, section_id, strip_id, tab_id, MenuFold, MenuLine};
/// Ordered grades and numeric knobs. An on/off switch is not in this list.
const SLIDER_KNOBS: &[&str] = &[
    "lighting_preset",
    "lighting_bounces",
    "lighting_direct_lights",
    "lighting_direct_refresh",
    "lighting_direct_scale",
    "lighting_lamp_pick_samples",
    "lighting_lamp_pick_candidates",
    "lighting_lamp_pick_radius",
    "lighting_lamp_pick_history",
    "lighting_restir_spatial_neighbors",
    "lighting_restir_spatial_radius",
    "lighting_restir_resolution_scale",
    "lighting_restir_candidates",
    "lighting_gi_refresh",
    "lighting_budget_shift",
    "lighting_surface_cache_firefly_sigma",
    "lighting_surface_cache_ray_budget",
    "lighting_surface_cache_dormant_rays",
    "lighting_world_stale",
    "lighting_denoise_iters",
    "lighting_denoise_variance",
    "lighting_screen_verify",
    "lighting_world_verify",
    "density",
    "height",
    "thickness",
    "albedo",
    "phase_g",
    // The mesh viewer's sliders. New names go at the end: the menu ID is the index.
    "mesh_replay",
    "mesh_time",
    "mesh_note",
    "mesh_filter",
    "mesh_statistics",
    "mesh_state",
    "mesh_command_filter",
];
/// Unordered choices. A long list of scenes uses the same dropdown.
const DROP_KNOBS: &[&str] = &[
    "lighting_shadow_filter",
    "lighting_ambient",
    "lighting_lamp_pick",
    "lighting_surface_cache_blend",
    "fog_view",
    // The mesh viewer's choices. New names go at the end: the menu ID is the index.
    "mesh_pick",
    "mesh_shade",
    "mesh_solo",
    "mesh_clip",
    "mesh_session",
    "mesh_workspace",
];
const VIEWS: [&str; 10] = [
    "full", "direct", "bounce", "albedo", "normal", "depth", "texels", "variance", "path",
    "voxels",
];

struct Options {
    scene: String,
    checkpoint: Option<String>,
    eye: Option<[f32; 3]>,
    look: Option<[f32; 3]>,
    preset: Option<String>,
    gpu_times: Option<u32>,
    script: Option<ScriptRun>,
    lighting_debug: bool,
    unfilled: bool,
    full_effort: bool,
    denoise_launch: Option<String>,
    denoise_screen: Option<String>,
    headless: bool,
    /// Wayland display name from `--display`. Empty uses the session compositor.
    display: Option<String>,
    frames: Option<u32>,
    /// Keep the debug menu, menu tab, frame-rate plate, and tool overlays
    /// in a `--frames` PNG. A capture leaves them out unless this is set.
    ui: bool,
    out: PathBuf,
    out_set: bool,
    size: (u32, u32),
    check: bool,
    /// Set by `--mcp-port`. Empty means `GENOS_MCP_PORT`, then [`MCP_PORT`].
    mcp_port: Option<u16>,
    /// `--lifecycle-block-ms`. The mirror applies when this stays empty.
    block_ms: Option<std::time::Duration>,
    /// `--lifecycle-close-early`. The mirror applies when this stays false.
    close_early: bool,
    /// `--fog*`. Empty fields were omitted.
    fog_cli: FogCli,
    /// Mirrors for the fog flags this launch left empty.
    fog_env: FogEnv,
    /// `--fx-*`. Empty fields were omitted.
    particle_cli: ParticleCli,
    /// Mirrors for the particle flags this launch left empty.
    particle_env: ParticleEnv,
}

/// Loopback port for this binary. `genos-stress` and `genos-camera` stay on 8765.
const MCP_PORT: u16 = 8766;

/// One program on this frame loop. `genos-lighting` and `genos-editor` are
/// the same loop with a different name, port, opening scene and scene list.
#[derive(Clone, Copy, Debug)]
pub struct Program {
    /// The binary's name: the window class and the log prefix.
    pub name: &'static str,
    /// The window title.
    pub title: &'static str,
    /// Loopback MCP port when neither `--mcp-port` nor `GENOS_MCP_PORT` names one.
    pub mcp_port: u16,
    /// The scene opened when none is named.
    pub default_scene: &'static str,
    /// Whether the program lists mesh scenes (mesh-workbench workspaces).
    pub meshes: bool,
    /// The menu is a bar across the top with a button for each page, and the
    /// chosen page opens as a drawer on the right edge (or a panel that
    /// drags). Without it the menu is one panel with a strip of tabs.
    pub top_bar: bool,
    /// Opens the mesh-workbench session on a workspace directory, for the
    /// mesh commands that edit. The binary supplies it, so this library does
    /// not link mesh-workbench: `genos-editor` does, `genos-lighting` does not.
    pub mesh_session: Option<OpenMeshSession>,
}

/// A mesh-workbench session on one workspace: it runs a command, or a list
/// of commands, and gives mesh-workbench's reply.
pub trait MeshSession: Send {
    /// # Errors
    ///
    /// The session could not run the request at all. A command mesh-workbench
    /// refuses is a reply with `ok: false`, not an error.
    fn execute(&mut self, request: &genos_mcp::json::Value) -> Result<genos_mcp::json::Value, String>;
}

/// Opens a [`MeshSession`] on a workspace directory.
pub type OpenMeshSession = fn(&Path) -> Result<Box<dyn MeshSession>, String>;

impl Program {
    /// The lighting debug binary: every lighting scene, no mesh scenes.
    pub const LIGHTING: Self = Self {
        name: "genos-lighting",
        title: "Genos lighting",
        mcp_port: MCP_PORT,
        default_scene: genos_scene_lib::DEFAULT_SCENE,
        meshes: false,
        top_bar: false,
        mesh_session: None,
    };
    /// The editor: opens on the mesh scene and lists every scene. It takes
    /// the engine's first MCP port, as `genos-camera` and `genos-stress` do.
    /// The binary adds [`Program::mesh_session`].
    pub const EDITOR: Self = Self {
        name: "genos-editor",
        title: "Genos editor",
        mcp_port: genos_mcp::PORT,
        default_scene: "mesh",
        meshes: true,
        top_bar: true,
        mesh_session: None,
    };
}

static PROGRAM: std::sync::OnceLock<Program> = std::sync::OnceLock::new();

/// The program this process runs. A test that never called
/// [`run_program`] gets the editor, which lists every scene.
fn program() -> Program {
    *PROGRAM.get_or_init(|| Program::EDITOR)
}

/// Name the program without running its window: the mesh viewer then has
/// the program's way to open a mesh-workbench session. For a test that
/// drives [`MeshView`] with no window. The first call in a process wins.
pub fn use_program(program: Program) {
    let _ = PROGRAM.set(program);
}

/// Run `program` until it quits, then exit the process with its code.
pub fn run_program(program: Program) -> ! {
    let _ = PROGRAM.set(program);
    // Quit drops the lifecycle before `_exit`. Skip the lavapipe teardown wait.
    genos_window::release_driver_on_process_exit();
    let code = match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}: {err}", program.name);
            1
        }
    };
    genos_window::exit(code)
}

fn run() -> Result<i32, String> {
    let run_start = Instant::now();
    let opts = options()?;
    apply_intent(&opts)?;
    std::fs::create_dir_all(opts.out.join("shots")).map_err(|err| err.to_string())?;
    #[cfg(feature = "mesh")]
    mesh_view::set_out_dir(opts.out.clone());
    std::fs::create_dir_all(opts.out.join("logs")).map_err(|err| err.to_string())?;
    std::fs::create_dir_all(opts.out.join("dumps")).map_err(|err| err.to_string())?;
    if std::env::var_os("GENOS_LIGHTING_DUMP").is_some() {
        let dump_set = std::env::var_os("GENOS_LIGHTING_DUMP_DIR").is_some();
        if opts.out_set || !dump_set {
            std::env::set_var("GENOS_LIGHTING_DUMP_DIR", opts.out.join("dumps"));
        }
    }
    let mut log = std::fs::File::create(opts.out.join("logs").join("run.log"))
        .map_err(|err| err.to_string())?;
    let root = genos_scene_lib::repo_root();
    let mut entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir())?;
    // --scenes DIR (or GENOS_SCENES): a game's own scene files join the list.
    // A name the list already has keeps its first file.
    if let Some(dir) = std::env::var_os("GENOS_SCENES").filter(|dir| !dir.is_empty()) {
        for entry in genos_scene_lib::load_dir(Path::new(&dir))? {
            if !entries.iter().any(|known| known.name == entry.name) {
                entries.push(entry);
            }
        }
    }
    if !program().meshes {
        entries.retain(|entry| !matches!(entry.kind, Kind::Mesh { .. }));
    }
    if entries.is_empty() {
        return Err("no scenes in debug/scenes".into());
    }
    let load_at = Instant::now();
    let mut app = App::open(
        entries,
        root,
        log.try_clone().map_err(|err| err.to_string())?,
    )?;
    eprintln!(
        "genos-startup scene catalog: {:.1} ms",
        load_at.elapsed().as_secs_f64() * 1000.0
    );
    app.fog_cli = opts.fog_cli.clone();
    app.fog_env = opts.fog_env;
    app.particle_cli = opts.particle_cli.clone();
    app.particle_env = opts.particle_env;
    app.overlay_fog();
    app.overlay_particles();
    if opts.check {
        check_all(&mut app, &opts, &mut log)?;
        return Ok(0);
    }
    if !app.entries.iter().any(|entry| entry.name == opts.scene) {
        return Err(format!("unknown scene {}", opts.scene));
    }
    let (width, height) = opts.size;
    let display = opts.display.as_deref();
    let window_at = Instant::now();
    let mut window = if opts.headless {
        Window::open_proof_on(width, height, display)?
    } else {
        Window::open_named_on(width, height, program().name, program().title, display)?
    };
    eprintln!(
        "genos-startup window: {:.1} ms",
        window_at.elapsed().as_secs_f64() * 1000.0
    );
    window.install_seat_pump();
    Renderer::set_blocker(genos_window::seat_enter, genos_window::seat_leave);
    let first = window.pump();
    if first.closing {
        return Ok(0);
    }
    let renderer_at = Instant::now();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, first.width, first.height)?;
    eprintln!(
        "genos-startup Renderer::open call: {:.1} ms",
        renderer_at.elapsed().as_secs_f64() * 1000.0
    );
    let server_hold = std::sync::Arc::new(std::sync::Mutex::new(None));
    let script_hold = std::sync::Arc::new(std::sync::Mutex::new(
        None::<genos_window::ThreadStop<bool>>,
    ));
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    if let Some(stall) = opts.block_ms {
        owner.plant_block(stall);
    }
    if opts.close_early || owner.window.pump().closing {
        owner.quit(0);
    }
    {
        let hold = std::sync::Arc::clone(&server_hold);
        owner.add_stop(move || {
            let mut guard = hold
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(guard.take());
            true
        });
    }
    {
        let hold = std::sync::Arc::clone(&script_hold);
        owner.add_stop(move || {
            let mut guard = hold
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match guard.as_mut() {
                Some(stop) => stop.release(),
                None => true,
            }
        });
    }
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let mut camera = Camera::opening();
    if app.entry().name != opts.scene {
        app.start_load(&opts.scene)?;
    }
    jump_camera(&mut app, opts.checkpoint.as_deref(), &mut camera)?;
    place_camera(&opts, &mut camera)?;
    let load_world = loading_world();
    let mcp_host = genos_mcp::Host::new(app.scene().clone(), camera.clone());
    let mcp_port = opts.mcp_port.unwrap_or_else(mcp_port_from_env);
    let server = genos_mcp::Server::start_on(mcp_host, mcp_port)?;
    *server_hold
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(server);
    let _ = writeln!(log, "mcp http://127.0.0.1:{mcp_port}/");
    let mut tools = genos_debug::Tools::new();
    tools.set_out_dir(opts.out.join("shots"));
    tools.set_gi_dir(opts.out.join("gi"));
    if opts.unfilled {
        tools.set_view(ViewMode::Texels);
    }
    if opts.frames.is_some() || opts.script.is_some() {
        tools.set_fixed_step(Some(STEP));
    }
    if let Some(run) = opts.script.clone() {
        let _ = writeln!(log, "script {}", run.script.display());
        let handle = genos_debug::script::spawn(run);
        let _ = writeln!(log, "script thread {:?}", handle.thread().id());
        *script_hold
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(genos_window::ThreadStop::new(handle));
    }
    let mut size = (renderer.width(), renderer.height());
    let mut focus = FocusGate::default();
    // The mesh viewer's Objects and Replay lists are a panel of their own.
    let (menu, mesh_panel) = split_mesh_panel(load_menu_layout()?);
    #[cfg(not(feature = "mesh"))]
    drop(mesh_panel);
    #[cfg(feature = "mesh")]
    let mut mesh_ui = State::default();
    #[cfg(feature = "mesh")]
    let mut mesh_ui_placed = false;
    let folds = menu_folds(&menu);
    let mut ui = State::default();
    // The top bar keeps its own pointer state; the drawer is `ui`.
    let mut bar_ui = State::default();
    let mut drawer_placed = false;
    let mut tracked_collapsed = app.scene().menu.collapsed.clone();
    let mut tracked_tab = app.scene().menu.tab.clone();
    if let Some(names) = load_menu_folds(&opts.out) {
        app.scene().menu.collapsed.clone_from(&names);
        tracked_collapsed.clone_from(&names);
        apply_collapsed(&mut ui, &folds, &names);
    }
    if let Some(name) = load_menu_tab(&opts.out) {
        app.scene().menu.tab.clone_from(&name);
        tracked_tab.clone_from(&name);
        ui.select_tab(strip_id(0), tab_id(&name));
    }
    let mut input = InputSystem::new();
    let controls = character_controller();
    let keys = debug_keys()?;
    let mut menu_open = app.scene().menu.effective().visible;
    let mut drawn = 0u32;
    let mut clock: Option<Instant> = None;
    let mut fps_overlay = FpsOverlay::new();
    let loop_at = Instant::now();
    eprintln!(
        "genos-startup pre-loop setup: {:.1} ms",
        renderer_at.elapsed().as_secs_f64() * 1000.0
    );
    loop {
        let now = Instant::now();
        let real_dt = clock
            .map(|then| now.saturating_duration_since(then).as_secs_f32())
            .unwrap_or(STEP);
        let paced = opts.frames.is_some() || opts.script.is_some();
        // Wall time between frames. A script or a frame count runs on the fixed step.
        let frame_dt = if paced {
            STEP
        } else {
            real_dt.min(MAX_FRAME_SECONDS)
        };
        let dt = tools.scene_dt(frame_dt);
        clock = Some(now);
        window
            .stall()
            .pull_published(app.scene().event_loop.longest_stall_ms);
        let frame = window.pump();
        let stall_ms = window.stall().longest_ms();
        app.scene().event_loop.note(stall_ms);
        if app.loading() {
            app.take_load()?;
            if !app.loading() {
                renderer.reset_lighting_cache();
            }
        }
        app.overlay_fog();
        app.overlay_particles();
        if frame.closing {
            let code = script_exit_code(&script_hold, tools.quit_code().unwrap_or(0));
            return Ok(code);
        }
        if frame.minimized {
            std::thread::sleep(std::time::Duration::from_millis(16));
            continue;
        }
        if extent_changed(size.0, size.1, frame.width, frame.height) || frame.resized {
            renderer.resize(frame.width, frame.height)?;
            size = (renderer.width(), renderer.height());
        }
        let gated = focus.gate(&frame);
        input.begin_frame();
        window.write_devices(&mut input, &gated);
        input.poll_gamepads();
        pull_menu(
            &mut app,
            &mut ui,
            &folds,
            &mut menu_open,
            &mut tracked_collapsed,
            &mut tracked_tab,
        );
        // While the note is being typed in a mesh scene the keys are the note's.
        let shift = input.keyboard.down(InputCode::key_shift_left)
            || input.keyboard.down(InputCode::key_shift_right);
        let control = input.keyboard.down(InputCode::key_control_left) || input.keyboard.down(InputCode::key_control_right);
        #[cfg(feature = "mesh")]
        let typed = app.mesh.as_mut().is_some_and(|view| {
            view.control = control;
            view.type_note(&|code| input.keyboard.pressed(code), shift)
        });
        // Only the mesh viewer takes typed text.
        #[cfg(not(feature = "mesh"))]
        let typed = {
            let _ = (control, shift);
            false
        };
        if !typed {
            hotkeys(&mut app, &input, &keys, &mut camera, &mut tools, renderer)?;
        }
        let move_axis = controls.axis_2d(&input, "move");
        let look_axis = controls.axis_2d(&input, "look");
        let page_keys = !camera.captured && menu_open;
        let pointer = Pointer {
            x: frame.pointer_x,
            y: frame.pointer_y,
            down: !camera.captured && (gated.mouse_left || gated.capture_click),
            wheel_y: if camera.captured { 0.0 } else { gated.wheel_y },
            key_dec: !camera.captured
                && (input.keyboard.pressed(InputCode::key_left)
                    || input.keyboard.pressed(InputCode::key_down)),
            key_inc: !camera.captured
                && (input.keyboard.pressed(InputCode::key_right)
                    || input.keyboard.pressed(InputCode::key_up)),
            key_activate: !camera.captured && input.keyboard.pressed(InputCode::key_enter),
            key_dismiss: !camera.captured && input.keyboard.pressed(InputCode::key_escape),
            key_blur: !camera.captured && input.keyboard.pressed(InputCode::key_grave),
            tab_prev: page_keys
                && (keys.pressed(&input, "tab_prev") || keys.pressed(&input, "tab_focus_prev")),
            tab_next: page_keys
                && (keys.pressed(&input, "tab_next") || keys.pressed(&input, "tab_focus_next")),
        };
        let mut drew_panel = false;
        let mut menu_blur = false;
        // How wide the docked drawer is this frame; 0 with none.
        let mut drawer_width = 0.0f32;
        #[cfg_attr(not(feature = "mesh"), expect(unused_mut, reason = "the mesh viewer's panel adds to these"))]
        let (mut paints, mut look_capture) = if opts.headless {
            (Vec::new(), true)
        } else {
            let viewport = [size.0 as f32, size.1 as f32];
            let mut paints = Vec::new();
            let mut capture = true;
            // The panel to draw this frame: its title and its blocks.
            let mut page: Option<(String, Vec<PanelBlock>)> = None;
            if program().top_bar {
                let fps_menu = app.scene().fps_counter.effective();
                let blocks = panel_blocks(&app, tools.view_mode(), &fps_menu, &menu);
                let (buttons, chosen) = bar_buttons(&blocks, &ui, menu_open);
                let bar = top_bar(&mut bar_ui, &buttons, viewport, pointer);
                let mut chosen = chosen;
                for action in &bar.actions {
                    let Action::Press(id) = *action else { continue };
                    let Some(button) = buttons.iter().find(|button| button.id == id) else { continue };
                    // A press on the open page's button closes the drawer.
                    if menu_open && chosen == Some(id) {
                        menu_open = false;
                    } else {
                        ui.select_tab(strip_id(0), id);
                        chosen = Some(id);
                        menu_open = true;
                        save_menu_tab(&opts.out, &button.text)?;
                    }
                }
                paints.extend(bar.paints);
                capture = bar.look_capture;
                ui.dockable = true;
                let docked = app.scene().menu.docked;
                ui.dock = docked.then_some(TOP_BAR_HEIGHT);
                if !docked && !drawer_placed {
                    drawer_placed = true;
                    // Left of the gizmo: the panel is at most a third of the picture wide.
                    ui.panel_offset = [(viewport[0] * (2.0 / 3.0) - 180.0).max(16.0), TOP_BAR_HEIGHT + 16.0];
                }
                if menu_open {
                    page = Some(bar_page(blocks, chosen));
                }
            } else if menu_open {
                let fps_menu = app.scene().fps_counter.effective();
                page = Some((panel_title(&app), panel_blocks(&app, tools.view_mode(), &fps_menu, &menu)));
            } else {
                let ui_frame = show_menu_tab(&mut ui, viewport, pointer);
                if ui_frame
                    .actions
                    .iter()
                    .any(|action| matches!(action, Action::Press(id) if *id == MENU_SHOW))
                {
                    menu_open = true;
                }
                paints.extend(ui_frame.paints);
                capture = ui_frame.look_capture;
            }
            drawer_width = 0.0;
            if let Some((title, blocks)) = page {
                drew_panel = true;
                let ui_frame = button_panel_blocks(&mut ui, &title, &blocks, viewport, &camera, pointer);
                for action in &ui_frame.actions {
                    match *action {
                        Action::Slide { id, value } => slide(&mut app, id, value)?,
                        Action::Select { id, value } => {
                            select(&mut app, &mut camera, &mut tools, renderer, id, value)?;
                        }
                        Action::Toggle { id, on } => toggle(&mut app, id, on)?,
                        Action::Press(id) => {
                            if id == MENU_HIDE {
                                menu_open = false;
                            } else if id == MENU_DOCK {
                                let docked = !app.scene().menu.docked;
                                app.scene().menu.docked = docked;
                            } else if id == ID_HDR_SAVE {
                                app.save_look_from_menu();
                            } else if folds.iter().any(|fold| fold.id == id) {
                                save_menu_folds(&opts.out, &ui, &folds)?;
                            } else if let Some(name) = tab_title(&menu, id) {
                                save_menu_tab(&opts.out, name)?;
                            }
                        }
                        _ => {}
                    }
                }
                if ui.dock.is_some() {
                    drawer_width = ui_frame.shown.iter().find(|shown| shown.id == PANEL_ID).map_or(0.0, |shown| shown.rect.w);
                }
                menu_blur = ui_frame.consumed.blur;
                paints.extend(ui_frame.paints);
                capture = capture && ui_frame.look_capture;
            }
            (paints, capture)
        };
        // The words and the gizmo on a mesh picture stand clear of the bar
        // and of a docked drawer.
        #[cfg(feature = "mesh")]
        if let Some(view) = &mut app.mesh {
            view.hud_inset = if program().top_bar && !opts.headless { [drawer_width, TOP_BAR_HEIGHT] } else { [0.0, 0.0] };
        }
        #[cfg(not(feature = "mesh"))]
        let _ = drawer_width;
        // The Objects and Replay panel of a mesh scene: a second panel, on the
        // right under the gizmo, with its own place and folds. Its HIDE
        // button and the OBJECTS button under the gizmo close it.
        #[cfg(feature = "mesh")]
        if !opts.headless && !app.loading() && app.mesh.as_ref().is_some_and(|view| view.panel_open) {
            let viewport = [size.0 as f32, size.1 as f32];
            if !mesh_ui_placed {
                mesh_ui_placed = true;
                // Under the gizmo on the right; with a top bar, on the left,
                // where the drawer does not open.
                mesh_ui.panel_offset = if program().top_bar {
                    [16.0, TOP_BAR_HEIGHT + 16.0]
                } else {
                    [(viewport[0] * (2.0 / 3.0) - 16.0).max(16.0), MESH_PANEL_TOP]
                };
            }
            let fps_menu = app.scene().fps_counter.effective();
            let blocks = panel_blocks(&app, tools.view_mode(), &fps_menu, &mesh_panel);
            let frame = button_panel_blocks(&mut mesh_ui, "Objects", &blocks, viewport, &camera, pointer);
            for action in &frame.actions {
                match *action {
                    Action::Slide { id, value } => slide(&mut app, id, value)?,
                    Action::Select { id, value } => select(&mut app, &mut camera, &mut tools, renderer, id, value)?,
                    Action::Toggle { id, on } => toggle(&mut app, id, on)?,
                    Action::Press(id) if id == MENU_HIDE => {
                        if let Some(view) = &mut app.mesh {
                            view.panel_open = false;
                        }
                    }
                    _ => {}
                }
            }
            paints.extend(frame.paints);
            look_capture = look_capture && frame.look_capture;
            menu_blur = menu_blur || frame.consumed.blur;
        }
        if keys.pressed(&input, "menu") && !menu_blur {
            menu_open = !menu_open;
        }
        push_menu(
            &mut app,
            &ui,
            &folds,
            &menu,
            menu_open,
            drew_panel,
            &mut MenuWrite {
                out: &opts.out,
                collapsed: &mut tracked_collapsed,
                tab: &mut tracked_tab,
            },
        )?;
        let mut overlay: Vec<ScreenRect> = Vec::new();
        if app.loading() {
            overlay.push(ScreenRect {
                x: 0.0,
                y: 0.0,
                w: size.0 as f32,
                h: size.1 as f32,
                color: [0.04, 0.04, 0.05],
            });
        }
        overlay.extend(paints.iter().map(|p| ScreenRect {
            x: p.x,
            y: p.y,
            w: p.w,
            h: p.h,
            color: p.color,
        }));
        let loading = app.loading();
        if !loading {
            app.advance(dt);
        }
        // A mesh scene is looked at, not walked through: the pointer orbits
        // and picks, and the fly camera stays out of it.
        #[cfg(feature = "mesh")]
        let mesh_scene = !loading && app.mesh.is_some();
        #[cfg(not(feature = "mesh"))]
        let mesh_scene = false;
        #[cfg(feature = "mesh")]
        if mesh_scene {
            if camera.captured {
                camera.captured = false;
            }
            let orbit = mesh_view::OrbitInput {
                x: frame.pointer_x,
                y: frame.pointer_y,
                left: gated.mouse_left,
                right: input.mouse.down(InputCode::mouse_right),
                middle: input.mouse.down(InputCode::mouse_middle),
                wheel: gated.wheel_y,
                over_ui: pointer_over_ui(gated.mouse_left, look_capture, &paints, [frame.pointer_x, frame.pointer_y]),
                add: input.keyboard.down(InputCode::key_shift_left)
                    || input.keyboard.down(InputCode::key_shift_right),
                frame_key: !typed && input.keyboard.pressed(InputCode::key_f),
                dt,
                box_key: input.keyboard.down(InputCode::key_control_left) || input.keyboard.down(InputCode::key_control_right),
                lasso_key: input.keyboard.down(InputCode::key_alt_left) || input.keyboard.down(InputCode::key_alt_right),
            };
            if let Some(view) = &mut app.mesh {
                view.pointer(&mut camera, [size.0 as f32, size.1 as f32], orbit);
            } else {
                // The orthographic lens belongs to the mesh viewer.
                camera.view_tan = genos_scene::DEFAULT_VIEW_TAN;
                camera.parallel_half = 0.0;
            }
            app.refresh_mesh_overlay();
        }
        if !loading && !mesh_scene {
            fly(
                &mut camera,
                app.scene(),
                &input,
                &Actions {
                    forward: move_axis.y,
                    strafe: move_axis.x,
                    mouse_dx: input.mouse.dx,
                    mouse_dy: input.mouse.dy,
                    look_x: look_axis.x,
                    look_y: look_axis.y,
                    capture_click: look_capture
                        && (gated.capture_click || controls.down(&input, "capture")),
                    escape: controls.down(&input, "release"),
                },
                dt,
            );
        }
        if camera.captured != frame.pointer_locked {
            if let Err(err) = window.set_pointer_capture(camera.captured) {
                eprintln!("{}: {err}", program().name);
                camera.captured = false;
            }
        }
        if opts.ui {
            app.scene().debug.ui = true;
        }
        let capture_ui = app.scene().debug.ui;
        // A mesh shade that mesh-workbench draws unlit shows the base colour.
        // Only the change of shade switches the view, so a person can still
        // pick another view after it.
        #[cfg(feature = "mesh")]
        let unlit = app.mesh.as_ref().is_some_and(mesh_view::MeshView::unlit);
        #[cfg(feature = "mesh")]
        if unlit != app.mesh_unlit {
            app.mesh_unlit = unlit;
            tools.set_view(if unlit { ViewMode::Albedo } else { ViewMode::Full });
        }
        let tools_read = tools.before_draw(&mut genos_debug::Ctx {
            renderer,
            camera: &mut camera,
            host: &mut app,
        });
        overlay.extend_from_slice(tools.overlay());
        // The topology shade's ID labels, each on a dark plate.
        #[cfg(feature = "mesh")]
        let mut gizmo_lines: Vec<genos_render::ScreenLine> = Vec::new();
        #[cfg(feature = "mesh")]
        if let Some(view) = &mut app.mesh {
            let (width, height) = (size.0 as f32, size.1 as f32);
            // The web viewer's corners: the workspace line, the counts, the
            // hints, and the axis gizmo. The menu has the top left corner.
            let write = |overlay: &mut Vec<ScreenRect>, words: &str, x: f32, y: f32, color: [f32; 3]| {
                let (paints, _) = genos_ui::small_text(words, 0.0, 0.0, color);
                overlay.extend(paints.iter().map(|p| ScreenRect { x: x + p.x, y: y + p.y, w: p.w, h: p.h, color: p.color }));
            };
            let wide = |words: &str| words.chars().count() as f32 * 6.0;
            let [title, counts, hints] = view.hud();
            let [right, top] = view.hud_inset;
            let span = width - right;
            write(&mut overlay, &title, ((span - wide(&title)) * 0.5).round(), top + 10.0, HUD_BRIGHT);
            write(&mut overlay, &counts, 16.0, height - 22.0, HUD_BRIGHT);
            write(&mut overlay, &hints, (span - wide(&hints) - 16.0).round(), height - 22.0, HUD_DIM);
            for end in view.gizmo(&camera, [width, height]) {
                if let Some(centre) = end.line_from {
                    gizmo_lines.push(genos_render::ScreenLine { a: centre, b: end.at, width: 2.0, color: end.color });
                }
                let plate = if end.look.is_some() { end.color.map(|c| c * 0.35) } else { LABEL_PLATE };
                overlay.push(ScreenRect { x: end.at[0] - end.half[0], y: end.at[1] - end.half[1], w: end.half[0] * 2.0, h: end.half[1] * 2.0, color: plate });
                let text = if end.look.is_some() { [1.0; 3] } else { end.color };
                write(&mut overlay, &end.label, (end.at[0] - wide(&end.label) * 0.5).round(), (end.at[1] - 4.0).round(), text);
            }
            for label in view.id_labels(&camera, width / height.max(1.0)) {
                let color = if label.face { mesh_view::FACE_LABEL_COLOR } else { mesh_view::VERTEX_LABEL_COLOR };
                let (paints, wide) = genos_ui::small_text(&label.text, 0.0, 0.0, color);
                let (x, y) = ((label.at[0] * width - wide * 0.5).round(), (label.at[1] * height - LABEL_HEIGHT * 0.5).round());
                overlay.push(ScreenRect { x: x - 2.0, y: y - 2.0, w: wide + 3.0, h: LABEL_HEIGHT + 4.0, color: LABEL_PLATE });
                overlay.extend(paints.iter().map(|p| ScreenRect { x: x + p.x, y: y + p.y, w: p.w, h: p.h, color: p.color }));
            }
        }
        let fps_now = app.scene().fps_counter.effective();
        fps_overlay.note(
            fps_now.enabled,
            fps_now.rate_hz,
            real_dt,
            [size.0 as f32, size.1 as f32],
        );
        if fps_now.enabled {
            overlay.extend(fps_overlay.paints().iter().map(|paint| ScreenRect {
                x: paint.x,
                y: paint.y,
                w: paint.w,
                h: paint.h,
                color: paint.color,
            }));
        }
        drawn += 1;
        let last = opts.frames.is_some_and(|limit| drawn >= limit);
        let want_read = tools_read || last;
        // The box or the lasso being dragged in a mesh scene.
        #[cfg(feature = "mesh")]
        let outline = app.mesh.as_ref().map(mesh_view::MeshView::region_outline).unwrap_or_default();
        #[cfg(feature = "mesh")]
        let dragged: Vec<genos_render::ScreenLine> = (0..outline.len())
            .map(|at| genos_render::ScreenLine {
                a: outline[at],
                b: outline[(at + 1) % outline.len()],
                width: 1.5,
                color: mesh_view::OVERLAY_COLOR,
            })
            .collect();
        #[cfg(feature = "mesh")]
        if app.mesh.is_some() {
            let mut lines = gizmo_lines;
            lines.extend(dragged);
            renderer.set_overlay_lines(&lines);
        }
        if want_read && !tools.show_capture_overlay(capture_ui) {
            overlay.clear();
            renderer.set_overlay_lines(&[]);
        }
        let draw_start = Instant::now();
        renderer.set_particle_dt(dt);
        renderer.set_frame_dt(tools.fixed_step().unwrap_or(frame_dt));
        let drawn_world = if loading { &load_world } else { app.world() };
        let pixels =
            renderer.draw_with_overlay(drawn_world, &camera, &overlay, want_read, false)?;
        if drawn == 1 {
            eprintln!(
                "genos-startup first draw: {:.1} ms (window to first frame {:.1} ms, process {:.1} ms, loop wait {:.1} ms)",
                draw_start.elapsed().as_secs_f64() * 1000.0,
                window_at.elapsed().as_secs_f64() * 1000.0,
                run_start.elapsed().as_secs_f64() * 1000.0,
                loop_at.elapsed().as_secs_f64() * 1000.0
            );
            for (name, took) in renderer.pipeline_times() {
                eprintln!(
                    "genos-startup pipeline {name}: {:.1} ms",
                    took.as_secs_f64() * 1000.0
                );
            }
        }
        tools.after_draw(
            renderer,
            pixels.as_deref(),
            genos_debug::FrameTiming {
                frame_ms: real_dt as f64 * 1000.0,
                draw_ms: draw_start.elapsed().as_secs_f64() * 1000.0,
                gpu_ms: None,
            },
        );
        note_gi(&mut app, renderer);
        if last {
            if let Some(pixels) = pixels {
                let path = opts
                    .out
                    .join("shots")
                    .join(format!("{}.png", app.entry().name));
                png::write_png(&path, renderer.width(), renderer.height(), &pixels)?;
                let (mean, spread) = picture_stats(&pixels);
                let _ = writeln!(
                    log,
                    "shot {} mean {mean:.1} spread {spread:.1}",
                    path.display()
                );
            }
        }
        if last || tools.quit_code().is_some() {
            let code = script_exit_code(&script_hold, tools.quit_code().unwrap_or(0));
            // The quit harness reads this line to know the run rendered.
            println!("genos-lighting frames={drawn}");
            return Ok(code);
        }
    }
}

fn script_exit_code(
    slot: &std::sync::Mutex<Option<genos_window::ThreadStop<bool>>>,
    code: i32,
) -> i32 {
    let mut guard = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(stop) = guard.as_mut() else {
        return code;
    };
    if stop.try_join() && stop.result().copied() == Some(false) {
        1
    } else {
        code
    }
}

fn check_all(app: &mut App, opts: &Options, log: &mut std::fs::File) -> Result<(), String> {
    let mut window = Window::open_proof_on(opts.size.0, opts.size.1, opts.display.as_deref())?;
    window.install_seat_pump();
    Renderer::set_blocker(genos_window::seat_enter, genos_window::seat_leave);
    let first = window.pump();
    if first.closing {
        return Ok(());
    }
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, first.width, first.height)?;
    let mut owner = genos_window::Lifecycle::new(window, renderer, permit);
    if let Some(stall) = opts.block_ms {
        owner.plant_block(stall);
    }
    if opts.close_early || owner.window.pump().closing {
        owner.quit(0);
    }
    let window = &mut *owner.window;
    let renderer = &mut *owner.device;
    let load_world = loading_world();
    let mut camera = Camera::opening();
    let mut passed = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();
    let names: Vec<String> = app.entries.iter().map(|e| e.name.clone()).collect();
    let mut checked_frames = 0u32;
    for name in &names {
        let _ = writeln!(log, "check {name}");
        eprintln!("{}: check {name}", program().name);
        if let Err(err) = app.start_load(name) {
            if gltf_scene(&app.entries, name) && gltf_bytes_absent(&err) {
                let line = skip_line(name, &err);
                skipped.push(name.clone());
                eprintln!("{}: {line}", program().name);
                let _ = writeln!(log, "{line}");
                continue;
            }
            return Err(err);
        }
        let mut spins = 0u32;
        while app.loading() {
            let frame = window.pump();
            if !frame.minimized {
                renderer.set_particle_dt(STEP);
                let _ = renderer.draw(&load_world, &camera, false);
            }
            if let Err(err) = app.take_load() {
                if gltf_scene(&app.entries, name) && gltf_bytes_absent(&err) {
                    let line = skip_line(name, &err);
                    skipped.push(name.clone());
                    eprintln!("{}: {line}", program().name);
                    let _ = writeln!(log, "{line}");
                    break;
                }
                return Err(err);
            }
            spins += 1;
            if spins > 100_000 {
                return Err(format!("{name}: scene load did not finish"));
            }
        }
        if skipped
            .last()
            .is_some_and(|skipped_name| skipped_name == name)
        {
            continue;
        }
        let check = app.entry().check.clone();
        jump_camera(app, Some(&check.checkpoint), &mut camera)?;
        renderer.reset_lighting_cache();
        let mut pixels = None;
        for frame in 0..4 {
            let _ = window.pump();
            app.advance(STEP);
            let read = frame == 3;
            renderer.set_particle_dt(STEP);
            pixels = renderer.draw(app.world(), &camera, read)?;
            checked_frames += 1;
        }
        let pixels = pixels.ok_or_else(|| format!("{name}: no picture"))?;
        let (mean, spread) = picture_stats(&pixels);
        let path = opts.out.join("shots").join(format!("{name}.png"));
        png::write_png(&path, renderer.width(), renderer.height(), &pixels)?;
        let verdict = if mean >= check.min_mean {
            passed.push(name.clone());
            "pass"
        } else {
            failed.push(format!(
                "{name} at {} mean {mean:.1} is under {:.1}",
                check.checkpoint, check.min_mean
            ));
            "fail"
        };
        let line = format!(
            "check {name} {verdict} {} mean {mean:.1} spread {spread:.1} min {:.1} {}",
            check.checkpoint,
            check.min_mean,
            path.display()
        );
        eprintln!("{}: {line}", program().name);
        let _ = writeln!(log, "{line}");
    }
    // The quit harness reads this line to know the run rendered.
    println!("genos-lighting frames={checked_frames}");
    let summary = check_summary(&passed, &skipped, &failed);
    let _ = writeln!(log, "{summary}");
    if failed.is_empty() {
        eprintln!("{}: {summary}", program().name);
        Ok(())
    } else {
        Err(summary)
    }
}

/// A missing glTF is a skip. The tally does not count it as a pass.
fn skip_line(name: &str, err: &str) -> String {
    format!("skip {name} (not a pass): {err}")
}

fn check_summary(passed: &[String], skipped: &[String], failed: &[String]) -> String {
    let mut line = format!(
        "{} passed, {} skipped, {} failed",
        passed.len(),
        skipped.len(),
        failed.len()
    );
    if !skipped.is_empty() {
        line.push_str(" (skipped ");
        line.push_str(&skipped.join(", "));
        line.push(')');
    }
    if !failed.is_empty() {
        line.push_str(": ");
        line.push_str(&failed.join("; "));
    }
    line
}

struct LoadJob {
    rx: std::sync::mpsc::Receiver<Result<Show, String>>,
}

struct App {
    entries: Vec<Entry>,
    index: usize,
    checkpoint: usize,
    show: Show,
    root: PathBuf,
    log: std::fs::File,
    load: Option<LoadJob>,
    /// What the last "Save look to scene" did. The Output page shows it.
    look_note: String,
    /// Set while a mesh scene is open: the workspace file being followed.
    #[cfg(feature = "mesh")]
    mesh: Option<mesh_view::MeshView>,
    /// How many of the world's objects are the scene itself. The viewer's
    /// extra objects (wireframe, selection) come after them.
    #[cfg(feature = "mesh")]
    mesh_overlay: Option<usize>,
    /// The mesh shade is one mesh-workbench draws unlit, and the view was
    /// switched to the base colour for it.
    #[cfg(feature = "mesh")]
    mesh_unlit: bool,
    /// The mesh scene's own sky colour, while a shade shows another backdrop.
    #[cfg(feature = "mesh")]
    mesh_sky: Option<[f32; 3]>,
    /// Last GI pass times, milliseconds. Numbers only.
    gi_passes: String,
    /// Last GI work counters. Numbers only.
    gi_work: String,
    /// Launch fog arguments. Reapplied over each loaded scene.
    fog_cli: FogCli,
    /// Mirrors for the fog arguments this launch left empty.
    fog_env: FogEnv,
    /// Launch particle arguments. Reapplied over each loaded scene.
    particle_cli: ParticleCli,
    /// Mirrors for the particle arguments this launch left empty.
    particle_env: ParticleEnv,
}

enum Show {
    Stress(Stage),
    Still(World),
}

/// Seconds between looks at a mesh workspace's `project.json`. The
/// mesh-workbench web viewer polls at 0.4 s.
#[cfg(feature = "mesh")]
const MESH_POLL_SECONDS: f32 = 0.4;
#[cfg(feature = "mesh")]
const MESH_GROUND_COLOR: [f32; 3] = [0.028, 0.032, 0.04];
/// The ground reaches this many mesh sizes out, and at least this far.
#[cfg(feature = "mesh")]
const MESH_GROUND_SCALE: f32 = 4.0;
#[cfg(feature = "mesh")]
const MESH_GROUND_MIN: f32 = 6.0;

/// The workspace a mesh scene opens: `--mesh-workspace`, then
/// `GENOS_MESH_WORKSPACE`, then the scene file's path under the repository.
#[cfg(feature = "mesh")]
fn mesh_workspace_dir(root: &Path, named: &str) -> PathBuf {
    // The picker's choice comes first: it was made after the program started.
    if let Some(chosen) = mesh_view::chosen_workspace() {
        return chosen;
    }
    match std::env::var_os("GENOS_MESH_WORKSPACE") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => root.join(named),
    }
}

/// A mesh-workbench workspace as a world: every object a raster mesh, standing
/// on a ground plane. The mesh is lifted so its lowest point is on the ground.
/// It has no baked distance field, so it does not block or bounce light.
/// A mesh scene's show. Without the `mesh` feature there is no mesh viewer.
#[cfg(feature = "mesh")]
fn mesh_show(root: &Path, workspace: &str, sun: Option<&Sun>, sky: Option<[f32; 3]>, lamps: &[Lamp]) -> Result<Show, String> {
    Ok(Show::Still(mesh_world(&mesh_workspace_dir(root, workspace), sun, sky, lamps)?))
}

#[cfg(not(feature = "mesh"))]
fn mesh_show(_root: &Path, workspace: &str, _sun: Option<&Sun>, _sky: Option<[f32; 3]>, _lamps: &[Lamp]) -> Result<Show, String> {
    Err(format!("{workspace} is a mesh scene: this program is built without the mesh viewer; run genos-editor"))
}

#[cfg(feature = "mesh")]
fn mesh_world(dir: &Path, sun: Option<&Sun>, sky: Option<[f32; 3]>, lamps: &[Lamp]) -> Result<World, String> {
    let workspace = mesh_view::load_shown(dir)?;
    let size = (0..3)
        .map(|axis| workspace.max[axis] - workspace.min[axis])
        .fold(0.0f32, f32::max);
    let reach = (size * MESH_GROUND_SCALE).max(MESH_GROUND_MIN);
    let lift = -workspace.min[1];
    let mut scene = {
        let mut __scene = Scene::from_lists(
            Floor {
                position: Vec3::new(
                    f32::midpoint(workspace.min[0], workspace.max[0]),
                    0.0,
                    f32::midpoint(workspace.min[2], workspace.max[2]),
                ),
                half_x: reach,
                half_z: reach,
                color: MESH_GROUND_COLOR,
            },
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        __scene.ceiling = None;
        __scene.sky = sky.map(|color| Sky { color });
        __scene.lighting = genos_scene::LightingSettings::default();
        __scene.debug = DebugSettings::default();
        __scene.fps_counter = FpsCounterSettings::default();
        __scene.event_loop = EventLoopSettings::default();
        __scene.reflections = ReflectionSettings::default();
        __scene.hdr = genos_scene::HdrSettings::legacy_bytes();
        __scene.fog = FogSettings::default();
        __scene.particles = ParticleSettings::default();
        __scene.menu = MenuSettings::default();
        __scene
    };
    if let Some(sun) = sun {
        scene.push_light(sun_light(sun));
        // The web viewer lights a mesh with a key light and two weaker ones
        // from the other sides, so no face is black. The scene's sun is the
        // key; these are the other two, at the web viewer's strengths.
        for (direction, share) in MESH_FILL_LIGHTS {
            scene.push_light(sun_light(&Sun { direction, color: sun.color.map(|channel| channel * share) }));
        }
    }
    for lamp in lamps {
        scene.push_light(point_light(lamp));
    }
    let mut world = World::from_scene(scene);
    // Surface 0: the mesh's textures and the UV checker in one atlas. A part
    // draws with it only in the `textured` and `uv_checker` shades.
    let atlas = mesh_view::atlas(&workspace.textures);
    // In `mesh_view`'s order: SURFACE_CHECKER, then SURFACE_TEXTURES.
    world.surfaces.clear();
    let cutoffs = std::iter::once(0.0).chain(atlas.cutoffs.iter().copied());
    for (color, cutoff) in [atlas.checker].iter().chain(&atlas.rects).zip(cutoffs) {
        world.surfaces.push(genos_render::Surface {
            color: *color,
            normal: [0; 4],
            cutoff,
            two_sided: false,
            roughness: 1.0,
            metallic: 0.0,
            material_flags: genos_render::MAT_BASE,
        });
    }
    world.color_atlas = genos_render::MaterialAtlas::from_rgba(atlas.width, atlas.height, atlas.rgba);
    let mut pose = identity_pose();
    pose[13] = lift;
    for part in workspace.parts {
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        for p in &part.positions {
            for axis in 0..3 {
                low[axis] = low[axis].min(p[axis]);
                high[axis] = high[axis].max(p[axis]);
            }
        }
        world.objects.push(genos_render::Object {
            hidden: false,
            affects_light: false,
            bounds: genos_render::Bounds {
                center: [
                    f32::midpoint(low[0], high[0]),
                    f32::midpoint(low[1], high[1]) + lift,
                    f32::midpoint(low[2], high[2]),
                ],
                half: [
                    (high[0] - low[0]) * 0.5,
                    (high[1] - low[1]) * 0.5,
                    (high[2] - low[2]) * 0.5,
                ],
            },
            kind: DrawKind::Mesh {
                vertices: part.positions,
                normals: part.normals,
                texcoords: Vec::new(),
                color: mesh_view::MESH_COLOR,
                colors: Vec::new(),
                pose,
                emission: [0.0; 3],
                surface: None,
            },
        });
    }
    eprintln!(
        "{}: mesh workspace {} revision {}: {} vertices, {} faces", program().name,
        dir.display(),
        workspace.revision,
        workspace.vertices,
        workspace.faces
    );
    Ok(world)
}

impl App {
    fn open(entries: Vec<Entry>, root: PathBuf, log: std::fs::File) -> Result<Self, String> {
        let mut show = build_entry(&root, &entries[0])?;
        adopt_show(&mut show, &entries[0]);
        Ok(Self {
            entries,
            index: 0,
            checkpoint: 0,
            show,
            root,
            log,
            load: None,
            look_note: String::new(),
            #[cfg(feature = "mesh")]
            mesh: None,
            #[cfg(feature = "mesh")]
            mesh_overlay: None,
            #[cfg(feature = "mesh")]
            mesh_unlit: false,
            #[cfg(feature = "mesh")]
            mesh_sky: None,
            gi_passes: "direct 0.00 atrous 0.00 compose 0.00 trace 0.00 filter 0.00 gather 0.00 resolve 0.00".into(),
            gi_work: "rays 0 refreshed 0 lamps 0".into(),
            fog_cli: FogCli::default(),
            fog_env: FogEnv::default(),
            particle_cli: ParticleCli::default(),
            particle_env: ParticleEnv::default(),
        })
    }

    /// Arguments win. An omitted argument keeps the scene value, or the mirror
    /// when that variable was set.
    fn overlay_fog(&mut self) {
        let cli = self.fog_cli.clone();
        let env = self.fog_env;
        let scene = self.scene();
        scene.fog = scene.fog.resolve(&cli, &env);
    }

    /// Arguments win. An omitted argument keeps the scene value, or the mirror
    /// when that variable was set.
    fn overlay_particles(&mut self) {
        let cli = self.particle_cli.clone();
        let env = self.particle_env;
        let scene = self.scene();
        scene.particles = scene.particles.resolve(&cli, &env);
    }

    fn entry(&self) -> &Entry {
        &self.entries[self.index]
    }

    fn world(&self) -> &World {
        match &self.show {
            Show::Stress(stage) => &stage.world,
            Show::Still(world) => world,
        }
    }

    fn advance(&mut self, dt: f32) {
        if let Show::Stress(stage) = &mut self.show {
            stage.advance(dt);
        }
        self.follow_mesh();
    }

    /// A mesh scene follows its workspace: when `project.json` changes on
    /// disk, read it again. mesh-workbench replaces the file by rename and
    /// pushes nothing, so this looks at the modification time
    /// `MESH_POLL_SECONDS` apart. A file that does not parse (caught half
    /// written) keeps the picture and is tried again.
    /// Without the mesh viewer there is no workspace to follow.
    #[cfg(not(feature = "mesh"))]
    #[expect(clippy::unused_self, clippy::needless_pass_by_ref_mut, reason = "the same call as with the mesh viewer")]
    fn follow_mesh(&mut self) {}

    #[cfg(feature = "mesh")]
    fn follow_mesh(&mut self) {
        if self.load.is_some() {
            return;
        }
        let Kind::Mesh { workspace, .. } = &self.entry().kind else {
            self.mesh = None;
            self.mesh_overlay = None;
            self.mesh_sky = None;
            return;
        };
        let dir = mesh_workspace_dir(&self.root, workspace);
        let now = Instant::now();
        if let Some(watch) = &self.mesh {
            if watch.dir == dir && !watch.rebuild && now.duration_since(watch.looked).as_secs_f32() < MESH_POLL_SECONDS {
                return;
            }
        }
        let stamp = std::fs::metadata(genos_workbench::project_path(&dir))
            .and_then(|meta| meta.modified())
            .ok();
        let changed = match &mut self.mesh {
            Some(watch) if watch.dir == dir => {
                watch.looked = now;
                watch.poll_sessions();
                // A replayed state does not follow the live file.
                watch.rebuild || (watch.replay.is_none() && watch.stamp != stamp)
            }
            Some(_) => {
                // Another workspace: a new view, a new world, the mesh in frame.
                let mut view = mesh_view::MeshView::new(dir, stamp);
                view.frame_next = true;
                view.poll_sessions();
                self.mesh = Some(view);
                self.mesh_overlay = None;
                true
            }
            None => {
                let mut view = mesh_view::MeshView::new(dir, stamp);
                view.poll_sessions();
                self.mesh = Some(view);
                // A mesh scene opens the menu on the mesh controls.
                let menu = &mut self.scene().menu;
                if menu.tab.is_empty() || menu.tab == "Scene" {
                    menu.tab = MESH_TAB.into();
                }
                // A mesh is looked at for its shape and colour: no glow
                // around bright faces, and one brightness from every side.
                // The Output tab still changes either.
                let hdr = &mut self.scene().hdr;
                let _ = hdr.set_key("bloom", "off");
                let _ = hdr.set_key("exposure_mode", "fixed");
                let _ = hdr.set_key("ev100", MESH_EV100);
                // The scene file's own look wins over those.
                let entry = self.entry().clone();
                apply_entry_hdr(&mut self.scene().hdr, &entry);
                false
            }
        };
        if !changed {
            self.refresh_mesh_overlay();
            return;
        }
        if let Some(watch) = &mut self.mesh {
            watch.reload(stamp);
        }
        let index = self.index;
        match build_entry(&self.root, &self.entries[index]) {
            Ok(mut show) => {
                let from = self.scene().clone();
                adopt_show(&mut show, &self.entries[index]);
                self.show = show;
                let scene = self.scene();
                scene.menu = from.menu;
                scene.fps_counter = from.fps_counter;
                scene.debug = from.debug;
                scene.event_loop = from.event_loop;
                scene.lighting = from.lighting;
                scene.hdr = from.hdr;
                self.overlay_fog();
                self.overlay_particles();
                self.mesh_overlay = None;
                self.refresh_mesh_overlay();
                self.note("mesh workspace changed; reloaded");
            }
            Err(err) => self.note(&format!("mesh workspace changed; kept the picture: {err}")),
        }
    }

    /// Bring the picture's mesh objects up to the viewer's state: hidden
    /// objects and the colour mode on the workspace's objects, then the extra
    /// objects after them (the wireframe strips, the selection).
    #[cfg(feature = "mesh")]
    fn refresh_mesh_overlay(&mut self) {
        let Some(view) = &mut self.mesh else {
            return;
        };
        view.repose();
        if !view.overlay_stale && !view.display_stale {
            return;
        }
        view.overlay_stale = false;
        view.display_stale = false;
        let display = view.display();
        let posed = view.posed_positions();
        let lift = view.lift();
        // The ground and the sky the shade is seen against.
        let backdrop = view.backdrop();
        let ground = backdrop.map_or(MESH_GROUND_COLOR, |(color, _)| color);
        let furniture = backdrop.is_none_or(|(_, drawn)| drawn);
        let (shadow_at, shadow_up, shadow_colors) =
            if furniture { view.shadow_patches(ground) } else { (Vec::new(), Vec::new(), Vec::new()) };
        let plain = |(positions, normals): (Vec<[f32; 3]>, Vec<[f32; 3]>), color| mesh_view::Piece {
            positions,
            normals,
            colors: Vec::new(),
            texcoords: Vec::new(),
            surface: None,
            emission: [0.0; 3],
        }
        .tinted(color);
        // The grid lines stand on edge, where the sun does not reach them:
        // they carry a little light of their own, as the web viewer's do.
        let mut grid = plain(if furniture { view.grid_strips() } else { (Vec::new(), Vec::new()) }, mesh_view::GRID_COLOR);
        grid.0.emission = mesh_view::GRID_GLOW;
        let mut extras = vec![
            mesh_view::Piece { positions: shadow_at, normals: shadow_up, colors: shadow_colors, texcoords: Vec::new(), surface: None, emission: [0.0; 3] }
                .tinted([1.0; 3]),
            grid,
            plain(view.wire(), mesh_view::WIRE_COLOR),
            plain(view.bone_strips(), mesh_view::BONE_COLOR),
            plain(view.overlay(), mesh_view::OVERLAY_COLOR),
        ];
        // A part with several textures, one piece a texture.
        extras.extend(view.textured_pieces().into_iter().map(|piece| piece.tinted(mesh_view::MESH_COLOR)));
        let posed_normals = view.posed_normals();
        let rest_normals: Vec<Vec<[f32; 3]>> =
            view.workspace.iter().flat_map(|w| w.parts.iter().map(|p| p.normals.clone())).collect();
        let rest: Vec<Vec<[f32; 3]>> = view
            .workspace
            .iter()
            .flat_map(|w| w.parts.iter().map(|p| p.positions.clone()))
            .collect();
        let Show::Still(world) = &mut self.show else {
            return;
        };
        // The scene's own sky is kept the first time, to go back to.
        let own_sky = *self.mesh_sky.get_or_insert(world.scene.sky.as_ref().map_or([0.0; 3], |sky| sky.color));
        world.scene.sky = Some(genos_scene::Sky { color: backdrop.map_or(own_sky, |(color, _)| color) });
        world.scene.floor.color = ground;
        // The extras sit after the workspace's objects. Drop the old ones.
        let base = *self.mesh_overlay.get_or_insert(world.objects.len());
        world.objects.truncate(base);
        let meshes = world
            .objects
            .iter_mut()
            .filter(|object| matches!(object.kind, DrawKind::Mesh { .. }));
        for (index, (object, (hidden, shade, per_vertex, uvs))) in meshes.zip(display).enumerate() {
            object.hidden = hidden;
            if let DrawKind::Mesh { color, colors, vertices, normals, texcoords, surface, .. } = &mut object.kind {
                // The normals follow the pose, and go back with it.
                match (posed_normals.get(index).cloned().flatten(), rest_normals.get(index)) {
                    (Some(at), _) => *normals = at,
                    (None, Some(at)) if at.len() == normals.len() => normals.clone_from(at),
                    _ => {}
                }
                *color = shade;
                *colors = per_vertex;
                // The surfaces are the ones `mesh_world` made.
                let (drawn_with, uvs) = uvs.map_or((None, Vec::new()), |(at, uvs)| (Some(at), uvs));
                *surface = drawn_with;
                *texcoords = uvs;
                // The pose, or back to the rest positions.
                match (posed.get(index).cloned().flatten(), rest.get(index)) {
                    (Some(at), _) => *vertices = at,
                    (None, Some(at)) if at.len() == vertices.len() => vertices.clone_from(at),
                    _ => {}
                }
            }
        }
        let mut pose = identity_pose();
        pose[13] = lift;
        for (piece, color) in extras {
            let mesh_view::Piece { positions, normals, colors, texcoords, surface, emission } = piece;
            if positions.is_empty() {
                continue;
            }
            let mut low = [f32::INFINITY; 3];
            let mut high = [f32::NEG_INFINITY; 3];
            for p in &positions {
                for axis in 0..3 {
                    low[axis] = low[axis].min(p[axis]);
                    high[axis] = high[axis].max(p[axis]);
                }
            }
            world.objects.push(genos_render::Object {
                hidden: false,
                affects_light: false,
                bounds: genos_render::Bounds {
                    center: [
                        f32::midpoint(low[0], high[0]),
                        f32::midpoint(low[1], high[1]) + lift,
                        f32::midpoint(low[2], high[2]),
                    ],
                    half: [
                        (high[0] - low[0]) * 0.5,
                        (high[1] - low[1]) * 0.5,
                        (high[2] - low[2]) * 0.5,
                    ],
                },
                kind: DrawKind::Mesh {
                    vertices: positions,
                    normals,
                    texcoords,
                    color,
                    colors,
                    pose,
                    emission,
                    surface,
                },
            });
        }
    }

    fn note(&mut self, line: &str) {
        eprintln!("{}: {line}", program().name);
        let _ = writeln!(self.log, "{line}");
    }

    /// Write the scene's HDR look into the scene file it was read from, and
    /// keep the same lines on the entry, so a later load of this scene in
    /// this run opens with the saved look too.
    fn save_look(&mut self) -> Result<genos_debug::SavedLook, String> {
        let scene = self.entry().name.clone();
        let Some(path) = self.entry().path.clone() else {
            return Err(format!("scene {scene} was not read from a file; there is no file to save its HDR look to"));
        };
        let hdr = self.scene_ref().hdr.clone();
        let lines = genos_scene_lib::save_hdr_look(&path, &hdr)?;
        let index = self.index;
        self.entries[index].hdr = hdr.look_pairs();
        Ok(genos_debug::SavedLook {
            scene,
            path: path.display().to_string(),
            lines,
        })
    }

    /// The menu's button: save, and say what happened on the Output page and in the log.
    fn save_look_from_menu(&mut self) {
        let line = match self.save_look() {
            Ok(saved) => {
                let file = Path::new(&saved.path)
                    .file_name()
                    .map_or_else(|| saved.path.clone(), |name| name.to_string_lossy().into_owned());
                self.note(&format!("saved the HDR look of {} to {}", saved.scene, saved.path));
                format!("Saved to {file}")
            }
            Err(err) => {
                self.note(&format!("the HDR look was not saved: {err}"));
                format!("Not saved: {err}")
            }
        };
        self.look_note = line;
    }

    fn scene_ref(&self) -> &Scene {
        match &self.show {
            Show::Stress(stage) => &stage.world.scene,
            Show::Still(world) => &world.scene,
        }
    }

    fn loading(&self) -> bool {
        self.load.is_some()
    }

    /// Read `name` on a worker. The call returns while that read is still running.
    fn start_load(&mut self, name: &str) -> Result<LoadPace, String> {
        let name = genos_scene_lib::canonical(name);
        if self.entry().name == name && self.load.is_none() {
            return Ok(LoadPace::Ready);
        }
        let index = self
            .entries
            .iter()
            .position(|entry| entry.name == name)
            .ok_or_else(|| format!("unknown scene {name}"))?;
        let root = self.root.clone();
        let entry = self.entries[index].clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("genos-scene-load".into())
            .spawn(move || {
                let _ = tx.send(build_entry(&root, &entry));
            })
            .map_err(|err| err.to_string())?;
        self.index = index;
        self.checkpoint = 0;
        self.load = Some(LoadJob { rx });
        Ok(LoadPace::Pending)
    }

    fn take_load(&mut self) -> Result<LoadPace, String> {
        let incoming = {
            let Some(job) = self.load.as_ref() else {
                return Ok(LoadPace::Ready);
            };
            match job.rx.try_recv() {
                Ok(value) => value,
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(LoadPace::Pending),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.load = None;
                    return Err("scene load stopped".into());
                }
            }
        };
        self.load = None;
        let show = incoming?;
        self.install_show(show);
        Ok(LoadPace::Ready)
    }

    fn install_show(&mut self, show: Show) {
        let menu = self.scene().menu.clone();
        let fps = self.scene().fps_counter;
        let debug = self.scene().debug.clone();
        let event_loop = self.scene().event_loop;
        let name = self.entry().name.clone();
        let old = std::mem::replace(&mut self.show, show);
        self.scene().menu = menu;
        self.scene().fps_counter = fps;
        self.scene().debug = debug;
        self.scene().event_loop = event_loop;
        genos_window::retire(old);
        adopt_show(&mut self.show, &self.entries[self.index]);
        self.overlay_fog();
        self.overlay_particles();
        self.note(&format!("loaded {name}"));
    }
}

fn adopt_hdr_preset(scene: &mut genos_scene::Scene) {
    if !scene.hdr.legacy_bytes {
        return;
    }
    let preset = std::env::var("GENOS_LIGHTING_PRESET")
        .ok()
        .and_then(|name| genos_scene::LightingPreset::parse(&name))
        .unwrap_or(scene.lighting.preset);
    scene.hdr = genos_scene::HdrSettings::for_preset(preset);
}

/// Lay the scene file's `hdr KEY VALUE` lines over `hdr`. A key the file does
/// not name keeps its value. scene-lib refused a bad pair when it read the file.
fn apply_entry_hdr(hdr: &mut genos_scene::HdrSettings, entry: &Entry) {
    for (key, value) in &entry.hdr {
        if let Err(err) = hdr.set_key(key, value) {
            eprintln!("{}: scene {} hdr {key}: {err}", program().name, entry.name);
        }
    }
}

/// The HDR value a scene opens with: the lighting preset's defaults, then the
/// scene file's `hdr` lines. The arguments and the `GENOS_HDR_*` mirrors are
/// laid over it where it is read (`HdrSettings::effective`).
fn adopt_show(show: &mut Show, entry: &Entry) {
    let scene = match show {
        Show::Stress(stage) => &mut stage.world.scene,
        Show::Still(world) => &mut world.scene,
    };
    adopt_hdr_preset(scene);
    apply_entry_hdr(&mut scene.hdr, entry);
}

impl Host for App {
    fn scene(&mut self) -> &mut Scene {
        match &mut self.show {
            Show::Stress(stage) => &mut stage.world.scene,
            Show::Still(world) => &mut world.scene,
        }
    }

    fn gltf_surface(&self) -> Option<std::sync::Arc<genos_render::GltfSurface>> {
        match &self.show {
            Show::Stress(stage) => stage.world.gltf_surface.clone(),
            Show::Still(world) => world.gltf_surface.clone(),
        }
    }

    fn knobs(&self) -> Vec<Knob> {
        match &self.show {
            Show::Stress(stage) => Host::knobs(stage),
            Show::Still(world) => {
                let mut knobs = lighting_knobs(&world.scene.lighting);
                knobs.extend(fog_knobs(&world.scene.fog));
                knobs.extend(particle_knobs(&world.scene.particles));
                #[cfg(feature = "mesh")]
                if let Some(view) = &self.mesh {
                    let at = mesh_view::PICK_MODES
                        .iter()
                        .position(|mode| *mode == view.pick_mode)
                        .unwrap_or(0);
                    let shade = mesh_view::SHADES.iter().position(|mode| *mode == view.shade).unwrap_or(0);
                    knobs.push(Knob {
                        name: "mesh_shade".into(),
                        label: "Mesh shading".into(),
                        value: shade as f64,
                        min: 0.0,
                        max: (mesh_view::SHADES.len() - 1) as f64,
                        step: 1.0,
                        choices: mesh_view::SHADES.iter().map(|mode| (*mode).to_string()).collect(),
                        text: view.shade.clone(),
                    });
                    // "all", then each object: choosing one shows it alone.
                    let parts: Vec<(u64, String)> = view
                        .workspace
                        .iter()
                        .flat_map(|w| w.parts.iter().map(|p| (p.object, format!("{} {}", p.object, p.name))))
                        .collect();
                    let shown: Vec<&(u64, String)> = parts.iter().filter(|(id, _)| !view.hidden.contains(id)).collect();
                    let solo = match shown.as_slice() {
                        [only] if parts.len() > 1 => parts.iter().position(|p| p.0 == only.0).map_or(0, |i| i + 1),
                        _ => 0,
                    };
                    let mut choices = vec!["all".to_string()];
                    choices.extend(parts.iter().map(|p| p.1.clone()));
                    knobs.push(Knob {
                        name: "mesh_solo".into(),
                        label: "Show object".into(),
                        value: solo as f64,
                        min: 0.0,
                        max: parts.len() as f64,
                        step: 1.0,
                        text: choices[solo].clone(),
                        choices,
                    });
                    let clips: Vec<&genos_workbench::Clip> = view.workspace.iter().flat_map(|w| &w.clips).collect();
                    if !clips.is_empty() {
                        let at = view
                            .pose
                            .as_ref()
                            .and_then(|(name, _)| clips.iter().position(|clip| clip.name == *name))
                            .map_or(0, |i| i + 1);
                        let mut choices = vec!["rest".to_string()];
                        choices.extend(clips.iter().map(|clip| clip.name.clone()));
                        knobs.push(Knob {
                            name: "mesh_clip".into(),
                            label: "Clip".into(),
                            value: at as f64,
                            min: 0.0,
                            max: clips.len() as f64,
                            step: 1.0,
                            text: choices[at].clone(),
                            choices,
                        });
                        if let Some((name, time)) = &view.pose {
                            let length = clips.iter().find(|clip| clip.name == *name).map_or(1.0, |clip| clip.duration);
                            knobs.push(Knob {
                                name: "mesh_time".into(),
                                label: "Time".into(),
                                value: f64::from(*time),
                                min: 0.0,
                                max: f64::from(length.max(0.01)),
                                step: f64::from(length.max(0.01)) / 100.0,
                                choices: Vec::new(),
                                text: format!("{time:.2} s"),
                            });
                        }
                        knobs.push(Knob {
                            name: "mesh_play".into(),
                            label: "Play clip".into(),
                            value: f64::from(u8::from(view.is_playing())),
                            min: 0.0,
                            max: 1.0,
                            step: 1.0,
                            choices: vec!["off".into(), "on".into()],
                            text: if view.is_playing() { "on".into() } else { "off".into() },
                        });
                        knobs.push(Knob {
                            name: "mesh_surfaces".into(),
                            label: "Mesh surfaces".into(),
                            value: f64::from(u8::from(view.surfaces)),
                            min: 0.0,
                            max: 1.0,
                            step: 1.0,
                            choices: vec!["off".into(), "on".into()],
                            text: if view.surfaces { "on".into() } else { "off".into() },
                        });
                        knobs.push(Knob {
                            name: "mesh_bones".into(),
                            label: "Bones".into(),
                            value: f64::from(u8::from(view.bones)),
                            min: 0.0,
                            max: 1.0,
                            step: 1.0,
                            choices: vec!["off".into(), "on".into()],
                            text: if view.bones { "on".into() } else { "off".into() },
                        });
                    }
                    knobs.push(Knob {
                        name: "mesh_orthographic".into(),
                        label: "Orthographic".into(),
                        value: f64::from(u8::from(view.orthographic)),
                        min: 0.0,
                        max: 1.0,
                        step: 1.0,
                        choices: vec!["off".into(), "on".into()],
                        text: if view.orthographic { "on".into() } else { "off".into() },
                    });
                    knobs.push(Knob {
                        name: "mesh_shadow".into(),
                        label: "Contact shadow".into(),
                        value: f64::from(u8::from(view.shadow)),
                        min: 0.0,
                        max: 1.0,
                        step: 1.0,
                        choices: vec!["off".into(), "on".into()],
                        text: if view.shadow { "on".into() } else { "off".into() },
                    });
                    knobs.push(Knob {
                        name: "mesh_grid".into(),
                        label: "Grid".into(),
                        value: f64::from(u8::from(view.grid)),
                        min: 0.0,
                        max: 1.0,
                        step: 1.0,
                        choices: vec!["off".into(), "on".into()],
                        text: if view.grid { "on".into() } else { "off".into() },
                    });
                    knobs.push(Knob {
                        name: "mesh_turntable".into(),
                        label: "Turntable".into(),
                        value: f64::from(u8::from(view.turntable)),
                        min: 0.0,
                        max: 1.0,
                        step: 1.0,
                        choices: vec!["off".into(), "on".into()],
                        text: if view.turntable { "on".into() } else { "off".into() },
                    });
                    // The scrubber: 0 is the session start, each step a
                    // command, and the last step is the live mesh.
                    if view.replay_span().is_ok() {
                        let on = view.replay_playing.is_some();
                        knobs.push(Knob {
                            name: "mesh_replay_play".into(),
                            label: "Play replay".into(),
                            value: f64::from(u8::from(on)),
                            min: 0.0,
                            max: 1.0,
                            step: 1.0,
                            choices: vec!["off".into(), "on".into()],
                            text: if on { "on".into() } else { "off".into() },
                        });
                    }
                    if let Ok((_, last)) = view.replay_span() {
                        let at = view.replay.as_ref().map_or(last + 1, |r| r.event);
                        knobs.push(Knob {
                            name: "mesh_replay".into(),
                            label: "Replay".into(),
                            value: at as f64,
                            min: 0.0,
                            max: (last + 1) as f64,
                            step: 1.0,
                            choices: Vec::new(),
                            text: view
                                .replay
                                .as_ref()
                                .map_or_else(|| "live".to_string(), |r| format!("{} {}", r.event, r.command)),
                        });
                    }
                    // The readouts: what is shown, and whether a session is open.
                    for (name, label, words) in
                        [("mesh_statistics", "Mesh", view.statistics()), ("mesh_state", "Showing", view.state())]
                    {
                        knobs.push(Knob {
                            name: name.into(),
                            label: label.into(),
                            value: 0.0,
                            min: 0.0,
                            max: 0.0,
                            step: 0.0,
                            choices: Vec::new(),
                            text: words,
                        });
                    }
                    if !view.found.is_empty() {
                        // The open workspace is first. A name is the directory's own.
                        let root = std::fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
                        let names: Vec<String> =
                            view.found.iter().map(|dir| dir.strip_prefix(&root).unwrap_or(dir).display().to_string()).collect();
                        knobs.push(Knob {
                            name: "mesh_workspace".into(),
                            label: "Folder".into(),
                            value: 0.0,
                            min: 0.0,
                            max: (names.len() - 1) as f64,
                            step: 1.0,
                            text: names[0].clone(),
                            choices: names,
                        });
                    }
                    // Read-only: typed with Shift and T, or set by mesh_view filter.
                    let typing = view.typing && view.typing_filter && !view.typing_commands;
                    knobs.push(Knob {
                        name: "mesh_filter".into(),
                        label: if typing { "Filter (Enter ends)".into() } else { "Filter (Shift+T types)".into() },
                        value: 0.0,
                        min: 0.0,
                        max: 0.0,
                        step: 0.0,
                        choices: Vec::new(),
                        text: if typing { format!("{}_", view.filter) } else { view.filter.clone() },
                    });
                    let typing = view.typing && view.typing_commands;
                    knobs.push(Knob {
                        name: "mesh_command_filter".into(),
                        label: if typing { "Commands (Enter ends)".into() } else { "Commands (Ctrl+T types)".into() },
                        value: 0.0,
                        min: 0.0,
                        max: 0.0,
                        step: 0.0,
                        choices: Vec::new(),
                        text: if typing { format!("{}_", view.command_filter) } else { view.command_filter.clone() },
                    });
                    let (sessions, listed) = view.session_choices();
                    if !sessions.is_empty() {
                        knobs.push(Knob {
                            name: "mesh_session".into(),
                            label: "Session".into(),
                            value: listed as f64,
                            min: 0.0,
                            max: (sessions.len() - 1) as f64,
                            step: 1.0,
                            text: sessions[listed].clone(),
                            choices: sessions,
                        });
                    }
                    // Read-only: the note is typed with T, or set by mesh_selection.
                    knobs.push(Knob {
                        name: "mesh_note".into(),
                        label: if view.typing && !view.typing_filter { "Note (Enter ends)".into() } else { "Note (T types)".into() },
                        value: 0.0,
                        min: 0.0,
                        max: 0.0,
                        step: 0.0,
                        choices: Vec::new(),
                        text: if view.typing && !view.typing_filter {
                            format!("{}_", view.selection.note)
                        } else {
                            view.selection.note.clone()
                        },
                    });
                    knobs.push(Knob {
                        name: "mesh_pick".into(),
                        label: "Click selects".into(),
                        value: at as f64,
                        min: 0.0,
                        max: (mesh_view::PICK_MODES.len() - 1) as f64,
                        step: 1.0,
                        choices: mesh_view::PICK_MODES.iter().map(|mode| (*mode).to_string()).collect(),
                        text: view.pick_mode.clone(),
                    });
                }
                knobs
            }
        }
    }

    fn set_knob(&mut self, name: &str, value: f64) -> Result<(), String> {
        #[cfg(feature = "mesh")]
        if name == "mesh_note" {
            return Err("mesh_note is read-only: type it with T in the window, or set it with mesh_selection".into());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_replay_play" {
            let view = self.mesh.as_mut().ok_or("mesh_replay_play needs a mesh scene")?;
            if value >= 0.5 {
                if view.replay.is_none() {
                    view.scrub(0)?;
                }
                view.replay_playing = Some(0.7);
            } else {
                view.replay_playing = None;
            }
            self.follow_mesh();
            self.refresh_mesh_overlay();
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_play" {
            let view = self.mesh.as_mut().ok_or("mesh_play needs a mesh scene")?;
            let clip = view
                .pose
                .as_ref()
                .map(|(clip, _)| clip.clone())
                .or_else(|| view.workspace.as_ref().and_then(|w| w.clips.first().map(|c| c.name.clone())));
            view.play(if value >= 0.5 { clip } else { None })?;
            self.refresh_mesh_overlay();
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_clip" || name == "mesh_time" || name == "mesh_bones" || name == "mesh_surfaces" {
            let view = self.mesh.as_mut().ok_or("the animation knobs need a mesh scene")?;
            if name == "mesh_surfaces" {
                view.surfaces = value >= 0.5;
                view.display_stale = true;
            } else if name == "mesh_bones" {
                view.bones = value >= 0.5;
                view.overlay_stale = true;
            } else if name == "mesh_clip" {
                let clips: Vec<String> = view.workspace.iter().flat_map(|w| w.clips.iter().map(|c| c.name.clone())).collect();
                let pick = (value.round().max(0.0) as usize).checked_sub(1).and_then(|i| clips.get(i).cloned());
                view.set_pose(pick.map(|clip| (clip, 0.0)))?;
            } else if let Some((clip, _)) = view.pose.clone() {
                view.set_pose(Some((clip, value.max(0.0) as f32)))?;
            }
            self.refresh_mesh_overlay();
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_orthographic" {
            let view = self.mesh.as_mut().ok_or("mesh_orthographic needs a mesh scene")?;
            // The camera is not here: the pointer step applies it next frame.
            view.orthographic = value >= 0.5;
            view.projection_stale = true;
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_shadow" {
            let view = self.mesh.as_mut().ok_or("mesh_shadow needs a mesh scene")?;
            view.shadow = value >= 0.5;
            view.overlay_stale = true;
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_grid" {
            let view = self.mesh.as_mut().ok_or("mesh_grid needs a mesh scene")?;
            view.grid = value >= 0.5;
            view.overlay_stale = true;
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_turntable" {
            let view = self.mesh.as_mut().ok_or("mesh_turntable needs a mesh scene")?;
            view.turntable = value >= 0.5;
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_replay" {
            let view = self.mesh.as_mut().ok_or("mesh_replay needs a mesh scene")?;
            view.scrub(value.round().max(0.0) as u64)?;
            self.follow_mesh();
            self.refresh_mesh_overlay();
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_shade" {
            let view = self.mesh.as_mut().ok_or("mesh_shade needs a mesh scene")?;
            let at = (value.round().max(0.0) as usize).min(mesh_view::SHADES.len() - 1);
            view.shade = mesh_view::SHADES[at].to_string();
            view.display_stale = true;
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_statistics" || name == "mesh_state" || name == "mesh_command_filter" {
            return Err(format!("{name} is a readout"));
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_workspace" {
            let view = self.mesh.as_mut().ok_or("mesh_workspace needs a mesh scene")?;
            let dir = view.found.get(value.round().max(0.0) as usize).ok_or("no such workspace")?.clone();
            mesh_view::set_workspace(Some(dir));
            // Look now, not at the next poll.
            view.rebuild = true;
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_filter" {
            return Err("mesh_filter is read-only: type it with Shift and T in the window, or set it with mesh_view filter".into());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_session" {
            let view = self.mesh.as_mut().ok_or("mesh_session needs a mesh scene")?;
            return view.pick_session(value.round().max(0.0) as usize);
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_solo" {
            let view = self.mesh.as_mut().ok_or("mesh_solo needs a mesh scene")?;
            let objects: Vec<u64> = view.workspace.iter().flat_map(|w| w.parts.iter().map(|p| p.object)).collect();
            let at = value.round().max(0.0) as usize;
            view.hidden = match at.checked_sub(1).and_then(|i| objects.get(i)) {
                Some(keep) => objects.iter().copied().filter(|id| id != keep).collect(),
                None => std::collections::BTreeSet::new(),
            };
            view.display_stale = true;
            return Ok(());
        }
        #[cfg(feature = "mesh")]
        if name == "mesh_pick" {
            let view = self.mesh.as_mut().ok_or("mesh_pick needs a mesh scene")?;
            let at = (value.round().max(0.0) as usize).min(mesh_view::PICK_MODES.len() - 1);
            view.pick_mode = mesh_view::PICK_MODES[at].to_string();
            return Ok(());
        }
        match &mut self.show {
            Show::Stress(stage) => Host::set_knob(stage, name, value),
            Show::Still(world) => {
                if apply_particle_knob(&mut world.scene.particles, name, value)?
                    || apply_fog_knob(&mut world.scene.fog, name, value)?
                    || apply_lighting_knob(&mut world.scene.lighting, name, value)?
                {
                    Ok(())
                } else {
                    Err(format!("unknown knob {name}"))
                }
            }
        }
    }

    fn list_scenes(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    fn load_scene(&mut self, name: &str) -> Result<(), String> {
        let name = genos_scene_lib::canonical(name);
        let menu = self.scene().menu.clone();
        let fps = self.scene().fps_counter;
        let debug = self.scene().debug.clone();
        let event_loop = self.scene().event_loop;
        let index = self
            .entries
            .iter()
            .position(|e| e.name == name)
            .ok_or_else(|| format!("unknown scene {name}"))?;
        let show = build_entry(&self.root, &self.entries[index])?;
        self.index = index;
        self.checkpoint = 0;
        self.load = None;
        self.show = show;
        self.scene().menu = menu;
        self.scene().fps_counter = fps;
        self.scene().debug = debug;
        self.scene().event_loop = event_loop;
        adopt_show(&mut self.show, &self.entries[self.index]);
        self.overlay_fog();
        self.overlay_particles();
        self.note(&format!("loaded {name}"));
        Ok(())
    }

    fn begin_load(&mut self, name: &str) -> Result<LoadPace, String> {
        self.start_load(name)
    }

    fn save_hdr_to_scene(&mut self) -> Result<genos_debug::SavedLook, String> {
        let saved = self.save_look()?;
        self.note(&format!("saved the HDR look of {} to {}", saved.scene, saved.path));
        self.look_note = "Saved by command".into();
        Ok(saved)
    }

    fn poll_load(&mut self) -> Result<LoadPace, String> {
        self.take_load()
    }

    #[cfg(feature = "mesh")]
    fn scene_command(
        &mut self,
        name: &str,
        args: &genos_mcp::json::Value,
        camera: &mut Camera,
        aspect: f32,
    ) -> Option<Result<genos_mcp::json::Value, String>> {
        self.follow_mesh();
        let mut result = self.mesh.as_mut()?.command(name, args, camera, aspect);
        // A replay step or a return to the live mesh rebuilds the picture now,
        // and the reply reports the mesh that is then on show.
        let rebuilt = self.mesh.as_ref().is_some_and(|view| view.rebuild);
        self.follow_mesh();
        // Only the view's own report is asked for again: an edit or a preview
        // keeps the reply mesh-workbench gave.
        if rebuilt && result.is_ok() && name == "mesh_view" {
            let none = genos_mcp::json::Value::Object(Vec::new());
            result = self.mesh.as_mut()?.command(name, &none, camera, aspect);
        }
        self.refresh_mesh_overlay();
        Some(result)
    }

    #[cfg(feature = "mesh")]
    fn scene_command_poll(&mut self, ticket: u64) -> Option<Result<genos_mcp::json::Value, String>> {
        let Some(view) = &mut self.mesh else {
            return Some(Err("the mesh scene was closed before the command finished".into()));
        };
        let result = view.poll_edit(ticket)?;
        // Show what the command changed before the reply goes out.
        self.follow_mesh();
        self.refresh_mesh_overlay();
        Some(result)
    }

    fn list_checkpoints(&self) -> Vec<String> {
        self.entry()
            .checkpoints
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    fn jump_checkpoint(&mut self, name: &str) -> Result<CheckpointPose, String> {
        let index = self
            .entry()
            .checkpoints
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| format!("unknown checkpoint {name}"))?;
        self.checkpoint = index;
        let cp = &self.entry().checkpoints[index];
        Ok(CheckpointPose {
            position: cp.eye,
            yaw: cp.yaw,
            pitch: cp.pitch,
        })
    }
}

fn loading_world() -> World {
    World::from_scene({
        let mut __scene = Scene::from_lists(
            Floor {
                position: Vec3::new(0.0, 0.0, 0.0),
                half_x: 2.0,
                half_z: 2.0,
                color: [0.15, 0.15, 0.16],
            },
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        __scene.ceiling = None;
        __scene.sky = Some(Sky {
            color: [0.02, 0.02, 0.025],
        });
        __scene.lighting = genos_scene::LightingSettings::preset(genos_scene::LightingPreset::Fallback);
        __scene.debug = DebugSettings::default();
        __scene.fps_counter = FpsCounterSettings::default();
        __scene.event_loop = EventLoopSettings::default();
        __scene.reflections = ReflectionSettings::default();
        __scene.hdr = genos_scene::HdrSettings::legacy_bytes();
        __scene.fog = FogSettings::default();
        __scene.particles = ParticleSettings::default();
        __scene.menu = MenuSettings::default();
        __scene
    })
}

fn gltf_scene(entries: &[Entry], name: &str) -> bool {
    entries
        .iter()
        .any(|entry| entry.name == name && matches!(entry.kind, Kind::Gltf { .. }))
}

/// A glTF scene whose bytes are not in this checkout. `--check` skips it.
/// `--scene` still returns the error.
fn gltf_bytes_absent(err: &str) -> bool {
    err.contains("Git LFS pointer") || err.contains("file not found")
}

static COOKED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn cooked_dir() -> Option<PathBuf> {
    COOKED
        .get()
        .cloned()
        .or_else(|| std::env::var_os("GENOS_COOKED").map(PathBuf::from))
}

fn build_entry(root: &Path, entry: &Entry) -> Result<Show, String> {
    match &entry.kind {
        Kind::Stress(spec) => {
            let scale = Scale::parse(&spec.scale)?;
            let layout = Layout::parse(&spec.layout)
                .ok_or_else(|| format!("unknown layout {}", spec.layout))?;
            let mut stage = Stage::new(
                scale,
                1,
                LightMix {
                    count: spec.lights,
                    dynamic_pct: spec.dynamic,
                    layout,
                },
                1.0,
                spec.time,
                120.0,
            );
            stage.sun_frozen = spec.freeze_sun;
            stage.boxes_still = spec.still_boxes;
            stage.lamps_still = spec.still_lamps;
            stage.sky_on = spec.sky;
            stage.apply();
            Ok(Show::Stress(stage))
        }
        Kind::Rhai { path } => {
            let full = root.join(path);
            let scene = genos_scene::load_path(&full)?;
            Ok(Show::Still(World::from_scene(scene)))
        }
        Kind::Mesh {
            workspace,
            sun,
            sky,
            lamps,
        } => mesh_show(root, workspace, sun.as_ref(), *sky, lamps),
        Kind::Gltf {
            path,
            sun,
            sky,
            lamps,
        } => {
            let full = root.join(path);
            eprintln!("{}: loading {}", program().name, full.display());
            let model = genos_load::load_gltf_file(&full).map_err(|err| match &err {
                genos_load::LoadError::LfsPointer { .. }
                | genos_load::LoadError::NotFound { .. } => err.to_string(),
                _ => format!("{}: {err}", full.display()),
            })?;
            let mut scene = {
                let mut __scene = Scene::from_lists(
                    Floor {
                        position: Vec3::new(1000.0, 0.0, 0.0),
                        half_x: 0.01,
                        half_z: 0.01,
                        color: [0.2, 0.2, 0.2],
                    },
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                );
                __scene.ceiling = None;
                __scene.sky = sky.map(|color| Sky { color });
                __scene.lighting = genos_scene::LightingSettings::default();
                __scene.debug = DebugSettings::default();
                __scene.fps_counter = FpsCounterSettings::default();
                __scene.event_loop = EventLoopSettings::default();
                __scene.reflections = ReflectionSettings::default();
                __scene.hdr = genos_scene::HdrSettings::legacy_bytes();
                __scene.fog = FogSettings::default();
                __scene.particles = ParticleSettings::default();
                __scene.menu = MenuSettings::default();
                __scene
            };
            if let Some(sun) = sun {
                scene.push_light(sun_light(sun));
            }
            for lamp in lamps {
                scene.push_light(point_light(lamp));
            }
            let mut world = World::from_scene(scene);
            for object in &mut world.objects {
                if let DrawKind::Fixed(FixedPart::Floor) = object.kind {
                    object.hidden = true;
                    object.affects_light = false;
                }
            }
            let mut bake = genos_render::bake::BakeSettings::default();
            if std::env::var_os("GENOS_BAKE").is_none() {
                bake.mode = genos_render::bake::BakeMode::OnLoad;
            }
            if let Some(dir) = cooked_dir() {
                let report = world.add_cooked(&dir, &full, &identity_pose(), false, true);
                for line in &report.fallback {
                    eprintln!("{}: {line}", program().name);
                }
                eprintln!(
                    "{}: cooked load {:.2} ms, texture bytes {}", program().name,
                    report.load_ms, report.texture_bytes
                );
                if report.objects.is_empty() {
                    world
                        .add_gltf_level(&model, &identity_pose(), &bake)
                        .map_err(|err| format!("{}: bake: {err:?}", full.display()))?;
                }
            } else {
                world
                    .add_gltf_level(&model, &identity_pose(), &bake)
                    .map_err(|err| format!("{}: bake: {err:?}", full.display()))?;
            }
            // The CPU reference traces the model's triangles (`reference`, `compare`).
            // The cooked path draws the same model, so it keeps the same surface.
            world.keep_gltf_surface(&model, &identity_pose());
            Ok(Show::Still(world))
        }
    }
}

fn sun_light(sun: &Sun) -> Light {
    let d = unit(sun.direction);
    Light {
        position: Vec3::new(0.0, 7.0, 0.0),
        color: sun.color,
        direction: Vec3::new(d[0], d[1], d[2]),
    }
}

fn point_light(lamp: &Lamp) -> Light {
    Light {
        position: Vec3::new(lamp.position[0], lamp.position[1], lamp.position[2]),
        color: lamp.color,
        direction: Vec3::ZERO,
    }
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1.0e-8 {
        v.map(|c| c / len)
    } else {
        v
    }
}

/// `--eye=X,Y,Z` and `--look=X,Y,Z`. The spaced form is a separate argument.
fn placed_flag(arg: &str) -> Option<(&'static str, &str)> {
    if let Some(value) = arg.strip_prefix("--eye=") {
        Some(("--eye", value))
    } else if let Some(value) = arg.strip_prefix("--look=") {
        Some(("--look", value))
    } else {
        None
    }
}

fn parse_vec3(flag: &str, text: &str) -> Result<[f32; 3], String> {
    let parts: Vec<&str> = text.split(',').collect();
    if parts.len() != 3 {
        return Err(format!("{flag} is x,y,z"));
    }
    let mut out = [0.0; 3];
    for (i, part) in parts.iter().enumerate() {
        out[i] = part
            .trim()
            .parse()
            .map_err(|_| format!("{flag} has a bad number {part}"))?;
    }
    Ok(out)
}

/// `--eye` moves the camera. `--look` aims it, from that eye or from the
/// checkpoint. Yaw and pitch match `look_yaw_pitch` (yaw 0 looks down -Z).
fn place_camera(opts: &Options, camera: &mut Camera) -> Result<(), String> {
    let eye = match opts.eye {
        Some(eye) => eye,
        None => [camera.position.x, camera.position.y, camera.position.z],
    };
    if opts.eye.is_some() && opts.look.is_none() {
        camera.set_pose(Vec3::new(eye[0], eye[1], eye[2]), camera.yaw, camera.pitch);
    }
    if let Some(target) = opts.look {
        let (yaw, pitch) = genos_scene_lib::look_yaw_pitch(eye, target);
        camera.set_pose(Vec3::new(eye[0], eye[1], eye[2]), yaw, pitch);
    }
    Ok(())
}

fn jump_camera(app: &mut App, name: Option<&str>, camera: &mut Camera) -> Result<(), String> {
    let chosen = match name {
        Some(name) => name.to_string(),
        None => app
            .entry()
            .checkpoints
            .first()
            .map_or_default(|c| c.name.clone()),
    };
    if chosen.is_empty() {
        return Ok(());
    }
    let pose = app.jump_checkpoint(&chosen)?;
    camera.set_pose(
        Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
        pose.yaw,
        pose.pitch,
    );
    Ok(())
}

fn hotkeys(
    app: &mut App,
    input: &InputSystem,
    keys: &KeyBindings,
    camera: &mut Camera,
    tools: &mut genos_debug::Tools,
    renderer: &mut Renderer,
) -> Result<(), String> {
    let n = app.entries.len();
    if input.keyboard.pressed(InputCode::key_page_down) {
        switch_to(app, (app.index + 1) % n, None, camera, tools, renderer)?;
    } else if input.keyboard.pressed(InputCode::key_page_up) {
        let index = if app.index == 0 { n - 1 } else { app.index - 1 };
        switch_to(app, index, None, camera, tools, renderer)?;
    }
    let digits = [
        InputCode::key_1,
        InputCode::key_2,
        InputCode::key_3,
        InputCode::key_4,
        InputCode::key_5,
        InputCode::key_6,
        InputCode::key_7,
        InputCode::key_8,
        InputCode::key_9,
    ];
    for (i, code) in digits.iter().enumerate() {
        if input.keyboard.pressed(*code) {
            if let Some(name) = app.entry().checkpoints.get(i).map(|c| c.name.clone()) {
                let pose = app.jump_checkpoint(&name)?;
                camera.set_pose(
                    Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
                    pose.yaw,
                    pose.pitch,
                );
            }
        }
    }
    if keys.pressed(input, "fps") {
        let on = app.scene().fps_counter.effective().enabled;
        let _ = app
            .scene()
            .fps_counter
            .set_key("enabled", if on { "0" } else { "1" });
    }
    if (input.keyboard.pressed(InputCode::key_home) || input.keyboard.pressed(InputCode::key_0))
        && app.checkpoint < app.entry().checkpoints.len()
    {
        let name = app.entry().checkpoints[app.checkpoint].name.clone();
        let pose = app.jump_checkpoint(&name)?;
        camera.set_pose(
            Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
            pose.yaw,
            pose.pitch,
        );
    }
    Ok(())
}

fn switch_to(
    app: &mut App,
    index: usize,
    checkpoint: Option<&str>,
    camera: &mut Camera,
    tools: &mut genos_debug::Tools,
    renderer: &mut Renderer,
) -> Result<(), String> {
    let name = app.entries[index].name.clone();
    app.start_load(&name)?;
    tools.drop_scene_edits();
    renderer.reset_lighting_cache();
    jump_camera(app, checkpoint, camera)
}

fn fly(camera: &mut Camera, scene: &mut Scene, input: &InputSystem, actions: &Actions, dt: f32) {
    update(camera, scene, actions, 0.0);
    camera.yaw += actions.look_x * STICK_LOOK * dt;
    camera.pitch =
        (camera.pitch + actions.look_y * STICK_LOOK * dt).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    let forward = look_direction(camera.yaw, 0.0);
    let right = look_direction(camera.yaw + std::f32::consts::FRAC_PI_2, 0.0);
    let mut lift = 0.0;
    if input.keyboard.down(InputCode::key_space) {
        lift += 1.0;
    }
    if input.keyboard.down(InputCode::key_c)
        || input.keyboard.down(InputCode::key_control_left)
        || input.keyboard.down(InputCode::key_control_right)
    {
        lift -= 1.0;
    }
    let boost = if input.keyboard.down(InputCode::key_shift_left)
        || input.keyboard.down(InputCode::key_shift_right)
    {
        4.0
    } else {
        1.0
    };
    let step = (forward * actions.forward + right * actions.strafe + Vec3::Y * lift)
        * (FLY_SPEED * boost * dt);
    camera.set_pose(camera.position + step, camera.yaw, camera.pitch);
}

fn view_name(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Direct => "direct",
        ViewMode::Bounce => "bounce",
        ViewMode::Albedo => "albedo",
        ViewMode::Normal => "normal",
        ViewMode::Depth => "depth",
        ViewMode::Texels => "texels",
        ViewMode::Variance => "variance",
        ViewMode::Path => "path",
        ViewMode::Voxels => "voxels",
        ViewMode::Light => "light",
        ViewMode::Full => "full",
    }
}

fn panel_title(app: &App) -> String {
    app.entry().name.clone()
}

fn index_steps(names: impl IntoIterator<Item = impl Into<String>>) -> Vec<SliderStep> {
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| SliderStep {
            name: name.into(),
            value: index as f64,
        })
        .collect()
}

fn named_slider(id: u32, value: f64, names: Vec<String>) -> PanelSlider {
    let mut slider = PanelSlider::named(id, value, index_steps(names));
    // Tick names carry the step. The side readout would repeat that name.
    slider.text.clear();
    slider
}

fn drop_row(label: &str, id: u32, value: f64, names: Vec<String>) -> Option<PanelRow> {
    if names.is_empty() {
        return None;
    }
    Some(PanelRow {
        label: label.into(),
        buttons: Vec::new(),
        slider: None,
        dropdown: Some(PanelDropdown::new(id, value, index_steps(names))),
        checks: Vec::new(),
    })
}

fn switch_knobs(app: &App) -> Vec<Knob> {
    app.knobs()
        .into_iter()
        .filter(|knob| knob.choices == ["off", "on"])
        .collect()
}

fn knob_row(app: &App, index: usize, name: &str) -> Option<PanelRow> {
    let knob = app.knobs().into_iter().find(|knob| knob.name == name)?;
    if knob.choices == ["off", "on"] {
        return None;
    }
    let label = menu_label(name, &knob.label);
    // A knob with no range is a readout: its words, no slider.
    if knob.choices.is_empty() && knob.min >= knob.max {
        return Some(PanelRow {
            label: if knob.text.is_empty() { label } else { format!("{label}: {}", knob.text) },
            buttons: Vec::new(),
            slider: None,
            dropdown: None,
            checks: Vec::new(),
        });
    }
    let slider = if knob.choices.is_empty() {
        PanelSlider::range(
            ID_KNOB + index as u32,
            knob.value,
            knob.min,
            knob.max,
            knob.step,
            knob.text,
        )
    } else {
        named_slider(ID_KNOB + index as u32, knob.value, knob.choices)
    };
    Some(PanelRow {
        label,
        buttons: Vec::new(),
        slider: Some(slider),
        dropdown: None,
        checks: Vec::new(),
    })
}

#[cfg(test)]
fn panel_rows(app: &App, view: ViewMode, fps: &FpsCounterSettings) -> Vec<PanelRow> {
    let menu = load_menu_layout().expect("layout");
    flatten_blocks(&panel_blocks(app, view, fps, &menu))
}

#[cfg(test)]
fn flatten_blocks(blocks: &[PanelBlock]) -> Vec<PanelRow> {
    let mut rows = Vec::new();
    for block in blocks {
        match block {
            PanelBlock::Section(section) => {
                rows.extend(section.rows.clone());
                rows.extend(flatten_blocks(&section.blocks));
            }
            PanelBlock::Tabs(tabs) => {
                for tab in &tabs.tabs {
                    rows.extend(flatten_blocks(&tab.blocks));
                }
            }
            PanelBlock::Rows(page) => rows.extend(page.clone()),
        }
    }
    rows
}

enum Piece {
    Row(PanelRow),
    Check(PanelCheck),
}

struct SectionAcc {
    tab: Option<String>,
    key: String,
    title: String,
    collapsed: bool,
    rows: Vec<PanelRow>,
    checks: Vec<PanelCheck>,
}

fn flush_checks(section: &mut SectionAcc) {
    if section.checks.is_empty() {
        return;
    }
    section.rows.push(PanelRow {
        label: String::new(),
        buttons: Vec::new(),
        slider: None,
        dropdown: None,
        checks: std::mem::take(&mut section.checks),
    });
}

/// The mesh scene's two fill lights: the way each shines, and its strength
/// as a share of the sun's. mesh-workbench's web viewer has lights towards
/// (-3, 5, 4), (4, 2, 1) and (1, 3, -4) at 0.8, 0.27 and 0.38.
#[cfg(feature = "mesh")]
const MESH_FILL_LIGHTS: [([f32; 3], f32); 2] = [([-4.0, -2.0, -1.0], 0.27 / 0.8), ([-1.0, -3.0, 4.0], 0.38 / 0.8)];

/// The words on a mesh scene's picture: the workspace line and the counts,
/// and the hints.
#[cfg(feature = "mesh")]
const HUD_BRIGHT: [f32; 3] = [0.82, 0.85, 0.9];
#[cfg(feature = "mesh")]
const HUD_DIM: [f32; 3] = [0.5, 0.53, 0.58];

/// The height of a label's small text in pixels, and the plate behind it.
#[cfg(feature = "mesh")]
const LABEL_HEIGHT: f32 = 8.0;
#[cfg(feature = "mesh")]
const LABEL_PLATE: [f32; 3] = [0.05, 0.06, 0.08];

/// The exposure a mesh scene holds: what the auto exposure settles at on the
/// sample seen from the front. (From above it settles at 1.9.)
#[cfg(feature = "mesh")]
const MESH_EV100: &str = "1.2";

/// The menu tab that holds the mesh viewer's controls. It has no rows, and
/// so is not drawn, outside a mesh scene.
const MESH_TAB: &str = "Mesh";

/// The top bar's buttons, one for each page of the menu's first strip, and
/// the page in force: the one `ui` has selected, or the first. A button is
/// drawn held while its page is open.
fn bar_buttons(blocks: &[PanelBlock], ui: &State, menu_open: bool) -> (Vec<genos_ui::PanelButton>, Option<u32>) {
    let strip = strip_id(0);
    let pages: Vec<&genos_ui::PanelTab> = blocks
        .iter()
        .filter_map(|block| match block {
            PanelBlock::Tabs(tabs) if tabs.id == strip => Some(tabs.tabs.iter()),
            _ => None,
        })
        .flatten()
        .collect();
    let chosen = ui
        .selected_tab(strip)
        .filter(|id| pages.iter().any(|page| page.id == *id))
        .or_else(|| pages.first().map(|page| page.id));
    let buttons = pages
        .iter()
        .map(|page| genos_ui::PanelButton {
            id: page.id,
            text: page.title.clone(),
            selected: menu_open && chosen == Some(page.id),
        })
        .collect();
    (buttons, chosen)
}

/// The drawer for the page `chosen`: its title and its blocks, with no strip
/// of tabs. A block outside the strip stays, under the page.
fn bar_page(blocks: Vec<PanelBlock>, chosen: Option<u32>) -> (String, Vec<PanelBlock>) {
    let strip = strip_id(0);
    let mut title = String::new();
    let mut page = Vec::new();
    for block in blocks {
        match block {
            PanelBlock::Tabs(tabs) if tabs.id == strip => {
                if let Some(tab) = tabs.tabs.into_iter().find(|tab| Some(tab.id) == chosen) {
                    title = tab.title;
                    page.extend(tab.blocks);
                }
            }
            other => page.push(other),
        }
    }
    (title, page)
}

/// Whether the pointer belongs to a panel, not to the picture under it.
///
/// While the left button is held, the panel says so: `look_capture` is true
/// from a press that missed every panel until its release. With no button
/// held `look_capture` is false whatever the pointer is over, so the test is
/// where the pointer is: on something a panel paints. The mesh viewer took
/// `!look_capture` alone, which made the wheel and the right button "over the
/// UI" on every frame: the wheel did not zoom and a drag did not pan.
#[cfg(feature = "mesh")]
fn pointer_over_ui(left_down: bool, look_capture: bool, paints: &[genos_ui::Paint], at: [f32; 2]) -> bool {
    if left_down {
        return !look_capture;
    }
    paints.iter().any(|paint| at[0] >= paint.x && at[0] < paint.x + paint.w && at[1] >= paint.y && at[1] < paint.y + paint.h)
}

/// Where the mesh viewer's own panel first stands, down from the top: under the gizmo.
#[cfg(feature = "mesh")]
const MESH_PANEL_TOP: f32 = 176.0;

/// Take the mesh viewer's Objects and Replay sections out of the menu
/// layout: the program draws them as a second panel. Returns the menu
/// without them, and their lines with no tab.
fn split_mesh_panel(menu: Vec<MenuLine>) -> (Vec<MenuLine>, Vec<MenuLine>) {
    let (panel, menu): (Vec<MenuLine>, Vec<MenuLine>) =
        menu.into_iter().partition(|line| matches!(line.section.as_str(), "mesh_objects" | "mesh_replay"));
    let panel = panel.into_iter().map(|line| MenuLine { tab: None, ..line }).collect();
    (menu, panel)
}

/// The rows of the mesh viewer's Objects (`mesh_objects`) and Replay
/// (`mesh_events`) lists. `None` for any other setting.
#[cfg(not(feature = "mesh"))]
fn mesh_panel_rows(_app: &App, _setting: &str) -> Option<Vec<PanelRow>> {
    None
}

#[cfg(feature = "mesh")]
fn mesh_panel_rows(app: &App, setting: &str) -> Option<Vec<PanelRow>> {
    let (first, lines) = match setting {
        "mesh_objects" => (ID_MESH_OBJECT, app.mesh.as_ref().map(mesh_view::MeshView::object_lines)),
        "mesh_events" => (ID_MESH_EVENT, app.mesh.as_ref().map(mesh_view::MeshView::replay_lines)),
        _ => return None,
    };
    let rows = lines
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(at, line)| {
            let mut checks = vec![PanelCheck { id: first + at as u32, text: line.check.into(), on: line.on }];
            // A collection's row folds its objects away.
            checks.extend(line.open.map(|open| PanelCheck { id: ID_MESH_FOLD + at as u32, text: "open".into(), on: open }));
            PanelRow { label: line.label, buttons: Vec::new(), slider: None, dropdown: None, checks }
        })
        .collect();
    Some(rows)
}

fn panel_blocks(
    app: &App,
    view: ViewMode,
    fps: &FpsCounterSettings,
    lines: &[MenuLine],
) -> Vec<PanelBlock> {
    let mut sections = Vec::new();
    for line in lines {
        let same = sections.last().is_some_and(|section: &SectionAcc| {
            section.tab == line.tab && section.key == line.section
        });
        if !same {
            if let Some(section) = sections.last_mut() {
                flush_checks(section);
            }
            sections.push(SectionAcc {
                tab: line.tab.clone(),
                key: line.section.clone(),
                title: line.title.clone(),
                collapsed: line.collapsed,
                rows: Vec::new(),
                checks: Vec::new(),
            });
        }
        // One line of the layout is one setting, except the mesh viewer's
        // lists: a row for each object, a row for each recorded command.
        let pieces: Vec<Piece> = match mesh_panel_rows(app, &line.setting) {
            Some(rows) => rows.into_iter().map(Piece::Row).collect(),
            None => setting_piece(app, view, fps, &line.setting).into_iter().collect(),
        };
        let Some(section) = sections.last_mut() else {
            continue;
        };
        for piece in pieces {
            match piece {
                Piece::Row(row) => {
                    flush_checks(section);
                    section.rows.push(row);
                }
                Piece::Check(check) => section.checks.push(check),
            }
        }
    }
    // The mesh viewer's sections have nothing in another scene: no empty fold.
    sections.retain(|section| {
        let mesh = section.key.starts_with("mesh_") || section.tab.as_deref() == Some(MESH_TAB);
        !(mesh && section.rows.is_empty() && section.checks.is_empty())
    });
    if let Some(section) = sections.last_mut() {
        flush_checks(section);
    }
    let mut blocks = Vec::new();
    let mut strip: Option<PanelTabs> = None;
    let mut strip_index = 0usize;
    for section in sections {
        let block = if section.key == "-" {
            PanelBlock::Rows(section.rows)
        } else {
            PanelBlock::Section(PanelSection::new(
                section_id(&section.key),
                section.title,
                section.collapsed,
                section.rows,
            ))
        };
        match section.tab {
            None => {
                if let Some(tabs) = strip.take() {
                    blocks.push(PanelBlock::Tabs(tabs));
                }
                blocks.push(block);
            }
            Some(name) => {
                if strip.is_none() {
                    strip = Some(PanelTabs {
                        id: strip_id(strip_index),
                        tabs: Vec::new(),
                    });
                    strip_index += 1;
                }
                let Some(tabs) = strip.as_mut() else {
                    continue;
                };
                if let Some(tab) = tabs.tabs.iter_mut().find(|tab| tab.title == name) {
                    tab.blocks.push(block);
                } else {
                    tabs.tabs.push(PanelTab {
                        id: tab_id(&name),
                        title: name,
                        blocks: vec![block],
                    });
                }
            }
        }
    }
    if let Some(tabs) = strip {
        blocks.push(PanelBlock::Tabs(tabs));
    }
    blocks
}

fn setting_piece(
    app: &App,
    view: ViewMode,
    fps: &FpsCounterSettings,
    setting: &str,
) -> Option<Piece> {
    let view_value = VIEWS
        .iter()
        .position(|name| *name == view_name(view))
        .unwrap_or(0) as f64;
    match setting {
        "review" => Some(Piece::Row(PanelRow {
            label: app.entry().review.clone(),
            buttons: Vec::new(),
            slider: None,
            dropdown: None,
            checks: Vec::new(),
        })),
        "scene" => drop_row(
            "Scene",
            ID_SCENE,
            app.index as f64,
            app.entries.iter().map(|entry| entry.name.clone()).collect(),
        )
        .map(Piece::Row),
        "checkpoint" => drop_row(
            "Checkpoint",
            ID_CP,
            app.checkpoint as f64,
            app.entry()
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.name.clone())
                .collect(),
        )
        .map(Piece::Row),
        "view" => drop_row(
            "View",
            ID_VIEW,
            view_value,
            VIEWS.iter().map(|name| (*name).to_string()).collect(),
        )
        .map(Piece::Row),
        "fps" => Some(Piece::Row(fps_row(fps))),
        "enabled" => Some(Piece::Row(reflection_check(
            "enabled",
            ID_REFL_ENABLED,
            reflections(app).enabled,
        ))),
        "water" => Some(Piece::Row(reflection_check(
            "water",
            ID_REFL_WATER,
            reflections(app).water,
        ))),
        "rough_cutoff" => Some(Piece::Row(reflection_slider(
            "rough_cutoff",
            ID_REFL_ROUGH,
            f64::from(reflections(app).rough_cutoff),
            0.0,
            1.0,
            0.01,
        ))),
        "mirror_cutoff" => Some(Piece::Row(reflection_slider(
            "mirror_cutoff",
            ID_REFL_MIRROR,
            f64::from(reflections(app).mirror_cutoff),
            0.0,
            1.0,
            0.001,
        ))),
        "mirror_views" => Some(Piece::Row(reflection_slider(
            "mirror_views",
            ID_REFL_VIEWS,
            f64::from(reflections(app).mirror_views),
            0.0,
            3.0,
            1.0,
        ))),
        "rays" => Some(Piece::Row(reflection_slider(
            "rays",
            ID_REFL_RAYS,
            f64::from(reflections(app).rays),
            0.0,
            128_000.0,
            1000.0,
        ))),
        "shore_cap" => Some(Piece::Row(reflection_slider(
            "shore_cap",
            ID_REFL_SHORE,
            f64::from(reflections(app).shore_cap),
            0.0,
            1.0,
            0.01,
        ))),
        "caustics" => drop_row(
            "caustics",
            ID_REFL_CAUSTICS,
            match reflections(app).caustics {
                genos_scene::Caustics::Off => 0.0,
                genos_scene::Caustics::Image => 1.0,
                genos_scene::Caustics::Cache => 2.0,
            },
            ["off", "image", "cache"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        )
        .map(Piece::Row),
        "tracer" => drop_row(
            "tracer",
            ID_REFL_TRACER,
            match reflections(app).tracer {
                genos_scene::ReflectionTracer::Off => 0.0,
                genos_scene::ReflectionTracer::Software => 1.0,
                genos_scene::ReflectionTracer::RayQuery => 2.0,
            },
            ["off", "software", "ray_query"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        )
        .map(Piece::Row),
        "stall" => Some(Piece::Row(PanelRow {
            label: format!(
                "Stall {:.0} ms",
                app.scene_ref().event_loop.longest_stall_ms
            ),
            buttons: Vec::new(),
            slider: None,
            dropdown: None,
            checks: Vec::new(),
        })),
        "gi_passes" => Some(Piece::Row(PanelRow {
            label: app.gi_passes.clone(),
            buttons: Vec::new(),
            slider: None,
            dropdown: None,
            checks: Vec::new(),
        })),
        "gi_work" => Some(Piece::Row(PanelRow {
            label: app.gi_work.clone(),
            buttons: Vec::new(),
            slider: None,
            dropdown: None,
            checks: Vec::new(),
        })),
        "menu_wheel" => {
            focus_row("Wheel focus", ID_MENU_WHEEL, menu_settings(app).wheel).map(Piece::Row)
        }
        "menu_keys" => {
            focus_row("Key focus", ID_MENU_KEYS, menu_settings(app).keys).map(Piece::Row)
        }
        "hdr_exposure_mode" => {
            let hdr = hdr_of(app);
            drop_row(
                "Exposure",
                ID_HDR_EXPOSURE,
                exposure_index(hdr.exposure_mode),
                ["auto", "camera", "fixed"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            )
            .map(Piece::Row)
        }
        "hdr_ev100" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "EV100", ID_HDR_EV, hdr.ev100, -8.0, 16.0, 0.1,
            )))
        }
        "hdr_compensation" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Compensation",
                ID_HDR_COMP,
                hdr.compensation,
                -8.0,
                8.0,
                0.1,
            )))
        }
        "hdr_iso" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "ISO", ID_HDR_ISO, hdr.iso, 50.0, 6400.0, 50.0,
            )))
        }
        "hdr_aperture" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Aperture",
                ID_HDR_APERTURE,
                hdr.aperture,
                0.7,
                22.0,
                0.1,
            )))
        }
        "hdr_shutter" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Shutter",
                ID_HDR_SHUTTER,
                hdr.shutter,
                0.000_125,
                1.0,
                0.001,
            )))
        }
        "hdr_display" => {
            let hdr = hdr_of(app);
            drop_row(
                "Display transform",
                ID_HDR_DISPLAY,
                display_index(hdr.display),
                ["agx", "aces", "legacy"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            )
            .map(Piece::Row)
        }
        "hdr_bloom" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(PanelRow {
                label: "Bloom intensity".into(),
                buttons: Vec::new(),
                slider: Some(PanelSlider::range(
                    ID_HDR_BLOOM_I,
                    hdr.bloom_intensity,
                    0.0,
                    1.0,
                    0.01,
                    format!("{:.2}", hdr.bloom_intensity),
                )),
                dropdown: None,
                checks: vec![PanelCheck {
                    id: ID_HDR_BLOOM,
                    text: "Bloom".into(),
                    on: hdr.bloom,
                }],
            }))
        }
        "hdr_bloom_levels" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Bloom levels",
                ID_HDR_LEVELS,
                f64::from(hdr.bloom_levels),
                0.0,
                6.0,
                1.0,
            )))
        }
        "hdr_adapt_down" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Adapt down (s)",
                ID_HDR_ADAPT_DOWN,
                hdr.adapt_down,
                0.05,
                5.0,
                0.05,
            )))
        }
        "hdr_adapt_up" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Adapt up (s)",
                ID_HDR_ADAPT_UP,
                hdr.adapt_up,
                0.05,
                5.0,
                0.05,
            )))
        }
        "hdr_adapt_strength" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Adapt strength",
                ID_HDR_ADAPT_STRENGTH,
                hdr.adapt_strength,
                0.0,
                1.0,
                0.05,
            )))
        }
        "hdr_adapt_anchor_seconds" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Adapt anchor seconds",
                ID_HDR_ADAPT_ANCHOR,
                hdr.adapt_anchor_seconds,
                0.0,
                60.0,
                1.0,
            )))
        }
        "hdr_adapt_knee_stops" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Adapt knee stops",
                ID_HDR_ADAPT_KNEE,
                hdr.adapt_knee_stops,
                0.0,
                10.0,
                0.25,
            )))
        }
        "hdr_highlight_percentile" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Highlight percentile",
                ID_HDR_HIGHLIGHT_PERCENTILE,
                hdr.highlight_percentile,
                0.5,
                1.0,
                0.01,
            )))
        }
        "hdr_highlight_stops" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Highlight stops",
                ID_HDR_HIGHLIGHT_STOPS,
                hdr.highlight_stops,
                0.5,
                10.0,
                0.25,
            )))
        }
        "hdr_transition" => {
            let hdr = hdr_of(app);
            Some(Piece::Row(number_row(
                "Look transition (s)",
                ID_HDR_TRANSITION,
                hdr.transition,
                0.0,
                5.0,
                0.05,
            )))
        }
        "hdr_save" => Some(Piece::Row(PanelRow {
            label: if app.look_note.is_empty() {
                "Scene look".into()
            } else {
                app.look_note.clone()
            },
            buttons: vec![genos_ui::PanelButton {
                id: ID_HDR_SAVE,
                text: SAVE_LOOK_BUTTON.into(),
                selected: false,
            }],
            slider: None,
            dropdown: None,
            checks: Vec::new(),
        })),
        "hdr_output" => {
            let hdr = hdr_of(app);
            drop_row(
                "SDR output",
                ID_HDR_OUTPUT,
                output_index(hdr.output),
                ["auto", "sdr", "hdr10", "scrgb"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            )
            .map(Piece::Row)
        }
        "fog_dispatches" | "fog_samples" | "fog_vram" | "fog_ms" | "fog_history_accepts" => {
            Some(Piece::Row(fog_counter_row(app, setting)))
        }
        name => {
            let knob = app.knobs().into_iter().find(|knob| knob.name == name)?;
            if knob.choices == ["off", "on"] {
                let index = switch_knobs(app)
                    .iter()
                    .position(|item| item.name == name)?;
                Some(Piece::Check(PanelCheck {
                    id: ID_SWITCH + index as u32,
                    text: menu_label(name, &knob.label),
                    on: knob.value >= 0.5,
                }))
            } else if let Some(index) = SLIDER_KNOBS.iter().position(|item| *item == name) {
                knob_row(app, index, name).map(Piece::Row)
            } else if let Some(index) = DROP_KNOBS.iter().position(|item| *item == name) {
                drop_knob_row(app, index, name).map(Piece::Row)
            } else {
                None
            }
        }
    }
}

fn menu_fold_path(out: &Path) -> PathBuf {
    out.join("menu").join("collapsed.txt")
}

fn menu_tab_path(out: &Path) -> PathBuf {
    out.join("menu").join("tab.txt")
}

fn tab_title(lines: &[MenuLine], id: u32) -> Option<&str> {
    lines.iter().find_map(|line| {
        let name = line.tab.as_deref()?;
        (tab_id(name) == id).then_some(name)
    })
}

fn load_menu_folds(out: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(menu_fold_path(out)).ok()?;
    Some(
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_string)
            .collect(),
    )
}

fn save_menu_folds(out: &Path, ui: &State, folds: &[MenuFold]) -> Result<(), String> {
    let names = folds
        .iter()
        .filter(|fold| ui.section_collapsed(fold.id))
        .map(|fold| fold.key.clone())
        .collect::<Vec<_>>();
    write_fold_names(out, &names)
}

fn write_fold_names(out: &Path, names: &[String]) -> Result<(), String> {
    let path = menu_fold_path(out);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let mut body = String::from("# collapsed menu sections\n");
    for name in names {
        body.push_str(name);
        body.push('\n');
    }
    std::fs::write(path, body).map_err(|err| err.to_string())
}

fn apply_collapsed(ui: &mut State, folds: &[MenuFold], names: &[String]) {
    for fold in folds {
        ui.fold_section(fold.id, names.iter().any(|name| name == &fold.key));
    }
}

/// A fold stays when this frame did not lay that section out. The open page
/// can change without dropping a fold that lives on another page.
fn merge_collapsed(previous: &[String], ui: &State, folds: &[MenuFold]) -> Vec<String> {
    let mut names = previous.to_vec();
    for fold in folds {
        if !ui.section_shown(fold.id) {
            continue;
        }
        if ui.section_collapsed(fold.id) {
            if !names.iter().any(|name| name == &fold.key) {
                names.push(fold.key.clone());
            }
        } else {
            names.retain(|name| name != &fold.key);
        }
    }
    names
}

fn pull_menu(
    app: &mut App,
    ui: &mut State,
    folds: &[MenuFold],
    menu_open: &mut bool,
    tracked_collapsed: &mut Vec<String>,
    tracked_tab: &mut String,
) {
    let menu = app.scene().menu.clone();
    if menu.collapsed != *tracked_collapsed {
        apply_collapsed(ui, folds, &menu.collapsed);
        *tracked_collapsed = menu.collapsed;
    }
    if menu.tab != *tracked_tab {
        if !menu.tab.is_empty() {
            ui.select_tab(strip_id(0), tab_id(&menu.tab));
        }
        *tracked_tab = menu.tab;
    }
    *menu_open = app.scene().menu.effective().visible;
    if !*menu_open {
        ui.set_focused_panel(None);
    }
    let menu = app.scene().menu.clone();
    ui.set_input_policy(
        PANEL_ID,
        InputPolicy {
            wheel: menu.wheel,
            keys: menu.keys,
        },
    );
}

struct MenuWrite<'a> {
    out: &'a Path,
    collapsed: &'a mut Vec<String>,
    tab: &'a mut String,
}

fn push_menu(
    app: &mut App,
    ui: &State,
    folds: &[MenuFold],
    lines: &[MenuLine],
    menu_open: bool,
    drew_panel: bool,
    write: &mut MenuWrite<'_>,
) -> Result<(), String> {
    if drew_panel {
        let merged = merge_collapsed(write.collapsed, ui, folds);
        let on_disk = load_menu_folds(write.out).unwrap_or_default();
        if merged != app.scene().menu.collapsed || merged != on_disk {
            app.scene().menu.collapsed.clone_from(&merged);
            write_fold_names(write.out, &merged)?;
        }
        write.collapsed.clone_from(&app.scene().menu.collapsed);
        if let Some(id) = ui.selected_tab(strip_id(0)) {
            if let Some(name) = tab_title(lines, id) {
                if app.scene().menu.tab != name {
                    app.scene().menu.tab = name.to_string();
                    save_menu_tab(write.out, name)?;
                }
                write.tab.clone_from(&app.scene().menu.tab);
            }
        }
    }
    if app.scene().menu.visible != menu_open {
        app.scene().menu.visible = menu_open;
    }
    Ok(())
}

fn load_menu_tab(out: &Path) -> Option<String> {
    let text = std::fs::read_to_string(menu_tab_path(out)).ok()?;
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
}

fn save_menu_tab(out: &Path, name: &str) -> Result<(), String> {
    let path = menu_tab_path(out);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, format!("# selected menu tab\n{name}\n")).map_err(|err| err.to_string())
}

fn drop_knob_row(app: &App, index: usize, name: &str) -> Option<PanelRow> {
    let knob = app.knobs().into_iter().find(|knob| knob.name == name)?;
    if knob.choices.is_empty() || knob.choices == ["off", "on"] {
        return None;
    }
    drop_row(
        &menu_label(name, &knob.label),
        ID_DROP + index as u32,
        knob.value,
        knob.choices,
    )
}

/// Panel text from `docs/lighting/GLOSSARY.md`. The knob name stays the config key.
fn fog_counter_row(app: &App, setting: &str) -> PanelRow {
    let scene = app.scene_ref();
    let probe = genos_render::fog_probe(&scene.fog, scene.lighting.preset);
    let label = match setting {
        "fog_dispatches" => format!("fog_dispatches {}", probe.dispatches),
        "fog_samples" => format!(
            "fog_samples {} grid {}x{}x{}",
            probe.samples_used,
            probe.dispatched_width,
            probe.dispatched_height,
            probe.dispatched_slices
        ),
        "fog_vram" => format!("fog_vram {}", probe.vram_bytes),
        "fog_ms" => format!("fog_ms {}", probe.pass_ms),
        "fog_history_accepts" => format!(
            "fog_history_accepts {} rejects {}",
            probe.history_accepts, probe.history_rejects
        ),
        _ => setting.to_string(),
    };
    PanelRow {
        label,
        buttons: Vec::new(),
        slider: None,
        dropdown: None,
        checks: Vec::new(),
    }
}

fn menu_label(name: &str, fallback: &str) -> String {
    match name {
        "lighting_preset" => "Preset",
        "lighting_bounces" => "Bounce count",
        "lighting_direct" => "Direct light",
        "lighting_direct_specular" => "Direct specular",
        "lighting_direct_refresh" => "Direct refresh frames",
        "lighting_gi_refresh" => "GI refresh frames",
        "lighting_budget_shift" => "Ray budget shift",
        "lighting_lamp_pick" => "Lamp pick",
        "lighting_lamp_pick_samples" => "Samples",
        "lighting_lamp_pick_candidates" => "Candidates",
        "lighting_probe_rays" => "Screen probe rays", // removed term, kept for the old key
        "lighting_probe_stale" => "Probe stale frames",
        "lighting_probes" => "Screen probes",
        "lighting_world_cache" => "Surface light cache",
        "lighting_world_stale" => "Cache stale frames",
        "lighting_denoise" => "Denoiser",
        "lighting_denoise_iters" => "Denoiser iterations",
        "lighting_denoise_variance" => "Denoiser variance",
        "lighting_denoise_launch" => "Denoiser launch",
        "lighting_denoise_adaptive" => "Adaptive denoise",
        "lighting_sky" => "Sky light",
        "lighting_prefill" => "Pre-fill",
        "lighting_ambient" => "Ambient fill",
        "lighting_shadow_filter" => "Shadow cell cache",
        "lighting_gi" => "GI",
        "lighting_emissive" => "Emissive",
        "lighting_cone_cascades" => "Cone-traced cascades",
        "lighting_compose_reuse" => "Compose reuse",
        "lighting_full_effort" => "Full effort",
        _ => fallback,
    }
    .to_string()
}

fn reflections(app: &App) -> genos_scene::ReflectionSettings {
    app.scene_ref().reflections.effective()
}

fn reflection_check(label: &str, id: u32, on: bool) -> PanelRow {
    PanelRow {
        label: label.into(),
        buttons: Vec::new(),
        slider: None,
        dropdown: None,
        checks: vec![PanelCheck {
            id,
            text: label.into(),
            on,
        }],
    }
}

fn reflection_slider(label: &str, id: u32, value: f64, min: f64, max: f64, step: f64) -> PanelRow {
    PanelRow {
        label: label.into(),
        buttons: Vec::new(),
        slider: Some(PanelSlider::range(
            id,
            value,
            min,
            max,
            step,
            format!("{value}"),
        )),
        dropdown: None,
        checks: Vec::new(),
    }
}

fn fps_row(fps: &FpsCounterSettings) -> PanelRow {
    PanelRow {
        label: "FPS".into(),
        buttons: Vec::new(),
        slider: Some(PanelSlider::range(
            ID_FPS_HZ,
            fps.rate_hz as f64,
            1.0,
            30.0,
            1.0,
            format!("{:.0} Hz", fps.rate_hz),
        )),
        dropdown: None,
        checks: vec![PanelCheck {
            id: ID_FPS,
            text: "Show".into(),
            on: fps.enabled,
        }],
    }
}

/// The words on the Output page's save button.
const SAVE_LOOK_BUTTON: &str = "Save look to scene";

fn hdr_of(app: &App) -> genos_scene::HdrSettings {
    let stored = app.scene_ref().hdr.clone();
    match stored.effective() {
        Ok(value) => value,
        Err(_) => stored,
    }
}

fn number_row(label: &str, id: u32, value: f64, min: f64, max: f64, step: f64) -> PanelRow {
    PanelRow {
        label: label.into(),
        buttons: Vec::new(),
        slider: Some(PanelSlider::range(
            id,
            value,
            min,
            max,
            step,
            format!("{value:.3}"),
        )),
        dropdown: None,
        checks: Vec::new(),
    }
}

fn exposure_index(mode: genos_scene::ExposureMode) -> f64 {
    match mode {
        genos_scene::ExposureMode::Auto => 0.0,
        genos_scene::ExposureMode::Camera => 1.0,
        genos_scene::ExposureMode::Fixed => 2.0,
    }
}

fn display_index(mode: genos_scene::DisplayTransform) -> f64 {
    match mode {
        genos_scene::DisplayTransform::Agx => 0.0,
        genos_scene::DisplayTransform::Aces => 1.0,
        genos_scene::DisplayTransform::Legacy => 2.0,
    }
}

fn output_index(mode: genos_scene::OutputMode) -> f64 {
    match mode {
        genos_scene::OutputMode::Auto => 0.0,
        genos_scene::OutputMode::Sdr => 1.0,
        genos_scene::OutputMode::Hdr10 => 2.0,
        genos_scene::OutputMode::Scrgb => 3.0,
    }
}

fn slide(app: &mut App, id: u32, value: f64) -> Result<(), String> {
    if id == ID_FPS_HZ {
        app.scene()
            .fps_counter
            .set_key("rate_hz", &format!("{value}"))?;
    } else if let Some(key) = reflection_slider_key(id) {
        app.scene().reflections.set_key(key, &format!("{value}"))?;
    } else if id == ID_HDR_EV {
        app.scene().hdr.set_key("ev100", &format!("{value}"))?;
    } else if id == ID_HDR_COMP {
        app.scene()
            .hdr
            .set_key("compensation", &format!("{value}"))?;
    } else if id == ID_HDR_ISO {
        app.scene().hdr.set_key("iso", &format!("{value}"))?;
    } else if id == ID_HDR_APERTURE {
        app.scene().hdr.set_key("aperture", &format!("{value}"))?;
    } else if id == ID_HDR_SHUTTER {
        app.scene().hdr.set_key("shutter", &format!("{value}"))?;
    } else if id == ID_HDR_BLOOM_I {
        app.scene()
            .hdr
            .set_key("bloom_intensity", &format!("{value}"))?;
    } else if id == ID_HDR_ADAPT_DOWN {
        app.scene().hdr.set_key("adapt_down", &format!("{value}"))?;
    } else if id == ID_HDR_ADAPT_UP {
        app.scene().hdr.set_key("adapt_up", &format!("{value}"))?;
    } else if id == ID_HDR_ADAPT_STRENGTH {
        app.scene()
            .hdr
            .set_key("adapt_strength", &format!("{}", value.clamp(0.0, 1.0)))?;
    } else if id == ID_HDR_ADAPT_KNEE {
        app.scene()
            .hdr
            .set_key("adapt_knee_stops", &format!("{}", value.clamp(0.0, 10.0)))?;
    } else if id == ID_HDR_ADAPT_ANCHOR {
        app.scene()
            .hdr
            .set_key("adapt_anchor_seconds", &format!("{}", value.clamp(0.0, 60.0)))?;
    } else if id == ID_HDR_HIGHLIGHT_PERCENTILE {
        app.scene()
            .hdr
            .set_key("highlight_percentile", &format!("{}", value.clamp(0.5, 1.0)))?;
    } else if id == ID_HDR_HIGHLIGHT_STOPS {
        app.scene()
            .hdr
            .set_key("highlight_stops", &format!("{}", value.clamp(0.5, 10.0)))?;
    } else if id == ID_HDR_TRANSITION {
        app.scene()
            .hdr
            .set_key("transition", &format!("{}", value.max(0.0)))?;
    } else if id == ID_HDR_LEVELS {
        let levels = value.round().clamp(0.0, 6.0);
        app.scene()
            .hdr
            .set_key("bloom_levels", &format!("{levels:.0}"))?;
    } else if (ID_KNOB..ID_KNOB + SLIDER_KNOBS.len() as u32).contains(&id) {
        let name = SLIDER_KNOBS[(id - ID_KNOB) as usize];
        app.set_knob(name, value)?;
    }
    Ok(())
}

fn select(
    app: &mut App,
    camera: &mut Camera,
    tools: &mut genos_debug::Tools,
    renderer: &mut Renderer,
    id: u32,
    value: f64,
) -> Result<(), String> {
    let index = value.round().max(0.0) as usize;
    if id == ID_SCENE {
        if index != app.index && index < app.entries.len() {
            switch_to(app, index, None, camera, tools, renderer)?;
        }
    } else if id == ID_CP {
        if index != app.checkpoint {
            if let Some(name) = app.entry().checkpoints.get(index).map(|c| c.name.clone()) {
                let pose = app.jump_checkpoint(&name)?;
                camera.set_pose(
                    Vec3::new(pose.position[0], pose.position[1], pose.position[2]),
                    pose.yaw,
                    pose.pitch,
                );
            }
        }
    } else if id == ID_VIEW {
        if let Some(name) = VIEWS.get(index) {
            let mode = view_from_name(name);
            if tools.view_mode() != mode {
                tools.set_view(mode);
            }
        }
    } else if id == ID_MENU_WHEEL || id == ID_MENU_KEYS {
        let name = ["hover", "modal", "click", "passive"]
            .get(index)
            .copied()
            .unwrap_or("hover");
        let key = if id == ID_MENU_WHEEL { "wheel" } else { "keys" };
        app.scene().menu.set_key(key, name)?;
    } else if id == ID_REFL_CAUSTICS || id == ID_REFL_TRACER {
        let names: &[&str] = if id == ID_REFL_CAUSTICS {
            &["off", "image", "cache"]
        } else {
            &["off", "software", "ray_query"]
        };
        let key = if id == ID_REFL_CAUSTICS {
            "caustics"
        } else {
            "tracer"
        };
        let name = names.get(index).copied().unwrap_or(names[0]);
        app.scene().reflections.set_key(key, name)?;
    } else if id == ID_HDR_EXPOSURE {
        let mode = ["auto", "camera", "fixed"]
            .get(index)
            .copied()
            .unwrap_or("auto");
        app.scene().hdr.set_key("exposure_mode", mode)?;
    } else if id == ID_HDR_DISPLAY {
        let mode = ["agx", "aces", "legacy"]
            .get(index)
            .copied()
            .unwrap_or("legacy");
        app.scene().hdr.set_key("display", mode)?;
    } else if id == ID_HDR_OUTPUT {
        let mode = ["auto", "sdr", "hdr10", "scrgb"]
            .get(index)
            .copied()
            .unwrap_or("sdr");
        app.scene().hdr.set_key("output", mode)?;
    } else if (ID_DROP..ID_DROP + DROP_KNOBS.len() as u32).contains(&id) {
        let name = DROP_KNOBS[(id - ID_DROP) as usize];
        app.set_knob(name, value)?;
    }
    Ok(())
}

fn menu_settings(app: &App) -> MenuSettings {
    match &app.show {
        Show::Stress(stage) => stage.world.scene.menu.clone(),
        Show::Still(world) => world.scene.menu.clone(),
    }
}

fn focus_index(policy: FocusPolicy) -> f64 {
    match policy {
        FocusPolicy::Hover => 0.0,
        FocusPolicy::Modal => 1.0,
        FocusPolicy::ClickToFocus => 2.0,
        FocusPolicy::Passive => 3.0,
    }
}

fn focus_row(label: &str, id: u32, policy: FocusPolicy) -> Option<PanelRow> {
    drop_row(
        label,
        id,
        focus_index(policy),
        ["hover", "modal", "click", "passive"]
            .into_iter()
            .map(str::to_string)
            .collect(),
    )
}

fn toggle(app: &mut App, id: u32, on: bool) -> Result<(), String> {
    if id == ID_FPS {
        app.scene()
            .fps_counter
            .set_key("enabled", if on { "1" } else { "0" })?;
        return Ok(());
    }
    if id == ID_REFL_ENABLED || id == ID_REFL_WATER {
        let key = if id == ID_REFL_ENABLED {
            "enabled"
        } else {
            "water"
        };
        app.scene()
            .reflections
            .set_key(key, if on { "on" } else { "off" })?;
        return Ok(());
    }
    if id == ID_HDR_BLOOM {
        app.scene()
            .hdr
            .set_key("bloom", if on { "on" } else { "off" })?;
        return Ok(());
    }
    #[cfg(feature = "mesh")]
    if (ID_MESH_FOLD..ID_MESH_FOLD + 500).contains(&id) {
        #[cfg(feature = "mesh")]
        if let Some(view) = &mut app.mesh {
            if let Some(line) = view.object_lines().into_iter().nth((id - ID_MESH_FOLD) as usize) {
                view.panel_fold(line.target, on);
            }
        }
        return Ok(());
    }
    #[cfg(feature = "mesh")]
    if (ID_MESH_OBJECT..ID_MESH_EVENT + 500).contains(&id) {
        let Some(view) = &mut app.mesh else {
            return Ok(());
        };
        let line = if id < ID_MESH_EVENT {
            view.object_lines().into_iter().nth((id - ID_MESH_OBJECT) as usize)
        } else {
            view.replay_lines().into_iter().nth((id - ID_MESH_EVENT) as usize)
        };
        if let Some(line) = line {
            view.panel_click(line.target, on)?;
        }
        return Ok(());
    }
    let index = id.wrapping_sub(ID_SWITCH) as usize;
    if let Some(name) = switch_knobs(app).get(index).map(|knob| knob.name.clone()) {
        app.set_knob(&name, if on { 1.0 } else { 0.0 })?;
    }
    Ok(())
}

fn reflection_slider_key(id: u32) -> Option<&'static str> {
    match id {
        ID_REFL_ROUGH => Some("rough_cutoff"),
        ID_REFL_MIRROR => Some("mirror_cutoff"),
        ID_REFL_VIEWS => Some("mirror_views"),
        ID_REFL_RAYS => Some("rays"),
        ID_REFL_SHORE => Some("shore_cap"),
        _ => None,
    }
}

fn view_from_name(name: &str) -> ViewMode {
    match name {
        "direct" => ViewMode::Direct,
        "bounce" => ViewMode::Bounce,
        "albedo" => ViewMode::Albedo,
        "normal" => ViewMode::Normal,
        "depth" => ViewMode::Depth,
        "texels" => ViewMode::Texels,
        "variance" => ViewMode::Variance,
        "path" => ViewMode::Path,
        "voxels" => ViewMode::Voxels,
        _ => ViewMode::Full,
    }
}

fn picture_stats(bgra: &[u8]) -> (f32, f32) {
    let mut sum = 0.0f64;
    let mut sum2 = 0.0f64;
    let mut n = 0.0f64;
    for px in bgra.chunks_exact(4).step_by(8) {
        let l = (px[0] as f64 + px[1] as f64 + px[2] as f64) / 3.0;
        sum += l;
        sum2 += l * l;
        n += 1.0;
    }
    if n == 0.0 {
        return (0.0, 0.0);
    }
    let mean = sum / n;
    let var = (sum2 / n - mean * mean).max(0.0);
    (mean as f32, var.sqrt() as f32)
}

/// Perf labels for the last recorded GI sample. Empty until a sample lands.
fn note_gi(app: &mut App, renderer: &mut Renderer) {
    let Some(sample) = renderer.gi_metrics_samples().last().copied() else {
        return;
    };
    app.gi_passes = format!(
        "direct {:.2} atrous {:.2} compose {:.2} trace {:.2} filter {:.2} gather {:.2} resolve {:.2}",
        sample.direct_ms,
        sample.atrous_ms,
        sample.compose_ms,
        sample.trace_ms,
        sample.filter_ms,
        sample.gather_ms,
        sample.resolve_ms
    );
    app.gi_work = format!(
        "rays {} refreshed {} lamps {}",
        sample.rays, sample.cache_entries_refreshed, sample.lamps_shaded
    );
}

fn apply_intent(opts: &Options) -> Result<(), String> {
    if let Some(preset) = &opts.preset {
        let parsed = genos_scene::LightingPreset::parse(preset)
            .ok_or_else(|| format!("unknown preset {preset}"))?;
        // Canonical name so deprecated aliases (potato) still set a live value.
        std::env::set_var("GENOS_LIGHTING_PRESET", parsed.name());
    } else if opts.full_effort {
        std::env::set_var("GENOS_LIGHTING_PRESET", "high");
    }
    if opts.full_effort {
        std::env::set_var("GENOS_LIGHTING_COMPOSE_ALL", "1");
    }
    if let Some(n) = opts.gpu_times {
        std::env::set_var("GENOS_GPU_TIMES", n.to_string());
    }
    if opts.lighting_debug {
        std::env::set_var("GENOS_LIGHTING_DEBUG", "1");
    }
    if let Some(value) = &opts.denoise_screen {
        std::env::set_var("GENOS_LIGHTING_DENOISE_SCREEN", value);
    }
    if let Some(value) = &opts.denoise_launch {
        std::env::set_var("GENOS_LIGHTING_DENOISE_LAUNCH", value);
    }
    Ok(())
}

/// `--out` wins. `GENOS_HDR_OUT` applies only when that argument was omitted.
fn hdr_out_dir(cli: Option<&str>, env: Option<&str>) -> String {
    if let Some(dir) = cli {
        return dir.to_string();
    }
    match env {
        Some(dir) if !dir.is_empty() => dir.to_string(),
        _ => "genos-debug".to_string(),
    }
}

fn need_arg(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn options() -> Result<Options, String> {
    let mut args = std::env::args().skip(1).peekable();
    let mut scene = std::env::var("GENOS_SCENE").ok();
    let mut checkpoint = None;
    let mut eye = None;
    let mut look = None;
    let mut preset = None;
    let mut gpu_times = None;
    let mut script = None;
    let mut report = None;
    let mut lighting_debug = false;
    let mut unfilled = false;
    let mut full_effort = false;
    let mut denoise_launch = None;
    let mut denoise_screen = None;
    let mut headless = false;
    let mut display = None;
    let mut frames = None;
    let mut ui = false;
    let mut out_cli = None;
    let mut size = (1280u32, 720u32);
    let mut check = false;
    let mut mcp_port = None;
    let mut block_ms = None;
    let mut close_early = false;
    let mut fog_cli = FogCli::default();
    let mut particle_cli = ParticleCli::default();
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            genos_window::exit(0);
        }
        match arg.as_str() {
            "--scene" => scene = Some(need_arg(&mut args, "--scene")?),
            "--checkpoint" => checkpoint = Some(need_arg(&mut args, "--checkpoint")?),
            "--eye" => eye = Some(parse_vec3("--eye", &need_arg(&mut args, "--eye")?)?),
            "--look" => look = Some(parse_vec3("--look", &need_arg(&mut args, "--look")?)?),
            "--preset" => preset = Some(need_arg(&mut args, "--preset")?),
            "--scenes" => std::env::set_var("GENOS_SCENES", need_arg(&mut args, "--scenes")?),
            "--mesh-workspace" => {
                std::env::set_var("GENOS_MESH_WORKSPACE", need_arg(&mut args, "--mesh-workspace")?);
            }
            "--gpu-times" => {
                gpu_times = Some(
                    need_arg(&mut args, "--gpu-times")?
                        .parse()
                        .map_err(|_| "bad --gpu-times".to_string())?,
                );
            }
            "--script" => script = Some(PathBuf::from(need_arg(&mut args, "--script")?)),
            "--report" => report = Some(PathBuf::from(need_arg(&mut args, "--report")?)),
            "--lighting-debug" => lighting_debug = true,
            "--cooked" => {
                COOKED
                    .set(PathBuf::from(need_arg(&mut args, "--cooked")?))
                    .ok();
            }
            "--unfilled" => unfilled = true,
            "--full-effort" => full_effort = true,
            "--denoise-launch" => denoise_launch = Some(need_arg(&mut args, "--denoise-launch")?),
            "--denoise-screen" => denoise_screen = Some(need_arg(&mut args, "--denoise-screen")?),
            "--headless" => headless = true,
            "--display" => {
                let name = need_arg(&mut args, "--display")?;
                if name.is_empty() {
                    return Err("display name is empty".into());
                }
                display = Some(name);
            }
            "--frames" => {
                frames = Some(
                    need_arg(&mut args, "--frames")?
                        .parse()
                        .map_err(|_| "bad --frames".to_string())?,
                );
            }
            "--ui" => ui = true,
            "--out" => out_cli = Some(need_arg(&mut args, "--out")?),
            "--size" => {
                let text = need_arg(&mut args, "--size")?;
                let (w, h) = text.split_once('x').ok_or("size is WxH")?;
                size = (
                    w.parse().map_err(|_| "bad width".to_string())?,
                    h.parse().map_err(|_| "bad height".to_string())?,
                );
            }
            "--check" => check = true,
            "--fps-counter" => {
                if args.peek().is_some_and(|next| !next.starts_with('-')) {
                    let value = args.next().unwrap();
                    parse_enabled(&value).map_err(|err| format!("bad --fps-counter: {err}"))?;
                }
            }
            "--fps-hz" => {
                let value = need_arg(&mut args, "--fps-hz")?;
                parse_rate(&value).map_err(|err| format!("bad --fps-hz: {err}"))?;
            }
            "--menu" => {
                let value = if args.peek().is_some_and(|next| !next.starts_with('-')) {
                    args.next().unwrap()
                } else {
                    return Err("--menu needs shown or hidden".into());
                };
                parse_menu_visible(&value).map_err(|err| format!("bad --menu: {err}"))?;
            }
            "--mcp-port" => {
                mcp_port = Some(parse_mcp_port(&need_arg(&mut args, "--mcp-port")?)?);
            }
            "--lifecycle-block-ms" => {
                block_ms = Some(genos_window::parse_block_ms(
                    "--lifecycle-block-ms",
                    Some(&need_arg(&mut args, "--lifecycle-block-ms")?),
                )?);
            }
            "--lifecycle-close-early" => close_early = true,
            "--fog" | "--fog-density" | "--fog-height" | "--fog-thickness" | "--fog-albedo"
            | "--fog-phase" | "--fog-gi" | "--fog-history" | "--fog-grid" | "--fog-samples" => {
                take_fog(&mut fog_cli, &arg, &mut args)?;
            }
            "--fx-preset" | "--fx-max" | "--fx-count" | "--fx-freeze" | "--fx-spawn" => {
                take_particles(&mut particle_cli, &arg, &mut args)?;
            }
            other => {
                if let Some((flag, value)) = placed_flag(other) {
                    let parsed = parse_vec3(flag, value)?;
                    if flag == "--eye" {
                        eye = Some(parsed);
                    } else {
                        look = Some(parsed);
                    }
                } else if let Some(value) = other.strip_prefix("--fps-counter=") {
                    parse_enabled(value).map_err(|err| format!("bad --fps-counter: {err}"))?;
                } else if let Some(value) = other.strip_prefix("--fps-hz=") {
                    parse_rate(value).map_err(|err| format!("bad --fps-hz: {err}"))?;
                } else if let Some(value) = other.strip_prefix("--menu=") {
                    parse_menu_visible(value).map_err(|err| format!("bad --menu: {err}"))?;
                } else if let Some(value) = other.strip_prefix("--mcp-port=") {
                    mcp_port = Some(parse_mcp_port(value)?);
                } else if genos_window::block_ms_needs_value(other) == Some(false) {
                    block_ms = Some(genos_window::parse_block_ms(other, None)?);
                } else if genos_scene::reflection_flag_needs_value(other).is_some() {
                    let needs = genos_scene::reflection_flag_needs_value(other).unwrap_or(false);
                    let next = needs.then(|| need_arg(&mut args, other)).transpose()?;
                    genos_scene::check_reflection_arg(other, next.as_deref())?;
                } else if genos_scene::lighting_flag_needs_value(other).is_some() {
                    let needs = genos_scene::lighting_flag_needs_value(other).unwrap_or(false);
                    let next = needs.then(|| need_arg(&mut args, other)).transpose()?;
                    genos_scene::check_lighting_arg(other, next.as_deref())?;
                } else if genos_render::gi_metrics_flag_needs_value(other).is_some() {
                    let needs = genos_render::gi_metrics_flag_needs_value(other).unwrap_or(false);
                    let next = needs.then(|| need_arg(&mut args, other)).transpose()?;
                    genos_render::check_gi_metrics_arg(other, next.as_deref())?;
                } else if genos_scene::fog_flag_needs_value(other).is_some() {
                    take_fog(&mut fog_cli, other, &mut args)?;
                } else if genos_scene::particle_flag_needs_value(other).is_some() {
                    take_particles(&mut particle_cli, other, &mut args)?;
                } else if genos_render::gpu_times_flag_needs_value(other).is_some() {
                    let needs = genos_render::gpu_times_flag_needs_value(other).unwrap_or(false);
                    let next = needs.then(|| need_arg(&mut args, other)).transpose()?;
                    genos_render::check_gpu_times_arg(other, next.as_deref())?;
                } else if genos_scene::hdr_flag_needs_value(other).is_some() {
                    let needs = genos_scene::hdr_flag_needs_value(other).unwrap_or(false);
                    if needs {
                        drop(need_arg(&mut args, other)?);
                    }
                } else {
                    return Err(format!("unknown argument {other}"));
                }
            }
        }
    }
    if check {
        headless = true;
        size = (320, 180);
    }
    if report.is_some() && script.is_none() {
        return Err("--report needs --script".into());
    }
    let out = PathBuf::from(hdr_out_dir(
        out_cli.as_deref(),
        std::env::var("GENOS_HDR_OUT").ok().as_deref(),
    ));
    let script = script.map(|path| {
        let stem = path.file_stem().map_or_else(
            || "script".to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        ScriptRun {
            script: path,
            report_dir: report.unwrap_or_else(|| out.join("gi").join(stem)),
            trace: false,
        }
    });
    Ok(Options {
        scene: scene.map_or_else(
            || program().default_scene.to_string(),
            |name| genos_scene_lib::canonical(&name).to_string(),
        ),
        checkpoint,
        eye,
        look,
        preset,
        gpu_times,
        script,
        lighting_debug,
        unfilled,
        full_effort,
        denoise_launch,
        denoise_screen,
        headless,
        display,
        frames,
        ui,
        out,
        out_set: out_cli.is_some(),
        size,
        check,
        mcp_port,
        block_ms: genos_window::block_ms_mirror(block_ms),
        close_early: genos_window::close_early_mirror(close_early),
        fog_cli: fog_cli.clone(),
        fog_env: fog_env_from_process(&fog_cli)?,
        particle_cli: particle_cli.clone(),
        particle_env: particle_env_from_process(&particle_cli)?,
    })
}

fn take_fog(
    cli: &mut FogCli,
    arg: &str,
    args: &mut impl Iterator<Item = String>,
) -> Result<(), String> {
    let needs = genos_scene::fog_flag_needs_value(arg).unwrap_or(false);
    let next = needs.then(|| need_arg(args, arg)).transpose()?;
    genos_scene::apply_fog_arg(cli, arg, next.as_deref())
}

fn take_particles(
    cli: &mut ParticleCli,
    arg: &str,
    args: &mut impl Iterator<Item = String>,
) -> Result<(), String> {
    let needs = genos_scene::particle_flag_needs_value(arg).unwrap_or(false);
    let next = needs.then(|| need_arg(args, arg)).transpose()?;
    genos_scene::apply_particle_arg(cli, arg, next.as_deref())
}

fn particle_env_from_process(cli: &ParticleCli) -> Result<ParticleEnv, String> {
    Ok(ParticleEnv {
        preset: mirror_preset(cli.preset.is_some())?,
        max_particles: mirror_count(cli.max_particles.is_some(), "GENOS_FX_MAX", "max_particles")?,
        count: mirror_count(cli.count.is_some(), "GENOS_FX_COUNT", "count")?,
        freeze: mirror_switch(cli.freeze.is_some(), "GENOS_FX_FREEZE")?,
    })
}

fn mirror_preset(argument_set: bool) -> Result<Option<ParticlePreset>, String> {
    if argument_set {
        return Ok(None);
    }
    match std::env::var("GENOS_FX_PRESET") {
        Err(_) => Ok(None),
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => ParticlePreset::parse(value.trim())
            .map(Some)
            .ok_or_else(|| format!("GENOS_FX_PRESET is low, medium, or high, got {value}")),
    }
}

fn fog_env_from_process(cli: &FogCli) -> Result<FogEnv, String> {
    Ok(FogEnv {
        enabled: mirror_switch(cli.enabled.is_some(), "GENOS_FOG")?,
        density: mirror_number(cli.density.is_some(), "GENOS_FOG_DENSITY", "density")?,
        height: mirror_number(cli.height.is_some(), "GENOS_FOG_HEIGHT", "height")?,
        thickness: mirror_number(cli.thickness.is_some(), "GENOS_FOG_THICKNESS", "thickness")?,
        albedo: mirror_number(cli.albedo.is_some(), "GENOS_FOG_ALBEDO", "albedo")?,
        phase_g: mirror_number(cli.phase_g.is_some(), "GENOS_FOG_PHASE", "phase_g")?,
        gi: mirror_switch(cli.gi.is_some(), "GENOS_FOG_GI")?,
        history: mirror_switch(cli.history.is_some(), "GENOS_FOG_HISTORY")?,
        grid: mirror_grid(cli.grid.is_some())?,
        samples: mirror_count(cli.samples.is_some(), "GENOS_FOG_SAMPLES", "samples")?,
    })
}

fn mirror_switch(argument_set: bool, name: &str) -> Result<Option<bool>, String> {
    if argument_set {
        return Ok(None);
    }
    match std::env::var(name) {
        Err(_) => Ok(None),
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => genos_scene::parse_fog_switch(&value)
            .map(Some)
            .map_err(|err| format!("{name}: {err}")),
    }
}

fn mirror_grid(argument_set: bool) -> Result<Option<(u32, u32, u32)>, String> {
    if argument_set {
        return Ok(None);
    }
    match std::env::var("GENOS_FOG_GRID") {
        Err(_) => Ok(None),
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => genos_scene::parse_fog_grid(&value)
            .map(Some)
            .map_err(|err| format!("GENOS_FOG_GRID: {err}")),
    }
}

fn mirror_count(argument_set: bool, name: &str, key: &str) -> Result<Option<u32>, String> {
    if argument_set {
        return Ok(None);
    }
    match std::env::var(name) {
        Err(_) => Ok(None),
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => genos_scene::parse_fog_count(key, &value)
            .map(Some)
            .map_err(|err| format!("{name}: {err}")),
    }
}

fn mirror_number(argument_set: bool, name: &str, key: &str) -> Result<Option<f32>, String> {
    if argument_set {
        return Ok(None);
    }
    match std::env::var(name) {
        Err(_) => Ok(None),
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => genos_scene::parse_fog_number(key, &value)
            .map(Some)
            .map_err(|err| format!("{name}: {err}")),
    }
}

fn parse_mcp_port(value: &str) -> Result<u16, String> {
    let port: u16 = value
        .trim()
        .parse()
        .map_err(|_| format!("--mcp-port needs 1 to 65535, got {value}"))?;
    if port == 0 {
        return Err("--mcp-port needs 1 to 65535, got 0".into());
    }
    Ok(port)
}

/// `GENOS_MCP_PORT` when it is a port, otherwise this binary's default.
fn mcp_port_from_env() -> u16 {
    std::env::var("GENOS_MCP_PORT")
        .ok()
        .and_then(|text| parse_mcp_port(&text).ok())
        .unwrap_or(program().mcp_port)
}

#[cfg(test)]
mod tests {
    use super::{check_summary, gltf_bytes_absent, gltf_scene, skip_line};
    use genos_render::ViewMode;
    use genos_scene_lib::{Check, Entry, Kind};
    use genos_ui::Paint;

    #[test]
    fn a_missing_gltf_is_skipped_and_a_bad_scene_is_not() {
        let entries = vec![Entry {
            name: "sponza".into(),
            review: String::new(),
            kind: Kind::Gltf {
                path: "assets/built/sponza/sponza.gltf".into(),
                sun: None,
                sky: None,
                lamps: Vec::new(),
            },
            checkpoints: Vec::new(),
            check: Check {
                checkpoint: "atrium".into(),
                min_mean: 8.0,
            },
            hdr: Vec::new(),
            path: None,
        }];
        assert!(gltf_scene(&entries, "sponza"));
        assert!(!gltf_scene(&entries, "room"));
        let pointer = "assets/built/sponza/sponza.gltf is a Git LFS pointer; the asset bytes are not in this checkout";
        assert!(gltf_bytes_absent(pointer));
        assert!(gltf_bytes_absent(
            "file not found: assets/built/sponza/sponza.gltf"
        ));
        assert!(!gltf_bytes_absent("sponza.gltf: unrecognized asset"));
        let line = skip_line("sponza", pointer);
        assert!(line.contains("not a pass"));
        let summary = check_summary(&["camera".into(), "hell".into()], &["sponza".into()], &[]);
        assert_eq!(summary, "2 passed, 1 skipped, 0 failed (skipped sponza)");
        assert!(!summary.contains("sponza passed"));
    }

    #[test]
    fn eye_and_look_place_the_camera() {
        let mut opts = super::Options {
            scene: genos_scene_lib::DEFAULT_SCENE.into(),
            checkpoint: None,
            eye: Some([1.0, 2.0, 3.0]),
            look: Some([1.0, 2.0, 1.0]),
            preset: None,
            gpu_times: None,
            script: None,
            lighting_debug: false,
            unfilled: false,
            full_effort: false,
            denoise_launch: None,
            denoise_screen: None,
            headless: true,
            display: None,
            frames: None,
            ui: false,
            out: std::path::PathBuf::from("genos-debug"),
            out_set: false,
            size: (320, 180),
            check: false,
            mcp_port: None,
            block_ms: None,
            close_early: false,
            fog_cli: genos_scene::FogCli::default(),
            fog_env: genos_scene::FogEnv::default(),
            particle_cli: genos_scene::ParticleCli::default(),
            particle_env: genos_scene::ParticleEnv::default(),
        };
        let mut camera = genos_scene::Camera::opening();
        super::place_camera(&opts, &mut camera).unwrap();
        assert!((camera.position.x - 1.0).abs() < 1.0e-5);
        assert!((camera.position.y - 2.0).abs() < 1.0e-5);
        assert!((camera.position.z - 3.0).abs() < 1.0e-5);
        // Target is 2 m down -Z from the eye. Yaw 0 looks down -Z.
        assert!(camera.yaw.abs() < 1.0e-4, "yaw {}", camera.yaw);
        assert!(camera.pitch.abs() < 1.0e-4, "pitch {}", camera.pitch);
        opts.look = None;
        camera.set_pose(genos_scene::Vec3::new(0.0, 0.0, 0.0), 0.4, -0.2);
        super::place_camera(&opts, &mut camera).unwrap();
        assert!((camera.position.x - 1.0).abs() < 1.0e-5);
        assert!((camera.yaw - 0.4).abs() < 1.0e-5);
        assert!((camera.pitch + 0.2).abs() < 1.0e-5);
        assert!(super::parse_vec3("--eye", "1, 2, 3").is_ok());
        assert!(super::parse_vec3("--eye", "1,2").is_err());
        let (flag, value) = super::placed_flag("--eye=4,5,6").unwrap();
        assert_eq!(super::parse_vec3(flag, value).unwrap(), [4.0, 5.0, 6.0]);
        let (flag, value) = super::placed_flag("--look=7,8,9").unwrap();
        assert_eq!(super::parse_vec3(flag, value).unwrap(), [7.0, 8.0, 9.0]);
        assert!(super::placed_flag("--eye").is_none());
    }

    #[test]
    fn a_scene_file_look_lies_over_the_preset_and_a_save_writes_it_back() {
        use genos_debug::Host;

        let root = genos_scene_lib::repo_root();
        let dir = std::env::temp_dir().join(format!("genos-scene-program-look-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("camera.scene");
        std::fs::copy(genos_scene_lib::scenes_dir().join("camera.scene"), &path).expect("copy");
        let plain = genos_scene_lib::parse_path(&path).expect("scene");
        let log = || std::fs::File::create(dir.join("run.log")).expect("log");

        // No hdr line: the preset's value.
        let preset = super::App::open(vec![plain.clone()], root.clone(), log()).expect("app").scene_ref().hdr.clone();
        assert!(!preset.legacy_bytes);

        // Two keys in the file: those two change, the rest stays the preset's.
        let mut entry = plain.clone();
        entry.hdr = vec![("compensation".into(), "-1".into()), ("bloom_intensity".into(), "0.02".into())];
        let mut app = super::App::open(vec![entry], root.clone(), log()).expect("app");
        let mut expected = preset.clone();
        expected.compensation = -1.0;
        expected.bloom_intensity = 0.02;
        assert_eq!(app.scene_ref().hdr, expected);

        // Tune, save: the file holds the look, and reads back to the same value.
        app.scene().hdr.set_key("transition", "1.5").expect("key");
        app.scene().hdr.set_key("ev100", "3").expect("key");
        let tuned = app.scene_ref().hdr.clone();
        let saved = app.save_hdr_to_scene().expect("save");
        assert_eq!(saved.scene, plain.name);
        assert_eq!(saved.path, path.display().to_string());
        assert!(saved.lines.contains(&"hdr transition 1.5".to_string()), "{:?}", saved.lines);
        let text = std::fs::read_to_string(&path).expect("read");
        let original = std::fs::read_to_string(genos_scene_lib::scenes_dir().join("camera.scene")).expect("read");
        assert_eq!(text, format!("{original}{}\n", saved.lines.join("\n")));
        let reread = genos_scene_lib::parse_path(&path).expect("scene");
        assert_eq!(reread.hdr, app.entry().hdr);
        let reopened = super::App::open(vec![reread], root.clone(), log()).expect("app");
        assert_eq!(reopened.scene_ref().hdr, tuned);

        // The Output page's button does the same and says so on its row.
        app.save_look_from_menu();
        assert_eq!(app.look_note, "Saved to camera.scene");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), text);

        // A scene that was never a file has nowhere to save to.
        let mut loose = plain;
        loose.path = None;
        let mut app = super::App::open(vec![loose], root, log()).expect("app");
        let err = app.save_hdr_to_scene().expect_err("no file");
        assert!(err.contains("was not read from a file"), "{err}");
        app.save_look_from_menu();
        assert!(app.look_note.starts_with("Not saved: "), "{}", app.look_note);
        std::fs::remove_dir_all(&dir).expect("clean");
    }

    #[test]
    fn each_setting_uses_a_dropdown_a_slider_or_a_toggle() {
        use genos_debug::Host;
        use genos_scene::Camera;
        use genos_ui::{button_panel, Pointer, State};

        let root = genos_scene_lib::repo_root();
        let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir()).expect("scenes");
        let log = std::fs::File::create(std::env::temp_dir().join("genos-lighting-menu.log"))
            .expect("log");
        let app = super::App::open(entries, root, log).expect("app");
        let fps = genos_scene::FpsCounterSettings::default();
        let rows = super::panel_rows(&app, ViewMode::Full, &fps);
        // The one button is an action, not a setting: it saves the look.
        for row in &rows {
            let texts: Vec<&str> = row.buttons.iter().map(|button| button.text.as_str()).collect();
            assert!(texts.is_empty() || texts == [super::SAVE_LOOK_BUTTON], "{}: {texts:?}", row.label);
        }
        assert_eq!(rows.iter().filter(|row| !row.buttons.is_empty()).count(), 1);
        for label in ["Adapt down (s)", "Adapt up (s)", "Look transition (s)"] {
            assert!(rows.iter().any(|row| row.label == label && row.slider.is_some()), "missing slider {label}");
        }
        let dropdowns: Vec<&str> = rows
            .iter()
            .filter(|row| row.dropdown.is_some())
            .map(|row| row.label.as_str())
            .collect();
        for label in [
            "Scene",
            "Checkpoint",
            "View",
            "Shadow cell cache",
            "Ambient fill",
            "Lamp pick",
            "Surface cache blend",
        ] {
            assert!(dropdowns.contains(&label), "missing dropdown {label}");
        }
        let lamp = rows
            .iter()
            .find(|row| row.label == "Lamp pick")
            .expect("lamp pick dropdown")
            .dropdown
            .as_ref()
            .unwrap();
        let lamp_names: Vec<&str> = lamp
            .choices
            .iter()
            .map(|choice| choice.name.as_str())
            .collect();
        assert_eq!(lamp_names, ["off", "hits", "pixels", "both", "reuse"]);
        assert_eq!(lamp.value, 4.0, "high uses the lamp reservoir (reuse)");
        let sliders: Vec<&str> = rows
            .iter()
            .filter(|row| row.slider.is_some())
            .map(|row| row.label.as_str())
            .collect();
        for label in [
            "Preset",
            "Bounce count",
            "Max lights",
            "Direct refresh frames",
            "Direct scale",
            "GI refresh frames",
            "Ray budget shift",
            "Firefly sigma",
            "Ray budget",
            "Dormant rays",
            "Cache stale frames",
            "Denoiser iterations",
            "Denoiser variance",
        ] {
            assert!(sliders.contains(&label), "missing slider {label}");
        }
        assert!(!sliders.contains(&"Shadow cell cache"));
        assert!(!sliders.contains(&"Ambient fill"));
        let checks: Vec<&str> = rows
            .iter()
            .flat_map(|row| row.checks.iter().map(|check| check.text.as_str()))
            .collect();
        for label in [
            "Direct light",
            "Shadows",
            "GI",
            "Denoiser",
            "Surface light cache",
            "Attach movers",
            "Sky light",
            "Pre-fill",
            "Full effort",
            "Show",
        ] {
            assert!(checks.contains(&label), "missing toggle {label}");
        }
        let names: Vec<String> = app.knobs().into_iter().map(|knob| knob.name).collect();
        for name in [
            "lighting_bounces",
            "lighting_direct",
            "lighting_world_cache",
            "lighting_denoise",
            "lighting_sky",
            "lighting_prefill",
            "lighting_ambient",
            "lighting_shadow_filter",
        ] {
            assert!(names.iter().any(|knob| knob == name), "config name {name}");
        }
        assert!(checks.iter().all(|label| !sliders.contains(label)));
        assert!(sliders.contains(&"FPS"));
        let fps_row = rows.iter().find(|row| row.label == "FPS").unwrap();
        let rate = fps_row.slider.as_ref().unwrap();
        assert_eq!(rate.id, super::ID_FPS_HZ);
        assert_eq!(rate.min, 1.0);
        assert_eq!(rate.max, 30.0);
        assert_eq!(rate.step, 1.0);
        assert!(fps_row.checks.iter().any(|check| check.id == super::ID_FPS));
        let preset = rows
            .iter()
            .find(|row| row.label == "Preset")
            .unwrap()
            .slider
            .as_ref()
            .unwrap();
        let names: Vec<&str> = preset.steps.iter().map(|step| step.name.as_str()).collect();
        assert!(names.contains(&"fallback"));
        assert!(names.contains(&"day-cycle"));
        assert!(names.contains(&"gi-1bounce"));
        assert!(names.contains(&"high"));
        assert!(rows.iter().all(|row| row.label != "Screen probe rays"), "removed from the menu");
        let denoise = rows
            .iter()
            .find(|row| row.label == "Denoiser iterations")
            .unwrap()
            .slider
            .as_ref()
            .unwrap();
        let denoise_names: Vec<&str> = denoise
            .steps
            .iter()
            .map(|step| step.name.as_str())
            .collect();
        assert_eq!(denoise_names, ["1", "3", "5"]);

        let Some(path) = std::env::var_os("GENOS_MENU_SHOT") else {
            return;
        };
        let mut ui = State::default();
        let camera = Camera::opening();
        let viewport = [1280.0, 720.0];
        let idle = button_panel(
            &mut ui,
            "Menu",
            &rows,
            viewport,
            &camera,
            Pointer {
                x: -1.0,
                y: -1.0,
                ..Pointer::default()
            },
        );
        let hit = idle
            .shown
            .iter()
            .find(|shown| shown.id == super::ID_VIEW)
            .expect("view dropdown");
        let mut ui = State::default();
        let opened = button_panel(
            &mut ui,
            "Menu",
            &rows,
            viewport,
            &camera,
            Pointer::at(hit.rect.x + hit.rect.w * 0.5, hit.rect.y + 8.0, true),
        );
        let choice = opened
            .shown
            .iter()
            .find(|shown| shown.id != super::ID_VIEW && shown.text.as_deref() == Some("direct"))
            .expect("direct row");
        let frame = button_panel(
            &mut ui,
            "Menu",
            &rows,
            viewport,
            &camera,
            Pointer::at(
                choice.rect.x + 8.0,
                choice.rect.y + choice.rect.h * 0.5,
                false,
            ),
        );
        assert!(frame
            .shown
            .iter()
            .any(|shown| shown.id != super::ID_VIEW && shown.text.as_deref() == Some("bounce")));
        let (width, height, rgb) = raster_menu(&frame.paints);
        std::fs::write(path, genos_debug::png::encode_rgb(width, height, &rgb)).expect("png");
    }

    fn raster_menu(paints: &[Paint]) -> (u32, u32, Vec<u8>) {
        let mut max_x = 8.0f32;
        let mut max_y = 8.0f32;
        for paint in paints {
            max_x = max_x.max(paint.x + paint.w);
            max_y = max_y.max(paint.y + paint.h);
        }
        let margin = 12.0;
        let width = (max_x + margin).ceil() as u32;
        let height = (max_y + margin).ceil() as u32;
        let mut rgb = vec![8u8; (width * height * 3) as usize];
        for paint in paints {
            let color = [
                (paint.color[0].clamp(0.0, 1.0) * 255.0) as u8,
                (paint.color[1].clamp(0.0, 1.0) * 255.0) as u8,
                (paint.color[2].clamp(0.0, 1.0) * 255.0) as u8,
            ];
            let x0 = paint.x.floor().max(0.0) as u32;
            let y0 = paint.y.floor().max(0.0) as u32;
            let x1 = ((paint.x + paint.w).ceil() as u32).min(width);
            let y1 = ((paint.y + paint.h).ceil() as u32).min(height);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * width + x) * 3) as usize;
                    rgb[i] = color[0];
                    rgb[i + 1] = color[1];
                    rgb[i + 2] = color[2];
                }
            }
        }
        (width, height, rgb)
    }

    #[cfg(feature = "mesh")]
    #[test]
    fn a_mesh_scene_puts_its_controls_on_the_scene_tab_and_other_scenes_do_not() {
        use genos_debug::Host as _;
        let root = genos_scene_lib::repo_root();
        let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir()).expect("scenes");
        let log = std::fs::File::create(std::env::temp_dir().join("genos-editor-mesh-menu.log"))
            .expect("log");
        let mut app = super::App::open(entries, root, log).expect("app");
        let fps = genos_scene::FpsCounterSettings::default();
        let menu = super::load_menu_layout().expect("layout");
        // The labels on the Mesh tab; none when the tab is not drawn.
        let labels = |app: &super::App| -> Vec<String> {
            let blocks = super::panel_blocks(app, ViewMode::Full, &fps, &menu);
            let genos_ui::PanelBlock::Tabs(tabs) = &blocks[0] else {
                panic!("the menu is one tab strip");
            };
            let Some(mesh) = tabs.tabs.iter().find(|tab| tab.title == super::MESH_TAB) else {
                return Vec::new();
            };
            mesh.blocks
                .iter()
                .flat_map(|block| match block {
                    genos_ui::PanelBlock::Rows(rows) => rows.iter().map(|row| row.label.clone()).collect(),
                    genos_ui::PanelBlock::Section(section) => {
                        section.rows.iter().map(|row| row.label.clone()).collect::<Vec<_>>()
                    }
                    genos_ui::PanelBlock::Tabs(_) => Vec::new(),
                })
                .collect()
        };
        assert!(!labels(&app).iter().any(|label| label == "Click selects"), "not a mesh scene yet");
        app.load_scene("mesh").expect("mesh scene");
        app.follow_mesh();
        let shown = labels(&app);
        for want in ["Click selects", "Mesh shading", "Show object", "Note (T types)", "Folder"] {
            assert!(shown.iter().any(|label| label == want), "{want} is missing from {shown:?}");
        }
        // The on/off controls are check boxes on the tab.
        let blocks = super::panel_blocks(&app, ViewMode::Full, &fps, &menu);
        let checks: Vec<String> = super::flatten_blocks(&blocks)
            .iter()
            .flat_map(|row| row.checks.iter().map(|check| check.text.clone()))
            .collect();
        assert!(checks.iter().any(|text| text == "Turntable"), "{checks:?}");
        // The Objects panel: a row for each object, with a box that hides it.
        let cube = "1 cube";
        assert!(shown.iter().any(|label| label == cube) && shown.iter().any(|label| label == "2 bracket"), "{shown:?}");
        assert!(shown.iter().any(|label| label == "Filter (Shift+T types)"), "{shown:?}");
        let rows = super::flatten_blocks(&blocks);
        let row = rows.iter().find(|row| row.label == cube).expect("the cube's row");
        assert!(row.checks[0].on, "shown at first");
        super::toggle(&mut app, row.checks[0].id, false).expect("hide the cube");
        assert!(app.mesh.as_ref().is_some_and(|view| view.hidden.contains(&1)));
        // The filter narrows the list.
        #[cfg(feature = "mesh")]
        if let Some(view) = &mut app.mesh {
            view.filter = "brack".into();
        }
        let narrowed = labels(&app);
        assert!(narrowed.iter().any(|label| label == "2 bracket") && !narrowed.iter().any(|label| label == cube));
        // With no button held the pointer is over the UI only where a panel
        // paints, so the wheel zooms and a middle drag pans on the picture.
        let panel = [genos_ui::Paint { x: 16.0, y: 16.0, w: 400.0, h: 300.0, color: [0.0; 3], alpha: 1.0 }];
        assert!(!super::pointer_over_ui(false, false, &panel, [800.0, 400.0]), "on the picture");
        assert!(super::pointer_over_ui(false, false, &panel, [100.0, 100.0]), "on the panel");
        // A held left button follows the press: one that missed the panels is the picture's.
        assert!(!super::pointer_over_ui(true, true, &panel, [100.0, 100.0]), "a drag that began on the picture");
        assert!(super::pointer_over_ui(true, false, &panel, [800.0, 400.0]), "a drag that began on a panel");
        // The program draws the Objects and Replay lists as a panel of their
        // own: its lines hold the object rows, and the menu's do not.
        let (rest, panel) = super::split_mesh_panel(menu.clone());
        let in_panel: Vec<String> = super::flatten_blocks(&super::panel_blocks(&app, ViewMode::Full, &fps, &panel)).iter().map(|row| row.label.clone()).collect();
        let in_menu: Vec<String> = super::flatten_blocks(&super::panel_blocks(&app, ViewMode::Full, &fps, &rest)).iter().map(|row| row.label.clone()).collect();
        assert!(in_panel.iter().any(|label| label == "2 bracket") && !in_menu.iter().any(|label| label == "2 bracket"), "{in_panel:?}");
        assert!(in_menu.iter().any(|label| label == "Click selects") && !in_panel.iter().any(|label| label == "Click selects"));
        // Another scene has no Objects section.
        app.load_scene("lighting-stress").expect("lighting scene");
        app.follow_mesh();
        assert!(labels(&app).is_empty(), "no Mesh tab outside a mesh scene");
    }

    #[test]
    fn the_top_bar_has_a_button_for_each_page_and_the_drawer_holds_one_page() {
        let root = genos_scene_lib::repo_root();
        let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir()).expect("scenes");
        let log = std::fs::File::create(std::env::temp_dir().join("genos-top-bar.log")).expect("log");
        let app = super::App::open(entries, root, log).expect("app");
        let fps = genos_scene::FpsCounterSettings::default();
        let menu = super::load_menu_layout().expect("layout");
        let blocks = super::panel_blocks(&app, ViewMode::Full, &fps, &menu);
        let genos_ui::PanelBlock::Tabs(strip) = &blocks[0] else { panic!("the menu is one tab strip") };
        let titles: Vec<String> = strip.tabs.iter().map(|tab| tab.title.clone()).collect();
        let mut ui = genos_ui::State::default();
        // Closed: a button for each page, none held; the first page is in force.
        let (buttons, chosen) = super::bar_buttons(&blocks, &ui, false);
        assert_eq!(buttons.iter().map(|button| button.text.clone()).collect::<Vec<_>>(), titles);
        assert!(buttons.iter().all(|button| !button.selected));
        assert_eq!(chosen, Some(strip.tabs[0].id));
        // Open on the second page: its button is held, and the drawer is
        // that page under its own title, with no strip of tabs in it.
        let second = strip.tabs[1].clone();
        ui.select_tab(super::strip_id(0), second.id);
        let (buttons, chosen) = super::bar_buttons(&blocks, &ui, true);
        assert_eq!(buttons.iter().filter(|button| button.selected).map(|button| button.id).collect::<Vec<_>>(), [second.id]);
        let (title, page) = super::bar_page(blocks.clone(), chosen);
        assert_eq!(title, second.title);
        assert_eq!(page, second.blocks);
    }

    #[test]
    fn sections_group_every_control_and_the_fold_file_round_trips() {
        let root = genos_scene_lib::repo_root();
        let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir()).expect("scenes");
        let log = std::fs::File::create(std::env::temp_dir().join("genos-lighting-folds.log"))
            .expect("log");
        let app = super::App::open(entries, root, log).expect("app");
        let fps = genos_scene::FpsCounterSettings::default();
        let menu = super::load_menu_layout().expect("layout");
        let blocks = super::panel_blocks(&app, ViewMode::Full, &fps, &menu);
        let tabs = match &blocks[0] {
            genos_ui::PanelBlock::Tabs(tabs) => tabs,
            genos_ui::PanelBlock::Section(_) | genos_ui::PanelBlock::Rows(_) => {
                panic!("the menu is one tab strip")
            }
        };
        let titles: Vec<&str> = tabs.tabs.iter().map(|tab| tab.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Scene",
                "Pipeline",
                "Lamps",
                "GI",
                "Denoise",
                "Shadows",
                "Perf",
                "Reflections",
                "Output",
                "Fog",
                "Info"
            ]
        );
        let scene = tabs.tabs.iter().find(|tab| tab.title == "Scene").unwrap();
        assert!(matches!(scene.blocks[0], genos_ui::PanelBlock::Rows(_)));
        let direct = tabs.tabs.iter().find(|tab| tab.title == "Lamps").unwrap();
        assert!(direct.blocks.iter().any(|block| matches!(
            block,
            genos_ui::PanelBlock::Section(section)
                if section.title == "Lamps"
                    && !section.collapsed
                    && section.rows.iter().any(|row| row.label == "Max lights")
        )));
        let gi = tabs.tabs.iter().find(|tab| tab.title == "GI").unwrap();
        assert!(gi.blocks.iter().any(|block| matches!(
            block,
            genos_ui::PanelBlock::Section(section) if section.title == "Indirect" && !section.collapsed
        )));
        assert!(gi.blocks.iter().any(|block| matches!(
            block,
            genos_ui::PanelBlock::Section(section)
                if section.title == "World cache" && section.collapsed
        )));
        let info = tabs.tabs.iter().find(|tab| tab.title == "Info").unwrap();
        assert!(matches!(
            &info.blocks[0],
            genos_ui::PanelBlock::Rows(rows) if rows[0].label == app.entry().review
        ));
        let flat = super::panel_rows(&app, ViewMode::Full, &fps);
        assert!(flat.iter().any(|row| row.label == "Bounce count"));
        assert!(flat.iter().any(|row| row.label == "Shadow cell cache"));
        assert!(flat.iter().any(|row| row.label.starts_with("direct ")));
        assert!(flat.iter().any(|row| row.label.starts_with("rays ")));
        assert!(flat.iter().all(|row| !row.label.contains("hides.")));
        let folds = super::menu_folds(&menu);
        assert!(folds.iter().all(|fold| fold.key != "-"));
        let mut ui = genos_ui::State::default();
        for fold in &folds {
            ui.fold_section(fold.id, fold.collapsed);
        }
        let dir = std::env::temp_dir().join(format!("genos-menu-{}", std::process::id()));
        super::save_menu_folds(&dir, &ui, &folds).expect("save");
        let names = super::load_menu_folds(&dir).expect("load");
        assert!(names.iter().any(|name| name == "world"));
        assert!(names.iter().all(|name| name != "direct" && name != "gi"));
        super::save_menu_tab(&dir, "GI").expect("tab");
        assert_eq!(super::load_menu_tab(&dir).as_deref(), Some("GI"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn help_lists_the_fps_counter() {
        assert!(super::USAGE.contains("--fps-counter"));
        assert!(super::USAGE.contains("--fps-hz"));
        assert!(super::USAGE.contains("Grave"));
        assert!(super::USAGE.contains("F1"));
        assert!(super::USAGE.contains("F toggles"));
        assert!(super::USAGE.contains("F3"));
        assert!(super::USAGE.contains("GENOS_DEBUG_KEYS"));
        assert!(super::USAGE.contains("] and ["));
        assert!(super::USAGE.contains("tab.txt"));
        assert!(super::USAGE.contains("GENOS_FPS_COUNTER"));
        assert!(super::USAGE.contains("--menu"));
        assert!(super::USAGE.contains("GENOS_MENU"));
        assert!(super::USAGE.contains("get_menu_settings"));
        assert!(super::USAGE.contains("--mcp-port"));
        assert!(super::USAGE.contains("--ui"));
        assert!(super::USAGE.contains("8766"));
        assert!(super::USAGE.contains("--gi-metrics"));
        assert!(super::USAGE.contains("--script"));
        assert!(super::USAGE.contains("GENOS_GI_METRICS"));
        assert!(super::USAGE.contains("--exposure-mode"));
        assert!(super::USAGE.contains("--hdr-display"));
        assert!(super::USAGE.contains("--ev100"));
        assert!(super::USAGE.contains("--compensation"));
        assert!(super::USAGE.contains("--iso"));
        assert!(super::USAGE.contains("--aperture"));
        assert!(super::USAGE.contains("--shutter"));
        assert!(super::USAGE.contains("--bloom"));
        assert!(super::USAGE.contains("--bloom-intensity"));
        assert!(super::USAGE.contains("--bloom-levels"));
        assert!(super::USAGE.contains("--output"));
        assert!(super::USAGE.contains("GENOS_HDR_EXPOSURE_MODE"));
        assert!(super::USAGE.contains("GENOS_HDR_EV100"));
        assert!(super::USAGE.contains("GENOS_HDR_COMPENSATION"));
        assert!(super::USAGE.contains("GENOS_HDR_ISO"));
        assert!(super::USAGE.contains("GENOS_HDR_APERTURE"));
        assert!(super::USAGE.contains("GENOS_HDR_SHUTTER"));
        assert!(super::USAGE.contains("GENOS_HDR_DISPLAY"));
        assert!(super::USAGE.contains("GENOS_HDR_BLOOM"));
        assert!(super::USAGE.contains("GENOS_HDR_BLOOM_INTENSITY"));
        assert!(super::USAGE.contains("GENOS_HDR_BLOOM_LEVELS"));
        assert!(super::USAGE.contains("GENOS_HDR_OUTPUT"));
        assert!(super::USAGE.contains("GENOS_HDR_ADAPT_DOWN"));
        assert!(super::USAGE.contains("GENOS_HDR_ADAPT_UP"));
        assert!(super::USAGE.contains("GENOS_HDR_OUT"));
        assert!(super::USAGE.contains("--adapt-down"));
        assert!(super::USAGE.contains("--adapt-up"));
        assert!(super::USAGE.contains("--hdr-transition"));
        for name in genos_scene::HDR_ENV_MIRRORS {
            assert!(super::USAGE.contains(name), "{name}");
        }
        assert_eq!(super::hdr_out_dir(Some("shots"), Some("env")), "shots");
        assert_eq!(super::hdr_out_dir(None, Some("env")), "env");
        assert_eq!(super::hdr_out_dir(None, Some("")), "genos-debug");
        assert_eq!(super::hdr_out_dir(None, None), "genos-debug");
        assert!(super::USAGE.contains("Output"));
        assert!(super::USAGE.contains("--fog"));
        assert!(super::USAGE.contains("--fog-density"));
        assert!(super::USAGE.contains("GENOS_FOG"));
        assert!(super::USAGE.contains("GENOS_FOG_HISTORY"));
        assert!(super::USAGE.contains("--fog-grid"));
        assert!(super::USAGE.contains("--fog-samples"));
        assert!(super::USAGE.contains("GENOS_FOG_GRID"));
        assert!(super::USAGE.contains("GENOS_FOG_SAMPLES"));
    }

    #[test]
    fn the_lighting_mcp_port_is_not_the_stress_port() {
        assert_eq!(super::MCP_PORT, 8766);
        assert_ne!(super::MCP_PORT, genos_mcp::PORT);
        assert_eq!(super::parse_mcp_port("8770").unwrap(), 8770);
        assert!(super::parse_mcp_port("0").is_err());
        assert!(super::parse_mcp_port("nope").is_err());
    }

    #[test]
    fn a_fold_sent_with_a_tab_survives_the_next_frame() {
        use genos_debug::Host;
        let root = genos_scene_lib::repo_root();
        let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir()).expect("scenes");
        let log = std::fs::File::create(std::env::temp_dir().join("genos-lighting-fold-tab.log"))
            .expect("log");
        let mut app = super::App::open(entries, root, log).expect("app");
        let fps = genos_scene::FpsCounterSettings::default();
        let menu = super::load_menu_layout().expect("layout");
        let folds = super::menu_folds(&menu);
        let blocks = super::panel_blocks(&app, ViewMode::Full, &fps, &menu);
        let mut ui = genos_ui::State::default();
        ui.select_tab(super::strip_id(0), super::tab_id("Lamps"));
        let _ = genos_ui::button_panel_blocks(
            &mut ui,
            "stress",
            &blocks,
            [1024.0, 832.0],
            &genos_scene::Camera::opening(),
            genos_ui::Pointer::default(),
        );
        assert!(ui.section_known(super::section_id("direct")));
        assert!(!ui.section_collapsed(super::section_id("direct")));
        let mut tracked_collapsed = super::merge_collapsed(&[], &ui, &folds);
        let mut tracked_tab = "Lamps".to_string();
        app.scene().menu.tab = "Lamps".into();
        app.scene().menu.collapsed = tracked_collapsed.clone();
        app.scene().menu.set_key("fold", "direct").expect("fold");
        app.scene().menu.set_key("tab", "GI").expect("tab");
        assert_eq!(app.scene().menu.collapsed, vec!["direct".to_string()]);
        let mut menu_open = true;
        let dir = std::env::temp_dir().join(format!("genos-fold-tab-{}", std::process::id()));
        super::pull_menu(
            &mut app,
            &mut ui,
            &folds,
            &mut menu_open,
            &mut tracked_collapsed,
            &mut tracked_tab,
        );
        let _ = genos_ui::button_panel_blocks(
            &mut ui,
            "stress",
            &blocks,
            [1024.0, 832.0],
            &genos_scene::Camera::opening(),
            genos_ui::Pointer::default(),
        );
        super::push_menu(
            &mut app,
            &ui,
            &folds,
            &menu,
            menu_open,
            true,
            &mut super::MenuWrite {
                out: &dir,
                collapsed: &mut tracked_collapsed,
                tab: &mut tracked_tab,
            },
        )
        .expect("push");
        assert_eq!(app.scene().menu.tab, "GI");
        assert!(
            app.scene()
                .menu
                .collapsed
                .iter()
                .any(|name| name == "direct"),
            "collapsed={:?}",
            app.scene().menu.collapsed
        );
        let saved = super::load_menu_folds(&dir).expect("fold file");
        assert!(
            saved.iter().any(|name| name == "direct"),
            "fold file={saved:?}"
        );

        // The lamps page left the section open, and the stored list already
        // names it. Showing GI must not drop that fold.
        ui.fold_section(super::section_id("direct"), false);
        app.scene().menu.collapsed = vec!["direct".to_string()];
        tracked_collapsed = vec!["direct".to_string()];
        app.scene().menu.tab = "GI".into();
        tracked_tab = "GI".into();
        ui.select_tab(super::strip_id(0), super::tab_id("GI"));
        let _ = genos_ui::button_panel_blocks(
            &mut ui,
            "stress",
            &blocks,
            [1024.0, 832.0],
            &genos_scene::Camera::opening(),
            genos_ui::Pointer::default(),
        );
        super::push_menu(
            &mut app,
            &ui,
            &folds,
            &menu,
            menu_open,
            true,
            &mut super::MenuWrite {
                out: &dir,
                collapsed: &mut tracked_collapsed,
                tab: &mut tracked_tab,
            },
        )
        .expect("push again");
        assert_eq!(app.scene().menu.tab, "GI");
        assert_eq!(
            app.scene().menu.collapsed,
            vec!["direct".to_string()],
            "a fold on another page was dropped"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unseen_fold_stays_in_the_collapsed_list() {
        let mut ui = genos_ui::State::default();
        ui.fold_section(1, true);
        let folds = vec![
            super::MenuFold {
                id: 1,
                key: "direct".into(),
                collapsed: false,
            },
            super::MenuFold {
                id: 2,
                key: "world".into(),
                collapsed: true,
            },
        ];
        let merged = super::merge_collapsed(&["world".into(), "direct".into()], &ui, &folds);
        assert_eq!(merged, vec!["world".to_string(), "direct".to_string()]);
        ui.fold_section(1, false);
        let still = super::merge_collapsed(&merged, &ui, &folds);
        assert_eq!(
            still,
            vec!["world".to_string(), "direct".to_string()],
            "an open fold stays until that section is on screen"
        );
        let section = genos_ui::PanelSection::new(1, "Lamps", false, vec![]);
        let _ = genos_ui::button_panel_sections(
            &mut ui,
            "stress",
            &[section],
            [800.0, 600.0],
            &genos_scene::Camera::opening(),
            genos_ui::Pointer::default(),
        );
        let merged = super::merge_collapsed(&still, &ui, &folds);
        assert_eq!(merged, vec!["world".to_string()]);
    }

    #[test]
    fn a_scene_load_returns_inside_the_pump_budget() {
        use std::sync::{Condvar, Mutex};
        use std::time::{Duration, Instant};
        fn wait_ms(ms: u64) {
            let pair = (Mutex::new(()), Condvar::new());
            let guard = pair
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = pair.1.wait_timeout(guard, Duration::from_millis(ms));
        }
        let root = genos_scene_lib::repo_root();
        let entries = genos_scene_lib::load_dir(&genos_scene_lib::scenes_dir()).expect("scenes");
        let sponza = entries
            .iter()
            .find(|entry| entry.name == "sponza")
            .expect("sponza")
            .clone();
        let before = Instant::now();
        let sync = super::build_entry(&root, &sponza);
        let before_ms = before.elapsed().as_secs_f64() * 1000.0;
        println!("MEASURE before_load_stall_ms={before_ms:.1}");
        let absent = sync
            .as_ref()
            .err()
            .is_some_and(|err| super::gltf_bytes_absent(err));
        let log =
            std::fs::File::create(std::env::temp_dir().join("genos-stall-load.log")).expect("log");
        let mut app = super::App::open(entries, root, log).expect("app");
        let meter = genos_window::StallMeter::new();
        meter.mark();
        let begin = Instant::now();
        app.start_load("sponza").expect("begin");
        let begin_elapsed = begin.elapsed();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            meter.mark();
            match app.take_load() {
                Ok(genos_debug::LoadPace::Pending) => {
                    assert!(Instant::now() <= deadline, "scene load did not finish");
                    wait_ms(16);
                }
                Ok(genos_debug::LoadPace::Ready) => break,
                Err(err) if super::gltf_bytes_absent(&err) || absent => {
                    meter.mark();
                    println!(
                        "MEASURE after_begin_ms={:.1} after_load_stall_ms={:.1} note=gltf bytes absent",
                        begin_elapsed.as_secs_f64() * 1000.0,
                        meter.longest_ms()
                    );
                    assert!(
                        begin_elapsed <= Duration::from_millis(100),
                        "begin_load blocked {begin_elapsed:?}"
                    );
                    assert!(
                        meter.longest_ms() <= genos_scene::EventLoopSettings::BUDGET_MS,
                        "poll stalled {:.1} ms",
                        meter.longest_ms()
                    );
                    return;
                }
                Err(err) => panic!("{err}"),
            }
        }
        meter.mark();
        println!(
            "MEASURE after_begin_ms={:.1} after_load_stall_ms={:.1}",
            begin_elapsed.as_secs_f64() * 1000.0,
            meter.longest_ms()
        );
        assert!(begin_elapsed <= Duration::from_millis(100));
        assert!(meter.longest_ms() <= genos_scene::EventLoopSettings::BUDGET_MS);
    }
}

#[cfg(test)]
mod fps_overlay;
