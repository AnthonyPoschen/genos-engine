//! Speculative contacts, and the gap query they share with time of impact.
//!
//! The gap is a GJK distance for two convex shapes. A positive gap is
//! separation. A negative gap is penetration from the discrete contact.

use genos_math::Vec3;

use crate::contact::{self, shape_support, solve_contact};
use crate::{Body, Mesh, MotionQuality, Piece, Shape};

const GJK_ITERS: usize = 32;
const DISTANCE_EPSILON: f32 = 1.0e-5;

/// Stop speculative bodies that would meet inside `min(speed * dt, margin)`.
pub(crate) fn speculative(bodies: &mut [Body], dt: f32, margin: f32) {
    let count = bodies.len();
    let mut index = 0;
    while index < count {
        let is_speculative = bodies
            .get(index)
            .is_some_and(|body| body.motion == MotionQuality::Speculative);
        if is_speculative {
            let mut other = 0;
            while other < count {
                if other != index {
                    speculative_pair(bodies, index, other, dt, margin);
                }
                other += 1;
            }
        }
        index += 1;
    }
}

fn speculative_pair(bodies: &mut [Body], index: usize, other: usize, dt: f32, margin: f32) {
    if other < index
        && bodies
            .get(other)
            .is_some_and(|body| body.motion == MotionQuality::Speculative)
    {
        return;
    }
    let (Some(first), Some(second)) = (bodies.get(index), bodies.get(other)) else {
        return;
    };
    if first.inverse_mass == 0.0 && second.inverse_mass == 0.0 {
        return;
    }
    let reach = approach(first, dt, margin).max(approach(second, dt, margin));
    if reach <= 0.0 {
        return;
    }
    let Some((normal, gap)) = signed_gap(first, second) else {
        return;
    };
    if gap >= reach {
        return;
    }
    let penetration = if gap < 0.0 { -gap } else { 0.0 };
    solve_indexed(bodies, index, other, normal, penetration);
}

fn approach(body: &Body, dt: f32, margin: f32) -> f32 {
    if body.motion != MotionQuality::Speculative {
        return 0.0;
    }
    let speed = body.velocity.length();
    if !speed.is_finite() {
        return 0.0;
    }
    (speed * dt).min(margin).max(0.0)
}

fn solve_indexed(bodies: &mut [Body], index: usize, other: usize, normal: Vec3, penetration: f32) {
    if index < other {
        let (left, right) = bodies.split_at_mut(other);
        let Some(first) = left.get_mut(index) else {
            return;
        };
        let Some(second) = right.first_mut() else {
            return;
        };
        solve_contact(first, second, normal, penetration);
        return;
    }
    let (left, right) = bodies.split_at_mut(index);
    let Some(first) = right.first_mut() else {
        return;
    };
    let Some(second) = left.get_mut(other) else {
        return;
    };
    solve_contact(first, second, normal, penetration);
}

/// Normal points from `second` toward `first`. Gap is positive when separated.
pub(crate) fn signed_gap(first: &Body, second: &Body) -> Option<(Vec3, f32)> {
    match (first.shape, second.shape) {
        (Shape::Plane { .. }, Shape::Plane { .. }) => None,
        (_, Shape::Plane { normal, offset }) => Some(plane_gap(normal, offset, first, true)),
        (Shape::Plane { normal, offset }, _) => Some(plane_gap(normal, offset, second, false)),
        (Shape::Mesh(mesh), _) => mesh_gap(first, &mesh, second, true),
        (_, Shape::Mesh(mesh)) => mesh_gap(second, &mesh, first, false),
        (
            Shape::Sphere { .. } | Shape::Capsule { .. } | Shape::Box { .. },
            Shape::Sphere { .. } | Shape::Capsule { .. } | Shape::Box { .. },
        ) => Some(gjk_gap(first, second)),
    }
}

/// `toward_first` is set when `body` is the first argument of [`signed_gap`].
fn plane_gap(normal: Vec3, offset: f32, body: &Body, toward_first: bool) -> (Vec3, f32) {
    let point = shape_support(body, -normal);
    let distance = normal.dot(point) - offset;
    let direction = if toward_first { normal } else { -normal };
    (direction, distance)
}

