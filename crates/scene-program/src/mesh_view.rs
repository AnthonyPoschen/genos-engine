#![expect(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    reason = "the view is public only for the editor crate's windowless test; each error is the refusal the caller reports"
)]
//! The mesh viewer's state and its agent commands.
//!
//! A mesh scene shows a mesh-workbench workspace. This module keeps what the
//! person selected in that picture (objects, faces, vertices, a point, a typed
//! note), answers a pick at a screen point, and runs a mesh-workbench command
//! on the session the binary supplies (`Program::mesh_session`; `genos-editor`
//! links `genos-meshbench`, ADR 0042). The record is `docs/systems/mesh-workbench.md`.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use genos_input::InputCode;
use genos_math::f32_from_f64;
use genos_mcp::json::{self, Value};
use genos_scene::Camera;
use genos_workbench::{Part, Workspace};

/// Backdates the last look at the workspace file so the next frame reads it.
const FOLLOW_NOW: std::time::Duration = std::time::Duration::from_secs(1);
/// Replies round numbers to five decimals.
const REPLY_DECIMALS: f64 = 1.0e5;
/// The selection overlay sits this far off the surface, metres, so it does
/// not fight the face under it for depth.
const OVERLAY_OFFSET: f32 = 0.003;
/// A selected vertex is drawn as a marker this share of the mesh size across.
const VERTEX_MARK: f32 = 0.012;
pub const OVERLAY_COLOR: [f32; 3] = [1.0, 0.52, 0.08];
/// Albedo of a mesh-workbench object with no texture.
pub const MESH_COLOR: [f32; 3] = [0.72, 0.72, 0.75];

/// The colour of the `index`-th object, face or collection in ID order: the
/// palette mesh-workbench's viewer, inspector and captures share.
#[expect(clippy::cast_precision_loss, reason = "a small count")]
pub fn id_color(index: usize) -> [f32; 3] {
    // mesh-workbench's `categorical_color`: the hue steps by the golden
    // ratio, at 0.95 value and 0.78 saturation, as display bytes.
    let hue = (index as f64 * 0.618_033_988_749_895).fract() * 6.0;
    let fraction = hue - hue.floor();
    let value = 0.95;
    let p = value * (1.0 - 0.78);
    let q = value * (1.0 - 0.78 * fraction);
    let t = value * (1.0 - 0.78 * (1.0 - fraction));
    let rgb = match crate::round_index(hue.floor()) {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    };
    // The picture takes linear colour.
    rgb.map(|channel| f32_from_f64(((channel * 255.0).round() / 255.0).powf(2.2)))
}

/// Radians of orbit per pixel of drag.
const ORBIT_TURN: f32 = 0.006;
/// Share of the orbit distance one pixel of middle drag pans.
const ORBIT_PAN: f32 = 0.0016;
/// Each wheel step moves this share of the distance toward or away.
const ORBIT_ZOOM: f32 = 0.12;
const ORBIT_NEAR: f32 = 0.05;
/// The opening view stands this many mesh sizes from its centre.
const ORBIT_FRAME: f32 = 1.8;
/// A press that moves fewer pixels than this before release is a click.
const CLICK_PIXELS: f32 = 4.0;
/// Radians a second the turntable turns: one turn in about 20 seconds.
const TURNTABLE_RATE: f32 = 0.314;
/// The camera counts as moved by someone else past this, metres or radians.
const ORBIT_SLACK: f32 = 1.0e-4;

/// The pointer for one frame of the mesh viewer.
#[derive(Clone, Copy, Debug, Default)]
pub struct OrbitInput {
    /// Pointer in pixels, origin top left.
    pub x: f32,
    pub y: f32,
    pub left: bool,
    /// The right button: a drag draws a box and selects what is in it.
    pub right: bool,
    /// The middle button: a drag pans.
    pub middle: bool,
    pub wheel: f32,
    /// The UI has the pointer: a press there is not for the mesh.
    pub over_ui: bool,
    /// Shift is down: a click adds to the selection.
    pub add: bool,
    pub frame_key: bool,
    /// Seconds since the last frame, for the turntable.
    pub dt: f32,
    /// Control is down: a left drag draws a box and selects what is in it.
    pub box_key: bool,
    /// Alt is down: a left drag draws a lasso and selects what is in it.
    pub lasso_key: bool,
}

/// Orbit about a point: left drag turns, middle drag pans, the wheel zooms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    target: [f32; 3],
    distance: f32,
    last: [f32; 2],
    /// Pixels the pointer moved since the left press. `None`: not pressed here.
    dragged: Option<f32>,
    panning: bool,
    /// The camera pose this controller last wrote.
    wrote: ([f32; 3], f32, f32),
}

/// What the person selected in the viewer. mesh-workbench IDs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub objects: BTreeSet<u64>,
    pub faces: BTreeSet<u64>,
    pub vertices: BTreeSet<u64>,
    /// The last picked surface point, in mesh coordinates.
    pub point: Option<[f32; 3]>,
    /// What the person typed about the selection.
    pub note: String,
}

impl Selection {
    fn is_empty(&self) -> bool {
        self.objects.is_empty() && self.faces.is_empty() && self.vertices.is_empty()
    }
}

/// The workspace a mesh scene shows.
pub struct MeshView {
    pub dir: PathBuf,
    /// `project.json` modification time at the last read.
    pub stamp: Option<SystemTime>,
    pub looked: Instant,
    pub workspace: Option<Workspace>,
    pub selection: Selection,
    /// The person is typing the note: keys go to the note, not to hotkeys.
    pub typing: bool,
    /// For the `face_orientation` shade: whether each triangle of each part
    /// faces the camera. Kept so the mesh is rebuilt only when one turns over.
    facing: Vec<Vec<bool>>,
    /// For the `depth` shade: a grey for each position of each part.
    depths: Vec<Vec<[f32; 3]>>,
    /// The picture's overlay no longer matches `selection`.
    pub overlay_stale: bool,
    /// What a click picks: `face`, `object`, or `vertex`.
    pub pick_mode: String,
    orbit: Option<Orbit>,
    /// Objects the picture leaves out. Local to the viewer, as in the
    /// mesh-workbench web viewer: the workspace is not changed.
    pub hidden: BTreeSet<u64>,
    /// `shaded` (one colour), `object_ids` (a colour per object), or
    /// `wireframe` (the polygon edges drawn over the shaded mesh).
    pub shade: String,
    /// The camera turns about the mesh on its own, as on a turntable.
    pub turntable: bool,
    /// The clip and the time, seconds, the mesh is posed at. `None` is rest.
    pub pose: Option<(String, f32)>,
    /// The posed position of each vertex, and of each bone's joint by name.
    posed: Option<Posed>,
    /// The pose has to be sampled again: the mesh was reloaded.
    repose: bool,
    /// The clip is playing: poses sampled once across it, and the play time.
    playing: Option<Playback>,
    /// The replay is stepping forward on its own: seconds to the next step.
    pub replay_playing: Option<f32>,
    /// Draw the bones over the mesh.
    pub bones: bool,
    /// Draw the mesh's surfaces. Off leaves the wireframe and the bones.
    pub surfaces: bool,
    /// Draw a grid on the ground.
    pub grid: bool,
    /// A soft dark patch on the ground under each object.
    pub shadow: bool,
    /// The orthographic view: a narrow lens from far back.
    pub orthographic: bool,
    /// The menu changed `orthographic`; the next pointer step moves the camera.
    pub projection_stale: bool,
    /// Text that narrows the window's Objects and Replay panels: an object or
    /// a collection whose name holds it, a command whose name holds it.
    pub filter: String,
    /// The keys go to `filter`, not to the note.
    pub typing_filter: bool,
    /// Text that narrows the Replay panel's commands by name.
    pub command_filter: String,
    /// The keys go to `command_filter`.
    pub typing_commands: bool,
    /// Control is down this frame: with T it types the command filter.
    pub control: bool,
    /// Collections whose objects the Objects panel does not list.
    pub folded: BTreeSet<u64>,
    /// The recorded sessions, oldest first, as of the last poll.
    pub recorded: Vec<genos_workbench::Session>,
    /// The topology shade (`wireframe`) labels vertices and faces with their IDs.
    pub labels: bool,
    /// The Objects and Replay panel is on the picture.
    pub panel_open: bool,
    /// The labels of the last look, and the look they were made for.
    label_cache: Option<(LabelKey, Vec<Label>)>,
    /// A box or a lasso being dragged, in pixels: the box's two corners, or
    /// the lasso's path.
    pub region: Option<Vec<[f32; 2]>>,
    /// Room the program's own UI takes on the picture's right edge and its
    /// top edge, in pixels: the gizmo and the words on the picture stand
    /// clear of it.
    pub hud_inset: [f32; 2],
    /// The drag is a lasso, not a box.
    region_lasso: bool,
    /// The right button holds the drag, not the left one.
    region_right: bool,
    /// The workspaces found on disk at the last poll. The open one is first.
    pub found: Vec<PathBuf>,
    /// Put the whole mesh in view on the next pointer step (a new workspace).
    pub frame_next: bool,
    /// The recorded state the picture shows in place of the live mesh.
    pub replay: Option<Replay>,
    /// The picture has to be rebuilt from [`MeshView::load`].
    pub rebuild: bool,
    /// The picture's objects no longer match `hidden` and `shade`.
    pub display_stale: bool,
    /// The mesh-workbench session on this workspace. Opened by the first
    /// `mesh_command`, so looking at a workspace writes nothing to it.
    /// The mesh-workbench session, on its own thread: a long command does
    /// not hold the frame.
    bridge: Option<Worker>,
    /// Tool requests the session has not answered yet.
    waiting: Vec<Waiting>,
    /// The commands that changed the mesh through this viewer, oldest first.
    edits: Vec<Edit>,
    /// A transaction mesh-workbench has prepared and not committed. The
    /// picture shows its result.
    preview: Option<Preview>,
}

/// The mesh tools that each take one family of mesh-workbench commands, and
/// what the family is for. `mesh_command` takes any of them.
/// The fields each of the viewer's own tools takes. The schema a client sees
/// (`genos_debug::commands::mesh_schema`) names the same ones; a test checks it.
pub const VIEW_ARGS: [&str; 25] = [
    "panel",
    "labels",
    "command_filter",
    "workspace",
    "projection",
    "filter",
    "pick",
    "hide",
    "show",
    "solo",
    "show_all",
    "shade",
    "look",
    "replay",
    "turntable",
    "pose",
    "bones",
    "surfaces",
    "solo_match",
    "hide_match",
    "solo_collection",
    "hide_collection",
    "event",
    "grid",
    "shadow",
];
pub const PICK_ARGS: [&str; 6] = ["screen", "mode", "select", "box", "lasso", "through"];
pub const SELECTION_ARGS: [&str; 6] = ["objects", "faces", "vertices", "note", "clear", "grow"];
pub const HELP_ARGS: [&str; 4] = ["topic", "all", "offset", "limit"];

pub const FAMILIES: [(&str, &str); 8] = [
    ("mesh_object", "objects and collections: object_* and collection_* (create, rename, move, rotate, duplicate, instance, join, separate, delete, stage), and cube"),
    ("mesh_select", "mesh-workbench's own vertex selection, which its move, scale and rotate act on: select and select_*"),
    ("mesh_material", "UVs, textures and vertex colours: uv_*, texture_* (import, create, assign, layers, masks, bake, maps), vertex_color_*, capture_uv, and their query_ and inspect_ commands"),
    ("mesh_rig", "bones and skin weights: rig_set, rig_clear, rig_display_tails_set, skin_weights_set, query_rig"),
    ("mesh_animation", "animation clips, tracks and keys: animation_* (set, update, remove, track_set, track_remove, keys_edit, ik_bake, sample) and query_animation"),
    ("mesh_query", "reading the mesh without changing it: inspect, inspect_*, query_*, measure_clearance, capture (a rendered PNG), and help"),
    ("mesh_document", "the document as a whole: undo, redo, transaction, save, load, import_glb, import_fbx, export, and the workflow_*, review_* and reference commands"),
    ("mesh_edit", "geometry edits and generators: move, scale, rotate, extrude, inset, bevel, bridge, split, weld, dissolve, delete_faces, triangulate, bisect_plane, shade_smooth, sweep, revolve, loft, extrude_polygon, and every other command"),
];

/// The family tool a mesh-workbench command belongs to, by its name.
pub fn family_of(command: &str) -> &'static str {
    let starts = |prefixes: &[&str]| prefixes.iter().any(|prefix| command.starts_with(prefix));
    let is = |names: &[&str]| names.contains(&command);
    if starts(&["rig_", "skin_weights_"]) || is(&["query_rig"]) {
        "mesh_rig"
    } else if starts(&["animation_"]) || is(&["query_animation"]) {
        "mesh_animation"
    } else if starts(&["uv_", "texture_", "vertex_color_", "query_texture_"])
        || is(&["query_uv", "inspect_uv", "query_vertex_colors", "capture_uv"])
    {
        "mesh_material"
    } else if starts(&["object_", "collection_"]) || is(&["cube", "query_object_review"]) {
        "mesh_object"
    } else if starts(&["select"]) {
        "mesh_select"
    } else if starts(&["workflow_", "review_", "reference_", "import_"])
        || is(&["undo", "redo", "transaction", "save", "load", "export", "fit_reference", "project_reference", "compare_reference", "move_in_reference", "query_reviews"])
    {
        "mesh_document"
    } else if starts(&["inspect", "query_"]) || is(&["measure_clearance", "capture", "help"]) {
        "mesh_query"
    } else {
        "mesh_edit"
    }
}

/// A sampled pose: where each vertex and each bone's joint is.
#[derive(Clone, Debug, Default, PartialEq)]
struct Posed {
    vertices: std::collections::BTreeMap<u64, [f32; 3]>,
    joints: std::collections::BTreeMap<String, [f32; 3]>,
}

/// The sign Shift gives on each digit key (US layout).
const NOTE_SIGNS: [(char, char); 10] =
    [('1', '!'), ('2', '@'), ('3', '#'), ('4', '$'), ('5', '%'), ('6', '^'), ('7', '&'), ('8', '*'), ('9', '('), ('0', ')')];
/// The other keys that type a mark: plain, then with Shift (US layout).
const NOTE_MARKS: [(InputCode, char, char); 16] = [
    (InputCode::key_semicolon, ';', ':'),
    (InputCode::key_equal, '=', '+'),
    (InputCode::key_comma, ',', '<'),
    (InputCode::key_minus, '-', '_'),
    (InputCode::key_period, '.', '>'),
    (InputCode::key_slash, '/', '?'),
    (InputCode::key_backslash, '\\', '|'),
    (InputCode::key_quote, '\'', '"'),
    (InputCode::key_left_bracket, '[', '{'),
    (InputCode::key_right_bracket, ']', '}'),
    (InputCode::key_grave, '`', '~'),
    (InputCode::key_numpad_decimal, '.', '.'),
    (InputCode::key_numpad_add, '+', '+'),
    (InputCode::key_numpad_subtract, '-', '-'),
    (InputCode::key_numpad_multiply, '*', '*'),
    (InputCode::key_numpad_divide, '/', '/'),
];

/// The longest note the window takes. An agent can set a longer one.
const NOTE_MAX: usize = 200;
/// The keys that type a letter or a digit into the note.
const NOTE_KEYS: [(InputCode, char); 36] = [
    (InputCode::key_a, 'a'),
    (InputCode::key_b, 'b'),
    (InputCode::key_c, 'c'),
    (InputCode::key_d, 'd'),
    (InputCode::key_e, 'e'),
    (InputCode::key_f, 'f'),
    (InputCode::key_g, 'g'),
    (InputCode::key_h, 'h'),
    (InputCode::key_i, 'i'),
    (InputCode::key_j, 'j'),
    (InputCode::key_k, 'k'),
    (InputCode::key_l, 'l'),
    (InputCode::key_m, 'm'),
    (InputCode::key_n, 'n'),
    (InputCode::key_o, 'o'),
    (InputCode::key_p, 'p'),
    (InputCode::key_q, 'q'),
    (InputCode::key_r, 'r'),
    (InputCode::key_s, 's'),
    (InputCode::key_t, 't'),
    (InputCode::key_u, 'u'),
    (InputCode::key_v, 'v'),
    (InputCode::key_w, 'w'),
    (InputCode::key_x, 'x'),
    (InputCode::key_y, 'y'),
    (InputCode::key_z, 'z'),
    (InputCode::key_0, '0'),
    (InputCode::key_1, '1'),
    (InputCode::key_2, '2'),
    (InputCode::key_3, '3'),
    (InputCode::key_4, '4'),
    (InputCode::key_5, '5'),
    (InputCode::key_6, '6'),
    (InputCode::key_7, '7'),
    (InputCode::key_8, '8'),
    (InputCode::key_9, '9'),
];

/// A clip being played: poses sampled across it, blended by the play time.
struct Playback {
    clip: String,
    length: f32,
    time: f32,
    samples: Vec<(f32, Posed)>,
}

impl Playback {
    /// The pose at the play time: the two samples around it, blended.
    fn blend(&self) -> Option<Posed> {
        let after = self.samples.iter().position(|(time, _)| *time >= self.time).unwrap_or(self.samples.len().checked_sub(1)?);
        let before = after.saturating_sub(1);
        let (t0, from) = self.samples.get(before)?;
        let (t1, to) = self.samples.get(after)?;
        let share = if t1 > t0 { ((self.time - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 0.0 };
        let mix = |a: [f32; 3], b: [f32; 3]| [a[0] + (b[0] - a[0]) * share, a[1] + (b[1] - a[1]) * share, a[2] + (b[2] - a[2]) * share];
        Some(Posed {
            vertices: from.vertices.iter().map(|(id, a)| (*id, to.vertices.get(id).map_or(*a, |b| mix(*a, *b)))).collect(),
            joints: from.joints.iter().map(|(name, a)| (name.clone(), to.joints.get(name).map_or(*a, |b| mix(*a, *b)))).collect(),
        })
    }
}

/// Poses sampled per second of clip for playback.
const PLAY_RATE: f32 = 12.0;
/// The most poses sampled for one clip.
const PLAY_SAMPLES_MAX: usize = 48;
/// Seconds between replay steps while it plays. The web viewer uses 700 ms.
const REPLAY_STEP_SECONDS: f32 = 0.7;

/// Vertices or bones asked for in one `animation_sample` page: the most
/// mesh-workbench gives.
const POSE_PAGE: u64 = 256;
/// The grid reaches this many mesh sizes from the mesh's centre.
const GRID_REACH: f32 = 2.0;
/// The most grid lines each way from the centre.
const GRID_LINES_MAX: f32 = 40.0;
/// Half the width of a grid line, as a share of the grid step.
const GRID_WIDTH: f32 = 0.004;
pub const GRID_COLOR: [f32; 3] = [0.05, 0.056, 0.068];
/// The light a grid line gives off, so it reads lighter than the dark ground.
pub const GRID_GLOW: [f32; 3] = [0.03, 0.034, 0.042];
/// Half the width of a bone strip, as a share of the mesh size.
const BONE_WIDTH: f32 = 0.006;
pub const BONE_COLOR: [f32; 3] = [0.1, 0.85, 0.35];

fn point(value: &Value) -> Option<[f32; 3]> {
    let items = value.as_array()?;
    if items.len() != 3 {
        return None;
    }
    Some([f32_from_f64(items[0].as_f64()?), f32_from_f64(items[1].as_f64()?), f32_from_f64(items[2].as_f64()?)])
}

/// Two crossed strips from `from` to `to`, `half` wide each side, seen from
/// either side. Appends positions and normals, three per triangle.
fn strip(from: [f32; 3], to: [f32; 3], half: f32, positions: &mut Vec<[f32; 3]>, normals: &mut Vec<[f32; 3]>) {
    let along = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let helper = if along[1].abs() > along[0].abs().max(along[2].abs()) { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let first = normalise(cross(along, helper));
    let second = normalise(cross(along, first));
    for side in [first, second] {
        let off = [side[0] * half, side[1] * half, side[2] * half];
        let quad = [
            [from[0] - off[0], from[1] - off[1], from[2] - off[2]],
            [from[0] + off[0], from[1] + off[1], from[2] + off[2]],
            [to[0] + off[0], to[1] + off[1], to[2] + off[2]],
            [to[0] - off[0], to[1] - off[1], to[2] - off[2]],
        ];
        let normal = normalise(cross(along, side));
        // Both windings: a strip is seen from either side.
        for corner in [0, 1, 2, 0, 2, 3, 0, 2, 1, 0, 3, 2] {
            positions.push(quad[corner]);
            normals.push(normal);
        }
    }
}

/// A recorded state on show.
#[derive(Clone, Debug, PartialEq)]
pub struct Replay {
    pub session: String,
    /// The command the picture stands after. 0 is the session start.
    pub event: u64,
    pub snapshot: u64,
    pub command: String,
}

/// The shade modes, in the order the menu lists them. The menu's list shows
/// six at a time and scrolls, so the ones used most come first.
pub const SHADES: [&str; 15] = [
    "shaded",
    "textured",
    "material",
    "wireframe",
    "uv_checker",
    "vertex_colors",
    "vertex_color_shaded",
    "object_ids",
    "face_ids",
    "collection_ids",
    "face_orientation",
    "tessellation",
    "normals",
    "depth",
    "silhouette",
];

/// Names a shade also answers to: mesh-workbench's own name for it, or an
/// earlier name of this viewer (deprecated: it logs a warning).
const SHADE_ALIASES: [(&str, &str, bool); 3] =
    [("orientation", "face_orientation", false), ("topology", "wireframe", false), ("vertex_colors_unlit", "vertex_colors", true)];

/// An object in no collection, in the `collection_ids` shade (mesh-workbench's 128 grey).
const UNASSIGNED_COLOR: [f32; 3] = [0.216, 0.216, 0.216];

/// The web viewer's background (23, 26, 32), as linear colour.
const WEB_BACKGROUND: [f32; 3] = [0.005, 0.0066, 0.0104];
/// The light an unmapped face gives off in the lit `material` shade, so it
/// is the same flat magenta from every side.
const UNMAPPED_GLOW: [f32; 3] = [1.0, 0.0, 1.0];

/// A face whose UVs are incomplete, in the textured and checker shades.
const UNMAPPED_COLOR: [f32; 3] = [1.0, 0.0, 1.0];

/// The most lines the window's Objects panel lists, and the most commands
/// its Replay panel lists.
const PANEL_OBJECTS: usize = 200;
const PANEL_EVENTS: usize = 12;

/// Triangles with a colour per vertex: positions, normals, colours.
pub type Painted = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 3]>);

/// One mesh-workbench command as its own reference documents it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDoc {
    pub command: String,
    /// The field names its reference row names, in the order they appear.
    pub fields: Vec<String>,
    /// The reference row's words: each field's form and what the command does.
    pub doc: String,
    /// The help topic the row is in.
    pub topic: String,
    /// A field's JSON type, from the examples mesh-workbench's help gives
    /// (`{"command":"uv_set","values":[...]}`): field, then `array`,
    /// `string`, `number`, `boolean` or `object`.
    pub types: Vec<(String, String)>,
    /// One example of the command from the help, as it is written there.
    pub example: String,
    /// Fields whose type is taken from other commands that have the field.
    pub inferred: Vec<String>,
}

/// The JSON type name of a value, for a schema.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Array(_) => "array",
        Value::String(_) => "string",
        Value::Bool(_) => "boolean",
        Value::Object(_) => "object",
        Value::Null => "null",
        Value::Number(_) => "number",
    }
}

