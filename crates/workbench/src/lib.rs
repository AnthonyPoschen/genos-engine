//! Read a mesh-workbench workspace for display.
//!
//! mesh-workbench (<https://github.com/NathanChambers/mesh-workbench>) keeps a
//! mesh as vertices and n-gon faces with stable numeric IDs, grouped into
//! objects. Its workspace directory holds `project.json`, replaced atomically
//! after every edit. This crate reads that file and turns each object into
//! triangles that remember the face and the vertices they came from, so a pick
//! in the viewer can name mesh-workbench IDs. It does not open a window or a
//! device, and it does not talk to the mesh-workbench server.
//!
//! The record is `docs/systems/mesh-workbench.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use genos_json::Value;

/// The newest `project.json` version this crate reads.
pub const NEWEST_VERSION: u64 = 8;

/// One object of the mesh, as triangles. Three entries of `positions`,
/// `normals` and `vertex_ids` make one triangle; `face_ids` has one entry per
/// triangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// mesh-workbench object ID. 0 holds the faces no object owns.
    pub object: u64,
    pub name: String,
    pub origin: [f32; 3],
    pub positions: Vec<[f32; 3]>,
    /// One per position. A face in `flat_faces`, or an edge sharper than the
    /// object's smooth angle, keeps the face normal.
    pub normals: Vec<[f32; 3]>,
    /// mesh-workbench vertex ID of each position.
    pub vertex_ids: Vec<u64>,
    /// mesh-workbench face ID of each triangle.
    pub face_ids: Vec<u64>,
    /// The polygon edges, each once: the outline of every face, not the
    /// triangulation.
    pub edges: Vec<[[f32; 3]; 2]>,
    /// The two mesh-workbench vertex IDs of each edge, in the order of `edges`.
    pub edge_ids: Vec<[u64; 2]>,
    /// The texture coordinate of each position, with V turned so 0 is the
    /// top row of the image (mesh-workbench keeps 0 at the bottom). A corner
    /// with no UV is at the origin.
    pub uvs: Vec<[f32; 2]>,
    /// Whether each position has a UV of its own. `uvs` holds the origin for
    /// one that has none.
    pub uv_mapped: Vec<bool>,
    /// The texture of each triangle's face: an index into
    /// [`Workspace::textures`], or `None`.
    pub face_textures: Vec<Option<usize>>,
    /// The vertex colour of each position (linear RGB). Empty when no vertex
    /// of the object is painted; an unpainted vertex of a painted object is
    /// white.
    pub colors: Vec<[f32; 3]>,
}

impl Part {
    #[must_use]
    pub fn triangles(&self) -> usize {
        self.face_ids.len()
    }
}

/// One bone of the rig, at rest.
#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    pub parent: Option<String>,
    pub position: [f32; 3],
}

/// One texture image of the mesh: RGBA8, row-major, top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureImage {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// A texel with less alpha than this is not drawn. `None`: the image is
    /// shown over mid grey by its alpha, as mesh-workbench previews it.
    pub alpha_cutoff: Option<f32>,
}

/// A named group of objects.
#[derive(Clone, Debug, PartialEq)]
pub struct Collection {
    pub id: u64,
    pub name: String,
    pub objects: Vec<u64>,
}

/// One animation clip: its name and its length in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
}

/// A workspace's mesh, ready to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub dir: PathBuf,
    /// `project.json` `version`.
    pub version: u64,
    /// `project.json` `revision`. 0 when the file has none.
    pub revision: u64,
    pub parts: Vec<Part>,
    pub vertices: usize,
    pub faces: usize,
    /// The vertex IDs mesh-workbench holds as its own selection.
    pub selection: Vec<u64>,
    /// The texture images, in name order.
    pub textures: Vec<TextureImage>,
    /// The collections, by ID.
    pub collections: Vec<Collection>,
    /// The rig's bones at rest. Empty when the mesh has no rig.
    pub bones: Vec<Bone>,
    /// The rig's animation clips, by name.
    pub clips: Vec<Clip>,
    /// Lowest and highest corner of every vertex. Zero when the mesh is empty.
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// `<dir>/project.json`.
#[must_use]
pub fn project_path(dir: &Path) -> PathBuf {
    dir.join("project.json")
}

