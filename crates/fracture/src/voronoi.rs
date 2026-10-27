//! Seeded Voronoi cells. Each cell is the source hull clipped by bisector planes
//! in increasing seed index. There is no hashmap and no `atan2`.

use genos_math::Vec3;

use crate::FractureError;

const EPS: f32 = 1.0e-5;
/// Other end of a hull face, and the world end of a support bond.
pub const HULL: u32 = u32::MAX;

#[derive(Clone, Debug)]
pub struct Cell {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<Vec<u16>>,
    /// Seed this face was cut against. [`HULL`] is the source hull.
    pub against: Vec<u32>,
}

#[must_use]
pub fn box_cell(min: Vec3, max: Vec3) -> Cell {
    let verts = [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(max.x, max.y, max.z),
        Vec3::new(min.x, max.y, max.z),
    ];
    let faces = [
        vec![1, 2, 6, 5],
        vec![0, 4, 7, 3],
        vec![3, 7, 6, 2],
        vec![0, 1, 5, 4],
        vec![4, 5, 6, 7],
        vec![0, 3, 2, 1],
    ];
    Cell {
        vertices: verts.to_vec(),
        faces: faces.to_vec(),
        against: vec![HULL; 6],
    }
}

#[must_use]
pub fn volume(cell: &Cell) -> f32 {
    let mut sum = 0.0_f32;
    for face in &cell.faces {
        if face.len() < 3 {
            continue;
        }
        let a = cell.vertices[usize::from(face[0])];
        for index in 1..face.len() - 1 {
            let b = cell.vertices[usize::from(face[index])];
            let c = cell.vertices[usize::from(face[index + 1])];
            sum += a.dot(b.cross(c));
        }
    }
    (sum / 6.0).abs()
}

/// Cell `i` is the hull clipped by the bisector of every other seed, low index first.
///
/// # Errors
///
/// Returns [`FractureError::ThinCell`] when a seed leaves a cell under `1e-8` volume.
pub fn voronoi(hull: &Cell, seeds: &[Vec3]) -> Result<Vec<Cell>, FractureError> {
    let mut cells = Vec::with_capacity(seeds.len());
    for (index, _) in seeds.iter().enumerate() {
        let mut cell = hull.clone();
        for (other, _) in seeds.iter().enumerate() {
            if other == index {
                continue;
            }
            let plane = bisector(seeds[index], seeds[other]);
            cell = clip_cell(
                cell,
                plane.0,
                plane.1,
                u32::try_from(other).unwrap_or(u32::MAX),
            )?;
            if volume(&cell) < 1.0e-8 {
                return Err(FractureError::ThinCell {
                    seed: u32::try_from(index).unwrap_or(u32::MAX),
                });
            }
        }
        if volume(&cell) < 1.0e-8 {
            return Err(FractureError::ThinCell {
                seed: u32::try_from(index).unwrap_or(u32::MAX),
            });
        }
        cells.push(cell);
    }
    Ok(cells)
}

fn bisector(keep: Vec3, other: Vec3) -> (Vec3, f32) {
    let delta = other - keep;
    let normal = if delta.length_squared() > EPS {
        delta.normalize()
    } else {
        Vec3::X
    };
    let mid = (keep + other) * 0.5;
    (normal, normal.dot(mid))
}

fn clip_cell(cell: Cell, normal: Vec3, offset: f32, against: u32) -> Result<Cell, FractureError> {
    let inside: Vec<bool> = cell
        .vertices
        .iter()
        .map(|vertex| normal.dot(*vertex) <= offset + EPS)
        .collect();
    if inside.iter().all(|kept| *kept) {
        return Ok(cell);
    }
    if inside.iter().all(|kept| !*kept) {
        return Err(FractureError::ThinCell { seed: against });
    }
    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    let mut owners = Vec::new();
    let mut cuts: Vec<(u16, u16)> = Vec::new();
    for (face_index, face) in cell.faces.iter().enumerate() {
        let owner = cell.against.get(face_index).copied().unwrap_or(HULL);
        let (polygon, cut) =
            clip_face(&cell.vertices, face, &inside, normal, offset, &mut vertices);
        if polygon.len() >= 3 {
            faces.push(polygon);
            owners.push(owner);
        }
        if let Some(edge) = cut {
            cuts.push(edge);
        }
    }
    if let Some(cap) = stitch_cap(&vertices, &cuts, normal) {
        faces.push(cap);
        owners.push(against);
    }
    if faces.is_empty() {
        return Err(FractureError::ThinCell { seed: against });
    }
    Ok(Cell {
        vertices,
        faces,
        against: owners,
    })
}

