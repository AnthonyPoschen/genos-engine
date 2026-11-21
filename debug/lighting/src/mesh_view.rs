//! The mesh viewer's state and its agent commands.
//!
//! A mesh scene shows a mesh-workbench workspace. This module keeps what the
//! person selected in that picture (objects, faces, vertices, a point, a typed
//! note), answers a pick at a screen point, and passes a mesh-workbench command
//! through to `mesh-server`. The record is `docs/systems/mesh-workbench.md`.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use genos_debug::value::{list, num, obj, text, vec3, Args};
use genos_mcp::json::{self, Value};
use genos_scene::Camera;
use genos_workbench::{Part, Workspace};

/// Port of the mesh-workbench editor server (`mesh-server --workspace DIR
/// --port N`) when no port is named. mesh-workbench's own editor default.
pub const SERVER_PORT: u16 = 7879;
/// A command to `mesh-server` waits this long for the connection and the reply.
const SERVER_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest reply taken from `mesh-server`.
const SERVER_REPLY_MAX: u64 = 64 << 20;
/// The selection overlay sits this far off the surface, metres, so it does
/// not fight the face under it for depth.
const OVERLAY_OFFSET: f32 = 0.003;
/// A selected vertex is drawn as a marker this share of the mesh size across.
const VERTEX_MARK: f32 = 0.012;
pub const OVERLAY_COLOR: [f32; 3] = [1.0, 0.52, 0.08];

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
    /// The picture's overlay no longer matches `selection`.
    pub overlay_stale: bool,
}

impl MeshView {
    pub fn new(dir: PathBuf, stamp: Option<SystemTime>) -> Self {
        let workspace = genos_workbench::load_workspace(&dir).ok();
        Self {
            dir,
            stamp,
            looked: Instant::now(),
            workspace,
            selection: Selection::default(),
            overlay_stale: true,
        }
    }

    /// Read the workspace again. IDs the mesh no longer has leave the selection.
    pub fn reload(&mut self, stamp: Option<SystemTime>) {
        self.stamp = stamp;
        self.workspace = genos_workbench::load_workspace(&self.dir).ok();
        if let Some(workspace) = &self.workspace {
            let objects: BTreeSet<u64> = workspace.parts.iter().map(|p| p.object).collect();
            let faces: BTreeSet<u64> = workspace.parts.iter().flat_map(|p| p.face_ids.iter().copied()).collect();
            let vertices: BTreeSet<u64> = workspace.parts.iter().flat_map(|p| p.vertex_ids.iter().copied()).collect();
            self.selection.objects.retain(|id| objects.contains(id));
            self.selection.faces.retain(|id| faces.contains(id));
            self.selection.vertices.retain(|id| vertices.contains(id));
        }
        self.overlay_stale = true;
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
                        let (p, n) = (part.positions[corner], part.normals[corner]);
                        positions.push([p[0] + n[0] * OVERLAY_OFFSET, p[1] + n[1] * OVERLAY_OFFSET, p[2] + n[2] * OVERLAY_OFFSET]);
                        normals.push(n);
                    }
                }
            }
            for (corner, vertex) in part.vertex_ids.iter().enumerate() {
                if self.selection.vertices.contains(vertex) && marked.insert(*vertex) {
                    octahedron(part.positions[corner], mark, &mut positions, &mut normals);
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
                for k in 0..3 {
                    let p = part.positions[triangle * 3 + k];
                    match project(&m, [p[0], p[1] + lift, p[2]]) {
                        Some(s) => screen[k] = s,
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
                for k in 0..3 {
                    let p = part.positions[triangle * 3 + k];
                    for axis in 0..3 {
                        point[axis] += w[k] * p[axis];
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
    pub fn command(&mut self, name: &str, args: &Value, camera: &Camera, aspect: f32) -> Result<Value, String> {
        let args = Args(args);
        match name {
            "mesh_view" => {
                args.only("mesh_view", &[])?;
                Ok(self.view_value())
            }
            "mesh_selection" => {
                args.only("mesh_selection", &["objects", "faces", "vertices", "note", "clear"])?;
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
                Ok(self.selection_value(camera))
            }
            "mesh_pick" => {
                args.only("mesh_pick", &["screen", "mode", "select"])?;
                let at = args.floats("screen", 2)?.map_or([0.5, 0.5], |v| [v[0], v[1]]);
                let mode = args.str("mode")?.unwrap_or_else(|| "face".into());
                let select = args.str("select")?.unwrap_or_else(|| "set".into());
                if !matches!(mode.as_str(), "face" | "object" | "vertex") {
                    return Err(format!("mesh_pick mode is face, object, or vertex, not {mode}"));
                }
                if !matches!(select.as_str(), "set" | "add" | "none") {
                    return Err(format!("mesh_pick select is set, add, or none, not {select}"));
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
                args.only("mesh_command", &["command", "port"])?;
                let command = args.0.get("command").ok_or("mesh_command needs command: one mesh-workbench command object, or a list of them")?;
                let port = match args.f64("port")? {
                    Some(port) if (1.0..=65535.0).contains(&port) => port as u16,
                    Some(port) => return Err(format!("mesh_command port {port} is not a port")),
                    None => server_port(),
                };
                post_command(port, &json::encode(command))
            }
            other => Err(format!("unknown command {other}")),
        }
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
                        ])
                    })
                    .collect(),
            )
        });
        obj(vec![
            ("workspace", text(self.dir.display().to_string())),
            ("loaded", Value::Bool(self.workspace.is_some())),
            ("revision", id(revision)),
            ("vertices", num(vertices as f64)),
            ("faces", num(faces as f64)),
            ("objects", list(objects)),
            ("server_port", num(f64::from(server_port()))),
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

struct Hit {
    object: u64,
    name: String,
    face: u64,
    vertex: u64,
    point: [f32; 3],
    depth: f32,
}

/// `GENOS_MESH_PORT`, else [`SERVER_PORT`].
pub fn server_port() -> u16 {
    std::env::var("GENOS_MESH_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(SERVER_PORT)
}

/// `POST /command` on the loopback `mesh-server` and return its JSON reply.
/// mesh-workbench answers HTTP/1.0 and closes the connection.
fn post_command(port: u16, body: &str) -> Result<Value, String> {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = std::net::TcpStream::connect_timeout(&address, SERVER_TIMEOUT).map_err(|err| {
        format!("mesh-server is not answering on 127.0.0.1:{port} ({err}). Start it with: mesh-server --workspace DIR --port {port}")
    })?;
    stream.set_read_timeout(Some(SERVER_TIMEOUT)).map_err(|err| err.to_string())?;
    stream.set_write_timeout(Some(SERVER_TIMEOUT)).map_err(|err| err.to_string())?;
    let request = format!(
        "POST /command HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).map_err(|err| format!("mesh-server write: {err}"))?;
    let mut raw = Vec::new();
    stream
        .take(SERVER_REPLY_MAX)
        .read_to_end(&mut raw)
        .map_err(|err| format!("mesh-server read: {err}"))?;
    let reply = String::from_utf8_lossy(&raw);
    let (_, payload) = reply.split_once("\r\n\r\n").ok_or("mesh-server sent no HTTP body")?;
    json::parse(payload).map_err(|err| format!("mesh-server reply is not JSON: {err}"))
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
    Some([(row(0) / w + 1.0) * 0.5, (row(1) / w + 1.0) * 0.5, row(2) / w])
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