fn mesh_gap(owner: &Body, mesh: &Mesh, other: &Body, owner_is_first: bool) -> Option<(Vec3, f32)> {
    let mut best: Option<(Vec3, f32)> = None;
    for piece in mesh.pieces() {
        let probe = piece_probe(owner, *piece);
        let (normal, gap) = if owner_is_first {
            gjk_gap(&probe, other)
        } else {
            gjk_gap(other, &probe)
        };
        best = closer(best, (normal, gap));
    }
    best
}

fn closer(best: Option<(Vec3, f32)>, hit: (Vec3, f32)) -> Option<(Vec3, f32)> {
    match best {
        Some((_, gap)) if gap <= hit.1 => best,
        _ => Some(hit),
    }
}

fn piece_probe(owner: &Body, piece: Piece) -> Body {
    let mut probe = *owner;
    match piece {
        Piece::Cuboid { center, half } => {
            probe.position = owner.position + owner.orientation.rotate(center);
            probe.shape = Shape::Box { half_extents: half };
        }
        Piece::Hull(hull) => {
            let mut mesh = Mesh::EMPTY;
            mesh.pieces[0] = Piece::Hull(hull);
            mesh.count = 1;
            probe.shape = Shape::Mesh(mesh);
        }
    }
    probe
}

fn gjk_gap(first: &Body, second: &Body) -> (Vec3, f32) {
    let mut direction = first.position - second.position;
    if direction.length_squared() <= DISTANCE_EPSILON {
        direction = Vec3::X;
    }
    let mut simplex = [
        minkowski(first, second, direction),
        Vec3::ZERO,
        Vec3::ZERO,
        Vec3::ZERO,
    ];
    let mut count = 1_usize;
    let mut closest = simplex[0];
    for _ in 0..GJK_ITERS {
        if closest.length_squared() <= DISTANCE_EPSILON {
            return overlap_gap(first, second);
        }
        direction = -closest;
        let support = minkowski(first, second, direction);
        if support.dot(direction) <= closest.dot(direction) + DISTANCE_EPSILON {
            return finish_gap(first, second, closest);
        }
        if count >= 4 {
            return overlap_gap(first, second);
        }
        simplex[count] = support;
        count += 1;
        closest = reduce_simplex(&mut simplex, &mut count);
    }
    finish_gap(first, second, closest)
}

fn finish_gap(first: &Body, second: &Body, closest: Vec3) -> (Vec3, f32) {
    let distance = closest.length();
    if distance <= DISTANCE_EPSILON {
        return overlap_gap(first, second);
    }
    (closest / distance, distance)
}

fn overlap_gap(first: &Body, second: &Body) -> (Vec3, f32) {
    if let Some((normal, depth)) = contact::primitive_contact(first, second) {
        return (normal, -depth);
    }
    (Vec3::Y, 0.0)
}

fn minkowski(first: &Body, second: &Body, direction: Vec3) -> Vec3 {
    shape_support(first, direction) - shape_support(second, -direction)
}

fn reduce_simplex(simplex: &mut [Vec3; 4], count: &mut usize) -> Vec3 {
    match *count {
        1 => simplex[0],
        2 => reduce_segment(simplex, count),
        3 => reduce_triangle(simplex, count),
        _ => reduce_tetra(simplex, count),
    }
}

fn reduce_segment(simplex: &mut [Vec3; 4], count: &mut usize) -> Vec3 {
    let (point, keep_first, keep_second) = closest_segment(simplex[0], simplex[1]);
    if keep_first && keep_second {
        return point;
    }
    simplex[0] = if keep_second { simplex[1] } else { simplex[0] };
    *count = 1;
    point
}