impl Workspace {
    /// A workspace with no mesh yet.
    #[must_use]
    pub fn empty(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            version: NEWEST_VERSION,
            revision: 0,
            parts: Vec::new(),
            vertices: 0,
            faces: 0,
            selection: Vec::new(),
            textures: Vec::new(),
            collections: Vec::new(),
            bones: Vec::new(),
            clips: Vec::new(),
            min: [0.0; 3],
            max: [0.0; 3],
        }
    }
}

/// Read and triangulate `<dir>/project.json`. A workspace mesh-workbench has
/// not written to yet has no such file: that is an empty workspace.
///
/// # Errors
/// The file is not JSON, has no `mesh`, or is newer than [`NEWEST_VERSION`].
pub fn load_workspace(dir: &Path) -> Result<Workspace, String> {
    let path = project_path(dir);
    if !path.exists() {
        return Ok(Workspace::empty(dir));
    }
    let text = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut workspace = parse_project(&text).map_err(|err| format!("{}: {err}", path.display()))?;
    workspace.dir = dir.to_path_buf();
    Ok(workspace)
}

/// One recorded command of a session.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: u64,
    pub command: String,
    pub ok: bool,
    /// The state snapshot after this command, when it changed the mesh.
    pub snapshot: Option<u64>,
}

/// One mesh-workbench session: the commands one server run recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub initial_snapshot: u64,
    pub events: Vec<Event>,
}

