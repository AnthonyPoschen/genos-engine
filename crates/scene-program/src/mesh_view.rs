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

/// A steady colour for an object ID: the golden-ratio turn of the hue, so
/// neighbouring IDs are far apart.
#[expect(clippy::cast_precision_loss, reason = "an object ID is a small counter")]
pub fn id_color(object: u64) -> [f32; 3] {
    let hue = (object as f32 * 0.618_034).fract() * 6.0;
    let x = 1.0 - (hue % 2.0 - 1.0).abs();
    let (r, g, b) = match hue as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    // Keep some grey in it so the shading still reads.
    [0.25 + 0.6 * r, 0.25 + 0.6 * g, 0.25 + 0.6 * b]
}

/// Radians of orbit per pixel of drag.
const ORBIT_TURN: f32 = 0.006;
/// Share of the orbit distance one pixel of right drag pans.
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
    pub right: bool,
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

/// Orbit about a point: left drag turns, right drag pans, the wheel zooms.
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
    /// The recorded sessions, oldest first, as of the last poll.
    pub recorded: Vec<genos_workbench::Session>,
    /// A box or a lasso being dragged, in pixels: the box's two corners, or
    /// the lasso's path.
    pub region: Option<Vec<[f32; 2]>>,
    /// The drag is a lasso, not a box.
    region_lasso: bool,
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
    bridge: Option<Box<dyn crate::MeshSession>>,
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
pub const VIEW_ARGS: [&str; 22] = [
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
pub const GRID_COLOR: [f32; 3] = [0.16, 0.17, 0.19];
/// Half the width of a bone strip, as a share of the mesh size.
const BONE_WIDTH: f32 = 0.006;
pub const BONE_COLOR: [f32; 3] = [0.1, 0.85, 0.35];

#[expect(clippy::cast_possible_truncation, reason = "the renderer draws f32 positions")]
fn point(value: &Value) -> Option<[f32; 3]> {
    let items = value.as_array()?;
    if items.len() != 3 {
        return None;
    }
    Some([items[0].as_f64()? as f32, items[1].as_f64()? as f32, items[2].as_f64()? as f32])
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

/// The shade modes, in the order the menu lists them.
pub const SHADES: [&str; 11] = [
    "shaded",
    "wireframe",
    "vertex_colors",
    "object_ids",
    "face_ids",
    "collection_ids",
    "silhouette",
    "textured",
    "uv_checker",
    "face_orientation",
    "vertex_colors_unlit",
];

/// A face whose UVs are incomplete, in the textured and checker shades.
const UNMAPPED_COLOR: [f32; 3] = [1.0, 0.0, 1.0];

/// The most lines the window's Objects panel lists, and the most commands
/// its Replay panel lists.
const PANEL_OBJECTS: usize = 200;
const PANEL_EVENTS: usize = 12;

/// Triangles with a colour per vertex: positions, normals, colours.
pub type Painted = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 3]>);

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
}

/// The orthographic view: the tangent of half its field of view (1.5
/// degrees: a part 1 m nearer draws under 1% larger at 3 m of mesh), and the
/// furthest the camera stands for it.
const ORTHO_TAN: f32 = 0.026_19;
const ORTHO_REACH: f32 = 120.0;

/// The contact shadow: how dark its centre is, how far past the footprint it
/// reaches, its sides, and (radius, darkness) from the centre to the rim.
const SHADOW_DARK: f32 = 0.6;
const SHADOW_REACH: f32 = 1.15;
const SHADOW_SIDES: usize = 40;
const SHADOW_RINGS: [(f32, f32); 5] = [(0.0, 1.0), (0.45, 0.9), (0.7, 0.55), (0.88, 0.2), (1.0, 0.0)];