/// Read the example commands out of a help topic: every JSON object in the
/// text that has a `command` name. Each gives its command's fields their
/// types, and the first is kept as the command's example. A command with no
/// table row (the rig and animation commands) is listed from its examples.
pub fn parse_examples(topic: &str, text: &str, out: &mut Vec<CommandDoc>) {
    let mut from = 0;
    while let Some(start) = text[from..].find("{\"command\"").map(|at| from + at) {
        // The matching brace, outside strings.
        let (mut depth, mut quoted, mut escaped, mut end) = (0usize, false, false, None);
        for (at, c) in text[start..].char_indices() {
            if quoted {
                match (escaped, c) {
                    (true, _) => escaped = false,
                    (false, '\\') => escaped = true,
                    (false, '"') => quoted = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => quoted = true,
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(start + at + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            break;
        };
        from = end;
        let written = &text[start..end];
        let Ok(Value::Object(pairs)) = json::parse(written) else {
            continue;
        };
        let Some(command) = pairs.iter().find(|(key, _)| key == "command").and_then(|(_, name)| name.as_str()) else {
            continue;
        };
        if !plain_name(command) {
            continue;
        }
        let at = out.iter().position(|known| known.command == command).unwrap_or_else(|| {
            out.push(CommandDoc {
                command: command.to_string(),
                fields: Vec::new(),
                doc: String::new(),
                topic: topic.to_string(),
                types: Vec::new(),
                example: String::new(),
                inferred: Vec::new(),
            });
            out.len() - 1
        });
        let known = &mut out[at];
        if known.example.is_empty() {
            known.example = written.to_string();
        }
        for (field, value) in pairs.iter().filter(|(key, _)| key != "command") {
            if !known.fields.contains(field) {
                known.fields.push(field.clone());
            }
            if !known.types.iter().any(|(named, _)| named == field) && !matches!(value, Value::Null) {
                known.types.push((field.clone(), type_name(value).to_string()));
            }
        }
    }
}

/// The fields `mesh_commands` takes.
pub const COMMANDS_ARGS: [&str; 2] = ["command", "family"];

/// Lines of a help topic asked for at a time (mesh-workbench's page limit).
const HELP_PAGE: u64 = 256;

/// The catalogue, read once a process from mesh-workbench's help.
static CATALOGUE: std::sync::OnceLock<Vec<CommandDoc>> = std::sync::OnceLock::new();

/// A backticked word that is a name: lower-case letters, digits, underscores.
fn plain_name(word: &str) -> bool {
    word.chars().next().is_some_and(|first| first.is_ascii_lowercase())
        && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The backticked spans of `text`.
fn ticked(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

/// Read the command rows out of one help topic. mesh-workbench's reference
/// is Markdown tables whose first cell names one or more commands in
/// backticks and whose other cells name the fields in backticks
/// (`values:[...]`, `object`) among prose. A command in several rows gets
/// the rows joined.
pub fn parse_reference(topic: &str, text: &str, out: &mut Vec<CommandDoc>) {
    let rows = || text.lines().filter(|line| line.starts_with("| `"));
    // Every command the topic documents: one named bare in another row's
    // words is a reference to it, not a field.
    let documented: Vec<&str> = rows()
        .filter_map(|line| line.trim_matches('|').split('|').next())
        .flat_map(ticked)
        .filter(|name| plain_name(name))
        .collect();
    for line in rows() {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        let [first, rest @ ..] = cells.as_slice() else {
            continue;
        };
        let commands: Vec<&str> = ticked(first).into_iter().filter(|name| plain_name(name)).collect();
        if commands.is_empty() || rest.is_empty() {
            continue;
        }
        let doc = rest.join(" - ");
        // A field is the name a backticked span starts with: `kind:"reference"`, `limit`, `crop:[...]`.
        let mut fields: Vec<String> = Vec::new();
        let mut types: Vec<(String, String)> = Vec::new();
        for span in ticked(&doc) {
            let name: String = span.chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').collect();
            let after = span[name.len()..].chars().next();
            let named_command = after.is_none() && documented.contains(&name.as_str());
            let is_field = plain_name(&name) && matches!(after, None | Some(':' | '?')) && !named_command;
            if !is_field {
                continue;
            }
            // The form written after the colon says the type: `value:[x,y,z]`,
            // `kind:"reference"`, `clean:true`, `limit:8`.
            let form = span[name.len()..].trim_start_matches(['?', ':']).trim_start();
            let kind = match form.chars().next() {
                Some('[') => Some("array"),
                Some('"') => Some("string"),
                Some('{') => Some("object"),
                Some(c) if c.is_ascii_digit() || c == '-' => Some("number"),
                _ if form.starts_with("true") || form.starts_with("false") => Some("boolean"),
                _ => None,
            };
            if let Some(kind) = kind.filter(|_| after == Some(':') && !types.iter().any(|(named, _): &(String, String)| *named == name)) {
                types.push((name.clone(), kind.to_string()));
            }
            if !fields.contains(&name) {
                fields.push(name);
            }
        }
        for command in commands {
            match out.iter_mut().find(|known| known.command == command) {
                Some(known) => {
                    known.doc = if known.doc.is_empty() { doc.clone() } else { format!("{}\n{doc}", known.doc) };
                    for field in &fields {
                        if !known.fields.contains(field) {
                            known.fields.push(field.clone());
                        }
                    }
                    for (field, kind) in &types {
                        if !known.types.iter().any(|(named, _)| named == field) {
                            known.types.push((field.clone(), kind.clone()));
                        }
                    }
                }
                None => out.push(CommandDoc {
                    command: command.to_string(),
                    fields: fields.clone(),
                    doc: doc.clone(),
                    topic: topic.to_string(),
                    types: types.clone(),
                    example: String::new(),
                inferred: Vec::new(),
                }),
            }
        }
    }
}

/// Give a field with no type the type it has in other commands, when every
/// command that types that field agrees (`object` is a number wherever an
/// example or a table writes it). Such a field is in `inferred`.
pub fn infer_types(found: &mut [CommandDoc]) {
    let mut seen: std::collections::BTreeMap<String, BTreeSet<String>> = std::collections::BTreeMap::new();
    for doc in found.iter() {
        for (field, kind) in &doc.types {
            seen.entry(field.clone()).or_default().insert(kind.clone());
        }
    }
    for doc in found.iter_mut() {
        for field in doc.fields.clone() {
            if doc.types.iter().any(|(named, _)| *named == field) {
                continue;
            }
            if let Some(kind) = seen.get(&field).filter(|kinds| kinds.len() == 1).and_then(|kinds| kinds.first()) {
                doc.types.push((field.clone(), kind.clone()));
                doc.inferred.push(field);
            }
        }
    }
}

/// Every command mesh-workbench's help documents. The help is read once,
/// through a session on an empty scratch workspace under the output
/// directory, so the open workspace's session record gains no help events.
fn catalogue() -> Result<&'static [CommandDoc], String> {
    if let Some(known) = CATALOGUE.get() {
        return Ok(known);
    }
    let open = crate::program().mesh_session.ok_or("this program does not link mesh-workbench: run genos-editor")?;
    let dir = out_dir().join("mesh-catalogue");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let mut session = open(&dir)?;
    let found = read_catalogue(session.as_mut());
    drop(session);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(CATALOGUE.get_or_init(|| found.unwrap_or_default()))
}

/// Read the catalogue through `session`: its help, parsed, with the names
/// mesh-workbench does not accept as commands left out. The session's
/// workspace is written to (a bare command may run), so give it a scratch one.
///
/// # Errors
/// mesh-workbench's help could not be read.
pub fn read_catalogue(session: &mut dyn crate::MeshSession) -> Result<Vec<CommandDoc>, String> {
    let mut ask = |request: Value| -> Result<Value, String> {
        let reply = session.execute(&request)?;
        if !reply_ok(&reply) {
            return Err(format!("mesh-workbench help: {}", json::encode(&reply)));
        }
        Ok(reply.get("result").cloned().unwrap_or(reply))
    };
    let topics = ask(obj(vec![("command", text("help")), ("all", Value::Bool(true))]))?;
    let names: Vec<String> = match topics.get("topics") {
        Some(Value::Object(pairs)) => pairs.iter().map(|(key, _)| key.clone()).collect(),
        _ => return Err("mesh-workbench help lists no topics".into()),
    };
    let mut found = Vec::new();
    for topic in names {
        let mut offset = Some(0);
        let mut whole = String::new();
        while let Some(at) = offset {
            let page = ask(obj(vec![("command", text("help")), ("topic", text(topic.clone())), ("offset", id(at)), ("limit", id(HELP_PAGE))]))?;
            whole.push_str(page.get("text").and_then(Value::as_str).unwrap_or(""));
            whole.push('\n');
            offset = page.get("next_offset").and_then(Value::as_u64);
        }
        parse_reference(&topic, &whole, &mut found);
        parse_examples(&topic, &whole, &mut found);
    }
    // The reference also has tables of render modes and stages. A row is a
    // command only if mesh-workbench knows the name: sent bare on the scratch
    // workspace, a command runs or asks for its fields, and any other name
    // is refused as unknown.
    found.retain(|doc| {
        let reply = session.execute(&obj(vec![("command", text(doc.command.clone()))]));
        let refused = match &reply {
            Ok(reply) => reply.get("error").and_then(Value::as_str).unwrap_or("").to_string(),
            Err(err) => err.clone(),
        };
        !refused.contains("unknown command")
    });
    infer_types(&mut found);
    // What is still untyped is asked of mesh-workbench itself: a field sent
    // as a value no field takes is refused by name when the command reaches
    // it ("cap_start must be boolean", "offset must be an unsigned integer").
    for doc in &mut found {
        for field in doc.fields.clone() {
            if doc.types.iter().any(|(named, _)| *named == field) {
                continue;
            }
            let probe = Value::Object(vec![
                ("command".to_string(), text(doc.command.clone())),
                (field.clone(), obj(vec![("genos_probe", Value::Bool(true))])),
            ]);
            let refused = match session.execute(&probe) {
                Ok(reply) => reply.get("error").and_then(Value::as_str).unwrap_or("").to_string(),
                Err(err) => err,
            };
            if let Some(kind) = type_from_refusal(&field, &refused) {
                doc.types.push((field, kind.to_string()));
            }
        }
    }
    found.sort_by(|a, b| a.command.cmp(&b.command));
    Ok(found)
}

/// The JSON type mesh-workbench's refusal names for `field`, when the
/// refusal is about that field: "`field` must be ...".
pub fn type_from_refusal(field: &str, refusal: &str) -> Option<&'static str> {
    let wanted = refusal.strip_prefix(field)?.strip_prefix(" must be ")?;
    let has = |words: &[&str]| words.iter().any(|word| wanted.contains(word));
    if has(&["boolean", "true or false"]) {
        Some("boolean")
    } else if wanted.starts_with('[') || has(&["array", "a list", "numbers"]) {
        Some("array")
    } else if has(&["integer", "finite", "degrees", "a number", "positive", "non-negative"]) {
        Some("number")
    } else if has(&["a name", "a string", "text", "bytes"]) {
        Some("string")
    } else if has(&["an object"]) {
        Some("object")
    } else {
        None
    }
}

/// What `mesh_commands` says of one command: its tool, its fields, the
/// reference's words, and a JSON schema that names the fields. The types
/// are in the words: mesh-workbench publishes none.
fn command_value(doc: &CommandDoc, full: bool) -> Value {
    let mut pairs = vec![
        ("command", text(doc.command.clone())),
        ("tool", text(family_of(&doc.command))),
        ("fields", list(doc.fields.iter().map(|field| text(field.clone())).collect())),
    ];
    if full {
        let mut properties = vec![(
            "command".to_string(),
            obj(vec![("const", text(doc.command.clone()))]),
        )];
        properties.extend(doc.fields.iter().map(|field| {
            // A type the help's examples show; the words say the rest.
            let property = match doc.types.iter().find(|(named, _)| named == field) {
                Some((_, kind)) if doc.inferred.contains(field) => obj(vec![
                    ("type", text(kind.clone())),
                    ("description", text("Type as this field has in mesh-workbench's other commands; see doc for its form.")),
                ]),
                Some((_, kind)) => obj(vec![("type", text(kind.clone()))]),
                None => obj(vec![("description", text("See doc for its form."))]),
            };
            (field.clone(), property)
        }));
        pairs.push(("doc", text(doc.doc.clone())));
        if !doc.example.is_empty() {
            pairs.push(("example", text(doc.example.clone())));
        }
        pairs.push(("topic", text(doc.topic.clone())));
        pairs.push((
            "schema",
            obj(vec![
                ("type", text("object")),
                ("properties", Value::Object(properties)),
                ("required", list(vec![text("command")])),
                ("additionalProperties", Value::Bool(true)),
            ]),
        ));
    }
    obj(pairs)
}

/// The key of a reply that is not the answer yet: its value is the ticket to
/// ask [`MeshView::poll_edit`] with.
pub const PENDING_KEY: &str = genos_debug::knobs::SCENE_PENDING;

/// A tool request the session is running.
struct Waiting {
    ticket: u64,
    command: String,
    request: Value,
    /// It is not a query: it goes in the edit record and ends a preview.
    changes: bool,
}

/// The mesh-workbench session on its own thread. Requests are answered in
/// the order they were sent. Dropping it closes the session.
struct Worker {
    requests: std::sync::mpsc::Sender<(u64, Value)>,
    replies: std::sync::mpsc::Receiver<(u64, Result<Value, String>)>,
    next: u64,
    done: Vec<(u64, Result<Value, String>)>,
}

impl Worker {
    fn start(mut session: Box<dyn crate::MeshSession>) -> Self {
        let (requests, inbox) = std::sync::mpsc::channel::<(u64, Value)>();
        let (outbox, replies) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Ends when the viewer drops its sender.
            for (ticket, request) in inbox {
                if outbox.send((ticket, session.execute(&request))).is_err() {
                    break;
                }
            }
        });
        Self { requests, replies, next: 1, done: Vec::new() }
    }

    fn send(&mut self, request: Value) -> Result<u64, String> {
        let ticket = self.next;
        self.next += 1;
        self.requests.send((ticket, request)).map_err(|_| "the mesh session stopped".to_string())?;
        Ok(ticket)
    }

    /// The reply to `ticket` if it has arrived.
    fn take(&mut self, ticket: u64) -> Option<Result<Value, String>> {
        self.done.extend(self.replies.try_iter());
        let at = self.done.iter().position(|(done, _)| *done == ticket)?;
        Some(self.done.swap_remove(at).1)
    }

    /// Block until `ticket` is answered.
    fn wait(&mut self, ticket: u64) -> Result<Value, String> {
        loop {
            if let Some(reply) = self.take(ticket) {
                return reply;
            }
            let reply = self.replies.recv().map_err(|_| "the mesh session stopped".to_string())?;
            self.done.push(reply);
        }
    }
}

/// One command that changed the mesh, as the context packet reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Edit {
    /// The command's name, or the names of a batch.
    command: String,
    /// The request as sent, cut to [`EDIT_TEXT`] characters.
    request: String,
    ok: bool,
    error: String,
    /// The note in the viewer when the command ran: what the person asked for.
    note: String,
}

/// A prepared transaction: mesh-workbench's `preview_id`, the revision it
/// was made from, and the scratch workspace that holds its result.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Preview {
    id: String,
    revision: u64,
    dir: PathBuf,
    commands: usize,
}

/// Edits kept for the context packet, characters of a request kept, and the
/// selected faces the packet describes one by one.
const EDITS_KEPT: usize = 32;
const EDIT_TEXT: usize = 400;
const CONTEXT_FACES: usize = 48;

/// The fields `mesh_preview` and `mesh_context` take.
pub const PREVIEW_ARGS: [&str; 4] = ["commands", "guards", "commit", "discard"];
pub const CONTEXT_ARGS: [&str; 2] = ["screenshot", "edits"];

/// The workspace the picker or `mesh_view workspace` chose, in place of the
/// one the scene file or `--mesh-workspace` names.
static WORKSPACE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

pub fn set_workspace(dir: Option<PathBuf>) {
    if let Ok(mut chosen) = WORKSPACE.lock() {
        *chosen = dir;
    }
}

#[must_use]
pub fn chosen_workspace() -> Option<PathBuf> {
    WORKSPACE.lock().ok().and_then(|chosen| chosen.clone())
}

/// The workspaces on disk a person can pick: every directory that holds a
/// `project.json` beside the open one and under each of `roots`.
fn find_workspaces(open: &std::path::Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    // One spelling a directory, so the open one is not listed twice.
    let plain = |dir: &std::path::Path| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut found = vec![plain(open)];
    let beside = open.parent().map(std::path::Path::to_path_buf);
    for root in beside.iter().chain(roots) {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = plain(&entry.path());
            if genos_workbench::project_path(&dir).is_file() && !found.contains(&dir) {
                found.push(dir);
            }
        }
    }
    found[1..].sort();
    found.truncate(WORKSPACES_LISTED);
    found
}

/// The most workspaces the picker lists.
const WORKSPACES_LISTED: usize = 64;

/// Where the program writes (`--out`, default `./genos-debug`): the preview's
/// scratch workspace goes under it.
static OUT: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

pub fn set_out_dir(dir: PathBuf) {
    if let Ok(mut out) = OUT.lock() {
        *out = Some(dir);
    }
}

fn out_dir() -> PathBuf {
    OUT.lock().ok().and_then(|out| out.clone()).unwrap_or_else(|| PathBuf::from("genos-debug"))
}

fn reply_ok(reply: &Value) -> bool {
    reply.get("ok").and_then(Value::as_bool).unwrap_or(false)
}

/// One clickable part of the axis gizmo: an axis end, or the projection's name.
#[derive(Clone, Debug, PartialEq)]
pub struct GizmoEnd {
    /// Its middle, in pixels.
    pub at: [f32; 2],
    /// Half its width and height.
    pub half: [f32; 2],
    pub label: String,
    pub color: [f32; 3],
    /// The side a click looks from (`mesh_view look`); `None` is the projection toggle.
    pub look: Option<&'static str>,
    /// A click opens or closes the Objects panel, not the projection.
    pub panel: bool,
    /// The gizmo's centre, for the ends that have a line to it.
    pub line_from: Option<[f32; 2]>,
    depth: f32,
}

impl GizmoEnd {
    /// Whether the pixel `at` is on it.
    #[must_use]
    pub fn holds(&self, at: [f32; 2]) -> bool {
        (at[0] - self.at[0]).abs() <= self.half[0] && (at[1] - self.at[1]).abs() <= self.half[1]
    }
}

/// The gizmo: its centre's distance from the picture's top right corner, an
/// arm's length, and half an end's size, in pixels.
const GIZMO_INSET: f32 = 70.0;
const GIZMO_ARM: f32 = 38.0;
const GIZMO_DOT: f32 = 9.0;
/// The side each end looks from: +X, -X, +Y, -Y, +Z, -Z.
const GIZMO_LOOKS: [&str; 6] = ["+x", "-x", "+y", "-y", "+z", "-z"];

/// An ID drawn on the picture: the number, where (screen, 0..1, origin top
/// left), and whether it names a face (else a vertex).
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub text: String,
    pub at: [f32; 2],
    pub face: bool,
}

/// What a set of labels was made for: the camera, the mesh's revision, how
/// many objects are hidden, and whether a pose is on.
type LabelKey = ([f32; 7], u64, usize, bool);

/// The most face labels, and the most vertex labels, on the picture.
const LABELS_MOST: usize = 160;
/// A face's label and a vertex's label, as mesh-workbench colours them apart.
pub const FACE_LABEL_COLOR: [f32; 3] = [1.0, 0.82, 0.35];
pub const VERTEX_LABEL_COLOR: [f32; 3] = [0.55, 0.85, 1.0];

/// What a panel line's check box acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelTarget {
    Collection(u64),
    Object(u64),
    Event(u64),
    Live,
}

/// One line of the window's Objects or Replay panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelLine {
    pub label: String,
    /// The check box's word.
    pub check: &'static str,
    pub on: bool,
    pub target: PanelTarget,
    /// A collection's second box: whether its objects are listed.
    pub open: Option<bool>,
}

/// The orthographic view: how many times the perspective distance its eye
/// stands back, and the furthest (the far plane is 200 m).
const ORTHO_BACK: f32 = 8.0;
const ORTHO_REACH: f32 = 120.0;

/// The contact shadow: how dark its centre is, how far past the footprint it
/// reaches, its sides, and (radius, darkness) from the centre to the rim.
const SHADOW_DARK: f32 = 0.6;
const SHADOW_REACH: f32 = 1.15;
const SHADOW_SIDES: usize = 40;
const SHADOW_RINGS: [(f32, f32); 5] = [(0.0, 1.0), (0.45, 0.9), (0.7, 0.55), (0.88, 0.2), (1.0, 0.0)];

/// `face_orientation`: a face seen from its front is the plain grey, and one
/// seen from behind is red, as in mesh-workbench.
const FRONT_COLOR: [f32; 3] = MESH_COLOR;
const BACK_COLOR: [f32; 3] = [0.9, 0.12, 0.08];

/// Side of the UV checker image, in texels, and of one of its squares.
const CHECKER: u32 = 128;
const CHECKER_CELL: u32 = 16;

/// The mesh's texture images and the UV checker side by side in one image:
/// the colour atlas of a mesh scene. A rect is `[x, y, width, height]` in texels.
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// RGBA8 sRGB, top row first.
    pub rgba: Vec<u8>,
    pub rects: Vec<[u32; 4]>,
    /// Each texture's alpha cutoff; 0 draws every texel.
    pub cutoffs: Vec<f32>,
    pub checker: [u32; 4],
}

#[must_use]
pub fn atlas(textures: &[genos_workbench::TextureImage]) -> Atlas {
    let usable = |t: &genos_workbench::TextureImage| {
        t.width > 0 && t.height > 0 && t.pixels.len() == (t.width as usize) * (t.height as usize) * 4
    };
    let height = textures.iter().filter(|t| usable(t)).map(|t| t.height).fold(CHECKER, u32::max);
    let width = CHECKER + textures.iter().filter(|t| usable(t)).map(|t| t.width).sum::<u32>();
    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    let row = (width as usize) * 4;
    for y in 0..CHECKER {
        for x in 0..CHECKER {
            // mesh-workbench's checker: eight squares a UV unit, 224 where
            // the squares' sum is even counted from the UV origin. The image's
            // top row is V = 1, so here that is an odd sum.
            let light = !(x / CHECKER_CELL + y / CHECKER_CELL).is_multiple_of(2);
            let texel = if light { [224, 224, 224, 255] } else { [48, 48, 48, 255] };
            let at = (y as usize) * row + (x as usize) * 4;
            rgba[at..at + 4].copy_from_slice(&texel);
        }
    }
    let mut left = CHECKER;
    let mut rects = Vec::with_capacity(textures.len());
    for texture in textures {
        if !usable(texture) {
            rects.push([0; 4]);
            continue;
        }
        let line = (texture.width as usize) * 4;
        for y in 0..texture.height as usize {
            let at = y * row + (left as usize) * 4;
            rgba[at..at + line].copy_from_slice(&texture.pixels[y * line..(y + 1) * line]);
            // With no cutoff the image stands over mid grey by its alpha, as
            // mesh-workbench previews it. With one, the surface's cutoff
            // drops the texels under it and the colour is left as it is.
            if texture.alpha_cutoff.is_none() {
                for texel in rgba[at..at + line].chunks_exact_mut(4) {
                    let alpha = u32::from(texel[3]);
                    for channel in &mut texel[..3] {
                        // A blend of two bytes is a byte: the clamp does not fire.
                        let blended = u8::try_from((u32::from(*channel) * alpha + 128 * (255 - alpha)) / 255).unwrap_or(u8::MAX);
                        *channel = blended;
                    }
                    texel[3] = 255;
                }
            }
        }
        rects.push([left, 0, texture.width, texture.height]);
        left += texture.width;
    }
    let cutoffs = textures.iter().map(|texture| texture.alpha_cutoff.unwrap_or(0.0)).collect();
    Atlas { width, height, rgba, rects, cutoffs, checker: [0, 0, CHECKER, CHECKER] }
}

/// Surfaces of a mesh scene, in the order `mesh_world` adds them: the
/// checker, then one per texture.
pub const SURFACE_CHECKER: usize = 0;
pub const SURFACE_TEXTURES: usize = 1;

/// A triangle's UVs, moved so the triangle starts in the first repeat (a
/// negative coordinate means "no texture" to the shader). `None` when a
/// corner has no UV.
fn triangle_uvs(part: &Part, triangle: usize) -> Option<[[f32; 2]; 3]> {
    let at = triangle * 3;
    if !(0..3).all(|corner| part.uv_mapped.get(at + corner).copied().unwrap_or(false)) {
        return None;
    }
    let uv = [0, 1, 2].map(|corner| part.uvs.get(at + corner).copied().unwrap_or([0.0; 2]));
    let shift = [0, 1].map(|axis| uv.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min).floor());
    Some(uv.map(|p| [p[0] - shift[0], p[1] - shift[1]]))
}

fn usable(texture: &genos_workbench::TextureImage) -> bool {
    texture.width > 0 && texture.height > 0 && texture.pixels.len() == (texture.width as usize) * (texture.height as usize) * 4
}

/// A texture coordinate and a colour for each position, and the texture of
/// each triangle.
type TexturedCorners = (Vec<[f32; 2]>, Vec<[f32; 3]>, Vec<Option<usize>>);

/// How a part draws in the `textured` or `uv_checker` shade: a texture
/// coordinate and a colour for each position, and the texture each triangle
/// draws with. As in mesh-workbench: a face with no texture is grey, a face
/// whose UVs are incomplete is magenta, and the vertex paint tints a texture.
fn textured_corners(part: &Part, textures: &[genos_workbench::TextureImage], checker: bool) -> TexturedCorners {
    let mut uvs = vec![[-1.0f32; 2]; part.positions.len()];
    let mut colors = vec![MESH_COLOR; part.positions.len()];
    let mut drawn = vec![None; part.triangles()];
    for (triangle, slot) in drawn.iter_mut().enumerate() {
        let texture = part.face_textures.get(triangle).copied().flatten().filter(|at| textures.get(*at).is_some_and(usable));
        let at = triangle * 3;
        // No UVs is magenta whether or not the face has a texture.
        let Some(mapped) = triangle_uvs(part, triangle) else {
            colors[at..at + 3].fill(UNMAPPED_COLOR);
            continue;
        };
        if !checker && texture.is_none() {
            continue;
        }
        uvs[at..at + 3].copy_from_slice(&mapped);
        for (corner, color) in colors[at..at + 3].iter_mut().enumerate() {
            *color = if checker { [1.0; 3] } else { part.colors.get(at + corner).copied().unwrap_or([1.0; 3]) };
        }
        *slot = texture;
    }
    (uvs, colors, drawn)
}

/// Whether a part is drawn as pieces (`MeshView::textured_pieces`): it uses
/// several textures, or in the lit `material` shade it has faces with no UVs.
fn in_pieces(shade: &str, colors: &[[f32; 3]], drawn: &[Option<usize>]) -> bool {
    let textures: BTreeSet<usize> = drawn.iter().flatten().copied().collect();
    textures.len() >= 2 || (shade == "material" && colors.contains(&UNMAPPED_COLOR))
}

/// A part's faces that draw with one surface, as an object of its own: a
/// part with several textures is drawn as one piece a texture, so each
/// repeats as it does in mesh-workbench.
pub struct Piece {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 3]>,
    pub texcoords: Vec<[f32; 2]>,
    pub surface: Option<usize>,
    /// Light the piece gives off.
    pub emission: [f32; 3],
}