/// The sessions recorded under `<dir>/sessions`, oldest first. A workspace
/// with none gives an empty list.
#[must_use]
pub fn sessions(dir: &Path) -> Vec<Session> {
    let mut found = Vec::new();
    let Ok(items) = std::fs::read_dir(dir.join("sessions")) else {
        return found;
    };
    for item in items.flatten() {
        let Ok(text) = std::fs::read_to_string(item.path().join("index.json")) else {
            continue;
        };
        let Ok(index) = genos_json::parse(&text) else {
            continue;
        };
        let events = index
            .get("events")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .map(|event| Event {
                id: event.get("id").and_then(Value::as_u64).unwrap_or(0),
                command: event.get("command").and_then(Value::as_str).unwrap_or("").to_string(),
                ok: event.get("ok").and_then(Value::as_bool).unwrap_or(false),
                snapshot: event.get("snapshot").and_then(Value::as_u64),
            })
            .collect();
        found.push(Session {
            id: index
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(|| item.file_name().to_string_lossy().into_owned(), str::to_string),
            initial_snapshot: index.get("initial_snapshot").and_then(Value::as_u64).unwrap_or(0),
            events,
        });
    }
    // A session ID starts with its start time, so name order is time order.
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

/// `<dir>/sessions/<session>/states/<snapshot>.json`.
#[must_use]
pub fn state_path(dir: &Path, session: &str, snapshot: u64) -> PathBuf {
    dir.join("sessions").join(session).join("states").join(format!("{snapshot}.json"))
}

/// Read one recorded state snapshot. It holds the mesh as `project.json` does.
///
/// # Errors
/// The file is missing or is not a state.
pub fn load_state(dir: &Path, session: &str, snapshot: u64) -> Result<Workspace, String> {
    let path = state_path(dir, session, snapshot);
    let text = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut workspace = parse_project(&text).map_err(|err| format!("{}: {err}", path.display()))?;
    workspace.dir = dir.to_path_buf();
    Ok(workspace)
}

/// Triangulate the text of a `project.json`.
///
/// # Errors
/// See [`load_workspace`].
pub fn parse_project(text: &str) -> Result<Workspace, String> {
    let root = genos_json::parse(text)?;
    let version = root.get("version").and_then(Value::as_u64).unwrap_or(1);
    if version > NEWEST_VERSION {
        return Err(format!(
            "project version {version} is newer than {NEWEST_VERSION}, the newest this viewer reads"
        ));
    }
    let revision = root.get("revision").and_then(Value::as_u64).unwrap_or(0);
    let mesh = root.get("mesh").ok_or("no mesh in the project")?;
    let vertices = id_map(mesh.get("vertices"), vec3)?;
    let faces = id_map(mesh.get("faces"), ids)?;
    let painted = id_map(mesh.get("vertex_colors"), vec3)?;
    let textures = read_textures(mesh);
    let face_texture = read_face_textures(mesh, &textures);
    let face_uvs = read_face_uvs(mesh);
    let flat: BTreeSet<u64> = mesh.get("flat_faces").map(ids).transpose()?.unwrap_or_default().into_iter().collect();
    let selection = mesh.get("selection").map(ids).transpose()?.unwrap_or_default();
    let (owner, mut parts) = read_objects(mesh)?;
    let mut face_normal: BTreeMap<u64, [f64; 3]> = BTreeMap::new();
    let mut faces_at: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for (&face, corners) in &faces {
        let Some(points) = corner_points(corners, &vertices) else {
            continue;
        };
        face_normal.insert(face, unit(newell(&points)));
        for vertex in corners {
            faces_at.entry(*vertex).or_default().push(face);
        }
    }
    let (min, max) = bounds(&vertices);
    let mut seen_edges: BTreeSet<(u64, u64)> = BTreeSet::new();
    for (&face, corners) in &faces {
        let Some(points) = corner_points(corners, &vertices) else {
            continue;
        };
        let object = corners.first().and_then(|v| owner.get(v)).copied().unwrap_or(0);
        let (part, smooth) = parts
            .entry(object)
            .or_insert_with(|| (empty_part(object, String::new(), [0.0; 3]), None));
        for (i, &a) in corners.iter().enumerate() {
            let b = corners[(i + 1) % corners.len()];
            if seen_edges.insert((a.min(b), a.max(b))) {
                part.edges.push([narrow(points[i]), narrow(points[(i + 1) % corners.len()])]);
                part.edge_ids.push([a, b]);
            }
        }
        let flat_normal = face_normal.get(&face).copied().unwrap_or([0.0, 1.0, 0.0]);
        // The mesh-workbench web viewer's rule (geometry.js buildCornerNormals):
        // no smooth angle, an angle of 0, or a face in flat_faces is flat.
        let limit = match smooth {
            Some(degrees) if *degrees != 0.0 && !flat.contains(&face) => Some(degrees.to_radians().cos()),
            _ => None,
        };
        let corner_normals: Vec<[f64; 3]> = corners
            .iter()
            .map(|vertex| match limit {
                Some(cosine) => smooth_corner(face, *vertex, cosine, &faces, &faces_at, &face_normal, &vertices),
                None => flat_normal,
            })
            .collect();
        for triangle in triangulate(&points) {
            for corner in triangle {
                let vertex = corners[corner];
                let normal = corner_normals[corner];
                part.positions.push(narrow(points[corner]));
                part.normals.push(narrow(normal));
                part.vertex_ids.push(vertex);
                part.colors.push(painted.get(&vertex).map_or([1.0; 3], |c| narrow(*c)));
                let uv = face_uvs.get(&face).and_then(|corners| corners.get(&vertex));
                part.uvs.push(corner_uv(uv));
                part.uv_mapped.push(uv.is_some());
            }
            part.face_ids.push(face);
            part.face_textures.push(face_texture.get(&face).copied());
        }
    }
    Ok(Workspace {
        dir: PathBuf::new(),
        version,
        revision,
        parts: parts
            .into_values()
            .map(|(mut part, _)| {
                if !part.vertex_ids.iter().any(|vertex| painted.contains_key(vertex)) {
                    part.colors.clear();
                }
                part
            })
            .filter(|part| part.triangles() > 0)
            .collect(),
        vertices: vertices.len(),
        faces: faces.len(),
        selection,
        textures,
        collections: read_collections(mesh),
        bones: rig_bones(mesh),
        clips: rig_clips(mesh),
        min: narrow(min),
        max: narrow(max),
    })
}

/// The object that owns each vertex, and an empty part with its smooth angle
/// for each object.
type Objects = (BTreeMap<u64, u64>, BTreeMap<u64, (Part, Option<f64>)>);

fn read_objects(mesh: &Value) -> Result<Objects, String> {
    let mut owner = BTreeMap::new();
    let mut parts = BTreeMap::new();
    if let Some(Value::Object(objects)) = mesh.get("objects") {
        for (key, object) in objects {
            let id: u64 = key.parse().map_err(|_| format!("object id {key}"))?;
            for vertex in object.get("vertices").map(ids).transpose()?.unwrap_or_default() {
                owner.insert(vertex, id);
            }
            let origin = object.get("origin").map(vec3).transpose()?.unwrap_or([0.0; 3]);
            let smooth = object.get("smooth_angle").and_then(Value::as_f64);
            let name = object.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            parts.insert(id, (empty_part(id, name, origin), smooth));
        }
    }
    Ok((owner, parts))
}

/// Face ID to its texture, as an index into `textures`.
fn read_face_textures(mesh: &Value, textures: &[TextureImage]) -> BTreeMap<u64, usize> {
    let Some(Value::Object(pairs)) = mesh.get("face_textures") else {
        return BTreeMap::new();
    };
    pairs
        .iter()
        .filter_map(|(face, name)| {
            let at = textures.iter().position(|texture| Some(texture.name.as_str()) == name.as_str())?;
            Some((face.parse().ok()?, at))
        })
        .collect()
}

/// Face ID to the UV of each of its corners, by vertex ID.
fn read_face_uvs(mesh: &Value) -> BTreeMap<u64, BTreeMap<u64, [f64; 2]>> {
    let Some(Value::Object(pairs)) = mesh.get("uvs") else {
        return BTreeMap::new();
    };
    pairs
        .iter()
        .filter_map(|(face, corners)| {
            let Value::Object(corners) = corners else {
                return None;
            };
            let map = corners
                .iter()
                .filter_map(|(vertex, uv)| {
                    let uv = uv.as_array()?;
                    Some((vertex.parse().ok()?, [uv.first()?.as_f64()?, uv.get(1)?.as_f64()?]))
                })
                .collect();
            Some((face.parse().ok()?, map))
        })
        .collect()
}

/// A corner's UV with V turned to the image's top row. No UV is the origin.
#[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "a texture coordinate")]
fn corner_uv(uv: Option<&[f64; 2]>) -> [f32; 2] {
    uv.map_or([0.0, 0.0], |uv| [uv[0] as f32, (1.0 - uv[1]) as f32])
}

fn read_textures(mesh: &Value) -> Vec<TextureImage> {
    let Some(Value::Object(textures)) = mesh.get("textures") else {
        return Vec::new();
    };
    textures
        .iter()
        .filter_map(|(name, texture)| {
            let width = u32::try_from(texture.get("width")?.as_u64()?).ok()?;
            let height = u32::try_from(texture.get("height")?.as_u64()?).ok()?;
            let pixels = decode_base64(texture.get("rgba_base64")?.as_str()?)?;
            let want = usize::try_from(u64::from(width) * u64::from(height) * 4).ok()?;
            #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "an alpha threshold in 0..1")]
            let alpha_cutoff = texture.get("alpha_cutoff").and_then(Value::as_f64).map(|cutoff| cutoff as f32);
            (pixels.len() == want).then(|| TextureImage { name: name.clone(), width, height, pixels, alpha_cutoff })
        })
        .collect()
}

/// Standard base64, with or without padding. `None` on a character outside
/// the alphabet.
#[must_use]
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut bits = 0u32;
    let mut held = 0u32;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push(u8::try_from((bits >> held) & 0xFF).ok()?);
        }
    }
    Some(out)
}

