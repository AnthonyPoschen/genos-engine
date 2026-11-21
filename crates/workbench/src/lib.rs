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
}

impl Part {
    #[must_use]
    pub fn triangles(&self) -> usize {
        self.face_ids.len()
    }
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
    /// Lowest and highest corner of every vertex. Zero when the mesh is empty.
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// `<dir>/project.json`.
#[must_use]
pub fn project_path(dir: &Path) -> PathBuf {
    dir.join("project.json")
}

/// Read and triangulate `<dir>/project.json`.
///
/// # Errors
/// The file is missing, is not JSON, has no `mesh`, or is newer than
/// [`NEWEST_VERSION`].
pub fn load_workspace(dir: &Path) -> Result<Workspace, String> {
    let path = project_path(dir);
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
    let flat: BTreeSet<u64> = mesh.get("flat_faces").map(ids).transpose()?.unwrap_or_default().into_iter().collect();
    let selection = mesh.get("selection").map(ids).transpose()?.unwrap_or_default();
    let mut owner: BTreeMap<u64, u64> = BTreeMap::new();
    let mut parts: BTreeMap<u64, (Part, Option<f64>)> = BTreeMap::new();
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
    // Vertex normals: the area-weighted sum of the faces at each vertex.
    let mut face_normal: BTreeMap<u64, [f64; 3]> = BTreeMap::new();
    let mut vertex_normal: BTreeMap<u64, [f64; 3]> = BTreeMap::new();
    for (&face, corners) in &faces {
        let Some(points) = corner_points(corners, &vertices) else {
            continue;
        };
        let n = newell(&points);
        face_normal.insert(face, unit(n));
        for vertex in corners {
            let sum = vertex_normal.entry(*vertex).or_insert([0.0; 3]);
            for (s, c) in sum.iter_mut().zip(n) {
                *s += c;
            }
        }
    }
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for point in vertices.values() {
        for axis in 0..3 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    if vertices.is_empty() {
        min = [0.0; 3];
        max = [0.0; 3];
    }
    for (&face, corners) in &faces {
        let Some(points) = corner_points(corners, &vertices) else {
            continue;
        };
        let object = corners.first().and_then(|v| owner.get(v)).copied().unwrap_or(0);
        let (part, smooth) = parts
            .entry(object)
            .or_insert_with(|| (empty_part(object, String::new(), [0.0; 3]), None));
        let flat_normal = face_normal.get(&face).copied().unwrap_or([0.0, 1.0, 0.0]);
        // No smooth angle shades the whole object smooth, as the web viewer does.
        let limit = smooth.map_or(-1.0, |degrees| degrees.to_radians().cos());
        for triangle in triangulate(&points) {
            for corner in triangle {
                let vertex = corners[corner];
                let smooth_normal = unit(vertex_normal.get(&vertex).copied().unwrap_or(flat_normal));
                let keep_flat = flat.contains(&face) || dot(smooth_normal, flat_normal) < limit;
                let normal = if keep_flat { flat_normal } else { smooth_normal };
                part.positions.push(narrow(points[corner]));
                part.normals.push(narrow(normal));
                part.vertex_ids.push(vertex);
            }
            part.face_ids.push(face);
        }
    }
    Ok(Workspace {
        dir: PathBuf::new(),
        version,
        revision,
        parts: parts.into_values().map(|(part, _)| part).filter(|part| part.triangles() > 0).collect(),
        vertices: vertices.len(),
        faces: faces.len(),
        selection,
        min: narrow(min),
        max: narrow(max),
    })
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

#[expect(clippy::cast_possible_truncation, reason = "the renderer draws f32 positions")]
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
        assert_eq!(part.vertex_ids, vec![1, 2, 3, 1, 3, 4]);
        // Counter-clockwise seen from +Z: the normal is +Z on every corner.
        assert!(part.normals.iter().all(|n| *n == [0.0, 0.0, 1.0]), "{:?}", part.normals);
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
        let newer = cube.replace("\"version\":8", "\"version\":9");
        assert!(parse_project(&newer).expect_err("refused").contains("newer"));
    }
}