/// `face_orientation`: a face seen from its front, and one seen from behind.
const FRONT_COLOR: [f32; 3] = [0.15, 0.3, 0.9];
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
            let light = (x / CHECKER_CELL + y / CHECKER_CELL).is_multiple_of(2);
            // U runs to red and V to green, so a flipped or turned UV shows.
            let texel = if light {
                [230, 230, 230, 255]
            } else {
                [(60 + x * 150 / CHECKER) as u8, (60 + y * 150 / CHECKER) as u8, 70, 255]
            };
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
                        #[expect(clippy::cast_possible_truncation, reason = "a blend of two bytes is a byte")]
                        let blended = ((u32::from(*channel) * alpha + 128 * (255 - alpha)) / 255) as u8;
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
        if !checker && texture.is_none() {
            continue;
        }
        let at = triangle * 3;
        let Some(mapped) = triangle_uvs(part, triangle) else {
            colors[at..at + 3].fill(UNMAPPED_COLOR);
            continue;
        };
        uvs[at..at + 3].copy_from_slice(&mapped);
        for (corner, color) in colors[at..at + 3].iter_mut().enumerate() {
            *color = if checker { [1.0; 3] } else { part.colors.get(at + corner).copied().unwrap_or([1.0; 3]) };
        }
        *slot = texture;
    }
    (uvs, colors, drawn)
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
            recorded: Vec::new(),
            region: None,
            region_lasso: false,
            found: Vec::new(),
            frame_next: false,
            replay: None,
            rebuild: false,
            display_stale: true,
            bridge: None,
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
            if pressed(InputCode::key_t) {
                self.typing = true;
                self.typing_filter = shift;
                return true;
            }
            return false;
        }
        let words = if self.typing_filter { &mut self.filter } else { &mut self.selection.note };
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
                    words.push(if shift { letter.to_ascii_uppercase() } else { letter });
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
            lines.push(PanelLine {
                label: format!("{} ({})", collection.name, collection.objects.len()),
                check: "show",
                on: collection.objects.iter().any(|object| shown(*object)),
                target: PanelTarget::Collection(collection.id),
            });
            lines.extend(members.into_iter().map(|part| line(part, "    ")));
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
        }];
        lines.extend(session.events.iter().rev().filter(|event| self.passes(&event.command)).take(PANEL_EVENTS).map(|event| {
            PanelLine {
                label: format!("{} {}{}", event.id, event.command, if event.ok { "" } else { " (refused)" }),
                check: "shown",
                on: at == Some(event.id),
                target: PanelTarget::Event(event.id),
            }
        }));
        lines
    }

    /// A check box of a panel line was set to `on`.
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
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small frame count")]
        let count = ((length * PLAY_RATE).ceil() as usize).clamp(1, PLAY_SAMPLES_MAX);
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
        for (at, part) in workspace.parts.iter().enumerate() {
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
            let height = low[1] - workspace.min[1];
            let strength = SHADOW_DARK * (1.0 - height / size).clamp(0.0, 1.0);
            let centre = [f32::midpoint(low[0], high[0]), f32::midpoint(low[2], high[2])];
            // The ellipse through the footprint's corners, and a soft margin.
            let reach = [0, 2].map(|axis| (high[axis] - low[axis]) * 0.5 * std::f32::consts::SQRT_2 * SHADOW_REACH);
            // Below the grid lines; each patch a little above the last, so two that overlap do not flicker.
            let y = workspace.min[1] + OVERLAY_OFFSET * (0.2 + 0.5 * at as f32 / workspace.parts.len() as f32);
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
        if self.shade != "wireframe" {
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
        let textured = checker || self.shade == "textured";
        workspace
            .parts
            .iter()
            .enumerate()
            .map(|(at, part)| {
                let color = match self.shade.as_str() {
                    "object_ids" => id_color(part.object),
                    // Black whatever the light: the outline against the ground and the sky.
                    "silhouette" => [0.0; 3],
                    // The first collection that holds the object; none is grey.
                    "collection_ids" => workspace
                        .collections
                        .iter()
                        .find(|collection| collection.objects.contains(&part.object))
                        .map_or(MESH_COLOR, |collection| id_color(collection.id)),
                    _ => MESH_COLOR,
                };
                let hidden = !self.surfaces || self.hidden.contains(&part.object);
                if textured {
                    let (uvs, colors, drawn) = textured_corners(part, &workspace.textures, checker);
                    let used: BTreeSet<usize> = drawn.iter().flatten().copied().collect();
                    let surface = match (checker, used.len()) {
                        (true, _) => SURFACE_CHECKER,
                        (false, 1) => SURFACE_TEXTURES + used.first().copied().unwrap_or(0),
                        // No texture: the colours alone. Several: `textured_pieces` draws it.
                        (false, 0) => return (hidden, color, colors, None),
                        (false, _) => return (true, color, Vec::new(), None),
                    };
                    return (hidden, color, colors, Some((surface, uvs)));
                }
                // One colour per vertex, or none for the object's one colour.
                let colors = match self.shade.as_str() {
                    "vertex_colors" | "vertex_colors_unlit" => part.colors.clone(),
                    "face_ids" => part.face_ids.iter().flat_map(|face| [id_color(*face); 3]).collect(),
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

    /// The shades mesh-workbench draws without light: the program shows the
    /// base colour for them.
    pub fn unlit(&self) -> bool {
        matches!(self.shade.as_str(), "textured" | "uv_checker" | "vertex_colors_unlit")
    }

    /// In the `textured` shade, each part that uses several textures, as one
    /// piece a texture and one for its faces with none. Positions are posed.
    pub fn textured_pieces(&self) -> Vec<Piece> {
        let Some(workspace) = self.workspace.as_ref().filter(|_| self.shade == "textured" && self.surfaces) else {
            return Vec::new();
        };
        let posed = self.posed_normals();
        let mut pieces = Vec::new();
        for (index, part) in workspace.parts.iter().enumerate() {
            if self.hidden.contains(&part.object) {
                continue;
            }
            let (uvs, colors, drawn) = textured_corners(part, &workspace.textures, false);
            let used: BTreeSet<Option<usize>> = drawn.iter().copied().collect();
            if used.iter().flatten().count() < 2 {
                continue;
            }
            let normals = posed.get(index).and_then(Option::as_ref).unwrap_or(&part.normals);
            for texture in used {
                let corners: Vec<usize> =
                    (0..part.triangles()).filter(|t| drawn[*t] == texture).flat_map(|t| t * 3..t * 3 + 3).collect();
                pieces.push(Piece {
                    positions: corners.iter().map(|at| self.corner(part, *at)).collect(),
                    normals: corners.iter().map(|at| normals.get(*at).copied().unwrap_or([0.0, 1.0, 0.0])).collect(),
                    colors: corners.iter().map(|at| colors[*at]).collect(),
                    texcoords: corners.iter().map(|at| uvs[*at]).collect(),
                    surface: texture.map(|at| SURFACE_TEXTURES + at),
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
    /// A left drag with Control (a box) or Alt (a lasso). Returns whether the
    /// drag has the left button this frame. The release selects what is
    /// inside, by [`Self::pick_mode`]; Shift adds to the selection.
    fn drag_region(&mut self, camera: &Camera, size: [f32; 2], input: &OrbitInput) -> bool {
        let at = [input.x, input.y];
        match (&mut self.region, input.left) {
            (None, true) if !input.over_ui && (input.box_key || input.lasso_key) && self.orbit.is_none_or(|o| o.dragged.is_none()) => {
                self.region_lasso = input.lasso_key && !input.box_key;
                self.region = Some(vec![at, at]);
                true
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
                true
            }
            (Some(_), false) => {
                let outline: Vec<[f32; 2]> = self
                    .region_outline()
                    .into_iter()
                    .map(|p| [p[0] / size[0].max(1.0), p[1] / size[1].max(1.0)])
                    .collect();
                self.region = None;
                let mode = self.pick_mode.clone();
                self.select_inside(camera, size[0] / size[1].max(1.0), &outline, &mode, input.add, false);
                true
            }
            (None, _) => false,
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
        self.watch([camera.position.x, camera.position.y, camera.position.z]);
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
            // The orbit's distance is the 60 degree lens's; a narrow lens stands further back.
            let lens = camera.view_tan / genos_scene::DEFAULT_VIEW_TAN;
            let along = (to[0] * forward.x + to[1] * forward.y + to[2] * forward.z).max(extent * 0.25 / lens);
            let previous = self.orbit;
            self.orbit = Some(Orbit {
                target: [pose.0[0] + forward.x * along, pose.0[1] + forward.y * along, pose.0[2] + forward.z * along],
                distance: along * lens,
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
        let turning = self.turntable && !input.left && !input.right;
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
        if input.right && !input.left {
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
                        #[expect(clippy::cast_possible_truncation, reason = "a time in seconds")]
                        let time = value.get("time").and_then(Value::as_f64).unwrap_or(0.0) as f32;
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
                                #[expect(clippy::cast_possible_truncation, reason = "a screen coordinate")]
                                Some([x, y]) => x.as_f64().zip(y.as_f64()).map(|(x, y)| [x as f32, y as f32]),
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
            "mesh_preview" => {
                args.only("mesh_preview", &PREVIEW_ARGS)?;
                self.preview_command(&args)
            }
            "mesh_context" => {
                args.only("mesh_context", &CONTEXT_ARGS)?;
                let edits = args.0.get("edits").and_then(Value::as_u64).map_or(8, |n| n.min(EDITS_KEPT as u64));
                #[expect(clippy::cast_possible_truncation, reason = "at most EDITS_KEPT")]
                Ok(self.context_value(camera, edits as usize))
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

    /// Run a mesh-workbench command, or a list, on the linked session. The
    /// first call opens the session on the workspace.
    fn run(&mut self, command: &Value) -> Result<Value, String> {
        if self.bridge.is_none() {
            let open = crate::program()
                .mesh_session
                .ok_or("this program does not link mesh-workbench: run genos-editor to edit a mesh")?;
            self.bridge = Some(open(&self.dir)?);
        }
        let bridge = self.bridge.as_mut().ok_or("the mesh session did not open")?;
        let reply = bridge.execute(command)?;
        // The session wrote project.json: show it on the next frame.
        if let Some(earlier) = Instant::now().checked_sub(FOLLOW_NOW) {
            self.looked = earlier;
        }
        Ok(reply)
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
        let result = self.run(request);
        if changes {
            self.note_edit(command, request, &result);
            self.end_preview();
        }
        result
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

/// The orthographic view's lens: [`ORTHO_TAN`], or wider when that would
/// stand the camera past [`ORTHO_REACH`] metres (the far plane is 200 m).
fn ortho_tan(distance: f32) -> f32 {
    ORTHO_TAN.max(genos_scene::DEFAULT_VIEW_TAN * distance / ORTHO_REACH).min(genos_scene::DEFAULT_VIEW_TAN)
}

impl Orbit {
    /// Stand back from the target along the camera's direction. `distance`
    /// is how far the 60 degree lens stands. The orthographic view keeps the
    /// same framing with a narrow lens from further back, so near and far
    /// parts of the mesh draw at almost the same size.
    fn write(&mut self, camera: &mut Camera, orthographic: bool) {
        camera.view_tan = if orthographic { ortho_tan(self.distance) } else { genos_scene::DEFAULT_VIEW_TAN };
        let back = self.distance * genos_scene::DEFAULT_VIEW_TAN / camera.view_tan;
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

    #[expect(clippy::cast_possible_truncation, reason = "a screen coordinate")]
    fn floats(&self, key: &str, count: usize) -> Result<Option<Vec<f32>>, String> {
        let Some(value) = self.0.get(key) else {
            return Ok(None);
        };
        let items = value.as_array().filter(|items| items.len() == count).ok_or_else(|| format!("{key} is {count} numbers"))?;
        items
            .iter()
            .map(|item| item.as_f64().map(|n| n as f32).ok_or_else(|| format!("{key} is {count} numbers")))
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
    #[expect(clippy::cast_possible_wrap, reason = "mesh-workbench IDs are small counters")]
    json::int(value as i64)
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
        assert_eq!(texel(0, 0), [230, 230, 230, 255]);

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
        // second has a texture and no UVs. The rest have no texture.
        part.uvs = vec![[0.0; 2]; part.positions.len()];
        part.uv_mapped = vec![false; part.positions.len()];
        part.uvs[..3].copy_from_slice(&[[-0.5, 0.25], [1.5, 0.25], [0.0, 0.75]]);
        part.uv_mapped[..3].fill(true);
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
        assert_eq!(shown[0].2[6], super::MESH_COLOR, "a face with no texture is grey");
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
        assert_eq!(positions.len(), per_object * 2, "the cube and the bracket");
        assert!(normals.iter().all(|n| *n == [0.0, 1.0, 0.0]));
        // Dark at the centre, the ground's own colour at the rim, never brighter.
        let darkest = colors.iter().map(|c| c[0]).fold(f32::INFINITY, f32::min);
        assert!((darkest - 0.3 * (1.0 - super::SHADOW_DARK)).abs() < 1.0e-5, "{darkest}");
        assert!(colors.contains(&ground) && colors.iter().all(|c| c[0] <= ground[0]));
        // It reaches past the cube (1 m across, centred on the origin) and stays on the ground.
        let cube = &positions[..per_object];
        let reach = cube.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        assert!(reach > 0.5 && reach < 1.0, "{reach}");
        // A hidden object has none; neither does a view with the shadow off.
        let mut camera = Camera::new(0.0, 0.0, 0.0);
        let hide = genos_mcp::json::parse(r#"{"hide":[1]}"#).expect("json");
        view.command("mesh_view", &hide, &mut camera, 1.0).expect("hide");
        assert_eq!(view.shadow_patches(ground).0.len(), per_object);
        let off = genos_mcp::json::parse(r#"{"shadow":false}"#).expect("json");
        view.command("mesh_view", &off, &mut camera, 1.0).expect("shadow off");
        assert!(view.shadow_patches(ground).0.is_empty());
    }

    #[test]
    fn each_mesh_tool_publishes_a_schema_that_names_the_fields_it_takes() {
        let keys = |schema: &genos_mcp::json::Value| match schema.get("properties") {
            Some(genos_mcp::json::Value::Object(pairs)) => pairs.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let lists: [(&str, &[&str]); 6] = [
            ("mesh_view", &super::VIEW_ARGS),
            ("mesh_pick", &super::PICK_ARGS),
            ("mesh_selection", &super::SELECTION_ARGS),
            ("mesh_help", &super::HELP_ARGS),
            ("mesh_preview", &super::PREVIEW_ARGS),
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
        assert!(view.replay_lines().is_empty());
        let event = |id, command: &str, ok| Event { id, command: command.into(), ok, snapshot: None };
        view.recorded = vec![
            Session { id: "old".into(), initial_snapshot: 0, events: vec![event(1, "inspect", true)] },
            Session { id: "new".into(), initial_snapshot: 0, events: vec![event(1, "cube", true), event(2, "extrude", false), event(3, "move", true)] },
        ];
        let lines = view.replay_lines();
        let commands: Vec<&str> = lines.iter().map(|line| line.label.as_str()).collect();
        assert_eq!(commands, ["live", "3 move", "2 extrude (refused)", "1 cube"]);
        assert!(lines[0].on && lines[1..].iter().all(|line| !line.on));
        view.filter = "ex".into();
        assert_eq!(view.replay_lines().len(), 2, "live and the one match");
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
        assert!(camera.view_tan < 0.03 && camera.position.x > stood * 10.0, "a narrow lens from far back");
        assert!(camera.position.x < genos_scene::CAMERA_FAR, "inside the far plane");
        let (plane_o, near_o) = (across(&camera, 0.0), across(&camera, 1.0));
        assert!((plane_o - plane).abs() < 1.0e-4, "the framing is kept: {plane_o} {plane}");
        assert!(near_o < plane_o * 1.02, "near and far draw the same size: {near_o} {plane_o}");
        // A pick still finds the mesh.
        assert!(view.pick(&camera, 1.0, [0.5, 0.5]).is_some());

        let back = genos_mcp::json::parse(r#"{"projection":"perspective"}"#).expect("json");
        view.command("mesh_view", &back, &mut camera, 1.0).expect("perspective");
        assert!((camera.view_tan - genos_scene::DEFAULT_VIEW_TAN).abs() < 1.0e-6 && (camera.position.x - stood).abs() < 1.0e-3);
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
        assert!(cube[0] == cube[1] && cube[1] == cube[2]);
        assert_ne!(cube[0], cube[35], "the first and last faces differ");
        assert!(view.command("mesh_view", &none, &mut camera, 1.0).is_ok());
        // The grid: the sample is 2.9 m across, so lines are 1 m apart, 6 each
        // way of the centre and both directions, two strips a line.
        assert_eq!(view.grid_strips().0.len(), 13 * 2 * 2 * 12);
        let off = genos_mcp::json::parse(r#"{"grid":false}"#).expect("json");
        view.command("mesh_view", &off, &mut camera, 1.0).expect("grid off");
        assert!(view.grid_strips().0.is_empty());
        // Wireframe: the cube has 12 edges and the bracket 18, two strips each.
        assert!(view.wire().0.is_empty());
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
        assert!(view.overlay().0.is_empty());
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