fn read_collections(mesh: &Value) -> Vec<Collection> {
    let Some(Value::Object(collections)) = mesh.get("collections") else {
        return Vec::new();
    };
    collections
        .iter()
        .filter_map(|(key, collection)| {
            Some(Collection {
                id: key.parse().ok()?,
                name: collection.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                objects: collection.get("objects").map(ids).and_then(Result::ok).unwrap_or_default(),
            })
        })
        .collect()
}

/// One recorded command of a session: what was sent and the compact reply.
/// The full reply in the file holds the whole mesh and is left out.
///
/// # Errors
/// The event file is missing or is not JSON.
pub fn load_event(dir: &Path, session: &str, event: u64) -> Result<Value, String> {
    let path = dir.join("sessions").join(session).join("events").join(format!("{event}.json"));
    let text = std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let record = genos_json::parse(&text).map_err(|err| format!("{}: {err}", path.display()))?;
    let Value::Object(fields) = record else {
        return Err(format!("{}: not an event record", path.display()));
    };
    Ok(Value::Object(fields.into_iter().filter(|(key, _)| key != "full_output").collect()))
}

fn rig_bones(mesh: &Value) -> Vec<Bone> {
    let Some(bones) = mesh.get("rig").and_then(|rig| rig.get("bones")).and_then(Value::as_array) else {
        return Vec::new();
    };
    bones
        .iter()
        .filter_map(|bone| {
            Some(Bone {
                name: bone.get("name")?.as_str()?.to_string(),
                parent: bone.get("parent").and_then(Value::as_str).map(str::to_string),
                position: narrow(vec3(bone.get("position")?).ok()?),
            })
        })
        .collect()
}