impl Piece {
    /// The piece with the one colour it draws in when it has none per vertex.
    #[must_use]
    pub fn tinted(self, color: [f32; 3]) -> (Self, [f32; 3]) {
        (self, color)
    }
}

/// Normals for the posed positions `at`, smooth where the rest normals are:
/// corners that share a vertex and a rest normal share the posed one.
fn posed_normals_of(part: &Part, at: &[[f32; 3]]) -> Vec<[f32; 3]> {
    #[expect(clippy::cast_possible_truncation, reason = "a unit vector's component times 1000")]
    let key = |corner: usize| {
        let rest = part.normals.get(corner).copied().unwrap_or([0.0; 3]);
        (part.vertex_ids.get(corner).copied().unwrap_or(0), rest.map(|c| (c * 1000.0).round() as i32))
    };
    let mut sums: std::collections::BTreeMap<(u64, [i32; 3]), [f32; 3]> = std::collections::BTreeMap::new();
    for triangle in at.chunks_exact(3).enumerate() {
        let (index, t) = triangle;
        // Not normalised: a larger face counts for more.
        let face = cross([0, 1, 2].map(|k| t[1][k] - t[0][k]), [0, 1, 2].map(|k| t[2][k] - t[0][k]));
        for corner in index * 3..index * 3 + 3 {
            let sum = sums.entry(key(corner)).or_insert([0.0; 3]);
            for axis in 0..3 {
                sum[axis] += face[axis];
            }
        }
    }
    (0..at.len()).map(|corner| normalise(sums.get(&key(corner)).copied().unwrap_or([0.0, 1.0, 0.0]))).collect()
}

/// What one object draws as: hidden, its colour, a colour per vertex (or
/// none), and its surface with a texture coordinate per vertex (or none).
pub type Display = (bool, [f32; 3], Vec<[f32; 3]>, Option<(usize, Vec<[f32; 2]>)>);
/// Half the width of a wireframe strip, as a share of the mesh size.
const WIRE_WIDTH: f32 = 0.0025;
pub const WIRE_COLOR: [f32; 3] = [0.04, 0.04, 0.05];
/// `mesh_view` lists this many of a session's latest commands.
const REPLAY_LISTED: usize = 12;

/// The state file the picture is built from in place of `project.json`, for
/// the world builder in `main.rs`.
static SOURCE: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

fn set_source(path: Option<PathBuf>) {
    if let Ok(mut source) = SOURCE.lock() {
        *source = path;
    }
}