fn reduce_triangle(simplex: &mut [Vec3; 4], count: &mut usize) -> Vec3 {
    let (point, feature) = closest_triangle(simplex[0], simplex[1], simplex[2]);
    match feature {
        Feature::Face => point,
        Feature::Edge(0) => {
            *count = 2;
            point
        }
        Feature::Edge(1) => {
            simplex[1] = simplex[2];
            *count = 2;
            point
        }
        Feature::Edge(_) => {
            simplex[0] = simplex[1];
            simplex[1] = simplex[2];
            *count = 2;
            point
        }
        Feature::Vertex(0) => {
            *count = 1;
            point
        }
        Feature::Vertex(1) => {
            simplex[0] = simplex[1];
            *count = 1;
            point
        }
        Feature::Vertex(_) => {
            simplex[0] = simplex[2];
            *count = 1;
            point
        }
    }
}

fn reduce_tetra(simplex: &mut [Vec3; 4], count: &mut usize) -> Vec3 {
    let points = [simplex[0], simplex[1], simplex[2], simplex[3]];
    if tetra_contains_origin(points) {
        *count = 4;
        return Vec3::ZERO;
    }
    let faces = [(0, 1, 2, 3), (0, 1, 3, 2), (0, 2, 3, 1), (1, 2, 3, 0)];
    let mut best_point = points[0];
    let mut best_distance = f32::MAX;
    let mut best_verts = [0_usize, 0, 0];
    let mut best_feature = Feature::Vertex(0);
    let mut found = false;
    for (a, b, c, opposite) in faces {
        if !origin_outside(points[a], points[b], points[c], points[opposite]) {
            continue;
        }
        let (point, feature) = closest_triangle(points[a], points[b], points[c]);
        let distance = point.length_squared();
        if !found || distance < best_distance {
            best_point = point;
            best_distance = distance;
            best_verts = [a, b, c];
            best_feature = feature;
            found = true;
        }
    }
    if !found {
        *count = 4;
        return Vec3::ZERO;
    }
    write_feature(simplex, count, points, best_verts, best_feature);
    best_point
}

fn write_feature(
    simplex: &mut [Vec3; 4],
    count: &mut usize,
    points: [Vec3; 4],
    verts: [usize; 3],
    feature: Feature,
) {
    let selected: [usize; 3] = match feature {
        Feature::Face => verts,
        Feature::Edge(0) => [verts[0], verts[1], verts[1]],
        Feature::Edge(1) => [verts[0], verts[2], verts[2]],
        Feature::Edge(_) => [verts[1], verts[2], verts[2]],
        Feature::Vertex(0) => [verts[0], verts[0], verts[0]],
        Feature::Vertex(1) => [verts[1], verts[1], verts[1]],
        Feature::Vertex(_) => [verts[2], verts[2], verts[2]],
    };
    let keep = match feature {
        Feature::Face => 3,
        Feature::Edge(_) => 2,
        Feature::Vertex(_) => 1,
    };
    for slot in 0..keep {
        simplex[slot] = points[selected[slot]];
    }
    *count = keep;
}

#[derive(Clone, Copy)]
enum Feature {
    Vertex(usize),
    Edge(usize),
    Face,
}

fn closest_segment(start: Vec3, end: Vec3) -> (Vec3, bool, bool) {
    let edge = end - start;
    let length_sq = edge.length_squared();
    if length_sq <= DISTANCE_EPSILON {
        return (start, true, false);
    }
    let t = (-start).dot(edge) / length_sq;
    if t <= 0.0 {
        (start, true, false)
    } else if t >= 1.0 {
        (end, false, true)
    } else {
        (start + edge * t, true, true)
    }
}

fn closest_triangle(a: Vec3, b: Vec3, c: Vec3) -> (Vec3, Feature) {
    let ab = b - a;
    let ac = c - a;
    let normal = ab.cross(ac);
    let normal_len_sq = normal.length_squared();
    if normal_len_sq > DISTANCE_EPSILON {
        let closest = normal * (a.dot(normal) / normal_len_sq);
        if let Some(feature) = triangle_feature(a, ab, ac, closest) {
            return (closest, feature);
        }
    }
    closest_boundary(a, b, c)
}