#[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "a clip length in seconds")]
fn rig_clips(mesh: &Value) -> Vec<Clip> {
    let Some(Value::Object(clips)) = mesh.get("rig").and_then(|rig| rig.get("clips")) else {
        return Vec::new();
    };
    clips
        .iter()
        .map(|(name, clip)| Clip {
            name: name.clone(),
            duration: clip.get("duration").and_then(Value::as_f64).unwrap_or(0.0) as f32,
        })
        .collect()
}

/// Lowest and highest corner of the vertices. Zero for an empty mesh.
fn bounds(vertices: &BTreeMap<u64, [f64; 3]>) -> ([f64; 3], [f64; 3]) {
    if vertices.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in vertices.values() {
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    (min, max)
}

/// Triangles of one polygon, as indices into `points`. A convex polygon is a
/// fan from its first corner. A concave one is clipped ear by ear, as the
/// mesh-workbench web viewer does. Fewer than three points give no triangle.
#[must_use]
pub fn triangulate(points: &[[f64; 3]]) -> Vec<[usize; 3]> {
    let count = points.len();
    if count < 3 {
        return Vec::new();
    }
    if count == 3 {
        return vec![[0, 1, 2]];
    }
    let normal = unit(newell(points));
    let turn = |a: usize, b: usize, c: usize| dot(cross(sub(points[b], points[a]), sub(points[c], points[b])), normal);
    let convex = (0..count).all(|i| turn(i, (i + 1) % count, (i + 2) % count) >= -EDGE_EPS);
    if convex {
        return (1..count - 1).map(|i| [0, i, i + 1]).collect();
    }
    let mut ring: Vec<usize> = (0..count).collect();
    let mut triangles = Vec::with_capacity(count - 2);
    while ring.len() > 3 {
        let size = ring.len();
        let ear = (0..size).find(|&i| {
            let (a, b, c) = (ring[(i + size - 1) % size], ring[i], ring[(i + 1) % size]);
            turn(a, b, c) > EDGE_EPS
                && ring.iter().all(|&p| p == a || p == b || p == c || !inside(points, normal, [a, b, c], p))
        });
        // A degenerate ring has no ear. Fan what is left so every corner is drawn.
        let i = ear.unwrap_or(0);
        triangles.push([ring[(i + size - 1) % size], ring[i], ring[(i + 1) % size]]);
        ring.remove(i);
    }
    triangles.push([ring[0], ring[1], ring[2]]);
    triangles
}

const EDGE_EPS: f64 = 1.0e-12;
/// Slack on the smooth-angle cosine, as in the web viewer.
const SMOOTH_EPS: f64 = 1.0e-10;

/// The normal of `vertex` as seen from `face`: the faces around the vertex
/// reached from `face` across edges no sharper than the smooth angle, each
/// weighted by its corner angle at the vertex.
fn smooth_corner(
    face: u64,
    vertex: u64,
    cosine: f64,
    faces: &BTreeMap<u64, Vec<u64>>,
    faces_at: &BTreeMap<u64, Vec<u64>>,
    face_normal: &BTreeMap<u64, [f64; 3]>,
    vertices: &BTreeMap<u64, [f64; 3]>,
) -> [f64; 3] {
    let around = faces_at.get(&vertex).map_or(&[][..], Vec::as_slice);
    let mut pending = vec![face];
    let mut seen = BTreeSet::new();
    let mut sum = [0.0; 3];
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        let (Some(polygon), Some(normal), Some(at)) = (faces.get(&current), face_normal.get(&current), vertices.get(&vertex)) else {
            continue;
        };
        let Some(i) = polygon.iter().position(|v| *v == vertex) else {
            continue;
        };
        let before = polygon[(i + polygon.len() - 1) % polygon.len()];
        let after = polygon[(i + 1) % polygon.len()];
        let (Some(pb), Some(pa)) = (vertices.get(&before), vertices.get(&after)) else {
            continue;
        };
        let weight = dot(unit(sub(*pb, *at)), unit(sub(*pa, *at))).clamp(-1.0, 1.0).acos();
        for axis in 0..3 {
            sum[axis] += normal[axis] * weight;
        }
        for neighbour in [before, after] {
            for adjacent in around {
                let shares = faces.get(adjacent).is_some_and(|other| other.contains(&neighbour));
                let near = face_normal.get(adjacent).is_some_and(|other| dot(*normal, *other) >= cosine - SMOOTH_EPS);
                if *adjacent != current && shares && near {
                    pending.push(*adjacent);
                }
            }
        }
    }
    let len = dot(sum, sum).sqrt();
    if len > 0.0 {
        [sum[0] / len, sum[1] / len, sum[2] / len]
    } else {
        face_normal.get(&face).copied().unwrap_or([0.0, 1.0, 0.0])
    }
}

