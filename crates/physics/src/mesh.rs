//! Build a mesh collider before the step.
//!
//! A convex mesh becomes one hull with at most [`MAX_HULL_VERTS`] vertices.
//! A concave mesh is voxelized and merged into a few boxes, so a hole stays open.
//! Contact then tests those pieces. The build may allocate. The step does not.

use genos_math::Vec3;

use crate::{Hull, HullFace, Mesh, Piece, MAX_HULL_FACES, MAX_HULL_VERTS, MAX_MESH_PIECES};

const INSIDE_EPS: f32 = 1.0e-4;

pub(crate) fn simplify(triangles: &[[Vec3; 3]]) -> Mesh {
    if triangles.is_empty() {
        return Mesh::EMPTY;
    }
    if is_convex(triangles) {
        let mut points = Vec::new();
        for tri in triangles {
            for vertex in tri {
                push_unique(&mut points, *vertex);
            }
        }
        let mut mesh = Mesh::EMPTY;
        mesh.pieces[0] = Piece::Hull(hull_from_points(&points));
        mesh.count = 1;
        return mesh;
    }
    voxel_boxes(triangles)
}

fn is_convex(triangles: &[[Vec3; 3]]) -> bool {
    let mut points = Vec::new();
    for tri in triangles {
        for vertex in tri {
            push_unique(&mut points, *vertex);
        }
    }
    for tri in triangles {
        let edge0 = tri[1] - tri[0];
        let edge1 = tri[2] - tri[0];
        let normal = edge0.cross(edge1);
        let length = normal.length();
        if length < 1.0e-8 {
            continue;
        }
        let normal = normal / length;
        for point in &points {
            if normal.dot(*point - tri[0]) > 1.0e-3 {
                return false;
            }
        }
    }
    true
}

fn hull_from_points(points: &[Vec3]) -> Hull {
    let reduced = reduce_points(points);
    let mut hull = Hull::EMPTY;
    let count = reduced.len().min(MAX_HULL_VERTS);
    hull.vert_count = count as u8;
    for (index, point) in reduced.iter().take(count).enumerate() {
        hull.verts[index] = *point;
    }
    fill_faces(&mut hull);
    hull
}