fn clip_face(
    verts: &[Vec3],
    face: &[u16],
    inside: &[bool],
    normal: Vec3,
    offset: f32,
    out: &mut Vec<Vec3>,
) -> (Vec<u16>, Option<(u16, u16)>) {
    let mut polygon = Vec::new();
    let mut first_cut = None;
    let mut second_cut = None;
    for slot in 0..face.len() {
        let current = usize::from(face[slot]);
        let next = usize::from(face[(slot + 1) % face.len()]);
        let current_in = inside.get(current).copied().unwrap_or(false);
        let next_in = inside.get(next).copied().unwrap_or(false);
        if current_in {
            polygon.push(push_vertex(out, verts[current]));
        }
        if current_in != next_in {
            let point = intersect(verts[current], verts[next], normal, offset);
            let index = push_vertex(out, point);
            polygon.push(index);
            if first_cut.is_none() {
                first_cut = Some(index);
            } else {
                second_cut = Some(index);
            }
        }
    }
    let cut = match (first_cut, second_cut) {
        (Some(start), Some(end)) => Some((start, end)),
        _ => None,
    };
    (polygon, cut)
}

fn push_vertex(out: &mut Vec<Vec3>, point: Vec3) -> u16 {
    for (index, vertex) in out.iter().enumerate() {
        if (*vertex - point).length_squared() <= EPS * EPS {
            return u16::try_from(index).unwrap_or(0);
        }
    }
    let index = u16::try_from(out.len()).unwrap_or(u16::MAX);
    out.push(point);
    index
}

fn intersect(start: Vec3, end: Vec3, normal: Vec3, offset: f32) -> Vec3 {
    let start_side = normal.dot(start) - offset;
    let end_side = normal.dot(end) - offset;
    let denom = start_side - end_side;
    if denom.abs() <= EPS {
        return start;
    }
    let t = start_side / denom;
    start + (end - start) * t
}

fn stitch_cap(vertices: &[Vec3], cuts: &[(u16, u16)], normal: Vec3) -> Option<Vec<u16>> {
    if cuts.is_empty() {
        return None;
    }
    let mut unused = vec![true; cuts.len()];
    let mut looped = Vec::new();
    let start = cuts[0];
    unused[0] = false;
    looped.push(start.0);
    looped.push(start.1);
    let mut cursor = start.1;
    for _ in 1..cuts.len() {
        let mut found = None;
        for (index, edge) in cuts.iter().enumerate() {
            if !unused[index] {
                continue;
            }
            if near(vertices, edge.0, cursor) {
                found = Some((index, edge.1));
                break;
            }
            if near(vertices, edge.1, cursor) {
                found = Some((index, edge.0));
                break;
            }
        }
        let Some((index, next)) = found else {
            break;
        };
        unused[index] = false;
        cursor = next;
        if near(vertices, cursor, looped[0]) {
            break;
        }
        looped.push(cursor);
    }
    if looped.len() < 3 {
        return None;
    }
    let forward = face_normal(vertices, &looped);
    if forward.dot(normal) < 0.0 {
        looped.reverse();
    }
    Some(looped)
}

fn near(vertices: &[Vec3], index: u16, other: u16) -> bool {
    let Some(a) = vertices.get(usize::from(index)) else {
        return false;
    };
    let Some(b) = vertices.get(usize::from(other)) else {
        return false;
    };
    (*a - *b).length_squared() <= 1.0e-8
}

pub fn face_normal_of(cell: &Cell, face: &[u16]) -> Vec3 {
    face_normal(&cell.vertices, face)
}

fn face_normal(vertices: &[Vec3], face: &[u16]) -> Vec3 {
    if face.len() < 3 {
        return Vec3::Y;
    }
    let a = vertices[usize::from(face[0])];
    let b = vertices[usize::from(face[1])];
    let c = vertices[usize::from(face[2])];
    let normal = (b - a).cross(c - a);
    if normal.length_squared() <= EPS {
        Vec3::Y
    } else {
        normal.normalize()
    }
}

pub fn face_area(cell: &Cell, face: &[u16]) -> f32 {
    if face.len() < 3 {
        return 0.0;
    }
    let origin = cell.vertices[usize::from(face[0])];
    let mut sum = Vec3::ZERO;
    for window in face.windows(2).skip(1) {
        let b = cell.vertices[usize::from(window[0])];
        let c = cell.vertices[usize::from(window[1])];
        sum += (b - origin).cross(c - origin);
    }
    0.5 * sum.length()
}

pub fn face_centroid(cell: &Cell, face: &[u16]) -> Vec3 {
    if face.is_empty() {
        return Vec3::ZERO;
    }
    let mut sum = Vec3::ZERO;
    for index in face {
        sum += cell.vertices[usize::from(*index)];
    }
    sum * (1.0 / f32::from(u16::try_from(face.len()).unwrap_or(1)))
}

pub fn centroid(cell: &Cell) -> Vec3 {
    if cell.vertices.is_empty() {
        return Vec3::ZERO;
    }
    let mut sum = Vec3::ZERO;
    for vertex in &cell.vertices {
        sum += *vertex;
    }
    sum * (1.0 / f32::from(u16::try_from(cell.vertices.len()).unwrap_or(1)))
}

pub fn bounds(cell: &Cell) -> (Vec3, Vec3) {
    let mut min = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
    let mut max = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
    for vertex in &cell.vertices {
        min = Vec3::new(
            min.x.min(vertex.x),
            min.y.min(vertex.y),
            min.z.min(vertex.z),
        );
        max = Vec3::new(
            max.x.max(vertex.x),
            max.y.max(vertex.y),
            max.z.max(vertex.z),
        );
    }
    (min, max)
}