fn inside(points: &[[f64; 3]], normal: [f64; 3], triangle: [usize; 3], p: usize) -> bool {
    (0..3).all(|k| {
        let a = points[triangle[k]];
        let b = points[triangle[(k + 1) % 3]];
        dot(cross(sub(b, a), sub(points[p], a)), normal) >= 0.0
    })
}

fn empty_part(object: u64, name: String, origin: [f64; 3]) -> Part {
    Part {
        object,
        name,
        origin: narrow(origin),
        positions: Vec::new(),
        normals: Vec::new(),
        vertex_ids: Vec::new(),
        face_ids: Vec::new(),
        edges: Vec::new(),
        edge_ids: Vec::new(),
        uvs: Vec::new(),
        uv_mapped: Vec::new(),
        face_textures: Vec::new(),
        colors: Vec::new(),
    }
}

fn corner_points(corners: &[u64], vertices: &BTreeMap<u64, [f64; 3]>) -> Option<Vec<[f64; 3]>> {
    if corners.len() < 3 {
        return None;
    }
    corners.iter().map(|id| vertices.get(id).copied()).collect()
}

fn id_map<T>(value: Option<&Value>, read: fn(&Value) -> Result<T, String>) -> Result<BTreeMap<u64, T>, String> {
    let mut map = BTreeMap::new();
    if let Some(Value::Object(pairs)) = value {
        for (key, item) in pairs {
            let id: u64 = key.parse().map_err(|_| format!("id {key}"))?;
            map.insert(id, read(item)?);
        }
    }
    Ok(map)
}

fn ids(value: &Value) -> Result<Vec<u64>, String> {
    value
        .as_array()
        .ok_or("expected a list of ids")?
        .iter()
        .map(|item| item.as_u64().ok_or_else(|| "expected an id".to_string()))
        .collect()
}

fn vec3(value: &Value) -> Result<[f64; 3], String> {
    let items = value.as_array().ok_or("expected three numbers")?;
    if items.len() != 3 {
        return Err("expected three numbers".into());
    }
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.as_f64().ok_or("expected a number")?;
    }
    Ok(out)
}

/// Newell's method: twice the polygon's area along its normal.
fn newell(points: &[[f64; 3]]) -> [f64; 3] {
    let mut n = [0.0; 3];
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let len = dot(v, v).sqrt();
    if len > 0.0 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 1.0, 0.0]
    }
}

#[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "the renderer draws f32 positions")]
fn narrow(v: [f64; 3]) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

#[cfg(test)]
mod tests {
    use super::{parse_project, triangulate};