/// The workspace a mesh scene draws: the replayed state, else `project.json`.
pub fn load_shown(dir: &std::path::Path) -> Result<Workspace, String> {
    let source = SOURCE.lock().ok().and_then(|source| source.clone());
    match source {
        Some(path) => {
            let text = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
            let mut workspace = genos_workbench::parse_project(&text)?;
            workspace.dir = dir.to_path_buf();
            Ok(workspace)
        }
        None => genos_workbench::load_workspace(dir),
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalise(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 0.0 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// The pick modes, in the order the menu lists them.
pub const PICK_MODES: [&str; 3] = ["face", "object", "vertex"];

impl MeshView {
    pub fn new(dir: PathBuf, stamp: Option<SystemTime>) -> Self {
        let workspace = genos_workbench::load_workspace(&dir).ok();
        set_source(None);
        Self {
            dir,
            stamp,
            looked: Instant::now(),
            workspace,
            selection: Selection::default(),
            typing: false,
            facing: Vec::new(),
            depths: Vec::new(),
            overlay_stale: true,
            pick_mode: "face".into(),
            orbit: None,
            hidden: BTreeSet::new(),
            shade: "shaded".into(),
            turntable: false,
            pose: None,
            posed: None,
            repose: false,
            playing: None,
            replay_playing: None,
            bones: false,
            surfaces: true,
            grid: true,
            shadow: true,
            orthographic: false,
            projection_stale: false,
            filter: String::new(),
            typing_filter: false,
            command_filter: String::new(),
            typing_commands: false,
            control: false,
            folded: BTreeSet::new(),
            recorded: Vec::new(),
            labels: true,
            panel_open: true,
            label_cache: None,
            region: None,
            hud_inset: [0.0, 0.0],
            region_lasso: false,
            region_right: false,
            found: Vec::new(),
            frame_next: false,
            replay: None,
            rebuild: false,
            display_stale: true,
            bridge: None,
            waiting: Vec::new(),
            edits: Vec::new(),
            preview: None,
        }
    }

    /// The note keys for one frame. `T` starts typing, Enter ends it, Escape
    /// ends it and clears the note, Backspace removes a letter. While typing,
    /// letters, digits and the space bar go to the note; Shift gives capitals.
    /// Returns whether the keys were the note's this frame.
    pub fn type_note(&mut self, pressed: &dyn Fn(InputCode) -> bool, shift: bool) -> bool {
        if !self.typing {
            // T types the note. Shift and T types the panels' filter (Tab
            // moves between the menu's controls).
            // Control and T types the Replay panel's command filter.
            if pressed(InputCode::key_t) {
                self.typing = true;
                self.typing_commands = self.control;
                self.typing_filter = shift && !self.control;
                return true;
            }
            return false;
        }
        let words = if self.typing_commands {
            &mut self.command_filter
        } else if self.typing_filter {
            &mut self.filter
        } else {
            &mut self.selection.note
        };
        if pressed(InputCode::key_enter) {
            self.typing = false;
        } else if pressed(InputCode::key_escape) {
            self.typing = false;
            words.clear();
        } else if pressed(InputCode::key_backspace) {
            words.pop();
        } else if pressed(InputCode::key_space) {
            words.push(' ');
        } else if words.len() < NOTE_MAX {
            for (code, letter) in NOTE_KEYS {
                if pressed(code) {
                    // Shift gives a capital, or the sign on a digit key.
                    let sign = NOTE_SIGNS.iter().find(|(digit, _)| *digit == letter).map(|(_, sign)| *sign);
                    words.push(match (shift, sign) {
                        (true, Some(sign)) => sign,
                        (true, None) => letter.to_ascii_uppercase(),
                        (false, _) => letter,
                    });
                }
            }
            for (code, plain, shifted) in NOTE_MARKS {
                if pressed(code) {
                    words.push(if shift { shifted } else { plain });
                }
            }
        }
        true
    }

    /// Apply [`Self::orthographic`] to the camera, keeping what is framed.
    pub fn set_projection(&mut self, camera: &mut Camera) {
        match &mut self.orbit {
            Some(orbit) => orbit.write(camera, self.orthographic),
            None => self.frame(camera),
        }
    }

    /// Read the recorded sessions again, for the Replay panel. Called at the
    /// workspace poll, not every frame: it reads the session index files.
    pub fn poll_sessions(&mut self) {
        self.recorded = genos_workbench::sessions(&self.dir);
        let samples = genos_scene_lib::repo_root().join("debug/scenes/mesh");
        self.found = find_workspaces(&self.dir, &[samples, out_dir()]);
    }

    /// One line on the mesh that is shown, for the window's readout.
    pub fn statistics(&self) -> String {
        self.workspace.as_ref().map_or_else(
            || "no workspace".to_string(),
            |w| {
                let triangles: usize = w.parts.iter().map(Part::triangles).sum();
                format!("rev {} - {} objects, {} faces, {} tris, {} verts", w.revision, w.parts.len(), w.faces, triangles, w.vertices)
            },
        )
    }

    /// What the picture is and whether a session is open, for the readout.
    pub fn state(&self) -> String {
        let shown = match (&self.preview, &self.replay) {
            (Some(preview), _) => format!("preview of {} commands", preview.commands),
            (None, Some(replay)) => format!("replay {} {}", replay.event, replay.command),
            (None, None) => "live".to_string(),
        };
        format!("{shown}, {}", if self.bridge.is_some() { "session open" } else { "viewing" })
    }

    fn passes(&self, name: &str) -> bool {
        self.filter.is_empty() || name.to_lowercase().contains(&self.filter.to_lowercase())
    }

    fn passes_command(&self, name: &str) -> bool {
        self.command_filter.is_empty() || name.to_lowercase().contains(&self.command_filter.to_lowercase())
    }

    /// The Objects panel: each collection with its objects under it, then the
    /// objects in no collection. The filter keeps an object whose name holds
    /// it, and every object of a collection whose name holds it. A line's
    /// check box shows or hides the object, or every object of the collection.
    pub fn object_lines(&self) -> Vec<PanelLine> {
        let Some(workspace) = &self.workspace else {
            return Vec::new();
        };
        let shown = |object: u64| !self.hidden.contains(&object);
        let line = |part: &Part, indent: &str| PanelLine {
            label: format!("{indent}{} {}", part.object, part.name),
            check: "show",
            on: shown(part.object),
            target: PanelTarget::Object(part.object),
            open: None,
        };
        let mut lines = Vec::new();
        for collection in &workspace.collections {
            let whole = self.passes(&collection.name);
            let members: Vec<&Part> = workspace
                .parts
                .iter()
                .filter(|part| collection.objects.contains(&part.object) && (whole || self.passes(&part.name)))
                .collect();
            if members.is_empty() && !(whole && self.filter.is_empty()) {
                continue;
            }
            // A fold hides the collection's objects; a filter lists what it found.
            let open = !self.folded.contains(&collection.id) || !self.filter.is_empty();
            lines.push(PanelLine {
                label: format!("{} ({})", collection.name, collection.objects.len()),
                check: "show",
                on: collection.objects.iter().any(|object| shown(*object)),
                target: PanelTarget::Collection(collection.id),
                open: Some(open),
            });
            if open {
                lines.extend(members.into_iter().map(|part| line(part, "    ")));
            }
        }
        let grouped = |object: u64| workspace.collections.iter().any(|c| c.objects.contains(&object));
        lines.extend(
            workspace.parts.iter().filter(|part| !grouped(part.object) && self.passes(&part.name)).map(|part| line(part, "")),
        );
        lines.truncate(PANEL_OBJECTS);
        lines
    }

    /// The session the Replay panel lists: the replayed one, else the latest.
    fn panel_session(&self) -> Option<&genos_workbench::Session> {
        match &self.replay {
            Some(replay) => self.recorded.iter().find(|session| session.id == replay.session),
            None => self.recorded.last(),
        }
    }

    /// The Replay panel: "live", then the latest commands of the session whose
    /// names hold the filter, newest first. A line's check box shows the mesh
    /// as it stood after that command.
    pub fn replay_lines(&self) -> Vec<PanelLine> {
        let Some(session) = self.panel_session() else {
            return Vec::new();
        };
        let at = self.replay.as_ref().map(|replay| replay.event);
        let mut lines = vec![PanelLine {
            label: "live".into(),
            check: "shown",
            on: at.is_none(),
            target: PanelTarget::Live,
            open: None,
        }];
        lines.extend(session.events.iter().rev().filter(|event| self.passes_command(&event.command)).take(PANEL_EVENTS).map(|event| {
            PanelLine {
                label: format!("{} {}{}", event.id, event.command, if event.ok { "" } else { " (refused)" }),
                check: "shown",
                on: at == Some(event.id),
                target: PanelTarget::Event(event.id),
                open: None,
            }
        }));
        lines
    }

    /// A check box of a panel line was set to `on`.
    /// A collection's "open" box was set to `open`.
    pub fn panel_fold(&mut self, target: PanelTarget, open: bool) {
        if let PanelTarget::Collection(id) = target {
            if open {
                self.folded.remove(&id);
            } else {
                self.folded.insert(id);
            }
        }
    }

    pub fn panel_click(&mut self, target: PanelTarget, on: bool) -> Result<(), String> {
        let objects = match target {
            PanelTarget::Object(object) => vec![object],
            PanelTarget::Collection(id) => self
                .workspace
                .iter()
                .flat_map(|w| &w.collections)
                .find(|collection| collection.id == id)
                .map(|collection| collection.objects.clone())
                .unwrap_or_default(),
            PanelTarget::Event(event) if on => {
                let session = self.panel_session().map(|session| session.id.clone()).ok_or("no session is recorded")?;
                return self.replay_at(&session, event);
            }
            // Unchecking the shown command, or checking "live": the live mesh.
            PanelTarget::Event(_) | PanelTarget::Live => {
                self.rebuild = self.replay.take().is_some();
                return Ok(());
            }
        };
        for object in objects {
            if on {
                self.hidden.remove(&object);
            } else {
                self.hidden.insert(object);
            }
        }
        self.display_stale = true;
        self.overlay_stale = true;
        Ok(())
    }

    /// The Replay panel's session picker: the ids, newest first, and which is listed.
    pub fn session_choices(&self) -> (Vec<String>, usize) {
        let ids: Vec<String> = self.recorded.iter().rev().map(|session| session.id.clone()).collect();
        let at = self.panel_session().and_then(|shown| ids.iter().position(|id| *id == shown.id)).unwrap_or(0);
        (ids, at)
    }

    /// Show the end of the session at `index` of [`Self::session_choices`].
    pub fn pick_session(&mut self, index: usize) -> Result<(), String> {
        let (ids, _) = self.session_choices();
        let id = ids.get(index).ok_or("no such session")?;
        let last = self.recorded.iter().find(|session| session.id == *id).and_then(|s| s.events.last()).map_or(0, |e| e.id);
        self.replay_at(&id.clone(), last)
    }

    /// Whether the clip is playing.
    pub fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    /// Play `clip` in a loop, or stop with `None` (the mesh keeps its pose).
    /// The clip is sampled once, `PLAY_RATE` times a second of clip, and the
    /// frames between are blended from those samples: asking mesh-workbench
    /// for every drawn frame would record a command each time.
    pub fn play(&mut self, clip: Option<String>) -> Result<(), String> {
        let Some(clip) = clip else {
            self.playing = None;
            return Ok(());
        };
        let length = self
            .workspace
            .iter()
            .flat_map(|w| &w.clips)
            .find(|c| c.name == clip)
            .map(|c| c.duration)
            .ok_or_else(|| format!("unknown clip {clip}"))?;
        let count = crate::round_index(f64::from((length * PLAY_RATE).ceil())).clamp(1, PLAY_SAMPLES_MAX);
        let mut samples = Vec::with_capacity(count + 1);
        for index in 0..=count {
            #[expect(clippy::cast_precision_loss, reason = "a small frame count")]
            let time = length * index as f32 / count as f32;
            self.set_pose(Some((clip.clone(), time)))?;
            if let Some(posed) = self.posed.clone() {
                samples.push((time, posed));
            }
        }
        self.playing = Some(Playback { clip, length, time: 0.0, samples });
        Ok(())
    }

    /// One frame of time: advance a playing clip and a playing replay.
    pub fn tick(&mut self, dt: f32) {
        if let Some(playback) = &mut self.playing {
            playback.time = if playback.length > 0.0 { (playback.time + dt) % playback.length } else { 0.0 };
            if let Some(posed) = playback.blend() {
                self.pose = Some((playback.clip.clone(), playback.time));
                self.posed = Some(posed);
                self.display_stale = true;
                self.overlay_stale = true;
            }
        }
        if let Some(wait) = &mut self.replay_playing {
            *wait -= dt;
            if *wait <= 0.0 {
                // The same pace as the mesh-workbench web viewer.
                *wait = REPLAY_STEP_SECONDS;
                let at_live = match self.replay_span() {
                    Ok((_, last)) => self.replay.as_ref().is_none_or(|r| r.event >= last),
                    Err(_) => true,
                };
                let next = self.replay.as_ref().map_or(0, |r| r.event + 1);
                if at_live || self.scrub(next).is_err() {
                    // Reached the last command: show the live mesh and stop.
                    self.replay_playing = None;
                    if self.replay.take().is_some() {
                        self.rebuild = true;
                    }
                    set_source(None);
                }
            }
        }
    }

    /// Show only the objects whose name contains `text`, or hide them.
    /// Letter case is ignored.
    fn match_names(&mut self, text: &str, solo: bool) {
        let want = text.to_lowercase();
        let hit: Vec<u64> = self
            .workspace
            .iter()
            .flat_map(|w| &w.parts)
            .filter(|part| part.name.to_lowercase().contains(&want))
            .map(|part| part.object)
            .collect();
        if solo {
            self.hidden = self
                .workspace
                .iter()
                .flat_map(|w| w.parts.iter().map(|p| p.object))
                .filter(|object| !hit.contains(object))
                .collect();
        } else {
            self.hidden.extend(hit);
        }
        self.display_stale = true;
    }

    /// Pose the mesh at `time` seconds of `clip`, or at rest with `None`.
    /// The posed vertices and joints come from mesh-workbench's
    /// `animation_sample`, so this opens the session, and each call is one
    /// recorded command.
    pub fn set_pose(&mut self, pose: Option<(String, f32)>) -> Result<(), String> {
        self.display_stale = true;
        self.overlay_stale = true;
        let Some((clip, time)) = pose else {
            self.pose = None;
            self.posed = None;
            return Ok(());
        };
        let mut posed = Posed::default();
        for kind in ["vertices", "bones"] {
            let mut offset = 0u64;
            loop {
                let request = obj(vec![
                    ("command", text("animation_sample")),
                    ("clip", text(clip.clone())),
                    ("time", num(f64::from(time))),
                    ("kind", text(kind)),
                    ("offset", id(offset)),
                    ("limit", id(POSE_PAGE)),
                ]);
                let reply = self.run(&request)?;
                if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                    let why = reply.get("error").and_then(Value::as_str).unwrap_or("animation_sample was refused");
                    return Err(why.to_string());
                }
                let result = reply.get("result").ok_or("animation_sample gave no result")?;
                for item in result.get("items").and_then(Value::as_array).unwrap_or(&[]) {
                    let Some(position) = item.get("position").and_then(point) else {
                        continue;
                    };
                    if let Some(vertex) = item.get("id").and_then(Value::as_u64) {
                        posed.vertices.insert(vertex, position);
                    } else if let Some(name) = item.get("name").and_then(Value::as_str) {
                        posed.joints.insert(name.to_string(), position);
                    }
                }
                match result.get("next_offset").and_then(Value::as_u64) {
                    Some(next) if next > offset => offset = next,
                    _ => break,
                }
            }
        }
        self.pose = Some((clip, time));
        self.posed = Some(posed);
        Ok(())
    }

    /// Bones as thin strips from each joint to its parent's, posed when a
    /// pose is set. Empty when bones are off or the mesh has no rig.
    pub fn bone_strips(&self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let Some(workspace) = &self.workspace else {
            return (positions, normals);
        };
        if !self.bones {
            return (positions, normals);
        }
        let joint = |name: &str| {
            self.posed
                .as_ref()
                .and_then(|posed| posed.joints.get(name).copied())
                .or_else(|| workspace.bones.iter().find(|bone| bone.name == name).map(|bone| bone.position))
        };
        let size = (0..3).map(|a| workspace.max[a] - workspace.min[a]).fold(0.0f32, f32::max);
        let half = (size * BONE_WIDTH).max(OVERLAY_OFFSET);
        for bone in &workspace.bones {
            let Some(at) = joint(&bone.name) else {
                continue;
            };
            octahedron(at, half * 2.0, &mut positions, &mut normals);
            if let Some(parent) = bone.parent.as_deref().and_then(joint) {
                strip(parent, at, half, &mut positions, &mut normals);
            }
        }
        (positions, normals)
    }

    /// Grid lines on the ground under the mesh, in mesh coordinates: a line
    /// every power of ten of metres that fits the mesh (1 m for a mesh a few
    /// metres across), out to `GRID_REACH` mesh sizes. Empty when the grid is off.
    pub fn grid_strips(&self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        if !self.grid {
            return (positions, normals);
        }
        let (min, max) = self.workspace.as_ref().map_or(([-0.5, 0.0, -0.5], [0.5, 1.0, 0.5]), |w| (w.min, w.max));
        let size = (0..3).map(|a| max[a] - min[a]).fold(0.0f32, f32::max).max(0.1);
        let step = 10f32.powf(size.log10().floor());
        let reach = (size * GRID_REACH / step).ceil().min(GRID_LINES_MAX);
        let centre = [
            (f32::midpoint(min[0], max[0]) / step).round() * step,
            (f32::midpoint(min[2], max[2]) / step).round() * step,
        ];
        // The ground is the mesh's lowest point. Sit just above it.
        let y = min[1] + OVERLAY_OFFSET;
        let half = step * GRID_WIDTH;
        let span = reach * step;
        #[expect(clippy::cast_possible_truncation, reason = "a small line count")]
        let lines = reach as i32;
        for index in -lines..=lines {
            #[expect(clippy::cast_precision_loss, reason = "a small line count")]
            let offset = index as f32 * step;
            strip([centre[0] + offset, y, centre[1] - span], [centre[0] + offset, y, centre[1] + span], half, &mut positions, &mut normals);
            strip([centre[0] - span, y, centre[1] + offset], [centre[0] + span, y, centre[1] + offset], half, &mut positions, &mut normals);
        }
        (positions, normals)
    }

    /// The contact shadow: under each shown object, a patch on the ground
    /// that is `ground` at its rim and darker inside. It is drawn, not traced
    /// (the mesh has no distance field), so it does not follow the light. An
    /// object off the ground has a fainter patch. Positions, normals and a
    /// colour per vertex, three per triangle.
    #[expect(clippy::cast_precision_loss, reason = "small counts")]
    pub fn shadow_patches(&self, ground: [f32; 3]) -> Painted {
        let (mut positions, mut colors) = (Vec::new(), Vec::new());
        let Some(workspace) = self.workspace.as_ref().filter(|_| self.shadow && self.surfaces) else {
            return (positions, Vec::new(), colors);
        };
        let size = (0..3).map(|a| workspace.max[a] - workspace.min[a]).fold(0.0f32, f32::max).max(0.1);
        // Each shown object's box. Two patches that overlapped would lie at
        // the same height and flicker, so objects whose patches would touch
        // share one patch over them all.
        let mut boxes: Vec<([f32; 3], [f32; 3])> = Vec::new();
        for part in &workspace.parts {
            if self.hidden.contains(&part.object) || part.positions.is_empty() {
                continue;
            }
            let mut low = [f32::INFINITY; 3];
            let mut high = [f32::NEG_INFINITY; 3];
            for p in &part.positions {
                for axis in 0..3 {
                    low[axis] = low[axis].min(p[axis]);
                    high[axis] = high[axis].max(p[axis]);
                }
            }
            boxes.push((low, high));
        }
        // A patch reaches this far past its box's middle, on X and on Z.
        let reach_of = |low: &[f32; 3], high: &[f32; 3]| [0, 2].map(|axis| (high[axis] - low[axis]) * 0.5 * std::f32::consts::SQRT_2 * SHADOW_REACH);
        let touch = |a: &([f32; 3], [f32; 3]), b: &([f32; 3], [f32; 3])| {
            let (ra, rb) = (reach_of(&a.0, &a.1), reach_of(&b.0, &b.1));
            [0usize, 2].iter().zip(0..2).all(|(axis, k)| {
                let (ca, cb) = (f32::midpoint(a.0[*axis], a.1[*axis]), f32::midpoint(b.0[*axis], b.1[*axis]));
                (ca - cb).abs() <= ra[k] + rb[k]
            })
        };
        'merge: loop {
            for first in 0..boxes.len() {
                for second in first + 1..boxes.len() {
                    if touch(&boxes[first], &boxes[second]) {
                        let other = boxes.swap_remove(second);
                        let joined = &mut boxes[first];
                        for axis in 0..3 {
                            joined.0[axis] = joined.0[axis].min(other.0[axis]);
                            joined.1[axis] = joined.1[axis].max(other.1[axis]);
                        }
                        continue 'merge;
                    }
                }
            }
            break;
        }
        for (low, high) in boxes {
            let height = low[1] - workspace.min[1];
            let strength = SHADOW_DARK * (1.0 - height / size).clamp(0.0, 1.0);
            let centre = [f32::midpoint(low[0], high[0]), f32::midpoint(low[2], high[2])];
            // The ellipse through the footprint's corners, and a soft margin.
            let reach = reach_of(&low, &high);
            // Below the grid lines.
            let y = workspace.min[1] + OVERLAY_OFFSET * 0.4;
            let ring = |radius: f32, k: usize| {
                let turn = k as f32 / SHADOW_SIDES as f32 * std::f32::consts::TAU;
                [centre[0] + turn.cos() * reach[0] * radius, y, centre[1] + turn.sin() * reach[1] * radius]
            };
            let shade = |dark: f32| ground.map(|c| c * (1.0 - strength * dark));
            // (radius, darkness) from the centre out. The rim is the ground's colour.
            for pair in SHADOW_RINGS.windows(2) {
                let ((r0, d0), (r1, d1)) = (pair[0], pair[1]);
                for k in 0..SHADOW_SIDES {
                    // Wound to face up.
                    positions.extend([ring(r0, k), ring(r1, k + 1), ring(r1, k), ring(r0, k), ring(r0, k + 1), ring(r1, k + 1)]);
                    colors.extend([shade(d0), shade(d1), shade(d1), shade(d0), shade(d0), shade(d1)]);
                }
            }
        }
        let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
        (positions, normals, colors)
    }

    /// Polygon edges as thin crossed strips, for the wireframe mode. Empty in
    /// the other modes. Positions then normals, three per triangle.
    pub fn wire(&self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let Some(workspace) = &self.workspace else {
            return (positions, normals);
        };
        let triangles = self.shade == "tessellation";
        if self.shade != "wireframe" && !triangles {
            return (positions, normals);
        }
        let size = (0..3).map(|a| workspace.max[a] - workspace.min[a]).fold(0.0f32, f32::max);
        let half = (size * WIRE_WIDTH).max(OVERLAY_OFFSET * 0.5);
        for part in &workspace.parts {
            if self.hidden.contains(&part.object) {
                continue;
            }
            // An edge follows the pose of its two vertices.
            let at = |vertex: u64, rest: [f32; 3]| {
                self.posed.as_ref().and_then(|posed| posed.vertices.get(&vertex).copied()).unwrap_or(rest)
            };
            if triangles {
                // Every edge of every triangle the picture draws, the
                // diagonals inside a polygon included, each once.
                let mut drawn = BTreeSet::new();
                for corner in 0..part.positions.len() {
                    let next = corner - corner % 3 + (corner + 1) % 3;
                    let (va, vb) = (part.vertex_ids[corner], part.vertex_ids[next]);
                    if drawn.insert((va.min(vb), va.max(vb))) {
                        strip(at(va, part.positions[corner]), at(vb, part.positions[next]), half, &mut positions, &mut normals);
                    }
                }
                continue;
            }
            for ([a, b], [va, vb]) in part.edges.iter().zip(&part.edge_ids) {
                strip(at(*va, *a), at(*vb, *b), half, &mut positions, &mut normals);
            }
        }
        (positions, normals)
    }

    /// The session a step moves in (the replayed one, else the latest) and
    /// the number of its last command.
    pub fn replay_span(&self) -> Result<(String, u64), String> {
        let sessions = genos_workbench::sessions(&self.dir);
        let found = match &self.replay {
            Some(replay) => sessions.iter().find(|s| s.id == replay.session),
            None => sessions.last(),
        }
        .ok_or_else(|| format!("no session recorded under {}", self.dir.display()))?;
        Ok((found.id.clone(), found.events.last().map_or(0, |event| event.id)))
    }

    /// Move the replay to command `event` of the current session; past its
    /// last command is the live mesh. For the menu's scrubber.
    pub fn scrub(&mut self, event: u64) -> Result<(), String> {
        let (session, last) = self.replay_span()?;
        if event > last {
            self.rebuild = self.replay.take().is_some();
        } else {
            self.replay_at(&session, event)?;
        }
        self.sync_source();
        Ok(())
    }

    /// Show a recorded state: `session` is a session ID or `latest`, `event`
    /// the command to stand after (0 is the state the session opened with).
    fn replay_at(&mut self, session: &str, event: u64) -> Result<(), String> {
        let sessions = genos_workbench::sessions(&self.dir);
        let found = if session == "latest" {
            sessions.last()
        } else {
            sessions.iter().find(|s| s.id == session)
        }
        .ok_or_else(|| format!("no session {session} under {}", self.dir.display()))?;
        // The state after `event` is the last snapshot at or before it.
        let mut snapshot = found.initial_snapshot;
        let mut command = String::from("session start");
        for item in found.events.iter().filter(|item| item.id <= event) {
            if let Some(after) = item.snapshot {
                snapshot = after;
            }
            command.clone_from(&item.command);
        }
        if event > found.events.last().map_or(0, |item| item.id) {
            return Err(format!("session {} has {} events", found.id, found.events.len()));
        }
        self.replay = Some(Replay { session: found.id.clone(), event, snapshot, command });
        self.rebuild = true;
        Ok(())
    }

    /// For each object of the workspace, in order: whether the picture
    /// leaves it out, and its colour.
    /// The positions each object is drawn at, in order: posed when a pose is
    /// set, else `None` for the rest positions the picture already has.
    pub fn posed_positions(&self) -> Vec<Option<Vec<[f32; 3]>>> {
        let Some(workspace) = &self.workspace else {
            return Vec::new();
        };
        workspace
            .parts
            .iter()
            .map(|part| {
                let posed = self.posed.as_ref()?;
                Some(
                    part.vertex_ids
                        .iter()
                        .zip(&part.positions)
                        .map(|(vertex, rest)| posed.vertices.get(vertex).copied().unwrap_or(*rest))
                        .collect(),
                )
            })
            .collect()
    }

    pub fn display(&self) -> Vec<Display> {
        let Some(workspace) = &self.workspace else {
            return Vec::new();
        };
        let checker = self.shade == "uv_checker";
        let textured = checker || matches!(self.shade.as_str(), "textured" | "material");
        let normals = if self.shade == "normals" { self.posed_normals() } else { Vec::new() };
        // mesh-workbench colours an ID by its place in ID order.
        let rank = |ids: BTreeSet<u64>| ids.into_iter().enumerate().map(|(at, id)| (id, at)).collect::<std::collections::BTreeMap<u64, usize>>();
        let objects = rank(workspace.parts.iter().map(|part| part.object).filter(|object| *object != 0).collect());
        let faces = rank(workspace.parts.iter().flat_map(|part| part.face_ids.iter().copied()).collect());
        let collections = rank(workspace.collections.iter().map(|collection| collection.id).collect());
        let tint = |ranks: &std::collections::BTreeMap<u64, usize>, id: u64| ranks.get(&id).map_or(MESH_COLOR, |place| id_color(*place));
        workspace
            .parts
            .iter()
            .enumerate()
            .map(|(at, part)| {
                let color = match self.shade.as_str() {
                    "object_ids" => tint(&objects, part.object),
                    // An unpainted object is white, as in mesh-workbench.
                    "vertex_colors" | "vertex_color_shaded" if part.colors.is_empty() => [1.0; 3],
                    // Black whatever the light: the outline against the ground and the sky.
                    "silhouette" => [0.0; 3],
                    // The first collection that holds the object; none is grey.
                    "collection_ids" => workspace
                        .collections
                        .iter()
                        .find(|collection| collection.objects.contains(&part.object))
                        .map_or(UNASSIGNED_COLOR, |collection| tint(&collections, collection.id)),
                    _ => MESH_COLOR,
                };
                let hidden = !self.surfaces || self.hidden.contains(&part.object);
                if textured {
                    let (uvs, colors, drawn) = textured_corners(part, &workspace.textures, checker);
                    let used: BTreeSet<usize> = drawn.iter().flatten().copied().collect();
                    let surface = match (checker, used.len()) {
                        (true, _) => SURFACE_CHECKER,
                        (false, 1) => SURFACE_TEXTURES + used.first().copied().unwrap_or(0),
                        // `textured_pieces` draws it: several textures, or
                        // unmapped faces in the lit shade.
                        _ if in_pieces(&self.shade, &colors, &drawn) => return (true, color, Vec::new(), None),
                        // No texture: the colours alone.
                        (false, 0) => return (hidden, color, colors, None),
                        (false, _) => return (true, color, Vec::new(), None),
                    };
                    return (hidden, color, colors, Some((surface, uvs)));
                }
                // One colour per vertex, or none for the object's one colour.
                let colors = match self.shade.as_str() {
                    "vertex_colors" | "vertex_color_shaded" => part.colors.clone(),
                    "face_ids" => part.face_ids.iter().flat_map(|face| [tint(&faces, *face); 3]).collect(),
                    // mesh-workbench's encoding: RGB = (N + 1) / 2, as display values.
                    "normals" => normals
                        .get(at)
                        .and_then(Option::as_ref)
                        .unwrap_or(&part.normals)
                        .iter()
                        .map(|n| n.map(|c| f32::midpoint(c, 1.0).clamp(0.0, 1.0).powf(2.2)))
                        .collect(),
                    "depth" => self.depths.get(at).cloned().unwrap_or_default(),
                    "face_orientation" => self
                        .facing
                        .get(at)
                        .into_iter()
                        .flatten()
                        .flat_map(|front| [if *front { FRONT_COLOR } else { BACK_COLOR }; 3])
                        .collect(),
                    _ => Vec::new(),
                };
                // The edge strips of a hidden object are left out in `wire`.
                (hidden, color, colors, None)
            })
            .collect()
    }

    /// The `depth` shade, as mesh-workbench encodes it: each vertex's
    /// distance along the view as a grey, 255 at the nearest and 32 at the
    /// farthest of what is shown. Made again when the camera or the mesh moves.
    fn watch_depth(&mut self, eye: [f32; 3]) {
        if self.shade != "depth" {
            self.depths.clear();
            return;
        }
        let Some(workspace) = &self.workspace else {
            return;
        };
        let eye = [eye[0], eye[1] - self.lift(), eye[2]];
        // The view runs from the eye to the middle of the mesh.
        let middle = [0, 1, 2].map(|axis| f32::midpoint(workspace.min[axis], workspace.max[axis]));
        let along = normalise([middle[0] - eye[0], middle[1] - eye[1], middle[2] - eye[2]]);
        let distance: Vec<Vec<f32>> = workspace
            .parts
            .iter()
            .map(|part| {
                (0..part.positions.len())
                    .map(|corner| {
                        let p = self.corner(part, corner);
                        (0..3).map(|k| (p[k] - eye[k]) * along[k]).sum()
                    })
                    .collect()
            })
            .collect();
        let shown = || workspace.parts.iter().zip(&distance).filter(|(part, _)| !self.hidden.contains(&part.object)).flat_map(|(_, d)| d.iter().copied());
        let (near, far) = (shown().fold(f32::INFINITY, f32::min), shown().fold(f32::NEG_INFINITY, f32::max));
        let span = (far - near).max(1.0e-6);
        let depths: Vec<Vec<[f32; 3]>> = distance
            .iter()
            .map(|part| {
                part.iter()
                    .map(|d| {
                        let code = (32.0 + 223.0 * ((far - d) / span).clamp(0.0, 1.0)) / 255.0;
                        // The picture takes linear colour; the code is a display value.
                        [code.powf(2.2); 3]
                    })
                    .collect()
            })
            .collect();
        if depths != self.depths {
            self.depths = depths;
            self.display_stale = true;
        }
    }

    /// The ground's and the sky's colour for the shade, and whether the grid
    /// and the contact shadow are drawn, as the web viewer clears: white for
    /// `silhouette`, black for `depth`, its dark background for the other
    /// unlit shades. `None` is the scene's own lit ground and sky.
    pub fn backdrop(&self) -> Option<([f32; 3], bool)> {
        match self.shade.as_str() {
            "silhouette" => Some(([1.0; 3], false)),
            "depth" => Some(([0.0; 3], false)),
            _ if self.unlit() => Some((WEB_BACKGROUND, true)),
            _ => None,
        }
    }

    /// The shades mesh-workbench draws without light: the program shows the
    /// base colour for them.
    pub fn unlit(&self) -> bool {
        // The ID colours and the silhouette carry no light in mesh-workbench either.
        matches!(
            self.shade.as_str(),
            "textured" | "uv_checker" | "vertex_colors" | "object_ids" | "face_ids" | "collection_ids" | "silhouette" | "normals" | "depth"
        )
    }

    /// In the `textured` and `material` shades, each part that uses several textures, as one
    /// piece a texture and one for its faces with none. Positions are posed.
    pub fn textured_pieces(&self) -> Vec<Piece> {
        let Some(workspace) = self.workspace.as_ref().filter(|_| matches!(self.shade.as_str(), "textured" | "material") && self.surfaces) else {
            return Vec::new();
        };
        let posed = self.posed_normals();
        let mut pieces = Vec::new();
        for (index, part) in workspace.parts.iter().enumerate() {
            if self.hidden.contains(&part.object) {
                continue;
            }
            let (uvs, colors, drawn) = textured_corners(part, &workspace.textures, false);
            if !in_pieces(&self.shade, &colors, &drawn) {
                continue;
            }
            // A group is a texture (or none), and whether its faces lack UVs.
            let group = |triangle: usize| (drawn[triangle], colors[triangle * 3] == UNMAPPED_COLOR);
            let groups: BTreeSet<(Option<usize>, bool)> = (0..part.triangles()).map(group).collect();
            let normals = posed.get(index).and_then(Option::as_ref).unwrap_or(&part.normals);
            for (texture, unmapped) in groups {
                let corners: Vec<usize> =
                    (0..part.triangles()).filter(|t| group(*t) == (texture, unmapped)).flat_map(|t| t * 3..t * 3 + 3).collect();
                // In the lit shade an unmapped face is black and gives off
                // magenta, so no light changes it.
                let glows = unmapped && self.shade == "material";
                pieces.push(Piece {
                    positions: corners.iter().map(|at| self.corner(part, *at)).collect(),
                    normals: corners.iter().map(|at| normals.get(*at).copied().unwrap_or([0.0, 1.0, 0.0])).collect(),
                    colors: corners.iter().map(|at| if glows { [0.0; 3] } else { colors[*at] }).collect(),
                    texcoords: corners.iter().map(|at| uvs[*at]).collect(),
                    surface: texture.map(|at| SURFACE_TEXTURES + at),
                    emission: if glows { UNMAPPED_GLOW } else { [0.0; 3] },
                });
            }
        }
        pieces
    }

    /// Normals for each object's posed positions, or `None` at rest.
    pub fn posed_normals(&self) -> Vec<Option<Vec<[f32; 3]>>> {
        let Some(workspace) = &self.workspace else {
            return Vec::new();
        };
        workspace.parts.iter().zip(self.posed_positions()).map(|(part, at)| Some(posed_normals_of(part, &at?))).collect()
    }

    /// Centre and size of the mesh in the picture (lifted onto the ground).
    fn extent(&self) -> ([f32; 3], f32) {
        let Some(w) = &self.workspace else {
            return ([0.0, 0.5, 0.0], 1.0);
        };
        let lift = -w.min[1];
        let centre = [
            f32::midpoint(w.min[0], w.max[0]),
            f32::midpoint(w.min[1], w.max[1]) + lift,
            f32::midpoint(w.min[2], w.max[2]),
        ];
        let size = (0..3).map(|a| w.max[a] - w.min[a]).fold(0.0f32, f32::max);
        (centre, size.max(0.1))
    }

    /// Put the whole mesh in view from the camera's current direction.
    pub fn frame(&mut self, camera: &mut Camera) {
        let (centre, size) = self.extent();
        let mut orbit = Orbit {
            target: centre,
            distance: size * ORBIT_FRAME,
            last: [0.0; 2],
            dragged: None,
            panning: false,
            wrote: ([0.0; 3], 0.0, 0.0),
        };
        orbit.write(camera, self.orthographic);
        self.orbit = Some(orbit);
    }

    /// One frame of the pointer. Moves the camera, and on a click picks under
    /// the pointer with [`MeshView::pick_mode`]. `size` is the picture in pixels.
    /// The axis gizmo in the picture's top right corner, as the web viewer
    /// has it: an end for each side of each axis, placed by how the camera
    /// is turned, and the projection's name under it. A click on an end looks
    /// from that side; a click on the name changes the projection. Pixels.
    pub fn gizmo(&self, camera: &Camera, size: [f32; 2]) -> Vec<GizmoEnd> {
        let forward = genos_scene::look_direction(camera.yaw, camera.pitch);
        let right = forward.cross(genos_scene::Vec3::Y);
        let length = right.length().max(1.0e-6);
        let right = right / length;
        let up = right.cross(forward);
        let centre = [size[0] - self.hud_inset[0] - GIZMO_INSET, self.hud_inset[1] + GIZMO_INSET];
        let mut ends = Vec::with_capacity(7);
        for (axis, name, color) in [(0, "X", [0.95, 0.42, 0.45]), (1, "Y", [0.62, 0.85, 0.45]), (2, "Z", [0.45, 0.65, 1.0])] {
            let unit: [f32; 3] = std::array::from_fn(|k| if k == axis { 1.0 } else { 0.0 });
            let across = unit[0] * right.x + unit[1] * right.y + unit[2] * right.z;
            let rise = unit[0] * up.x + unit[1] * up.y + unit[2] * up.z;
            let toward = -(unit[0] * forward.x + unit[1] * forward.y + unit[2] * forward.z);
            for (sign, label, look) in [(1.0, name.to_string(), GIZMO_LOOKS[axis * 2]), (-1.0, format!("-{name}"), GIZMO_LOOKS[axis * 2 + 1])] {
                ends.push(GizmoEnd {
                    at: [centre[0] + across * sign * GIZMO_ARM, centre[1] - rise * sign * GIZMO_ARM],
                    half: [GIZMO_DOT, GIZMO_DOT],
                    label,
                    // The far ends are dimmer, as in the web viewer.
                    color: if sign > 0.0 { color } else { color.map(|c| c * 0.45) },
                    look: Some(look),
                    panel: false,
                    line_from: (sign > 0.0).then_some(centre),
                    depth: toward * sign,
                });
            }
        }
        // Nearer ends draw last, over the far ones.
        ends.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        let name = if self.orthographic { "ORTHOGRAPHIC" } else { "PERSPECTIVE" };
        #[expect(clippy::cast_precision_loss, reason = "a short word")]
        let half_wide = name.len() as f32 * 3.0 + 6.0;
        ends.push(GizmoEnd {
            at: [centre[0], centre[1] + GIZMO_ARM + 22.0],
            half: [half_wide, 8.0],
            label: name.to_string(),
            color: [0.8, 0.84, 0.9],
            look: None,
            panel: false,
            line_from: None,
            depth: 2.0,
        });
        // Under it, the button that opens and closes the Objects panel, as
        // the web viewer's toolbar has.
        ends.push(GizmoEnd {
            at: [centre[0], centre[1] + GIZMO_ARM + 44.0],
            half: [27.0, 8.0],
            label: "OBJECTS".to_string(),
            color: if self.panel_open { [1.0, 0.82, 0.35] } else { [0.8, 0.84, 0.9] },
            look: None,
            panel: true,
            line_from: None,
            depth: 3.0,
        });
        ends
    }

    /// The words the picture carries, as the web viewer's corners do: the
    /// workspace with what is shown and its revision, the counts, and how the
    /// pointer moves the view.
    pub fn hud(&self) -> [String; 3] {
        let name = self.dir.file_name().map_or_else(|| self.dir.display().to_string(), |name| name.to_string_lossy().into_owned());
        let shown = match (&self.preview, &self.replay) {
            (Some(_), _) => "PREVIEW".to_string(),
            (None, Some(replay)) => format!("REPLAY {}", replay.event),
            (None, None) => "LIVE".to_string(),
        };
        let revision = self.workspace.as_ref().map_or(0, |w| w.revision);
        let counts = self.workspace.as_ref().map_or_else(String::new, |w| format!("{} OBJECTS  {} VERTICES  {} FACES", w.parts.len(), w.vertices, w.faces));
        [
            format!("{name}   {shown}   REV {revision}").to_uppercase(),
            counts,
            "DRAG ORBITS   MIDDLE-DRAG PANS   WHEEL ZOOMS   RIGHT-DRAG BOX   ALT-DRAG LASSO   T NOTE".to_string(),
        ]
    }

    /// The ID labels of the topology shade (`wireframe`), as mesh-workbench's
    /// `topology` mode shows them: each face's ID at its centre and each
    /// vertex's ID at the vertex, for what the camera sees. Made again only
    /// when the camera, the mesh or what is hidden changes.
    pub fn id_labels(&mut self, camera: &Camera, aspect: f32) -> &[Label] {
        if self.shade != "wireframe" || !self.labels {
            self.label_cache = None;
            return &[];
        }
        let key: LabelKey = (
            [camera.position.x, camera.position.y, camera.position.z, camera.yaw, camera.pitch, camera.parallel_half, aspect],
            self.workspace.as_ref().map_or(0, |w| w.revision),
            self.hidden.len(),
            self.posed.is_some(),
        );
        if self.label_cache.as_ref().is_none_or(|(made_for, _)| *made_for != key) {
            let labels = self.make_labels(camera, aspect);
            self.label_cache = Some((key, labels));
        }
        self.label_cache.as_ref().map_or(&[], |(_, labels)| labels)
    }

    fn make_labels(&self, camera: &Camera, aspect: f32) -> Vec<Label> {
        let lift = self.lift();
        let on_screen = |p: [f32; 3]| genos_scene::viewport_uv(camera, aspect, [p[0], p[1] + lift, p[2]]).filter(|uv| (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
        let (mut faces, mut vertices) = (Vec::new(), Vec::new());
        for part in self.workspace.iter().flat_map(|w| &w.parts).filter(|part| !self.hidden.contains(&part.object)) {
            let mut centres: std::collections::BTreeMap<u64, ([f32; 3], f32)> = std::collections::BTreeMap::new();
            let mut seen = BTreeSet::new();
            for (corner, face) in part.face_ids.iter().flat_map(|face| [face; 3]).enumerate() {
                let p = self.corner(part, corner);
                let sum = centres.entry(*face).or_insert(([0.0; 3], 0.0));
                for (total, place) in sum.0.iter_mut().zip(p) {
                    *total += place;
                }
                sum.1 += 1.0;
                let vertex = part.vertex_ids[corner];
                if vertices.len() < LABELS_MOST && seen.insert(vertex) {
                    if let Some(at) = on_screen(p) {
                        // In sight: the surface under it is one of its own faces.
                        if self.pick(camera, aspect, at).is_none_or(|hit| self.face_vertices(hit.face).contains(&vertex)) {
                            vertices.push(Label { text: vertex.to_string(), at, face: false });
                        }
                    }
                }
            }
            for (face, (sum, count)) in centres {
                if faces.len() == LABELS_MOST {
                    break;
                }
                if let Some(at) = on_screen(sum.map(|c| c / count)) {
                    if self.pick(camera, aspect, at).is_some_and(|hit| hit.face == face) {
                        faces.push(Label { text: face.to_string(), at, face: true });
                    }
                }
            }
        }
        faces.extend(vertices);
        faces
    }

    /// A right drag (a box), or a left drag with Control (a box) or Alt (a
    /// lasso). Returns whether the drag has the left button this frame. The
    /// release selects what is inside and in sight, by [`Self::pick_mode`];
    /// Shift adds to the selection. A right press released where it went
    /// down is not a box and leaves the selection as it is.
    fn drag_region(&mut self, camera: &Camera, size: [f32; 2], input: &OrbitInput) -> bool {
        let at = [input.x, input.y];
        let held = if self.region_right { input.right } else { input.left };
        match (&mut self.region, held) {
            (None, _) => {
                let free = !input.over_ui && self.orbit.is_none_or(|o| o.dragged.is_none());
                let keyed = input.left && (input.box_key || input.lasso_key);
                let right = input.right && !input.left && !input.middle;
                if !free || !(keyed || right) {
                    self.region_right = false;
                    return false;
                }
                self.region_right = !keyed;
                self.region_lasso = keyed && input.lasso_key && !input.box_key;
                self.region = Some(vec![at, at]);
                keyed
            }
            (Some(points), true) => {
                if self.region_lasso {
                    // A point every few pixels is the path.
                    let last = points[points.len() - 1];
                    if (last[0] - at[0]).abs() + (last[1] - at[1]).abs() >= LASSO_STEP {
                        points.push(at);
                    }
                } else {
                    points[1] = at;
                }
                !self.region_right
            }
            (Some(points), false) => {
                let travel = (points[0][0] - points[1][0]).abs() + (points[0][1] - points[1][1]).abs();
                let taken = !self.region_right;
                if taken || travel >= CLICK_PIXELS {
                    let outline: Vec<[f32; 2]> = self
                        .region_outline()
                        .into_iter()
                        .map(|p| [p[0] / size[0].max(1.0), p[1] / size[1].max(1.0)])
                        .collect();
                    let mode = self.pick_mode.clone();
                    self.select_inside(camera, size[0] / size[1].max(1.0), &outline, &mode, input.add, false);
                }
                self.region = None;
                self.region_right = false;
                taken
            }
        }
    }

    /// The outline being dragged, in pixels: a box's four corners or the
    /// lasso's path. Empty when nothing is dragged. The program draws it.
    pub fn region_outline(&self) -> Vec<[f32; 2]> {
        match &self.region {
            Some(points) if !self.region_lasso => {
                let (a, b) = (points[0], points[1]);
                vec![a, [b[0], a[1]], b, [a[0], b[1]]]
            }
            Some(points) => points.clone(),
            None => Vec::new(),
        }
    }

    /// Select what lies inside `outline` (a closed polygon in screen
    /// coordinates, 0..1, origin top left): faces by their centre, vertices by
    /// their place, objects that have a face inside. Unless `through`, only
    /// what the camera sees: a face or a vertex behind another surface is
    /// left out. Returns how many were added.
    pub fn select_inside(&mut self, camera: &Camera, aspect: f32, outline: &[[f32; 2]], mode: &str, add: bool, through: bool) -> usize {
        let lift = self.lift();
        let mut faces = BTreeSet::new();
        let mut vertices = BTreeSet::new();
        let mut objects = BTreeSet::new();
        if let Some(workspace) = &self.workspace {
            let on_screen = |p: [f32; 3]| genos_scene::viewport_uv(camera, aspect, [p[0], p[1] + lift, p[2]]).filter(|uv| inside(outline, *uv));
            for part in workspace.parts.iter().filter(|part| !self.hidden.contains(&part.object)) {
                if mode == "vertex" {
                    let mut seen = BTreeSet::new();
                    for (corner, vertex) in part.vertex_ids.iter().enumerate() {
                        if !seen.insert(*vertex) {
                            continue;
                        }
                        let Some(uv) = on_screen(self.corner(part, corner)) else {
                            continue;
                        };
                        // In sight: the surface under it is one of its own faces.
                        if through || self.pick(camera, aspect, uv).is_none_or(|hit| self.face_vertices(hit.face).contains(vertex)) {
                            vertices.insert(*vertex);
                        }
                    }
                    continue;
                }
                let mut sums: std::collections::BTreeMap<u64, ([f32; 3], f32)> = std::collections::BTreeMap::new();
                for (corner, face) in part.face_ids.iter().flat_map(|face| [face; 3]).enumerate() {
                    let p = self.corner(part, corner);
                    let sum = sums.entry(*face).or_insert(([0.0; 3], 0.0));
                    for (total, place) in sum.0.iter_mut().zip(p) {
                        *total += place;
                    }
                    sum.1 += 1.0;
                }
                for (face, (sum, count)) in sums {
                    let Some(uv) = on_screen(sum.map(|c| c / count)) else {
                        continue;
                    };
                    if through || self.pick(camera, aspect, uv).is_some_and(|hit| hit.face == face) {
                        faces.insert(face);
                        objects.insert(part.object);
                    }
                }
            }
        }
        if !add {
            let note = std::mem::take(&mut self.selection.note);
            self.selection = Selection { note, ..Selection::default() };
        }
        let (into, found) = match mode {
            "vertex" => (&mut self.selection.vertices, vertices),
            "object" => (&mut self.selection.objects, objects),
            _ => (&mut self.selection.faces, faces),
        };
        let before = into.len();
        into.extend(found);
        self.overlay_stale = true;
        into.len() - before
    }

    /// Grow the selection. `connected`: every face (or vertex) joined to a
    /// selected one through shared vertices (or edges). `loop`: from two
    /// selected faces that share an edge, the ring of four-sided faces that
    /// runs on through the opposite edges.
    fn grow(&mut self, how: &str) -> Result<usize, String> {
        let Some(workspace) = &self.workspace else {
            return Ok(0);
        };
        // Each face's vertices, by part.
        let mut faces: std::collections::BTreeMap<u64, BTreeSet<u64>> = std::collections::BTreeMap::new();
        for part in &workspace.parts {
            for (triangle, face) in part.face_ids.iter().enumerate() {
                faces.entry(*face).or_default().extend(part.vertex_ids[triangle * 3..triangle * 3 + 3].iter().copied());
            }
        }
        let before = self.selection.faces.len() + self.selection.vertices.len();
        match how {
            "connected" => {
                // Vertices reached: from the selected faces' and the selected vertices' own.
                let mut reached: BTreeSet<u64> = self.selection.vertices.clone();
                reached.extend(self.selection.faces.iter().filter_map(|face| faces.get(face)).flatten().copied());
                loop {
                    let size = reached.len();
                    for corners in faces.values() {
                        if corners.iter().any(|vertex| reached.contains(vertex)) {
                            reached.extend(corners.iter().copied());
                        }
                    }
                    if reached.len() == size {
                        break;
                    }
                }
                if !self.selection.vertices.is_empty() {
                    self.selection.vertices = reached.clone();
                }
                if !self.selection.faces.is_empty() {
                    self.selection.faces =
                        faces.iter().filter(|(_, corners)| corners.iter().all(|v| reached.contains(v))).map(|(face, _)| *face).collect();
                }
            }
            "loop" => {
                let picked: Vec<u64> = self.selection.faces.iter().copied().collect();
                let shared = |a: u64, b: u64| -> Vec<u64> {
                    match (faces.get(&a), faces.get(&b)) {
                        (Some(x), Some(y)) => x.intersection(y).copied().collect(),
                        _ => Vec::new(),
                    }
                };
                let pair = picked
                    .iter()
                    .flat_map(|a| picked.iter().map(move |b| (*a, *b)))
                    .find(|(a, b)| a < b && shared(*a, *b).len() == 2)
                    .ok_or("a loop needs two selected faces that share an edge")?;
                // Walk away from each of the two, through the edge opposite the one just crossed.
                for (mut from, mut at) in [pair, (pair.1, pair.0)] {
                    for _ in 0..faces.len() {
                        let Some(corners) = faces.get(&at).filter(|corners| corners.len() == 4) else {
                            break;
                        };
                        let crossed = shared(from, at);
                        let opposite: Vec<u64> = corners.iter().copied().filter(|v| !crossed.contains(v)).collect();
                        let next = faces
                            .iter()
                            .find(|(face, other)| **face != at && opposite.iter().all(|v| other.contains(v)))
                            .map(|(face, _)| *face);
                        let Some(next) = next else {
                            break;
                        };
                        if !self.selection.faces.insert(next) {
                            break;
                        }
                        from = at;
                        at = next;
                    }
                }
            }
            other => return Err(format!("mesh_selection grow is connected or loop, not {other}")),
        }
        self.overlay_stale = true;
        Ok((self.selection.faces.len() + self.selection.vertices.len()).saturating_sub(before))
    }

    /// Which way each triangle faces the eye, for the `face_orientation`
    /// shade. The winding gives the front, as in mesh-workbench. Nothing is
    /// culled, so a face wound the wrong way shows red from outside.
    pub fn watch(&mut self, eye: [f32; 3]) {
        self.watch_depth(eye);
        if self.shade != "face_orientation" {
            self.facing.clear();
            return;
        }
        let Some(workspace) = &self.workspace else {
            return;
        };
        let eye = [eye[0], eye[1] - self.lift(), eye[2]];
        let posed = self.posed_positions();
        let facing: Vec<Vec<bool>> = workspace
            .parts
            .iter()
            .enumerate()
            .map(|(at, part)| {
                let positions = posed.get(at).and_then(Option::as_ref).unwrap_or(&part.positions);
                positions
                    .chunks_exact(3)
                    .map(|t| {
                        let (a, b) = ([0, 1, 2].map(|k| t[1][k] - t[0][k]), [0, 1, 2].map(|k| t[2][k] - t[0][k]));
                        let normal = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                        (0..3).map(|k| normal[k] * (eye[k] - t[0][k])).sum::<f32>() >= 0.0
                    })
                    .collect()
            })
            .collect();
        if facing != self.facing {
            self.facing = facing;
            self.display_stale = true;
        }
    }

    pub fn pointer(&mut self, camera: &mut Camera, size: [f32; 2], input: OrbitInput) {
        if std::mem::take(&mut self.frame_next) {
            self.frame(camera);
        }
        if std::mem::take(&mut self.projection_stale) {
            self.set_projection(camera);
        }
        // In the parallel view every face is seen along the view: an eye very far back.
        let back = if camera.parallel_half > 0.0 { 1.0e5 } else { 0.0 };
        let along = genos_scene::look_direction(camera.yaw, camera.pitch);
        self.watch([camera.position.x - along.x * back, camera.position.y - along.y * back, camera.position.z - along.z * back]);
        let (centre, extent) = self.extent();
        let pose = ([camera.position.x, camera.position.y, camera.position.z], camera.yaw, camera.pitch);
        // A checkpoint, a script or an agent moved the camera: orbit about the
        // point in front of it at the mesh's distance.
        let moved = self.orbit.as_ref().is_none_or(|orbit| {
            (0..3).any(|a| (orbit.wrote.0[a] - pose.0[a]).abs() > ORBIT_SLACK)
                || (orbit.wrote.1 - pose.1).abs() > ORBIT_SLACK
                || (orbit.wrote.2 - pose.2).abs() > ORBIT_SLACK
        });
        if moved {
            let to = [centre[0] - pose.0[0], centre[1] - pose.0[1], centre[2] - pose.0[2]];
            let forward = genos_scene::look_direction(camera.yaw, camera.pitch);
            let along = (to[0] * forward.x + to[1] * forward.y + to[2] * forward.z).max(extent * 0.25);
            // The orbit's distance is the perspective view's. A parallel view
            // gives it by the height it shows, wherever its eye stands.
            let framing =
                if camera.parallel_half > 0.0 { camera.parallel_half / genos_scene::DEFAULT_VIEW_TAN } else { along };
            let previous = self.orbit;
            self.orbit = Some(Orbit {
                target: [pose.0[0] + forward.x * along, pose.0[1] + forward.y * along, pose.0[2] + forward.z * along],
                distance: framing,
                last: previous.map_or([input.x, input.y], |o| o.last),
                dragged: previous.and_then(|o| o.dragged),
                panning: previous.is_some_and(|o| o.panning),
                wrote: pose,
            });
        }
        self.tick(input.dt);
        if input.frame_key {
            self.frame(camera);
        }
        let dragging = self.drag_region(camera, size, &input);
        // A press takes the camera back from the turntable for that frame.
        let turning = self.turntable && !input.left && !input.right && !input.middle;
        let Some(orbit) = &mut self.orbit else {
            return;
        };
        let delta = [input.x - orbit.last[0], input.y - orbit.last[1]];
        orbit.last = [input.x, input.y];
        let mut changed = false;
        let mut click = false;
        // A box or lasso drag has the left button: it does not turn the camera.
        let left = input.left && !dragging;
        if dragging {
            orbit.dragged = None;
        }
        match (left, orbit.dragged) {
            (true, None) if !input.over_ui => orbit.dragged = Some(0.0),
            (true, Some(travel)) => {
                let travel = travel + delta[0].abs() + delta[1].abs();
                orbit.dragged = Some(travel);
                if travel >= CLICK_PIXELS {
                    camera.yaw += delta[0] * ORBIT_TURN;
                    camera.pitch = (camera.pitch - delta[1] * ORBIT_TURN)
                        .clamp(-genos_scene::PITCH_LIMIT, genos_scene::PITCH_LIMIT);
                    changed = true;
                }
            }
            (false, Some(travel)) => {
                click = travel < CLICK_PIXELS;
                orbit.dragged = None;
            }
            _ => {}
        }
        if input.middle && !input.left {
            if orbit.panning {
                let forward = genos_scene::look_direction(camera.yaw, camera.pitch);
                let right = forward.cross(genos_scene::Vec3::Y);
                let length = right.length().max(1.0e-6);
                let right = right / length;
                let up = right.cross(forward);
                let step = orbit.distance * ORBIT_PAN;
                for (axis, (r, u)) in [(right.x, up.x), (right.y, up.y), (right.z, up.z)].into_iter().enumerate() {
                    orbit.target[axis] += (-delta[0] * r + delta[1] * u) * step;
                }
                changed = true;
            }
            orbit.panning = !input.over_ui || orbit.panning;
        } else {
            orbit.panning = false;
        }
        if input.wheel != 0.0 && !input.over_ui {
            orbit.distance = (orbit.distance * (-input.wheel * ORBIT_ZOOM).exp()).max(ORBIT_NEAR);
            changed = true;
        }
        if turning {
            camera.yaw += TURNTABLE_RATE * input.dt;
            changed = true;
        }
        if changed {
            orbit.write(camera, self.orthographic);
        }
        // A click on the axis gizmo turns the view; it is not a pick.
        if click {
            if let Some(press) = self.gizmo(camera, size).into_iter().find(|end| end.holds([input.x, input.y])) {
                if press.panel {
                    self.panel_open = !self.panel_open;
                } else if let Some(from) = press.look {
                    let _ = self.look(camera, from);
                } else {
                    self.orthographic = !self.orthographic;
                    self.set_projection(camera);
                }
                return;
            }
        }
        if click {
            let at = [input.x / size[0].max(1.0), input.y / size[1].max(1.0)];
            let aspect = size[0] / size[1].max(1.0);
            let hit = self.pick(camera, aspect, at);
            if !input.add {
                let note = std::mem::take(&mut self.selection.note);
                self.selection = Selection { note, ..Selection::default() };
            }
            if let Some(hit) = hit {
                match self.pick_mode.as_str() {
                    "object" => toggle(&mut self.selection.objects, hit.object, input.add),
                    "vertex" => toggle(&mut self.selection.vertices, hit.vertex, input.add),
                    _ => toggle(&mut self.selection.faces, hit.face, input.add),
                }
                self.selection.point = Some(hit.point);
            }
            self.overlay_stale = true;
        }
    }

    /// Read the workspace again. IDs the mesh no longer has leave the selection.
    pub fn reload(&mut self, stamp: Option<SystemTime>) {
        self.stamp = stamp;
        self.rebuild = false;
        self.workspace = match (&self.preview, &self.replay) {
            // The preview's copy stands in for the workspace it was made from.
            (Some(preview), _) => genos_workbench::load_workspace(&preview.dir).ok().map(|mut shown| {
                shown.dir.clone_from(&self.dir);
                shown
            }),
            (None, Some(replay)) => genos_workbench::load_state(&self.dir, &replay.session, replay.snapshot).ok(),
            (None, None) => genos_workbench::load_workspace(&self.dir).ok(),
        };
        if let Some(workspace) = &self.workspace {
            let objects: BTreeSet<u64> = workspace.parts.iter().map(|p| p.object).collect();
            let faces: BTreeSet<u64> = workspace.parts.iter().flat_map(|p| p.face_ids.iter().copied()).collect();
            let vertices: BTreeSet<u64> = workspace.parts.iter().flat_map(|p| p.vertex_ids.iter().copied()).collect();
            self.selection.objects.retain(|id| objects.contains(id));
            self.selection.faces.retain(|id| faces.contains(id));
            self.selection.vertices.retain(|id| vertices.contains(id));
        }
        self.overlay_stale = true;
        self.display_stale = true;
        // The mesh changed under the pose: sample it again.
        self.posed = None;
        self.repose = self.pose.is_some();
    }

    /// Sample the pose again after a reload. A clip the edit removed leaves
    /// the mesh at rest.
    pub fn repose(&mut self) {
        if std::mem::take(&mut self.repose) {
            let pose = self.pose.clone();
            if self.set_pose(pose).is_err() {
                self.pose = None;
                self.posed = None;
            }
        }
    }

    /// Where a corner of a part is drawn: its posed place when a pose is
    /// set, else its rest place. Picking and the highlight use this, so
    /// they follow the pose.
    fn corner(&self, part: &Part, corner: usize) -> [f32; 3] {
        self.posed
            .as_ref()
            .and_then(|posed| posed.vertices.get(&part.vertex_ids[corner]).copied())
            .unwrap_or(part.positions[corner])
    }

    /// Metres the mesh is lifted so its lowest point stands on the ground.
    pub fn lift(&self) -> f32 {
        self.workspace.as_ref().map_or(0.0, |w| -w.min[1])
    }

    /// Triangles that mark the selection, in mesh coordinates: every face of a
    /// selected object, every selected face, and a marker on every selected
    /// vertex. Positions then normals, three per triangle.
    pub fn overlay(&self) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let Some(workspace) = &self.workspace else {
            return (positions, normals);
        };
        if self.selection.is_empty() {
            return (positions, normals);
        }
        let size = (0..3).map(|a| workspace.max[a] - workspace.min[a]).fold(0.0f32, f32::max);
        let mark = (size * VERTEX_MARK).max(OVERLAY_OFFSET);
        let mut marked = BTreeSet::new();
        for part in &workspace.parts {
            let whole = self.selection.objects.contains(&part.object);
            for (triangle, face) in part.face_ids.iter().enumerate() {
                if whole || self.selection.faces.contains(face) {
                    for corner in triangle * 3..triangle * 3 + 3 {
                        let (p, n) = (self.corner(part, corner), part.normals[corner]);
                        positions.push([p[0] + n[0] * OVERLAY_OFFSET, p[1] + n[1] * OVERLAY_OFFSET, p[2] + n[2] * OVERLAY_OFFSET]);
                        normals.push(n);
                    }
                }
            }
            for (corner, vertex) in part.vertex_ids.iter().enumerate() {
                if self.selection.vertices.contains(vertex) && marked.insert(*vertex) {
                    octahedron(self.corner(part, corner), mark, &mut positions, &mut normals);
                }
            }
        }
        (positions, normals)
    }

    /// The nearest triangle under screen point `at` (0..1, origin top left).
    fn pick(&self, camera: &Camera, aspect: f32, at: [f32; 2]) -> Option<Hit> {
        let workspace = self.workspace.as_ref()?;
        let m = genos_scene::view_proj(camera, aspect);
        let lift = self.lift();
        let mut best: Option<Hit> = None;
        for part in &workspace.parts {
            for triangle in 0..part.triangles() {
                let mut screen = [[0.0f32; 3]; 3];
                let mut ahead = true;
                let corners: [[f32; 3]; 3] = std::array::from_fn(|k| self.corner(part, triangle * 3 + k));
                for (slot, p) in screen.iter_mut().zip(&corners) {
                    match project(&m, [p[0], p[1] + lift, p[2]]) {
                        Some(s) => *slot = s,
                        None => ahead = false,
                    }
                }
                if !ahead {
                    continue;
                }
                let Some(w) = barycentric(screen, at) else {
                    continue;
                };
                let depth = w[0] * screen[0][2] + w[1] * screen[1][2] + w[2] * screen[2][2];
                if best.as_ref().is_some_and(|b| b.depth <= depth) {
                    continue;
                }
                let mut point = [0.0f32; 3];
                for (weight, p) in w.iter().zip(&corners) {
                    for (sum, c) in point.iter_mut().zip(p) {
                        *sum += weight * c;
                    }
                }
                let nearest = (0..3)
                    .max_by(|a, b| w[*a].total_cmp(&w[*b]))
                    .unwrap_or(0);
                best = Some(Hit {
                    object: part.object,
                    name: part.name.clone(),
                    face: part.face_ids[triangle],
                    vertex: part.vertex_ids[triangle * 3 + nearest],
                    point,
                    depth,
                });
            }
        }
        best
    }

    fn face_vertices(&self, face: u64) -> Vec<u64> {
        let mut out = BTreeSet::new();
        if let Some(workspace) = &self.workspace {
            for part in &workspace.parts {
                for (triangle, id) in part.face_ids.iter().enumerate() {
                    if *id == face {
                        out.extend(part.vertex_ids[triangle * 3..triangle * 3 + 3].iter().copied());
                    }
                }
            }
        }
        out.into_iter().collect()
    }

    fn part(&self, object: u64) -> Option<&Part> {
        self.workspace.as_ref()?.parts.iter().find(|p| p.object == object)
    }

    /// Run one of the `mesh_*` commands.
    /// Look along an axis at the mesh, or frame it: `+x`, `-x`, `+y`, `-y`,
    /// `+z`, `-z` name the side the camera stands on. `frame` keeps the
    /// direction.
    fn look(&mut self, camera: &mut Camera, side: &str) -> Result<(), String> {
        use std::f32::consts::{FRAC_PI_2, PI};
        let steep = genos_scene::PITCH_LIMIT;
        // Yaw 0 looks along -Z and positive yaw turns toward +X.
        let (yaw, pitch) = match side {
            "+z" => (0.0, 0.0),
            "-z" => (PI, 0.0),
            "+x" => (-FRAC_PI_2, 0.0),
            "-x" => (FRAC_PI_2, 0.0),
            "+y" => (camera.yaw, -steep),
            "-y" => (camera.yaw, steep),
            "frame" => (camera.yaw, camera.pitch),
            other => return Err(format!("mesh_view look is +x, -x, +y, -y, +z, -z, or frame, not {other}")),
        };
        camera.yaw = yaw;
        camera.pitch = pitch;
        self.frame(camera);
        Ok(())
    }

    pub fn command(&mut self, name: &str, args: &Value, camera: &mut Camera, aspect: f32) -> Result<Value, String> {
        let args = Args(args);
        match name {
            "mesh_view" => {
                args.only("mesh_view", &VIEW_ARGS)?;
                for (key, solo) in [("solo_collection", true), ("hide_collection", false)] {
                    if let Some(value) = args.0.get(key) {
                        let want = value.as_u64().ok_or_else(|| format!("{key} is a collection id"))?;
                        let members: Vec<u64> = self
                            .workspace
                            .iter()
                            .flat_map(|w| &w.collections)
                            .find(|collection| collection.id == want)
                            .map(|collection| collection.objects.clone())
                            .ok_or_else(|| format!("unknown collection {want}"))?;
                        if solo {
                            self.hidden = self
                                .workspace
                                .iter()
                                .flat_map(|w| w.parts.iter().map(|p| p.object))
                                .filter(|object| !members.contains(object))
                                .collect();
                        } else {
                            self.hidden.extend(members);
                        }
                        self.display_stale = true;
                    }
                }
                // What a recorded command sent and what came back.
                if let Some(value) = args.0.get("event") {
                    let number = value.as_u64().ok_or("event is a command number of the session")?;
                    let (session, _) = self.replay_span()?;
                    return genos_workbench::load_event(&self.dir, &session, number);
                }
                if let Some(name) = args.str("solo_match")? {
                    self.match_names(&name, true);
                }
                if let Some(name) = args.str("hide_match")? {
                    self.match_names(&name, false);
                }
                if let Some(on) = args.bool("grid")? {
                    self.grid = on;
                    self.overlay_stale = true;
                }
                if let Some(dir) = args.str("workspace")? {
                    // The program opens it at its next look; a directory with
                    // no project.json is an empty workspace to build in.
                    let dir = PathBuf::from(dir);
                    let dir = if dir.is_absolute() { dir } else { std::env::current_dir().map_err(|err| err.to_string())?.join(dir) };
                    set_workspace(Some(dir));
                    self.rebuild = true;
                }
                if let Some(kind) = args.str("projection")? {
                    self.orthographic = match kind.as_str() {
                        "orthographic" => true,
                        "perspective" => false,
                        _ => return Err(format!("mesh_view projection is perspective or orthographic, not {kind}")),
                    };
                    self.set_projection(camera);
                }
                if let Some(words) = args.str("filter")? {
                    self.filter = words;
                }
                if let Some(on) = args.bool("labels")? {
                    self.labels = on;
                }
                if let Some(on) = args.bool("panel")? {
                    self.panel_open = on;
                }
                if let Some(words) = args.str("command_filter")? {
                    self.command_filter = words;
                }
                if let Some(on) = args.bool("shadow")? {
                    self.shadow = on;
                    self.overlay_stale = true;
                }
                if let Some(on) = args.bool("surfaces")? {
                    self.surfaces = on;
                    self.display_stale = true;
                }
                if let Some(on) = args.bool("bones")? {
                    self.bones = on;
                    self.overlay_stale = true;
                }
                match args.0.get("pose") {
                    Some(Value::String(rest)) if rest == "rest" => {
                        self.playing = None;
                        self.set_pose(None)?;
                    }
                    Some(Value::String(stop)) if stop == "stop" => self.playing = None,
                    Some(value @ Value::Object(_)) if value.get("play").and_then(Value::as_bool) == Some(true) => {
                        let clip = value.get("clip").and_then(Value::as_str).ok_or("pose needs clip: a clip name")?.to_string();
                        self.play(Some(clip))?;
                    }
                    Some(value @ Value::Object(_)) => {
                        self.playing = None;
                        let clip = value.get("clip").and_then(Value::as_str).ok_or("pose needs clip: a clip name")?.to_string();
                        let time = f32_from_f64(value.get("time").and_then(Value::as_f64).unwrap_or(0.0));
                        self.set_pose(Some((clip, time)))?;
                    }
                    Some(_) => return Err("pose is \"rest\", \"stop\", {clip, time}, or {clip, play: true}".into()),
                    None => {}
                }
                if let Some(on) = args.bool("turntable")? {
                    self.turntable = on;
                }
                if let Some(look) = args.str("look")? {
                    self.look(camera, &look)?;
                }
                match args.0.get("replay") {
                    Some(Value::String(live)) if live == "live" => {
                        self.replay_playing = None;
                        self.rebuild = self.replay.take().is_some();
                    }
                    Some(Value::String(play)) if play == "play" => {
                        // From the session start, unless a state is already shown.
                        if self.replay.is_none() {
                            let (session, _) = self.replay_span()?;
                            self.replay_at(&session, 0)?;
                        }
                        self.replay_playing = Some(REPLAY_STEP_SECONDS);
                    }
                    Some(Value::String(stop)) if stop == "stop" => self.replay_playing = None,
                    Some(Value::String(step)) if step == "next" || step == "previous" => {
                        let (session, last) = self.replay_span()?;
                        let at = self.replay.as_ref().map_or(last, |r| r.event);
                        let to = if step == "next" { at + 1 } else { at.saturating_sub(1) };
                        if to > last {
                            self.rebuild = self.replay.take().is_some();
                        } else {
                            self.replay_at(&session, to)?;
                        }
                    }
                    Some(value @ Value::Object(_)) => {
                        let session = value.get("session").and_then(Value::as_str).unwrap_or("latest").to_string();
                        let event = value.get("event").and_then(Value::as_u64).ok_or("replay needs event: a command number, 0 for the session start")?;
                        self.replay_at(&session, event)?;
                    }
                    Some(_) => return Err("replay is \"live\", \"next\", \"previous\", \"play\", \"stop\", or {session, event}".into()),
                    None => {}
                }
                self.sync_source();
                if args.bool("show_all")?.unwrap_or(false) {
                    self.hidden.clear();
                    self.display_stale = true;
                }
                if let Some(value) = args.0.get("solo") {
                    let keep = id_list("solo", value)?;
                    self.hidden = self
                        .workspace
                        .iter()
                        .flat_map(|w| w.parts.iter().map(|p| p.object))
                        .filter(|object| !keep.contains(object))
                        .collect();
                    self.display_stale = true;
                }
                if let Some(value) = args.0.get("hide") {
                    self.hidden.extend(id_list("hide", value)?);
                    self.display_stale = true;
                }
                if let Some(value) = args.0.get("show") {
                    for object in id_list("show", value)? {
                        self.hidden.remove(&object);
                    }
                    self.display_stale = true;
                }
                if let Some(shade) = args.str("shade")? {
                    let shade = match SHADE_ALIASES.iter().find(|(other, _, _)| *other == shade) {
                        Some((other, name, deprecated)) => {
                            if *deprecated {
                                eprintln!("mesh_view: the shade name {other} is deprecated; use {name}");
                            }
                            (*name).to_string()
                        }
                        None => shade,
                    };
                    if !SHADES.contains(&shade.as_str()) {
                        return Err(format!("mesh_view shade is {}, not {shade}", SHADES.join(", ")));
                    }
                    self.shade = shade;
                    self.display_stale = true;
                }
                if let Some(mode) = args.str("pick")? {
                    if !matches!(mode.as_str(), "face" | "object" | "vertex") {
                        return Err(format!("mesh_view pick is face, object, or vertex, not {mode}"));
                    }
                    self.pick_mode = mode;
                }
                Ok(self.view_value())
            }
            "mesh_selection" => {
                args.only("mesh_selection", &SELECTION_ARGS)?;
                if args.bool("clear")?.unwrap_or(false) {
                    self.selection = Selection::default();
                    self.overlay_stale = true;
                }
                for (key, set) in [
                    ("objects", &mut self.selection.objects),
                    ("faces", &mut self.selection.faces),
                    ("vertices", &mut self.selection.vertices),
                ] {
                    if let Some(value) = args.0.get(key) {
                        *set = id_list(key, value)?;
                        self.overlay_stale = true;
                    }
                }
                if let Some(note) = args.str("note")? {
                    self.selection.note = note;
                }
                if let Some(how) = args.str("grow")? {
                    self.grow(&how)?;
                }
                Ok(self.selection_value(camera))
            }
            "mesh_pick" => {
                args.only("mesh_pick", &PICK_ARGS)?;
                let at = args.floats("screen", 2)?.map_or([0.5, 0.5], |v| [v[0], v[1]]);
                let mode = args.str("mode")?.unwrap_or_else(|| "face".into());
                let select = args.str("select")?.unwrap_or_else(|| "set".into());
                if !matches!(mode.as_str(), "face" | "object" | "vertex") {
                    return Err(format!("mesh_pick mode is face, object, or vertex, not {mode}"));
                }
                if !matches!(select.as_str(), "set" | "add" | "none") {
                    return Err(format!("mesh_pick select is set, add, or none, not {select}"));
                }
                // A box or a lasso selects everything inside it.
                let outline: Option<Vec<[f32; 2]>> = match (args.floats("box", 4)?, args.0.get("lasso")) {
                    (Some(b), _) => Some(vec![[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]),
                    (None, Some(Value::Array(points))) => Some(
                        points
                            .iter()
                            .map(|point| match point.as_array() {
                                Some([x, y]) => x.as_f64().zip(y.as_f64()).map(|(x, y)| [f32_from_f64(x), f32_from_f64(y)]),
                                _ => None,
                            })
                            .collect::<Option<Vec<_>>>()
                            .filter(|path| path.len() >= 3)
                            .ok_or("mesh_pick lasso is three or more [x, y] points")?,
                    ),
                    (None, Some(_)) => return Err("mesh_pick lasso is three or more [x, y] points".into()),
                    (None, None) => None,
                };
                if let Some(outline) = outline {
                    if select == "none" {
                        return Err("mesh_pick with box or lasso selects: select is set or add".into());
                    }
                    let through = args.bool("through")?.unwrap_or(false);
                    let added = self.select_inside(camera, aspect, &outline, &mode, select == "add", through);
                    return Ok(obj(vec![("added", num(added as f64)), ("selection", self.selection_value(camera))]));
                }
                let hit = self.pick(camera, aspect, at);
                if select != "none" {
                    if select == "set" {
                        let note = std::mem::take(&mut self.selection.note);
                        self.selection = Selection { note, ..Selection::default() };
                    }
                    if let Some(hit) = &hit {
                        match mode.as_str() {
                            "object" => self.selection.objects.insert(hit.object),
                            "vertex" => self.selection.vertices.insert(hit.vertex),
                            _ => self.selection.faces.insert(hit.face),
                        };
                        self.selection.point = Some(hit.point);
                    }
                    self.overlay_stale = true;
                }
                let picked = hit.map_or(Value::Null, |hit| {
                    obj(vec![
                        ("object", id(hit.object)),
                        ("object_name", text(hit.name.clone())),
                        ("face", id(hit.face)),
                        ("face_vertices", ids(self.face_vertices(hit.face))),
                        ("vertex", id(hit.vertex)),
                        ("point", vec3(hit.point)),
                    ])
                });
                Ok(obj(vec![("hit", picked), ("selection", self.selection_value(camera))]))
            }
            "mesh_command" => {
                args.only("mesh_command", &["command"])?;
                let command = args.0.get("command").ok_or("mesh_command needs command: one mesh-workbench command object, or a list of them")?;
                self.edit(command)
            }
            "mesh_commands" => {
                args.only("mesh_commands", &COMMANDS_ARGS)?;
                let known = catalogue()?;
                if let Some(name) = args.str("command")? {
                    let doc = known.iter().find(|doc| doc.command == name).ok_or_else(|| {
                        format!("mesh-workbench's help has no row for {name}; mesh_help topic command-api describes the rest")
                    })?;
                    return Ok(command_value(doc, true));
                }
                let family = args.str("family")?;
                if let Some(family) = &family {
                    if !FAMILIES.iter().any(|(tool, _)| tool == family) {
                        return Err(format!("mesh_commands family is one of {}", FAMILIES.map(|(tool, _)| tool).join(", ")));
                    }
                }
                let listed: Vec<Value> = known
                    .iter()
                    .filter(|doc| family.as_deref().is_none_or(|want| family_of(&doc.command) == want))
                    .map(|doc| command_value(doc, false))
                    .collect();
                Ok(obj(vec![
                    ("count", num(listed.len() as f64)),
                    ("commands", list(listed)),
                    ("source", text("Parsed from the tables of mesh-workbench's own help. command: NAME gives one command's words and schema.")),
                ]))
            }
            "mesh_preview" => {
                args.only("mesh_preview", &PREVIEW_ARGS)?;
                self.preview_command(&args)
            }
            "mesh_context" => {
                args.only("mesh_context", &CONTEXT_ARGS)?;
                let edits = args.0.get("edits").and_then(Value::as_u64).map_or(8, |n| n.min(EDITS_KEPT as u64));
                Ok(self.context_value(camera, usize::try_from(edits).unwrap_or(EDITS_KEPT)))
            }
            "mesh_help" => {
                args.only("mesh_help", &HELP_ARGS)?;
                // The same fields as mesh-workbench's help, with its name added.
                let mut request = vec![("command".to_string(), text("help"))];
                if let Value::Object(pairs) = args.0 {
                    request.extend(pairs.iter().cloned());
                }
                self.run(&Value::Object(request))
            }
            family if FAMILIES.iter().any(|(tool, _)| *tool == family) => {
                let command = args.0.get("command").and_then(Value::as_str).ok_or_else(|| {
                    format!("{family} takes one mesh-workbench command object: {{\"command\": \"...\", ...}}")
                })?;
                let belongs = family_of(command);
                if belongs != family {
                    return Err(format!("{command} is a {belongs} command, not {family}"));
                }
                self.edit(args.0)
            }
            other => Err(format!("unknown command {other}")),
        }
    }

    /// Open the session on the workspace, on its worker thread, if it is not open.
    fn open_session(&mut self) -> Result<(), String> {
        if self.bridge.is_none() {
            let open = crate::program()
                .mesh_session
                .ok_or("this program does not link mesh-workbench: run genos-editor to edit a mesh")?;
            self.bridge = Some(Worker::start(open(&self.dir)?));
        }
        Ok(())
    }

    /// Run a mesh-workbench command, or a list, and wait for the reply. For
    /// the viewer's own short requests (a pose sample, a preview).
    fn run(&mut self, command: &Value) -> Result<Value, String> {
        self.open_session()?;
        let worker = self.bridge.as_mut().ok_or("the mesh session did not open")?;
        let ticket = worker.send(command.clone())?;
        let reply = worker.wait(ticket)?;
        self.follow_now();
        Ok(reply)
    }

    /// The session wrote project.json: show it on the next frame.
    fn follow_now(&mut self) {
        if let Some(earlier) = Instant::now().checked_sub(FOLLOW_NOW) {
            self.looked = earlier;
        }
    }

    /// The reply to the tool request `ticket`, once the session has it.
    /// `None` while it runs. A ticket this view does not hold belongs to a
    /// workspace that was closed.
    pub fn poll_edit(&mut self, ticket: u64) -> Option<Result<Value, String>> {
        let Some(at) = self.waiting.iter().position(|waiting| waiting.ticket == ticket) else {
            return Some(Err("the workspace was closed before the command finished".into()));
        };
        let result = self.bridge.as_mut()?.take(ticket)?;
        let waiting = self.waiting.remove(at);
        self.follow_now();
        if waiting.changes {
            self.note_edit(waiting.command, &waiting.request, &result);
            self.end_preview();
        }
        Some(result)
    }

    /// Run a request a tool sent, and keep it for the context packet when it
    /// is not a query. An edit ends a preview: mesh-workbench drops its
    /// prepared transaction when the revision moves.
    fn edit(&mut self, request: &Value) -> Result<Value, String> {
        let names: Vec<&str> = match request {
            Value::Array(items) => items.iter().filter_map(|item| item.get("command").and_then(Value::as_str)).collect(),
            one => one.get("command").and_then(Value::as_str).into_iter().collect(),
        };
        let changes = names.iter().any(|name| family_of(name) != "mesh_query");
        let command = names.join(", ");
        // The session runs it on its thread. The reply is a ticket: the
        // program asks `poll_edit` for the answer each frame.
        let sent = self.open_session().and_then(|()| {
            let worker = self.bridge.as_mut().ok_or("the mesh session did not open")?;
            worker.send(request.clone())
        });
        match sent {
            Ok(ticket) => {
                self.waiting.push(Waiting { ticket, command, request: request.clone(), changes });
                Ok(obj(vec![(PENDING_KEY, id(ticket))]))
            }
            Err(err) => {
                if changes {
                    self.note_edit(command, request, &Err(err.clone()));
                }
                Err(err)
            }
        }
    }

    fn note_edit(&mut self, command: String, request: &Value, result: &Result<Value, String>) {
        let mut sent = json::encode(request);
        if sent.len() > EDIT_TEXT {
            let cut = (0..=EDIT_TEXT).rev().find(|at| sent.is_char_boundary(*at)).unwrap_or(0);
            sent.truncate(cut);
            sent.push_str("...");
        }
        let (ok, error) = match result {
            Ok(reply) => (reply_ok(reply), reply.get("error").and_then(Value::as_str).unwrap_or("").to_string()),
            Err(err) => (false, err.clone()),
        };
        self.edits.push(Edit { command, request: sent, ok, error, note: self.selection.note.clone() });
        if self.edits.len() > EDITS_KEPT {
            self.edits.remove(0);
        }
    }

    /// The picture's source: the preview's scratch workspace, else the
    /// replayed state, else the live file.
    fn sync_source(&self) {
        set_source(match (&self.preview, &self.replay) {
            (Some(preview), _) => Some(genos_workbench::project_path(&preview.dir)),
            (None, Some(replay)) => Some(genos_workbench::state_path(&self.dir, &replay.session, replay.snapshot)),
            (None, None) => None,
        });
    }

    /// Stop showing a preview. The scratch workspace is this viewer's own.
    fn end_preview(&mut self) {
        if let Some(preview) = self.preview.take() {
            let _ = std::fs::remove_dir_all(&preview.dir);
            self.rebuild = true;
            self.sync_source();
        }
    }

    /// `mesh_preview`. With `commands`: mesh-workbench prepares them as one
    /// transaction and does not commit, and the picture shows the result.
    /// The result is made by running the same commands on a copy of the
    /// workspace under the output directory, because mesh-workbench keeps its
    /// prepared mesh to itself. `commit: true` commits the prepared
    /// transaction; `discard: true` drops the picture.
    fn preview_command(&mut self, args: &Args) -> Result<Value, String> {
        if args.bool("discard")?.unwrap_or(false) {
            self.end_preview();
            return Ok(obj(vec![("preview", Value::Null)]));
        }
        if args.bool("commit")?.unwrap_or(false) {
            let preview = self.preview.clone().ok_or("no preview is shown: send commands first")?;
            let request = obj(vec![
                ("command", text("transaction")),
                ("commit", Value::Bool(true)),
                ("preview_id", text(preview.id.clone())),
                ("revision", id(preview.revision)),
            ]);
            let result = self.run(&request);
            self.note_edit(format!("transaction ({} commands, previewed)", preview.commands), &request, &result);
            self.end_preview();
            return result;
        }
        let commands = match args.0.get("commands") {
            Some(Value::Array(items)) if !items.is_empty() => items.clone(),
            _ => return Err("mesh_preview takes commands (a list of mesh-workbench commands), commit: true, or discard: true".into()),
        };
        let transaction = |commit: bool| {
            let mut request = vec![
                ("command", text("transaction")),
                ("commit", Value::Bool(commit)),
                ("commands", Value::Array(commands.clone())),
            ];
            if let Some(guards) = args.0.get("guards") {
                request.push(("guards", guards.clone()));
            }
            obj(request)
        };
        self.end_preview();
        let mut reply = self.run(&transaction(false))?;
        if !reply_ok(&reply) {
            return Ok(reply);
        }
        let result = reply.get("result").unwrap_or(&reply);
        let id = result.get("preview_id").and_then(Value::as_str).ok_or("mesh-workbench gave no preview_id")?.to_string();
        let revision = result.get("revision").and_then(Value::as_u64).ok_or("mesh-workbench gave no revision")?;
        // The same commands, committed on a copy, are the picture.
        let dir = out_dir().join("mesh-preview");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
        std::fs::copy(genos_workbench::project_path(&self.dir), genos_workbench::project_path(&dir))
            .map_err(|err| format!("{}: {err}", dir.display()))?;
        let open = crate::program().mesh_session.ok_or("this program does not link mesh-workbench: run genos-editor")?;
        let mut scratch = open(&dir)?;
        let shown = scratch.execute(&transaction(true))?;
        drop(scratch);
        if !reply_ok(&shown) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("the preview's copy refused the commands: {}", json::encode(&shown)));
        }
        self.preview = Some(Preview { id, revision, dir, commands: commands.len() });
        self.rebuild = true;
        self.sync_source();
        if let Value::Object(pairs) = &mut reply {
            pairs.push(("previewing".to_string(), Value::Bool(true)));
        }
        Ok(reply)
    }

    fn preview_value(&self) -> Value {
        self.preview.as_ref().map_or(Value::Null, |preview| {
            obj(vec![
                ("preview_id", text(preview.id.clone())),
                ("revision", id(preview.revision)),
                ("commands", num(preview.commands as f64)),
            ])
        })
    }

    /// `mesh_context`: what an agent needs to act on a request in one reply.
    /// The selection with the note and the camera, each selected face's
    /// vertices, centre and normal, the objects in the picture, how the
    /// picture is shown, and the latest edits with the note each ran under.
    fn context_value(&self, camera: &Camera, edits: usize) -> Value {
        let mut faces = Vec::new();
        'parts: for part in self.workspace.iter().flat_map(|w| &w.parts) {
            let mut last = None;
            for (triangle, face) in part.face_ids.iter().enumerate() {
                if last == Some(*face) || !self.selection.faces.contains(face) {
                    continue;
                }
                last = Some(*face);
                if faces.len() == CONTEXT_FACES {
                    break 'parts;
                }
                let corners: Vec<[f32; 3]> = part
                    .face_ids
                    .iter()
                    .enumerate()
                    .filter(|(_, other)| *other == face)
                    .flat_map(|(at, _)| (0..3).map(move |corner| at * 3 + corner))
                    .filter_map(|at| part.positions.get(at).copied())
                    .collect();
                #[expect(clippy::cast_precision_loss, reason = "a face has few corners")]
                let centre = [0, 1, 2].map(|axis| corners.iter().map(|p| p[axis]).sum::<f32>() / corners.len().max(1) as f32);
                let first = triangle * 3;
                let normal = match (part.positions.get(first), part.positions.get(first + 1), part.positions.get(first + 2)) {
                    (Some(a), Some(b), Some(c)) => {
                        normalise(cross([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]))
                    }
                    _ => [0.0; 3],
                };
                faces.push(obj(vec![
                    ("face", id(*face)),
                    ("object", id(part.object)),
                    ("vertices", ids(self.face_vertices(*face))),
                    ("centre", vec3(centre)),
                    ("normal", vec3(normal)),
                ]));
            }
        }
        let visible: Vec<Value> = self
            .workspace
            .iter()
            .flat_map(|w| &w.parts)
            .filter(|part| !self.hidden.contains(&part.object))
            .map(|part| {
                let mut low = [f32::INFINITY; 3];
                let mut high = [f32::NEG_INFINITY; 3];
                for p in &part.positions {
                    for axis in 0..3 {
                        low[axis] = low[axis].min(p[axis]);
                        high[axis] = high[axis].max(p[axis]);
                    }
                }
                obj(vec![
                    ("id", id(part.object)),
                    ("name", text(part.name.clone())),
                    ("triangles", num(part.triangles() as f64)),
                    ("min", vec3(low)),
                    ("max", vec3(high)),
                ])
            })
            .collect();
        let recent: Vec<Value> = self.edits[self.edits.len().saturating_sub(edits)..]
            .iter()
            .map(|edit| {
                obj(vec![
                    ("command", text(edit.command.clone())),
                    ("request", text(edit.request.clone())),
                    ("ok", Value::Bool(edit.ok)),
                    ("error", text(edit.error.clone())),
                    ("note", text(edit.note.clone())),
                ])
            })
            .collect();
        obj(vec![
            ("workspace", text(self.dir.display().to_string())),
            ("revision", id(self.workspace.as_ref().map_or(0, |w| w.revision))),
            // Every position and length here and in mesh-workbench: metres, Y up.
            ("units", text("metres, Y up")),
            // What the person typed about the selection: the request to act on.
            ("intent", text(self.selection.note.clone())),
            ("selection", self.selection_value(camera)),
            ("selected_faces", list(faces)),
            ("selected_faces_listed", Value::Bool(self.selection.faces.len() <= CONTEXT_FACES)),
            ("visible_objects", list(visible)),
            ("hidden_objects", ids(self.hidden.iter().copied().collect())),
            (
                "view",
                obj(vec![
                    ("shade", text(self.shade.clone())),
                    ("projection", text(if self.orthographic { "orthographic" } else { "perspective" })),
                    ("pick", text(self.pick_mode.clone())),
                    ("replay", self.replay.as_ref().map_or(Value::Null, |r| obj(vec![("session", text(r.session.clone())), ("event", id(r.event))]))),
                    ("preview", self.preview_value()),
                ]),
            ),
            ("recent_edits", list(recent)),
        ])
    }

    fn view_value(&self) -> Value {
        let (revision, vertices, faces, objects) = self.workspace.as_ref().map_or((0, 0, 0, Vec::new()), |w| {
            (
                w.revision,
                w.vertices,
                w.faces,
                w.parts
                    .iter()
                    .map(|p| {
                        obj(vec![
                            ("id", id(p.object)),
                            ("name", text(p.name.clone())),
                            ("origin", vec3(p.origin)),
                            ("triangles", num(p.triangles() as f64)),
                            ("hidden", Value::Bool(self.hidden.contains(&p.object))),
                        ])
                    })
                    .collect(),
            )
        });
        obj(vec![
            ("workspace", text(self.dir.display().to_string())),
            ("workspaces", list(self.found.iter().map(|dir| text(dir.display().to_string())).collect())),
            ("state", text(self.state())),
            ("loaded", Value::Bool(self.workspace.is_some())),
            ("revision", id(revision)),
            ("vertices", num(vertices as f64)),
            ("faces", num(faces as f64)),
            ("objects", list(objects)),
            ("pick", text(self.pick_mode.clone())),
            ("shade", text(self.shade.clone())),
            ("turntable", Value::Bool(self.turntable)),
            (
                "collections",
                list(
                    self.workspace
                        .iter()
                        .flat_map(|w| &w.collections)
                        .map(|collection| {
                            obj(vec![
                                ("id", id(collection.id)),
                                ("name", text(collection.name.clone())),
                                ("objects", ids(collection.objects.clone())),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("playing", Value::Bool(self.playing.is_some())),
            ("replay_playing", Value::Bool(self.replay_playing.is_some())),
            ("bones", Value::Bool(self.bones)),
            ("surfaces", Value::Bool(self.surfaces)),
            ("grid", Value::Bool(self.grid)),
            ("shadow", Value::Bool(self.shadow)),
            ("projection", text(if self.orthographic { "orthographic" } else { "perspective" })),
            ("preview", self.preview_value()),
            ("filter", text(self.filter.clone())),
            ("labels", Value::Bool(self.labels)),
            ("panel", Value::Bool(self.panel_open)),
            ("command_filter", text(self.command_filter.clone())),
            (
                "pose",
                self.pose.as_ref().map_or(Value::Null, |(clip, time)| {
                    obj(vec![("clip", text(clip.clone())), ("time", num(f64::from(*time)))])
                }),
            ),
            (
                "clips",
                list(
                    self.workspace
                        .iter()
                        .flat_map(|w| &w.clips)
                        .map(|clip| obj(vec![("name", text(clip.name.clone())), ("duration", num(f64::from(clip.duration)))]))
                        .collect(),
                ),
            ),
            ("rig_bones", num(self.workspace.as_ref().map_or(0, |w| w.bones.len()) as f64)),
            (
                "replay",
                self.replay.as_ref().map_or(Value::Null, |r| {
                    obj(vec![
                        ("session", text(r.session.clone())),
                        ("event", id(r.event)),
                        ("command", text(r.command.clone())),
                    ])
                }),
            ),
            (
                "sessions",
                list(
                    genos_workbench::sessions(&self.dir)
                        .iter()
                        .map(|s| {
                            obj(vec![
                                ("id", text(s.id.clone())),
                                ("events", num(s.events.len() as f64)),
                                (
                                    "commands",
                                    list(s.events.iter().rev().take(REPLAY_LISTED).rev().map(|e| text(format!("{} {}", e.id, e.command))).collect()),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("editing", Value::Bool(self.bridge.is_some())),
            ("lift", num(f64::from(self.lift()))),
        ])
    }

    fn selection_value(&self, camera: &Camera) -> Value {
        let names: Vec<Value> = self
            .selection
            .objects
            .iter()
            .map(|object| text(self.part(*object).map_or(String::new(), |p| p.name.clone())))
            .collect();
        let lift = self.lift();
        obj(vec![
            ("objects", ids(self.selection.objects.iter().copied().collect())),
            ("object_names", list(names)),
            ("faces", ids(self.selection.faces.iter().copied().collect())),
            ("vertices", ids(self.selection.vertices.iter().copied().collect())),
            ("point", self.selection.point.map_or(Value::Null, vec3)),
            ("note", text(self.selection.note.clone())),
            ("revision", id(self.workspace.as_ref().map_or(0, |w| w.revision))),
            (
                "camera",
                obj(vec![
                    // Mesh coordinates: the picture lifts the mesh onto the ground.
                    ("position", vec3([camera.position.x, camera.position.y - lift, camera.position.z])),
                    ("yaw", num(f64::from(camera.yaw))),
                    ("pitch", num(f64::from(camera.pitch))),
                ]),
            ),
        ])
    }
}

impl Orbit {
    /// Stand back from the target along the camera's direction. `distance`
    /// is how far the perspective view stands. The orthographic view is a
    /// parallel projection that shows the same height at the target; its eye
    /// stands further back, so the whole mesh is in front of it.
    fn write(&mut self, camera: &mut Camera, orthographic: bool) {
        camera.view_tan = genos_scene::DEFAULT_VIEW_TAN;
        camera.parallel_half = if orthographic { self.distance * genos_scene::DEFAULT_VIEW_TAN } else { 0.0 };
        let back = if orthographic { (self.distance * ORTHO_BACK).clamp(self.distance.min(ORTHO_REACH), ORTHO_REACH) } else { self.distance };
        let forward = genos_scene::look_direction(camera.yaw, camera.pitch);
        camera.position = genos_scene::Vec3::new(
            self.target[0] - forward.x * back,
            self.target[1] - forward.y * back,
            self.target[2] - forward.z * back,
        );
        self.wrote = ([camera.position.x, camera.position.y, camera.position.z], camera.yaw, camera.pitch);
    }
}

/// A plain click selects `id` alone (the caller cleared the set). A shift
/// click adds it, or removes it when it was already selected.
/// Pixels of travel between two points of a lasso.
const LASSO_STEP: f32 = 6.0;

/// Whether `at` is inside the closed polygon `outline` (even-odd rule).
fn inside(outline: &[[f32; 2]], at: [f32; 2]) -> bool {
    let mut within = false;
    for (index, a) in outline.iter().enumerate() {
        let b = outline[(index + 1) % outline.len()];
        if (a[1] > at[1]) != (b[1] > at[1]) && at[0] < a[0] + (at[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            within = !within;
        }
    }
    within
}

fn toggle(set: &mut BTreeSet<u64>, id: u64, add: bool) {
    if add && set.contains(&id) {
        set.remove(&id);
    } else {
        set.insert(id);
    }
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(pairs.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
}

fn num(value: f64) -> Value {
    // Short decimals keep replies readable.
    json::float((value * REPLY_DECIMALS).round() / REPLY_DECIMALS)
}

fn vec3(v: [f32; 3]) -> Value {
    json::array(v.iter().map(|c| num(f64::from(*c))).collect())
}

fn list(items: Vec<Value>) -> Value {
    json::array(items)
}

fn text(value: impl Into<String>) -> Value {
    json::string(value)
}

/// The arguments of one command, with errors that name the field.
struct Args<'a>(&'a Value);

impl Args<'_> {
    /// Fail on a key that is not in `known`, so a misspelt field is an error.
    fn only(&self, command: &str, known: &[&str]) -> Result<(), String> {
        if let Value::Object(pairs) = self.0 {
            if let Some((key, _)) = pairs.iter().find(|(key, _)| !known.contains(&key.as_str())) {
                return Err(format!("{command} has no field {key}; it takes {}", known.join(", ")));
            }
        }
        Ok(())
    }

    fn str(&self, key: &str) -> Result<Option<String>, String> {
        match self.0.get(key) {
            None => Ok(None),
            Some(value) => value.as_str().map(|s| Some(s.to_string())).ok_or_else(|| format!("{key} is text")),
        }
    }

    fn bool(&self, key: &str) -> Result<Option<bool>, String> {
        match self.0.get(key) {
            None => Ok(None),
            Some(value) => value.as_bool().map(Some).ok_or_else(|| format!("{key} is true or false")),
        }
    }

    fn floats(&self, key: &str, count: usize) -> Result<Option<Vec<f32>>, String> {
        let Some(value) = self.0.get(key) else {
            return Ok(None);
        };
        let items = value.as_array().filter(|items| items.len() == count).ok_or_else(|| format!("{key} is {count} numbers"))?;
        items
            .iter()
            .map(|item| item.as_f64().map(f32_from_f64).ok_or_else(|| format!("{key} is {count} numbers")))
            .collect::<Result<Vec<f32>, String>>()
            .map(Some)
    }
}

struct Hit {
    object: u64,
    name: String,
    face: u64,
    vertex: u64,
    point: [f32; 3],
    depth: f32,
}

fn id(value: u64) -> Value {
    // mesh-workbench IDs are small counters; a larger one wraps, as before.
    json::int(value.cast_signed())
}

fn ids(values: Vec<u64>) -> Value {
    list(values.into_iter().map(id).collect())
}

fn id_list(key: &str, value: &Value) -> Result<BTreeSet<u64>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{key} is a list of ids"))?
        .iter()
        .map(|item| item.as_u64().ok_or_else(|| format!("{key} is a list of ids")))
        .collect()
}

/// Screen x, y in 0..1 (origin top left) and depth of a world point. `None`
/// behind the eye.
fn project(m: &[f32; 16], p: [f32; 3]) -> Option<[f32; 3]> {
    let row = |r: usize| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r];
    let w = row(3);
    if w <= 1.0e-4 {
        return None;
    }
    // Vulkan clip space: +Y is the lower half of the picture.
    Some([f32::midpoint(row(0) / w, 1.0), f32::midpoint(row(1) / w, 1.0), row(2) / w])
}

/// Weights of `at` inside the screen triangle, or `None` outside it.
fn barycentric(t: [[f32; 3]; 3], at: [f32; 2]) -> Option<[f32; 3]> {
    let area = (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1]);
    if area.abs() < 1.0e-12 {
        return None;
    }
    let w1 = ((at[0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (at[1] - t[0][1])) / area;
    let w2 = ((t[1][0] - t[0][0]) * (at[1] - t[0][1]) - (at[0] - t[0][0]) * (t[1][1] - t[0][1])) / area;
    let w0 = 1.0 - w1 - w2;
    (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0).then_some([w0, w1, w2])
}

fn octahedron(centre: [f32; 3], radius: f32, positions: &mut Vec<[f32; 3]>, normals: &mut Vec<[f32; 3]>) {
    let tip = |axis: usize, sign: f32| {
        let mut p = centre;
        p[axis] += sign * radius;
        p
    };
    for sx in [1.0f32, -1.0] {
        for sy in [1.0f32, -1.0] {
            for sz in [1.0f32, -1.0] {
                let (a, b, c) = (tip(0, sx), tip(1, sy), tip(2, sz));
                // Keep the winding outward for every octant.
                let flip = sx * sy * sz < 0.0;
                let corners = if flip { [a, c, b] } else { [a, b, c] };
                let n = [sx * 0.577_35, sy * 0.577_35, sz * 0.577_35];
                for corner in corners {
                    positions.push(corner);
                    normals.push(n);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{barycentric, MeshView, Selection};
    use genos_scene::Camera;

    fn view() -> MeshView {
        let dir = genos_scene_lib::repo_root().join("debug/scenes/mesh/sample");
        MeshView::new(dir, None)
    }

    #[test]
    fn a_textured_shade_gives_each_face_a_place_in_its_image() {
        use genos_workbench::TextureImage;
        // The checker comes first, then each texture to its right.
        let red = TextureImage { name: "red".into(), width: 4, height: 2, pixels: [255, 0, 0, 255].repeat(8), alpha_cutoff: None };
        let blue = TextureImage { name: "blue".into(), width: 2, height: 2, pixels: [0, 0, 255, 64].repeat(4), alpha_cutoff: Some(0.5) };
        let atlas = super::atlas(&[red, blue]);
        assert_eq!((atlas.width, atlas.height), (128 + 6, 128));
        assert_eq!(atlas.rects, vec![[128, 0, 4, 2], [132, 0, 2, 2]]);
        let texel = |x: usize, y: usize| atlas.rgba[(y * 134 + x) * 4..][..4].to_vec();
        assert_eq!(texel(128, 1), [255, 0, 0, 255]);
        // The blue image has a cutoff: its texels and their alpha are kept for the surface to cut.
        assert_eq!(texel(133, 0), [0, 0, 255, 64]);
        assert_eq!(atlas.cutoffs, [0.0, 0.5]);
        // No cutoff: a half-clear texel stands over mid grey.
        let clear = TextureImage { name: "clear".into(), width: 1, height: 1, pixels: vec![255, 0, 0, 0], alpha_cutoff: None };
        assert_eq!(super::atlas(&[clear]).rgba[128 * 4..][..4], [128, 128, 128, 255]);
        assert_eq!((texel(0, 0), texel(16, 0)), (vec![48, 48, 48, 255], vec![224, 224, 224, 255]));

        let mut view = view();
        view.shade = "textured".to_string();
        let textures = vec![
            TextureImage { name: "red".into(), width: 4, height: 2, pixels: [255, 0, 0, 255].repeat(8), alpha_cutoff: None },
            TextureImage { name: "blue".into(), width: 2, height: 2, pixels: [0, 0, 255, 64].repeat(4), alpha_cutoff: Some(0.5) },
        ];
        let workspace = view.workspace.as_mut().expect("workspace");
        workspace.textures = textures;
        let part = &mut workspace.parts[0];
        // The first triangle tiles past one repeat and starts below zero. The
        // second has a texture and no UVs. The rest have no UVs either.
        part.uvs = vec![[0.0; 2]; part.positions.len()];
        part.uv_mapped = vec![false; part.positions.len()];
        part.uvs[..3].copy_from_slice(&[[-0.5, 0.25], [1.5, 0.25], [0.0, 0.75]]);
        part.uv_mapped[..3].fill(true);
        // The third triangle has UVs and no texture.
        part.uv_mapped[6..9].fill(true);
        part.face_textures = vec![None; part.triangles()];
        part.face_textures[0] = Some(0);
        part.face_textures[1] = Some(0);
        let shown = view.display();
        let (surface, uvs) = shown[0].3.clone().expect("the cube draws with a surface");
        assert_eq!(surface, super::SURFACE_TEXTURES, "one texture: its own surface, so it repeats");
        assert_eq!(uvs[..3], [[0.5, 0.25], [2.5, 0.25], [1.0, 0.75]]);
        assert!(uvs[3..].iter().all(|uv| uv[0] < 0.0), "no UV or no texture: no coordinate");
        assert_eq!(shown[0].2[0], [1.0; 3], "white under the texture");
        assert_eq!(shown[0].2[3], super::UNMAPPED_COLOR, "a textured face with no UVs is magenta");
        assert_eq!(shown[0].2[6], super::MESH_COLOR, "a face with UVs and no texture is grey");
        assert_eq!(shown[0].2[9], super::UNMAPPED_COLOR, "no UVs is magenta, texture or not");
        assert!(view.textured_pieces().is_empty() && view.unlit());
        // A second texture: the part is drawn as a piece a texture, each repeating.
        let part = view.workspace.as_mut().map(|w| &mut w.parts[0]).expect("cube");
        part.uv_mapped[6..9].fill(true);
        part.face_textures[2] = Some(1);
        assert!(view.display()[0].0, "the whole part is left to the pieces");
        let pieces = view.textured_pieces();
        let surfaces: Vec<Option<usize>> = pieces.iter().map(|piece| piece.surface).collect();
        assert_eq!(surfaces, [None, Some(super::SURFACE_TEXTURES), Some(super::SURFACE_TEXTURES + 1)]);
        assert_eq!(pieces[1].texcoords, [[0.5, 0.25], [2.5, 0.25], [1.0, 0.75]], "still repeating");
        assert_eq!(pieces.iter().map(|piece| piece.positions.len()).sum::<usize>(), 36);
        // The checker covers every mapped face and marks the rest.
        view.shade = "uv_checker".to_string();
        let shown = view.display();
        assert_eq!(shown[0].3.as_ref().map(|(surface, _)| *surface), Some(super::SURFACE_CHECKER));
        assert_eq!((shown[0].2[0], shown[0].2[3]), ([1.0; 3], super::UNMAPPED_COLOR));
    }

    #[test]
    fn each_shown_object_has_a_contact_shadow_that_ends_at_the_ground_colour() {
        let mut view = view();
        let ground = [0.3, 0.31, 0.33];
        let (positions, normals, colors) = view.shadow_patches(ground);
        let per_object = (super::SHADOW_RINGS.len() - 1) * super::SHADOW_SIDES * 6;
        assert_eq!(positions.len(), per_object * 2, "the cube and the bracket stand apart: a patch each");
        assert_eq!(normals, vec![[0.0, 1.0, 0.0]; normals.len()]);
        // Dark at the centre, the ground's own colour at the rim, never brighter.
        let darkest = colors.iter().map(|c| c[0]).fold(f32::INFINITY, f32::min);
        assert!((darkest - 0.3 * (1.0 - super::SHADOW_DARK)).abs() < 1.0e-5, "{darkest}");
        assert!(colors.contains(&ground) && colors.iter().all(|c| c[0] <= ground[0]));
        // It reaches past the cube (1 m across, centred on the origin) and stays on the ground.
        let cube = &positions[..per_object];
        let reach = cube.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        assert!(reach > 0.5 && reach < 1.0, "{reach}");
        // Objects whose patches would overlap share one, so no two lie on each other.
        let mut close = self::view();
        if let Some(workspace) = &mut close.workspace {
            let shift = workspace.parts[1].positions.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min) - 0.6;
            for p in &mut workspace.parts[1].positions {
                p[0] -= shift;
            }
        }
        assert_eq!(close.shadow_patches(ground).0.len(), per_object, "the bracket moved against the cube");
        // A hidden object has none; neither does a view with the shadow off.
        let mut camera = Camera::new(0.0, 0.0, 0.0);
        let hide = genos_mcp::json::parse(r#"{"hide":[1]}"#).expect("json");
        view.command("mesh_view", &hide, &mut camera, 1.0).expect("hide");
        assert_eq!(view.shadow_patches(ground).0.len(), per_object);
        let off = genos_mcp::json::parse(r#"{"shadow":false}"#).expect("json");
        view.command("mesh_view", &off, &mut camera, 1.0).expect("shadow off");
        assert_eq!(view.shadow_patches(ground).0.len(), 0);
    }

    #[test]
    fn each_mesh_tool_publishes_a_schema_that_names_the_fields_it_takes() {
        let keys = |schema: &genos_mcp::json::Value| match schema.get("properties") {
            Some(genos_mcp::json::Value::Object(pairs)) => pairs.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let lists: [(&str, &[&str]); 7] = [
            ("mesh_view", &super::VIEW_ARGS),
            ("mesh_pick", &super::PICK_ARGS),
            ("mesh_selection", &super::SELECTION_ARGS),
            ("mesh_help", &super::HELP_ARGS),
            ("mesh_preview", &super::PREVIEW_ARGS),
            ("mesh_commands", &super::COMMANDS_ARGS),
            ("mesh_context", &super::CONTEXT_ARGS),
        ];
        for (tool, takes) in lists {
            let schema = genos_debug::commands::mesh_schema(tool).expect(tool);
            let mut named = keys(&schema);
            let mut takes: Vec<String> = takes.iter().map(|field| (*field).to_string()).collect();
            named.sort();
            takes.sort();
            assert_eq!(named, takes, "{tool}");
        }
        // The shades in the schema are the shades the viewer has.
        let view = genos_debug::commands::mesh_schema("mesh_view").expect("mesh_view");
        let shades = view.get("properties").and_then(|p| p.get("shade")).and_then(|s| s.get("enum")).expect("shade enum");
        let shades: Vec<&str> = match shades {
            genos_mcp::json::Value::Array(items) => items.iter().filter_map(|item| item.as_str()).collect(),
            _ => Vec::new(),
        };
        assert_eq!(shades, super::SHADES);
        // Every family tool, and mesh_command, has one; each needs `command`.
        for (tool, _) in super::FAMILIES.iter().chain(&[("mesh_command", "")]) {
            let schema = genos_debug::commands::mesh_schema(tool).expect(tool);
            assert_eq!(keys(&schema), ["command"], "{tool}");
            assert!(genos_mcp::json::encode(&schema).contains(r#""required":["command"]"#), "{tool}");
        }
        // And the catalogue carries them.
        let listed = genos_debug::commands::specs();
        let listed = listed.iter().find(|spec| spec.name == "mesh_pick").expect("mesh_pick");
        assert_eq!(keys(&listed.schema).len(), super::PICK_ARGS.len());
    }

    #[test]
    fn the_panels_list_collections_objects_and_commands_and_act_on_a_click() {
        use super::PanelTarget;
        use genos_workbench::{Collection, Event, Session};
        let mut view = view();
        let labels = |view: &MeshView| view.object_lines().into_iter().map(|line| line.label).collect::<Vec<_>>();
        assert_eq!(labels(&view), ["1 cube", "2 bracket"]);
        // A collection lists its objects under it; the rest come after.
        if let Some(workspace) = &mut view.workspace {
            workspace.collections = vec![Collection { id: 7, name: "Props".into(), objects: vec![2] }];
        }
        assert_eq!(labels(&view), ["Props (1)", "    2 bracket", "1 cube"]);
        // The filter keeps a matching object, or all of a matching collection.
        view.filter = "CUBE".into();
        assert_eq!(labels(&view), ["1 cube"]);
        view.filter = "prop".into();
        assert_eq!(labels(&view), ["Props (1)", "    2 bracket"]);
        view.filter.clear();
        // The collection's box hides its objects; the object's box shows it again.
        view.panel_click(PanelTarget::Collection(7), false).expect("hide Props");
        assert!(view.hidden.contains(&2) && !view.hidden.contains(&1));
        assert!(!view.object_lines()[0].on && view.object_lines()[2].on);
        view.panel_click(PanelTarget::Object(2), true).expect("show the bracket");
        assert!(view.hidden.is_empty());

        // The Replay panel: nothing without a session, then live and the commands, newest first.
        assert_eq!(view.replay_lines().len(), 0);
        let event = |id, command: &str, ok| Event { id, command: command.into(), ok, snapshot: None };
        view.recorded = vec![
            Session { id: "old".into(), initial_snapshot: 0, events: vec![event(1, "inspect", true)] },
            Session { id: "new".into(), initial_snapshot: 0, events: vec![event(1, "cube", true), event(2, "extrude", false), event(3, "move", true)] },
        ];
        let lines = view.replay_lines();
        let commands: Vec<&str> = lines.iter().map(|line| line.label.as_str()).collect();
        assert_eq!(commands, ["live", "3 move", "2 extrude (refused)", "1 cube"]);
        assert!(lines[0].on && lines[1..].iter().all(|line| !line.on));
        view.command_filter = "ex".into();
        assert_eq!(view.replay_lines().len(), 2, "live and the one match");
        // Folding the collection leaves its row and takes its objects off the list.
        view.panel_fold(PanelTarget::Collection(7), false);
        assert_eq!(labels(&view), ["Props (1)", "1 cube"]);
        assert_eq!(view.object_lines()[0].open, Some(false));
        view.filter = "brack".into();
        assert_eq!(labels(&view), ["Props (1)", "    2 bracket"], "a filter lists what it found, folded or not");
        view.filter.clear();
        view.panel_fold(PanelTarget::Collection(7), true);
        assert_eq!(labels(&view).len(), 3);
        assert_eq!(view.session_choices(), (vec!["new".to_string(), "old".to_string()], 0));
    }

    #[test]
    fn shift_and_t_types_the_filter_and_leaves_the_note_alone() {
        use genos_input::InputCode;
        let mut view = view();
        let press = |view: &mut MeshView, key: InputCode| view.type_note(&|code| code == key, false);
        assert!(view.type_note(&|code| code == InputCode::key_t, true) && view.typing && view.typing_filter);
        press(&mut view, InputCode::key_c);
        press(&mut view, InputCode::key_u);
        press(&mut view, InputCode::key_enter);
        assert_eq!((view.filter.as_str(), view.selection.note.as_str(), view.typing), ("cu", "", false));
        // T types the note again, and Escape while filtering clears the filter only.
        press(&mut view, InputCode::key_t);
        press(&mut view, InputCode::key_a);
        press(&mut view, InputCode::key_enter);
        view.type_note(&|code| code == InputCode::key_t, true);
        press(&mut view, InputCode::key_escape);
        assert_eq!((view.filter.as_str(), view.selection.note.as_str()), ("", "a"));
        // Control and T types the command filter. Shift on a digit is its sign,
        // and the number pad gives the point.
        view.control = true;
        press(&mut view, InputCode::key_t);
        view.control = false;
        press(&mut view, InputCode::key_u);
        view.type_note(&|code| code == InputCode::key_1, true);
        press(&mut view, InputCode::key_numpad_decimal);
        press(&mut view, InputCode::key_5);
        press(&mut view, InputCode::key_enter);
        assert_eq!((view.command_filter.as_str(), view.filter.as_str(), view.selection.note.as_str()), ("u!.5", "", "a"));
        // A sentence: comma, hyphen, full stop, and Shift for the question mark and the quote.
        press(&mut view, InputCode::key_t);
        for (key, shift) in [
            (InputCode::key_comma, false),
            (InputCode::key_space, false),
            (InputCode::key_minus, false),
            (InputCode::key_period, false),
            (InputCode::key_slash, true),
            (InputCode::key_quote, true),
        ] {
            view.type_note(&|code| code == key, shift);
        }
        press(&mut view, InputCode::key_enter);
        assert_eq!(view.selection.note, "a, -.?\"");
    }

    #[test]
    fn the_orthographic_view_keeps_the_framing_and_draws_near_and_far_the_same_size() {
        let mut view = view();
        let mut camera = Camera::new(0.0, 4.0, 0.0);
        let side = genos_mcp::json::parse(r#"{"look":"+x"}"#).expect("json");
        view.command("mesh_view", &side, &mut camera, 1.0).expect("look");
        // Two points level with the camera's aim, 0.4 m to the side: one in
        // the plane the camera turns about, one a metre nearer the camera.
        let aim = view.orbit.map(|orbit| orbit.target).expect("orbit");
        let across = |camera: &Camera, nearer: f32| {
            let uv = genos_scene::viewport_uv(camera, 1.0, [aim[0] + nearer, aim[1], aim[2] + 0.4]).expect("in view");
            (uv[0] - 0.5).abs()
        };
        let (plane, near) = (across(&camera, 0.0), across(&camera, 1.0));
        assert!(near > plane * 1.15, "perspective draws the nearer point further out: {near} {plane}");
        let stood = camera.position.x;

        let ortho = genos_mcp::json::parse(r#"{"projection":"orthographic"}"#).expect("json");
        view.command("mesh_view", &ortho, &mut camera, 1.0).expect("orthographic");
        assert!(camera.parallel_half > 0.0 && camera.position.x > stood * 4.0, "a parallel view, the eye further back");
        assert!(camera.position.x < genos_scene::CAMERA_FAR, "inside the far plane");
        let (plane_o, near_o) = (across(&camera, 0.0), across(&camera, 1.0));
        assert!((plane_o - plane).abs() < 1.0e-4, "the framing is kept: {plane_o} {plane}");
        assert!((near_o - plane_o).abs() < 1.0e-5, "near and far draw the same size: {near_o} {plane_o}");
        // A pick still finds the mesh.
        assert!(view.pick(&camera, 1.0, [0.5, 0.5]).is_some());

        let back = genos_mcp::json::parse(r#"{"projection":"perspective"}"#).expect("json");
        view.command("mesh_view", &back, &mut camera, 1.0).expect("perspective");
        assert!(camera.parallel_half == 0.0 && (camera.position.x - stood).abs() < 1.0e-3);
    }

    #[test]
    fn the_context_packet_carries_the_intent_the_selected_face_and_what_is_in_view() {
        let mut view = view();
        let mut camera = Camera::new(0.0, 4.0, 0.0);
        let say = genos_mcp::json::parse(r#"{"faces":[2],"note":"pull this out 20 cm"}"#).expect("json");
        view.command("mesh_selection", &say, &mut camera, 1.0).expect("select");
        let hide = genos_mcp::json::parse(r#"{"hide":[2]}"#).expect("json");
        view.command("mesh_view", &hide, &mut camera, 1.0).expect("hide");
        let none = genos_mcp::json::parse("{}").expect("json");
        let packet = view.command("mesh_context", &none, &mut camera, 1.0).expect("context");
        assert_eq!(packet.get("intent").and_then(|v| v.as_str()), Some("pull this out 20 cm"));
        let text = genos_mcp::json::encode(&packet);
        // The face's own vertices, where it is and which way it faces.
        assert!(text.contains(r#""selected_faces":[{"face":2,"object":1,"vertices":[5,6,7,8],"centre":[0,0.5,0.5],"normal":[0,0,1]}]"#), "{text}");
        // The hidden bracket is not in view; the cube is, with its bounds.
        assert!(text.contains(r#""visible_objects":[{"id":1,"name":"cube","triangles":12,"min":[-0.5,0,-0.5],"max":[0.5,1,0.5]}]"#), "{text}");
        assert!(text.contains(r#""hidden_objects":[2]"#) && text.contains(r#""recent_edits":[]"#), "{text}");
        // An edit needs the editor's session; the lighting program says so, and
        // the failed command is still in the record with the note it ran under.
        let cube = genos_mcp::json::parse(r#"{"command":{"command":"cube"}}"#).expect("json");
        if crate::program().mesh_session.is_none() {
            let refused = view.command("mesh_command", &cube, &mut camera, 1.0).expect_err("no session");
            assert!(refused.contains("genos-editor"), "{refused}");
            let packet = genos_mcp::json::encode(&view.command("mesh_context", &none, &mut camera, 1.0).expect("context"));
            assert!(packet.contains(r#""command":"cube""#) && packet.contains(r#""ok":false"#) && packet.contains("pull this out"), "{packet}");
        }
        // A preview with nothing to show is refused before any session is opened.
        assert!(view.command("mesh_preview", &none, &mut camera, 1.0).is_err());
    }

    #[test]
    fn the_normals_turn_with_the_pose() {
        let view = view();
        let part = &view.workspace.as_ref().expect("workspace").parts[0];
        // The cube turned a quarter about Y: x goes to z, z goes to -x.
        let turned: Vec<[f32; 3]> = part.positions.iter().map(|p| [-p[2], p[1], p[0]]).collect();
        let normals = super::posed_normals_of(part, &turned);
        for (rest, now) in part.normals.iter().zip(&normals) {
            let want = [-rest[2], rest[1], rest[0]];
            assert!((0..3).all(|k| (want[k] - now[k]).abs() < 1.0e-4), "{rest:?} became {now:?}");
        }
        // At rest the rule gives the rest normals back.
        let same = super::posed_normals_of(part, &part.positions);
        assert!(part.normals.iter().zip(&same).all(|(a, b)| (0..3).all(|k| (a[k] - b[k]).abs() < 1.0e-4)));
        assert!(view.posed_normals().iter().all(Option::is_none), "no pose, no posed normals");
    }

    #[test]
    fn the_picker_lists_the_open_workspace_first_and_each_once_and_the_readout_counts_the_mesh() {
        let mut view = view();
        view.poll_sessions();
        let sample = std::fs::canonicalize(&view.dir).expect("sample");
        assert_eq!(view.found[0], sample);
        assert_eq!(view.found.iter().filter(|dir| **dir == sample).count(), 1);
        assert_eq!(view.statistics(), "rev 1 - 2 objects, 14 faces, 32 tris, 20 verts");
        assert_eq!(view.state(), "live, viewing");
    }

    #[test]
    fn a_box_and_a_lasso_select_what_is_inside_and_in_sight() {
        let mut view = view();
        let mut camera = Camera::new(0.0, 4.0, 0.0);
        let front = genos_mcp::json::parse(r#"{"look":"+z"}"#).expect("json");
        view.command("mesh_view", &front, &mut camera, 1.0).expect("look");
        let pick = |view: &mut MeshView, camera: &mut Camera, args: &str| {
            let args = genos_mcp::json::parse(args).expect("json");
            view.command("mesh_pick", &args, camera, 1.0).expect("pick")
        };
        // The whole picture, faces in sight: the fronts, not the backs.
        pick(&mut view, &mut camera, r#"{"box":[0,0,1,1]}"#);
        let seen = view.selection.faces.clone();
        assert!(seen.contains(&2), "the cube's +Z face: {seen:?}");
        // Through the mesh: every face whose centre is on screen.
        pick(&mut view, &mut camera, r#"{"box":[0,0,1,1],"through":true}"#);
        assert!(view.selection.faces.len() > seen.len() && view.selection.faces.is_superset(&seen));
        assert_eq!(view.selection.faces.len(), 14, "all the faces of both objects");
        // Objects by a box, vertices by a lasso; an empty corner selects nothing.
        pick(&mut view, &mut camera, r#"{"box":[0,0,1,1],"mode":"object"}"#);
        assert_eq!(view.selection.objects.iter().copied().collect::<Vec<_>>(), [1, 2]);
        pick(&mut view, &mut camera, r#"{"lasso":[[0,0],[1,0],[1,1],[0,1]],"mode":"vertex","through":true}"#);
        assert_eq!(view.selection.vertices.len(), 20);
        pick(&mut view, &mut camera, r#"{"box":[0,0,0.02,0.02]}"#);
        assert!(view.selection.is_empty());
        // In the window: Control and a drag is the box; the camera does not turn.
        let size = [1280.0, 720.0];
        let yaw = camera.yaw;
        let at = super::OrbitInput { x: 10.0, y: 10.0, ..super::OrbitInput::default() };
        view.pointer(&mut camera, size, at);
        view.pointer(&mut camera, size, super::OrbitInput { left: true, box_key: true, ..at });
        view.pointer(&mut camera, size, super::OrbitInput { left: true, box_key: true, x: 1270.0, y: 710.0, ..at });
        assert_eq!(view.region_outline().len(), 4, "the box is drawn while it is dragged");
        view.pointer(&mut camera, size, super::OrbitInput { x: 1270.0, y: 710.0, ..at });
        assert!(view.region_outline().is_empty() && (camera.yaw - yaw).abs() < 1.0e-6);
        // A right drag is the box too: it selects the faces in sight, not the
        // backs, and the camera stays.
        let place = camera.position;
        view.pointer(&mut camera, size, super::OrbitInput { right: true, ..at });
        view.pointer(&mut camera, size, super::OrbitInput { right: true, x: 1270.0, y: 710.0, ..at });
        assert_eq!(view.region_outline().len(), 4, "the right drag draws the box");
        view.pointer(&mut camera, size, super::OrbitInput { x: 1270.0, y: 710.0, ..at });
        assert_eq!(view.selection.faces, seen, "the faces in sight");
        assert!(view.region_outline().is_empty() && (camera.yaw - yaw).abs() < 1.0e-6 && (camera.position - place).length() < 1.0e-6);
        // A right click in place is not a box: the selection stays.
        view.pointer(&mut camera, size, super::OrbitInput { right: true, ..at });
        view.pointer(&mut camera, size, at);
        assert_eq!(view.selection.faces, seen);
        assert_eq!(view.selection.faces, seen, "the drag selected what the box command did");
        assert!(!super::inside(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]], [0.1, 0.9]));
    }

    #[test]
    fn grow_selects_what_is_connected_and_the_ring_through_two_faces() {
        let mut view = view();
        let mut camera = Camera::new(0.0, 4.0, 0.0);
        let run = |view: &mut MeshView, camera: &mut Camera, args: &str| {
            let args = genos_mcp::json::parse(args).expect("json");
            view.command("mesh_selection", &args, camera, 1.0)
        };
        // One face of the cube grows to the cube's six, not to the bracket.
        run(&mut view, &mut camera, r#"{"faces":[2],"grow":"connected"}"#).expect("connected");
        assert_eq!(view.selection.faces.len(), 6);
        // A loop needs two neighbours.
        assert!(run(&mut view, &mut camera, r#"{"faces":[2],"grow":"loop"}"#).is_err());
        // Two side faces that share an edge: the ring of four sides, not the two ends.
        let sides: Vec<u64> = {
            let part = &view.workspace.as_ref().expect("workspace").parts[0];
            let upright = |face: u64| {
                let at = part.face_ids.iter().position(|id| *id == face).expect("face") * 3;
                part.normals[at][1].abs() < 0.5
            };
            let mut faces: Vec<u64> = part.face_ids.clone();
            faces.dedup();
            faces.into_iter().filter(|face| upright(*face)).collect()
        };
        assert_eq!(sides.len(), 4);
        let pair = sides
            .iter()
            .flat_map(|a| sides.iter().map(move |b| (*a, *b)))
            .find(|(a, b)| a < b && view.face_vertices(*a).iter().filter(|v| view.face_vertices(*b).contains(v)).count() == 2)
            .expect("two sides that share an edge");
        run(&mut view, &mut camera, &format!(r#"{{"faces":[{},{}],"grow":"loop"}}"#, pair.0, pair.1)).expect("loop");
        assert_eq!(view.selection.faces.iter().copied().collect::<Vec<_>>(), {
            let mut all = sides.clone();
            all.sort_unstable();
            all
        });
    }

    #[test]
    fn a_slow_command_does_not_hold_the_caller_and_replies_come_by_ticket() {
        use std::time::{Duration, Instant};
        struct Slow;
        impl crate::MeshSession for Slow {
            fn execute(&mut self, request: &genos_mcp::json::Value) -> Result<genos_mcp::json::Value, String> {
                std::thread::sleep(Duration::from_millis(150));
                Ok(request.clone())
            }
        }
        let mut worker = super::Worker::start(Box::new(Slow));
        let began = Instant::now();
        let first = worker.send(genos_mcp::json::parse(r#"{"command":"one"}"#).expect("json")).expect("send");
        let second = worker.send(genos_mcp::json::parse(r#"{"command":"two"}"#).expect("json")).expect("send");
        assert!(began.elapsed() < Duration::from_millis(100), "sending does not wait for the session");
        assert!(worker.take(first).is_none(), "still running");
        // The second is answered after the first, and each by its own ticket.
        let two = worker.wait(second).expect("reply");
        assert_eq!(two.get("command").and_then(|v| v.as_str()), Some("two"));
        let one = worker.take(first).expect("the first finished before the second").expect("reply");
        assert_eq!(one.get("command").and_then(|v| v.as_str()), Some("one"));
        assert!(began.elapsed() >= Duration::from_millis(290));
        // A ticket the view does not hold is a closed workspace, not a wait for ever.
        let mut view = view();
        assert!(view.poll_edit(7).is_some_and(|reply| reply.is_err()));
    }

    #[test]
    fn a_reference_table_row_gives_each_command_its_fields() {
        let text = "## UV editing\n\n| Command | Fields | Notes |\n|---|---|---|\n\
| `uv_set` | `values:[{vertex,uv:[u,v],face?},...]` (1-4096 entries), optional `object`, `faces`. Writes corners. |\n\
| `texture_import`, `texture_define` | `name`, `path` to a PNG | Named images; see `uv_set` |\n\
| `uv_set` | `limit` 1..64 | paged |\n\
not a row | `nothing` |\n";
        let mut found = Vec::new();
        super::parse_reference("uv-editing", text, &mut found);
        let names: Vec<&str> = found.iter().map(|doc| doc.command.as_str()).collect();
        assert_eq!(names, ["uv_set", "texture_import", "texture_define"]);
        // Both of its rows: the fields of each, once, and both rows' words.
        assert_eq!(found[0].fields, ["values", "object", "faces", "limit"]);
        // `values:[...]` is written as an array, so the table alone types it.
        assert_eq!(found[0].types, [("values".to_string(), "array".to_string())]);
        assert!(found[0].doc.contains("Writes corners") && found[0].doc.contains("paged"));
        // A command named in the notes is not a field.
        assert_eq!(found[1].fields, ["name", "path"]);
        // An example in the prose types the fields it shows, and lists a
        // command that has no table row.
        let prose = "Send `{\"command\":\"uv_set\",\"faces\":[2],\"values\":[{\"vertex\":5,\"uv\":[0,0]}]}` then\n\
```json\n{\"command\":\"rig_set\",\"bones\":[{\"name\":\"root\",\"head\":[0,0,0]}],\"replace\":true,\"note\":\"a } brace\"}\n```\n";
        super::parse_examples("rigging", prose, &mut found);
        assert_eq!(found[0].types, [("values".to_string(), "array".to_string()), ("faces".to_string(), "array".to_string())]);
        let rig = found.iter().find(|doc| doc.command == "rig_set").expect("listed from its example");
        assert_eq!(rig.fields, ["bones", "replace", "note"]);
        assert_eq!(rig.types[1], ("replace".to_string(), "boolean".to_string()));
        assert!(rig.example.ends_with("brace\"}"), "{}", rig.example);
        // `texture_import` names `path` bare; another command types it, so it takes that type.
        super::parse_examples("io", r#"{"command":"import_glb","path":"a.glb"}"#, &mut found);
        super::infer_types(&mut found);
        let import = found.iter().find(|doc| doc.command == "texture_import").expect("texture_import");
        assert_eq!(import.types, [("path".to_string(), "string".to_string())]);
        assert_eq!(import.inferred, ["path"]);
        // mesh-workbench's refusal types a field when it is about that field.
        assert_eq!(super::type_from_refusal("cap_start", "cap_start must be boolean"), Some("boolean"));
        assert_eq!(super::type_from_refusal("offset", "offset must be an unsigned integer"), Some("number"));
        assert_eq!(super::type_from_refusal("depth_range", "depth_range must be [far_z, near_z]"), Some("array"));
        assert_eq!(super::type_from_refusal("region", "region must be a name"), Some("string"));
        assert_eq!(super::type_from_refusal("up", "profile must contain [u,v] points"), None, "about another field");
        assert_eq!(super::type_from_refusal("view_id", "view_id must be an ID"), None, "an ID's type is not said");
        let value = genos_mcp::json::encode(&super::command_value(&found[0], true));
        assert!(value.contains(r#""faces":{"type":"array"}"#), "{value}");
        assert!(value.contains(r#""tool":"mesh_material""#) && value.contains(r#""command":{"const":"uv_set"}"#), "{value}");
        assert!(value.contains(r#""required":["command"]"#) && value.contains(r#""object":{"description""#), "{value}");
    }

    #[test]
    fn a_click_on_the_gizmo_turns_the_view_and_its_name_changes_the_projection() {
        let mut view = view();
        let mut camera = Camera::new(0.0, 4.0, 0.0);
        let size = [1280.0, 720.0];
        let rest = super::OrbitInput { x: 10.0, y: 400.0, ..super::OrbitInput::default() };
        view.pointer(&mut camera, size, rest);
        let click = |view: &mut MeshView, camera: &mut Camera, at: [f32; 2]| {
            let here = super::OrbitInput { x: at[0], y: at[1], ..super::OrbitInput::default() };
            view.pointer(camera, size, here);
            view.pointer(camera, size, super::OrbitInput { left: true, ..here });
            view.pointer(camera, size, here);
        };
        // Six ends and the projection's name, in the top right corner.
        let ends = view.gizmo(&camera, size);
        assert_eq!(ends.len(), 8);
        assert!(ends.iter().all(|end| end.at[0] > 1100.0 && end.at[1] < 170.0), "{ends:?}");
        // The +X end: the camera goes to the +X side, and nothing is picked.
        let x = ends.iter().find(|end| end.label == "X").expect("X").at;
        click(&mut view, &mut camera, x);
        assert!(camera.position.x > 2.0 && camera.pitch.abs() < 1.0e-6, "{:?}", camera.position);
        assert!(view.selection.is_empty(), "a gizmo click is not a pick");
        // From +X, the X end is the nearest: it sits at the gizmo's centre.
        let ends = view.gizmo(&camera, size);
        let x = ends.iter().find(|end| end.label == "X").expect("X");
        assert!((x.at[0] - (size[0] - super::GIZMO_INSET)).abs() < 1.0 && (x.at[1] - super::GIZMO_INSET).abs() < 1.0);
        // The name under it toggles the projection.
        let name = ends.iter().find(|end| end.look.is_none() && !end.panel).expect("projection").clone();
        assert_eq!(name.label, "PERSPECTIVE");
        click(&mut view, &mut camera, name.at);
        assert!(view.orthographic && camera.parallel_half > 0.0);
        assert!(view.gizmo(&camera, size).iter().any(|end| end.label == "ORTHOGRAPHIC"));
        // The button under it closes and opens the Objects panel.
        let button = view.gizmo(&camera, size).into_iter().find(|end| end.panel).expect("panel button");
        assert!(view.panel_open);
        click(&mut view, &mut camera, button.at);
        assert!(!view.panel_open);
        click(&mut view, &mut camera, button.at);
        assert!(view.panel_open);
        // The words on the picture.
        let hud = view.hud();
        assert_eq!(hud[0], "SAMPLE   LIVE   REV 1");
        assert_eq!(hud[1], "2 OBJECTS  20 VERTICES  14 FACES");
    }

    #[test]
    fn a_face_wound_the_wrong_way_is_red_from_outside() {
        let mut view = view();
        view.shade = "face_orientation".to_string();
        // Far out on +X +Y +Z: every triangle the eye can see shows its front.
        let eye = [50.0, 50.0, 50.0];
        view.watch(eye);
        let before = view.display()[0].2.clone();
        let front = before.iter().position(|c| *c == super::FRONT_COLOR).expect("a front face");
        assert!(before.contains(&super::BACK_COLOR), "the far side of the cube is seen from behind");
        // Turn that triangle over.
        let part = view.workspace.as_mut().map(|w| &mut w.parts[0]).expect("cube");
        part.positions.swap(front + 1, front + 2);
        view.display_stale = false;
        view.watch(eye);
        assert!(view.display_stale, "a face that turned over redraws the mesh");
        assert_eq!(view.display()[0].2[front], super::BACK_COLOR);
        // The same eye again changes nothing, so nothing is rebuilt.
        view.display_stale = false;
        view.watch(eye);
        assert!(!view.display_stale);
    }

    #[test]
    fn a_point_inside_a_screen_triangle_has_weights_and_one_outside_has_none() {
        let t = [[0.0, 0.0, 0.5], [1.0, 0.0, 0.5], [0.0, 1.0, 0.5]];
        let w = barycentric(t, [0.25, 0.25]).expect("inside");
        assert!((w[0] - 0.5).abs() < 1.0e-6 && (w[1] - 0.25).abs() < 1.0e-6);
        assert!(barycentric(t, [0.9, 0.9]).is_none());
    }

    #[test]
    fn a_pick_at_the_cube_returns_the_cube_and_a_pick_at_the_sky_returns_nothing() {
        let view = view();
        // Stand in front of the cube (centred on x 0, 0 to 1 m high) and look at it.
        let mut camera = Camera::opening();
        camera.position = genos_scene::Vec3::new(0.0, 0.5, 4.0);
        camera.yaw = 0.0;
        camera.pitch = 0.0;
        let hit = view.pick(&camera, 16.0 / 9.0, [0.5, 0.5]).expect("the cube is under the centre");
        assert_eq!((hit.object, hit.name.as_str()), (1, "cube"));
        assert_eq!(hit.face, 2, "the +Z face looks at the camera");
        assert!((hit.point[2] - 0.5).abs() < 1.0e-3, "{:?}", hit.point);
        assert!(view.pick(&camera, 16.0 / 9.0, [0.5, 0.02]).is_none());
    }

    #[test]
    fn a_click_selects_a_drag_orbits_about_the_mesh_and_the_wheel_zooms() {
        use super::OrbitInput;
        let mut view = view();
        let mut camera = Camera::opening();
        camera.position = genos_scene::Vec3::new(0.0, 0.5, 4.0);
        camera.yaw = 0.0;
        camera.pitch = 0.0;
        let size = [1280.0, 720.0];
        let at = OrbitInput { x: 640.0, y: 360.0, ..OrbitInput::default() };
        // Press and release in place: a click on the cube's +Z face.
        view.pointer(&mut camera, size, at);
        view.pointer(&mut camera, size, OrbitInput { left: true, ..at });
        view.pointer(&mut camera, size, at);
        assert_eq!(view.selection.faces.iter().copied().collect::<Vec<_>>(), vec![2]);
        assert!((camera.position.z - 4.0).abs() < 1.0e-5, "a click does not move the camera");
        // Press, drag 100 px right, release: the camera turns and keeps its distance.
        let far = |c: &Camera| ((c.position.x).powi(2) + (c.position.y - 0.5).powi(2) + c.position.z.powi(2)).sqrt();
        let before = far(&camera);
        view.pointer(&mut camera, size, OrbitInput { left: true, ..at });
        view.pointer(&mut camera, size, OrbitInput { left: true, x: 740.0, ..at });
        view.pointer(&mut camera, size, OrbitInput { x: 740.0, ..at });
        assert!(camera.yaw > 0.5, "yaw {}", camera.yaw);
        assert!((far(&camera) - before).abs() < 1.0e-3, "orbit keeps the distance");
        assert_eq!(view.selection.faces.len(), 1, "a drag is not a click");
        // One wheel step in moves closer.
        view.pointer(&mut camera, size, OrbitInput { x: 740.0, wheel: 1.0, ..at });
        assert!(far(&camera) < before * 0.95);
        // A press over the UI does nothing here.
        let yaw = camera.yaw;
        view.pointer(&mut camera, size, OrbitInput { x: 740.0, left: true, over_ui: true, ..at });
        view.pointer(&mut camera, size, OrbitInput { x: 840.0, left: true, over_ui: true, ..at });
        assert!((camera.yaw - yaw).abs() < 1.0e-6);
        // The turntable turns with time and keeps the distance.
        view.pointer(&mut camera, size, OrbitInput { x: 840.0, ..at });
        view.turntable = true;
        let (yaw, distance) = (camera.yaw, far(&camera));
        view.pointer(&mut camera, size, OrbitInput { x: 840.0, dt: 1.0, ..at });
        assert!(camera.yaw - yaw > 0.3, "the turntable turned {}", camera.yaw - yaw);
        assert!((far(&camera) - distance).abs() < 1.0e-3);
        view.turntable = false;
        // A middle drag pans: the camera moves and does not turn.
        let (yaw, place) = (camera.yaw, camera.position);
        view.pointer(&mut camera, size, OrbitInput { x: 840.0, middle: true, ..at });
        view.pointer(&mut camera, size, OrbitInput { x: 800.0, middle: true, ..at });
        view.pointer(&mut camera, size, OrbitInput { x: 800.0, ..at });
        assert!((camera.position - place).length() > 0.05, "a middle drag moved the camera");
        assert!((camera.yaw - yaw).abs() < 1.0e-6);
        // A right drag does not move it.
        let place = camera.position;
        view.pointer(&mut camera, size, OrbitInput { x: 800.0, right: true, ..at });
        view.pointer(&mut camera, size, OrbitInput { x: 840.0, right: true, ..at });
        view.pointer(&mut camera, size, OrbitInput { x: 840.0, ..at });
        assert!((camera.position - place).length() < 1.0e-6);
    }

    #[test]
    fn solo_hides_the_other_objects_and_object_ids_gives_each_its_own_colour() {
        let mut view = view();
        let camera = Camera::opening();
        let none = genos_mcp::json::parse("{}").expect("json");
        assert!(view.display().iter().all(|(hidden, color, colors, uvs)| !hidden && *color == super::MESH_COLOR && colors.is_empty() && uvs.is_none()));
        let mut camera = camera;
        let solo = genos_mcp::json::parse(r#"{"solo":[2],"shade":"object_ids"}"#).expect("json");
        view.command("mesh_view", &solo, &mut camera, 1.0).expect("solo");
        let shown = view.display();
        assert_eq!((shown[0].0, shown[1].0), (true, false), "the cube is hidden, the bracket stays");
        assert_ne!(shown[0].1, shown[1].1, "two objects, two colours");
        let all = genos_mcp::json::parse(r#"{"show_all":true}"#).expect("json");
        view.command("mesh_view", &all, &mut camera, 1.0).expect("show all");
        assert!(view.display().iter().all(|(hidden, _, _, _)| !hidden));
        // Face IDs: a colour for every vertex, the same on a triangle's three.
        let faces = genos_mcp::json::parse(r#"{"shade":"face_ids"}"#).expect("json");
        view.command("mesh_view", &faces, &mut camera, 1.0).expect("face ids");
        let cube = &view.display()[0].2;
        assert_eq!(cube.len(), 36);
        assert_eq!((cube[0], cube[1]), (cube[1], cube[2]));
        assert_ne!(cube[0], cube[35], "the first and last faces differ");
        assert!(view.command("mesh_view", &none, &mut camera, 1.0).is_ok());
        // The grid: the sample is 2.9 m across, so lines are 1 m apart, 6 each
        // way of the centre and both directions, two strips a line.
        assert_eq!(view.grid_strips().0.len(), 13 * 2 * 2 * 12);
        let off = genos_mcp::json::parse(r#"{"grid":false}"#).expect("json");
        view.command("mesh_view", &off, &mut camera, 1.0).expect("grid off");
        assert_eq!(view.grid_strips().0.len(), 0);
        // Tessellation: every triangle edge once, the diagonal of each quad
        // included. The cube: 12 edges and 6 diagonals.
        let hide = genos_mcp::json::parse(r#"{"shade":"tessellation","hide":[2]}"#).expect("json");
        view.command("mesh_view", &hide, &mut camera, 1.0).expect("tessellation");
        assert_eq!(view.wire().0.len(), (12 + 6) * 2 * 12);
        let show = genos_mcp::json::parse(r#"{"show_all":true,"shade":"shaded"}"#).expect("json");
        view.command("mesh_view", &show, &mut camera, 1.0).expect("show");
        // The topology shade names what the camera sees: from +Z, the cube's
        // front face (2) and its four front vertices are among the labels,
        // and the back face (1) is not.
        let front = genos_mcp::json::parse(r#"{"shade":"topology","look":"+z"}"#).expect("json");
        view.command("mesh_view", &front, &mut camera, 1.0).expect("topology");
        let labels = view.id_labels(&camera, 1.0).to_vec();
        let named = |face: bool, id: &str| labels.iter().any(|label| label.face == face && label.text == id);
        assert!(named(true, "2") && !named(true, "1"), "{labels:?}");
        assert!(["5", "6", "7", "8"].iter().all(|vertex| named(false, vertex)), "{labels:?}");
        let off = genos_mcp::json::parse(r#"{"labels":false}"#).expect("json");
        view.command("mesh_view", &off, &mut camera, 1.0).expect("labels off");
        assert_eq!(view.id_labels(&camera, 1.0).len(), 0);
        let on = genos_mcp::json::parse(r#"{"labels":true,"shade":"shaded","look":"frame"}"#).expect("json");
        view.command("mesh_view", &on, &mut camera, 1.0).expect("labels on");
        assert!(view.id_labels(&camera, 1.0).is_empty(), "only the topology shade has labels");
        // Wireframe: the cube has 12 edges and the bracket 18, two strips each.
        assert_eq!(view.wire().0.len(), 0);
        let wire = genos_mcp::json::parse(r#"{"shade":"wireframe"}"#).expect("json");
        view.command("mesh_view", &wire, &mut camera, 1.0).expect("wireframe");
        assert_eq!(view.wire().0.len(), (12 + 18) * 2 * 12);
        // Looking from +X puts the camera on the +X side, level with the mesh.
        let side = genos_mcp::json::parse(r#"{"look":"+x"}"#).expect("json");
        view.command("mesh_view", &side, &mut camera, 1.0).expect("look");
        assert!(camera.position.x > 2.4 && camera.pitch.abs() < 1.0e-6, "{:?}", camera.position);
    }

    #[test]
    fn each_mesh_command_has_one_family_and_a_tool_refuses_another_family() {
        use super::family_of;
        for (command, family) in [
            ("rig_set", "mesh_rig"),
            ("skin_weights_set", "mesh_rig"),
            ("animation_keys_edit", "mesh_animation"),
            ("texture_layer_paint", "mesh_material"),
            ("uv_project", "mesh_material"),
            ("vertex_color_set", "mesh_material"),
            ("object_create_box", "mesh_object"),
            ("select_loop", "mesh_select"),
            ("undo", "mesh_document"),
            ("import_glb", "mesh_document"),
            ("inspect_region", "mesh_query"),
            ("query_mesh", "mesh_query"),
            ("extrude", "mesh_edit"),
            ("bevel_edges", "mesh_edit"),
        ] {
            assert_eq!(family_of(command), family, "{command}");
        }
        let mut view = view();
        let mut camera = Camera::opening();
        // The family check comes before the session opens: nothing is written.
        let wrong = genos_mcp::json::parse(r#"{"command":"rig_set"}"#).expect("json");
        let err = view.command("mesh_animation", &wrong, &mut camera, 1.0).expect_err("refused");
        assert!(err.contains("mesh_rig"), "{err}");
    }

    #[test]
    fn t_starts_the_note_letters_fill_it_and_enter_ends_it() {
        use genos_input::InputCode;
        let mut view = view();
        let key = |want: InputCode| move |code: InputCode| code == want;
        assert!(!view.type_note(&key(InputCode::key_a), false), "a letter before T is not the note's");
        assert!(view.type_note(&key(InputCode::key_t), false));
        assert!(view.typing && view.selection.note.is_empty(), "T starts typing and is not typed");
        view.type_note(&key(InputCode::key_h), true);
        view.type_note(&key(InputCode::key_i), false);
        view.type_note(&key(InputCode::key_space), false);
        view.type_note(&key(InputCode::key_2), false);
        view.type_note(&key(InputCode::key_x), false);
        view.type_note(&key(InputCode::key_backspace), false);
        assert_eq!(view.selection.note, "Hi 2");
        assert!(view.type_note(&key(InputCode::key_enter), false));
        assert!(!view.typing);
        assert_eq!(view.selection.note, "Hi 2", "Enter keeps the note");
    }

    #[test]
    fn the_overlay_covers_a_selected_object_and_marks_a_selected_vertex() {
        let mut view = view();
        assert_eq!(view.overlay().0.len(), 0);
        view.selection = Selection { objects: [1].into(), ..Selection::default() };
        // The cube: 6 quads, 12 triangles.
        assert_eq!(view.overlay().0.len(), 36);
        view.selection = Selection { vertices: [9].into(), ..Selection::default() };
        // One octahedron: 8 triangles.
        assert_eq!(view.overlay().0.len(), 24);
        view.selection = Selection { faces: [7].into(), ..Selection::default() };
        // The L-shaped top face: a hexagon, 4 triangles.
        assert_eq!(view.overlay().0.len(), 12);
    }
}