fn triangle_feature(a: Vec3, ab: Vec3, ac: Vec3, closest: Vec3) -> Option<Feature> {
    let rel = closest - a;
    let d00 = ab.dot(ab);
    let d01 = ab.dot(ac);
    let d11 = ac.dot(ac);
    let d20 = rel.dot(ab);
    let d21 = rel.dot(ac);
    let denom = d00 * d11 - d01 * d01;
    if denom.abs() <= DISTANCE_EPSILON {
        return None;
    }
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    let u = 1.0 - v - w;
    if u >= -1.0e-4 && v >= -1.0e-4 && w >= -1.0e-4 {
        Some(Feature::Face)
    } else {
        None
    }
}

struct BoundaryBest {
    point: Vec3,
    feature: Feature,
    distance: f32,
}

fn closest_boundary(a: Vec3, b: Vec3, c: Vec3) -> (Vec3, Feature) {
    let mut best = BoundaryBest {
        point: a,
        feature: Feature::Vertex(0),
        distance: f32::MAX,
    };
    consider_edge(&mut best, a, b, 0, 0, 1);
    consider_edge(&mut best, a, c, 1, 0, 2);
    consider_edge(&mut best, b, c, 2, 1, 2);
    (best.point, best.feature)
}

fn consider_edge(
    best: &mut BoundaryBest,
    start: Vec3,
    end: Vec3,
    edge: usize,
    start_index: usize,
    end_index: usize,
) {
    let (point, keep_start, keep_end) = closest_segment(start, end);
    let distance = point.length_squared();
    if distance < best.distance {
        best.distance = distance;
        best.point = point;
        best.feature = edge_feature(keep_start, keep_end, edge, start_index, end_index);
    }
}

fn edge_feature(
    keep_start: bool,
    keep_end: bool,
    edge: usize,
    start: usize,
    end: usize,
) -> Feature {
    if keep_start && keep_end {
        Feature::Edge(edge)
    } else if keep_end {
        Feature::Vertex(end)
    } else {
        Feature::Vertex(start)
    }
}

fn origin_outside(a: Vec3, b: Vec3, c: Vec3, opposite: Vec3) -> bool {
    let normal = (b - a).cross(c - a);
    let opposite_side = normal.dot(opposite - a);
    let origin_side = normal.dot(-a);
    opposite_side * origin_side < 0.0
}

fn tetra_contains_origin(points: [Vec3; 4]) -> bool {
    let faces = [(0, 1, 2, 3), (0, 1, 3, 2), (0, 2, 3, 1), (1, 2, 3, 0)];
    faces.iter().all(|(a, b, c, opposite)| {
        !origin_outside(points[*a], points[*b], points[*c], points[*opposite])
    })
}

#[cfg(test)]
mod tests {
    use genos_math::Vec3;

    use super::signed_gap;
    use crate::{Body, MotionQuality};

    fn gap_of(first: &Body, second: &Body) -> f32 {
        signed_gap(first, second).map_or(f32::NAN, |(_, gap)| gap)
    }

    #[test]
    fn separated_spheres_report_the_surface_gap() {
        let left = Body::sphere(Vec3::ZERO, 1.0, 1.0);
        let right = Body::sphere(Vec3::new(5.0, 0.0, 0.0), 1.0, 1.0);
        let gap = gap_of(&left, &right);
        assert!((gap - 3.0).abs() < 1.0e-3, "sphere gap {gap} wanted 3");
    }

    #[test]
    fn head_on_ship_boxes_report_a_two_metre_gap() {
        let mut left = Body::cuboid(Vec3::new(0.0, 2.0, 0.0), 1_000.0, Vec3::new(2.0, 1.0, 1.0));
        left.motion = MotionQuality::Discrete;
        let right = Body::cuboid(Vec3::new(6.0, 2.0, 0.0), 1_000.0, Vec3::new(2.0, 1.0, 1.0));
        let gap = gap_of(&left, &right);
        assert!((gap - 2.0).abs() < 1.0e-3, "ship gap {gap} wanted 2");
    }
}