    const PANEL: &str = r#"{"version":3,"mesh":{"vertices":{"1":[-1,-1,0],"2":[1,-1,0],"3":[1,1,0],"4":[-1,1,0]},"faces":{"7":[1,2,3,4]},"selection":[2],"objects":{"5":{"name":"panel","origin":[0,0,0],"vertices":[1,2,3,4]}}},"revision":12}"#;

    #[test]
    fn a_quad_becomes_two_triangles_that_keep_their_ids() {
        let workspace = parse_project(PANEL).expect("parse");
        assert_eq!((workspace.version, workspace.revision), (3, 12));
        assert_eq!((workspace.vertices, workspace.faces), (4, 1));
        assert_eq!(workspace.selection, vec![2]);
        assert_eq!(workspace.min, [-1.0, -1.0, 0.0]);
        assert_eq!(workspace.max, [1.0, 1.0, 0.0]);
        let part = &workspace.parts[0];
        assert_eq!((part.object, part.name.as_str()), (5, "panel"));
        assert_eq!(part.face_ids, vec![7, 7]);
        assert_eq!(part.edges.len(), 4, "the outline, not the diagonal");
        assert!(part.colors.is_empty(), "no vertex is painted");
        let painted = parse_project(&PANEL.replace("\"selection\":[2]", "\"selection\":[2],\"vertex_colors\":{\"2\":[1,0,0]}")).expect("parse");
        let colors = &painted.parts[0].colors;
        assert_eq!(colors.len(), 6);
        // Positions are vertices 1 2 3 1 3 4: only vertex 2 is red.
        assert_eq!((colors[0], colors[1]), ([1.0, 1.0, 1.0], [1.0, 0.0, 0.0]));
        assert_eq!(part.vertex_ids, vec![1, 2, 3, 1, 3, 4]);
        // Counter-clockwise seen from +Z: the normal is +Z on every corner.
        assert!(part.normals.iter().all(|n| *n == [0.0, 0.0, 1.0]), "{:?}", part.normals);
    }

    #[test]
    fn a_directory_with_no_project_file_is_an_empty_workspace() {
        let dir = std::env::temp_dir().join("genos-workbench-no-such-workspace");
        let workspace = super::load_workspace(&dir).expect("empty");
        assert_eq!((workspace.parts.len(), workspace.vertices, workspace.revision), (0, 0, 0));
    }

    #[test]
    fn a_textured_face_gives_its_image_and_its_uvs_with_v_turned() {
        // A 2 by 1 image: red then blue. "/wAA/wAA//8=" is ff 00 00 ff 00 00 ff ff.
        let textured = PANEL.replace(
            "\"selection\":[2]",
            "\"selection\":[2],\"textures\":{\"paint\":{\"width\":2,\"height\":1,\"rgba_base64\":\"/wAA/wAA//8=\"}},\"face_textures\":{\"7\":\"paint\"},\"uvs\":{\"7\":{\"1\":[0,0],\"2\":[1,0],\"3\":[1,1],\"4\":[0,1]}}",
        );
        let workspace = parse_project(&textured).expect("parse");
        assert_eq!(workspace.textures.len(), 1);
        assert_eq!(workspace.textures[0].pixels, vec![255, 0, 0, 255, 0, 0, 255, 255]);
        let part = &workspace.parts[0];
        assert_eq!(part.face_textures, vec![Some(0), Some(0)]);
        // Vertex 1 has UV (0, 0): the bottom left, so V is 1 from the top.
        assert_eq!(part.uvs[0], [0.0, 1.0]);
        assert!(part.uv_mapped.iter().all(|mapped| *mapped));
        assert_eq!(part.uvs[2], [1.0, 0.0]);
        assert_eq!(super::decode_base64("aGk="), Some(b"hi".to_vec()));
        assert_eq!(super::decode_base64("a?"), None);
        let plain = parse_project(PANEL).expect("parse");
        assert!(plain.textures.is_empty() && plain.parts[0].face_textures == vec![None, None]);
    }