fn reduce_points(points: &[Vec3]) -> Vec<Vec3> {
    if points.len() <= MAX_HULL_VERTS {
        return points.to_vec();
    }
    let mut origin = Vec3::ZERO;
    for point in points {
        origin += *point;
    }
    origin = origin / points.len() as f32;
    let mut kept = Vec::new();
    for direction in support_directions() {
        let mut best = points[0];
        let mut best_dot = best.dot(direction);
        for point in points.iter().skip(1) {
            let dot = point.dot(direction);
            if dot > best_dot {
                best = *point;
                best_dot = dot;
            }
        }
        push_unique(&mut kept, best);
    }
    if kept.len() <= MAX_HULL_VERTS {
        return kept;
    }
    kept.sort_by(|left, right| {
        let left_d = (*left - origin).length_squared();
        let right_d = (*right - origin).length_squared();
        right_d
            .partial_cmp(&left_d)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    kept.truncate(MAX_HULL_VERTS);
    kept
}

fn support_directions() -> [Vec3; 26] {
    let mut dirs = [Vec3::ZERO; 26];
    let mut count = 0;
    for x in [-1.0, 0.0, 1.0] {
        for y in [-1.0, 0.0, 1.0] {
            for z in [-1.0, 0.0, 1.0] {
                if x == 0.0 && y == 0.0 && z == 0.0 {
                    continue;
                }
                dirs[count] = Vec3::new(x, y, z).normalize();
                count += 1;
            }
        }
    }
    dirs
}

#[derive(Clone, Copy)]
struct FacePlane {
    normal: Vec3,
    offset: f32,
    index: [u8; MAX_HULL_VERTS],
    index_count: u8,
}

fn fill_faces(hull: &mut Hull) {
    let count = hull.vert_count as usize;
    hull.face_count = 0;
    if count < 4 {
        return;
    }
    let verts = hull.verts;
    let mut planes = [FacePlane {
        normal: Vec3::ZERO,
        offset: 0.0,
        index: [0; MAX_HULL_VERTS],
        index_count: 0,
    }; MAX_HULL_FACES];
    let mut plane_count = 0usize;
    for i in 0..count {
        for j in (i + 1)..count {
            for k in (j + 1)..count {
                let a = verts[i];
                let b = verts[j];
                let c = verts[k];
                let mut normal = (b - a).cross(c - a);
                let length_sq = normal.length_squared();
                if length_sq < 1.0e-12 {
                    continue;
                }
                normal = normal / length_sq.sqrt();
                let mut positive = false;
                let mut negative = false;
                for (index, point) in verts.iter().take(count).enumerate() {
                    if index == i || index == j || index == k {
                        continue;
                    }
                    let side = normal.dot(*point - a);
                    if side > INSIDE_EPS {
                        positive = true;
                    } else if side < -INSIDE_EPS {
                        negative = true;
                    }
                }
                if positive && negative {
                    continue;
                }
                if positive {
                    normal = -normal;
                }
                let offset = normal.dot(a);
                let merged = planes[..plane_count].iter().any(|plane| {
                    plane.normal.dot(normal) > 0.999 && (plane.offset - offset).abs() < 1.0e-3
                });
                if merged || plane_count >= MAX_HULL_FACES {
                    continue;
                }
                planes[plane_count].normal = normal;
                planes[plane_count].offset = offset;
                plane_count += 1;
            }
        }
    }
    for plane in planes.iter_mut().take(plane_count) {
        let mut on = [0u8; MAX_HULL_VERTS];
        let mut on_count = 0usize;
        for (index, point) in verts.iter().take(count).enumerate() {
            if (plane.normal.dot(*point) - plane.offset).abs() <= 1.0e-3 {
                on[on_count] = index as u8;
                on_count += 1;
            }
        }
        if on_count < 3 {
            continue;
        }
        order_around(&mut on[..on_count], &verts, plane.normal);
        plane.index[..on_count].copy_from_slice(&on[..on_count]);
        plane.index_count = on_count as u8;
    }
    // One triangle per plane first, so a full table still closes every face.
    for plane in planes.iter().take(plane_count) {
        push_fan_triangle(hull, plane, 1);
    }
    for plane in planes.iter().take(plane_count) {
        let mut fan = 2usize;
        while fan + 1 < plane.index_count as usize {
            if !push_fan_triangle(hull, plane, fan) {
                return;
            }
            fan += 1;
        }
    }
}

fn push_fan_triangle(hull: &mut Hull, plane: &FacePlane, fan: usize) -> bool {
    let count = plane.index_count as usize;
    if fan + 1 >= count {
        return true;
    }
    if hull.face_count as usize >= MAX_HULL_FACES {
        return false;
    }
    let index = [plane.index[0], plane.index[fan], plane.index[fan + 1]];
    let a = hull.verts[index[0] as usize];
    let b = hull.verts[index[1] as usize];
    let c = hull.verts[index[2] as usize];
    if (b - a).cross(c - a).length_squared() < 1.0e-12 {
        return true;
    }
    let slot = hull.face_count as usize;
    hull.faces[slot] = HullFace {
        normal: plane.normal,
        offset: plane.offset,
        index,
    };
    hull.face_count += 1;
    true
}

fn order_around(indices: &mut [u8], verts: &[Vec3], normal: Vec3) {
    let mut origin = Vec3::ZERO;
    for index in indices.iter() {
        origin += verts[*index as usize];
    }
    origin = origin / indices.len() as f32;
    let tangent = perpendicular(normal);
    let bitangent = normal.cross(tangent);
    indices.sort_by(|left, right| {
        let left_offset = verts[*left as usize] - origin;
        let right_offset = verts[*right as usize] - origin;
        let left_angle = bitangent.dot(left_offset).atan2(tangent.dot(left_offset));
        let right_angle = bitangent.dot(right_offset).atan2(tangent.dot(right_offset));
        left_angle
            .partial_cmp(&right_angle)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn perpendicular(normal: Vec3) -> Vec3 {
    let axis = if normal.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let side = normal.cross(axis);
    let length = side.length();
    if length < 1.0e-8 {
        Vec3::X
    } else {
        side / length
    }
}

fn voxel_boxes(triangles: &[[Vec3; 3]]) -> Mesh {
    let mut min = triangles[0][0];
    let mut max = min;
    for tri in triangles {
        for vertex in tri {
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
    }
    let extent = max - min;
    let longest = extent.x.max(extent.y).max(extent.z).max(1.0e-4);
    let mut cells = 8.0_f32;
    let mut mesh = Mesh::EMPTY;
    for _ in 0..4 {
        let cell = longest / cells;
        mesh = merge_voxels(triangles, min, max, cell);
        if mesh.count as usize <= MAX_MESH_PIECES && mesh.count > 0 {
            return mesh;
        }
        cells *= 0.5;
    }
    mesh.count = mesh.count.min(MAX_MESH_PIECES as u8);
    mesh
}

fn merge_voxels(triangles: &[[Vec3; 3]], min: Vec3, max: Vec3, cell: f32) -> Mesh {
    let nx = ((max.x - min.x) / cell).ceil().clamp(1.0, 12.0) as usize;
    let ny = ((max.y - min.y) / cell).ceil().clamp(1.0, 12.0) as usize;
    let nz = ((max.z - min.z) / cell).ceil().clamp(1.0, 12.0) as usize;
    let mut solid = vec![false; nx * ny * nz];
    let index = |x, y, z| (z * ny + y) * nx + x;
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                let center = Vec3::new(
                    min.x + (x as f32 + 0.5) * cell,
                    min.y + (y as f32 + 0.5) * cell,
                    min.z + (z as f32 + 0.5) * cell,
                );
                if point_inside(triangles, center) {
                    solid[index(x, y, z)] = true;
                }
            }
        }
    }
    let mut used = vec![false; solid.len()];
    let mut pieces = Vec::new();
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                let start = index(x, y, z);
                if !solid[start] || used[start] {
                    continue;
                }
                let mut x1 = x + 1;
                while x1 < nx && solid[index(x1, y, z)] && !used[index(x1, y, z)] {
                    x1 += 1;
                }
                let mut y1 = y + 1;
                'grow_y: while y1 < ny {
                    for xi in x..x1 {
                        let id = index(xi, y1, z);
                        if !solid[id] || used[id] {
                            break 'grow_y;
                        }
                    }
                    y1 += 1;
                }
                let mut z1 = z + 1;
                'grow_z: while z1 < nz {
                    for yi in y..y1 {
                        for xi in x..x1 {
                            let id = index(xi, yi, z1);
                            if !solid[id] || used[id] {
                                break 'grow_z;
                            }
                        }
                    }
                    z1 += 1;
                }
                for zi in z..z1 {
                    for yi in y..y1 {
                        for xi in x..x1 {
                            used[index(xi, yi, zi)] = true;
                        }
                    }
                }
                let box_min = Vec3::new(
                    min.x + x as f32 * cell,
                    min.y + y as f32 * cell,
                    min.z + z as f32 * cell,
                );
                let box_max = Vec3::new(
                    min.x + x1 as f32 * cell,
                    min.y + y1 as f32 * cell,
                    min.z + z1 as f32 * cell,
                );
                let half = (box_max - box_min) * 0.5;
                let center = box_min + half;
                pieces.push(Piece::Cuboid { center, half });
            }
        }
    }
    let mut mesh = Mesh::EMPTY;
    let count = pieces.len().min(MAX_MESH_PIECES);
    mesh.count = count as u8;
    for (slot, piece) in pieces.into_iter().take(count).enumerate() {
        mesh.pieces[slot] = piece;
    }
    mesh
}

fn point_inside(triangles: &[[Vec3; 3]], point: Vec3) -> bool {
    let origin = point + Vec3::new(0.0, 1.0e-5, 1.3e-4);
    let mut hits = 0u32;
    for tri in triangles {
        if ray_hits_triangle(origin, tri) {
            hits += 1;
        }
    }
    hits % 2 == 1
}

fn ray_hits_triangle(origin: Vec3, tri: &[Vec3; 3]) -> bool {
    let edge0 = tri[1] - tri[0];
    let edge1 = tri[2] - tri[0];
    let direction = Vec3::X;
    let p = direction.cross(edge1);
    let det = edge0.dot(p);
    if det.abs() < 1.0e-8 {
        return false;
    }
    let inv = 1.0 / det;
    let tvec = origin - tri[0];
    let u = tvec.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = tvec.cross(edge0);
    let v = direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    let t = edge1.dot(q) * inv;
    t > 1.0e-5
}

fn push_unique(points: &mut Vec<Vec3>, point: Vec3) {
    for kept in points.iter() {
        if (*kept - point).length_squared() < 1.0e-8 {
            return;
        }
    }
    points.push(point);
}