    #[test]
    fn a_rig_gives_its_bones_and_clips() {
        let rigged = PANEL.replace(
            "\"selection\":[2]",
            "\"selection\":[2],\"rig\":{\"bones\":[{\"name\":\"root\",\"parent\":null,\"position\":[0,0,0]},{\"name\":\"tip\",\"parent\":\"root\",\"position\":[0,1,0]}],\"clips\":{\"Lean\":{\"duration\":2.0,\"looping\":false,\"tracks\":{}}},\"weights\":{}}",
        );
        let workspace = parse_project(&rigged).expect("parse");
        assert_eq!(workspace.bones.len(), 2);
        assert_eq!(workspace.bones[1].parent.as_deref(), Some("root"));
        assert_eq!(workspace.bones[1].position, [0.0, 1.0, 0.0]);
        assert_eq!((workspace.clips[0].name.as_str(), workspace.clips[0].duration), ("Lean", 2.0));
        assert!(parse_project(PANEL).expect("parse").bones.is_empty());
        let grouped = PANEL.replace("\"selection\":[2]", "\"selection\":[2],\"collections\":{\"3\":{\"name\":\"props\",\"objects\":[5]}}");
        let collections = parse_project(&grouped).expect("parse").collections;
        assert_eq!((collections[0].id, collections[0].name.as_str(), collections[0].objects.clone()), (3, "props", vec![5]));
    }

    #[test]
    fn a_concave_polygon_is_clipped_inside_its_outline() {
        // An L: the fan from corner 0 would cover the notch.
        let l = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ];
        let triangles = triangulate(&l);
        assert_eq!(triangles.len(), 4);
        let area: f64 = triangles
            .iter()
            .map(|t| {
                let (a, b, c) = (l[t[0]], l[t[1]], l[t[2]]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) / 2.0
            })
            .sum();
        assert!((area - 3.0).abs() < 1.0e-9, "area {area}");
        // No triangle's centre is in the notch (x > 1 and y > 1).
        for t in &triangles {
            let cx = (l[t[0]][0] + l[t[1]][0] + l[t[2]][0]) / 3.0;
            let cy = (l[t[0]][1] + l[t[1]][1] + l[t[2]][1]) / 3.0;
            assert!(!(cx > 1.0 && cy > 1.0), "triangle {t:?} is in the notch");
        }
    }

    #[test]
    fn a_cube_with_a_smooth_angle_keeps_flat_faces_and_a_newer_file_is_refused() {
        let cube = r#"{"version":8,"revision":1,"mesh":{"vertices":{"1":[0,0,0],"2":[1,0,0],"3":[1,1,0],"4":[0,1,0],"5":[0,0,1],"6":[1,0,1],"7":[1,1,1],"8":[0,1,1]},"faces":{"1":[1,4,3,2],"2":[5,6,7,8],"3":[1,2,6,5],"4":[2,3,7,6],"5":[3,4,8,7],"6":[4,1,5,8]},"selection":[],"objects":{"1":{"name":"cube","origin":[0,0,0],"vertices":[1,2,3,4,5,6,7,8],"smooth_angle":30}}}}"#;
        let workspace = parse_project(cube).expect("parse");
        let part = &workspace.parts[0];
        assert_eq!(part.triangles(), 12);
        // A 90 degree edge is sharper than 30 degrees: every corner keeps an axis normal.
        for n in &part.normals {
            let big = n.iter().filter(|c| c.abs() > 0.99).count();
            assert_eq!(big, 1, "normal {n:?} is not a face normal");
        }
        // With no smooth angle the cube is flat. Past 90 degrees every corner
        // blends its three faces.
        let plain = parse_project(&cube.replace(",\"smooth_angle\":30", "")).expect("parse");
        assert!(plain.parts[0].normals.iter().all(|n| n.iter().filter(|c| c.abs() > 0.99).count() == 1));
        let round = parse_project(&cube.replace("\"smooth_angle\":30", "\"smooth_angle\":120")).expect("parse");
        assert!(round.parts[0].normals.iter().all(|n| n.iter().all(|c| (c.abs() - 0.577_35).abs() < 1.0e-3)), "{:?}", round.parts[0].normals[0]);
        let newer = cube.replace("\"version\":8", "\"version\":9");
        assert!(parse_project(&newer).expect_err("refused").contains("newer"));
    }
}
