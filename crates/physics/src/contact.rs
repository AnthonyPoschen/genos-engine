//! Contact between the shapes on two bodies.
//!
//! Primitives use closest points. A mesh tests each convex piece. The normal
//! points from the second body toward the first, matching [`solve_contact`].

use genos_math::{Quat, Vec3};

use crate::{Body, Hull, Mesh, Piece, Shape};

const SLOP: f32 = 0.001;
const PERCENT: f32 = 0.8;

pub(crate) fn collide(bodies: &mut [Body]) {
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            let (left, right) = bodies.split_at_mut(j);
            collide_pair(&mut left[i], &mut right[0]);
        }
    }
}

fn collide_pair(a: &mut Body, b: &mut Body) {
    match (a.shape, b.shape) {
        (Shape::Plane { .. }, Shape::Plane { .. }) => {}
        (Shape::Plane { normal, offset }, _) => {
            collide_plane(b, a, normal, offset);
        }
        (_, Shape::Plane { normal, offset }) => {
            collide_plane(a, b, normal, offset);
        }
        (Shape::Mesh(mesh), _) => collide_mesh(a, mesh, b),
        (_, Shape::Mesh(mesh)) => {
            if let Some((normal, depth)) = mesh_contact(b, &mesh, a) {
                solve_contact(a, b, -normal, depth);
            }
        }
        _ => {
            if let Some((normal, depth)) = primitive_contact(a, b) {
                solve_contact(a, b, normal, depth);
            }
        }
    }
}

fn collide_plane(body: &mut Body, plane: &mut Body, normal: Vec3, offset: f32) {
    match body.shape {
        Shape::Mesh(mesh) => {
            for piece in mesh.pieces() {
                if let Some((hit_normal, depth)) = piece_plane(body, *piece, normal, offset) {
                    solve_contact(body, plane, hit_normal, depth);
                }
            }
        }
        _ => {
            let point = support_point(body, -normal, None);
            let dist = normal.dot(point) - offset;
            if dist < 0.0 {
                solve_contact(body, plane, normal, -dist);
            }
        }
    }
}

fn collide_mesh(a: &mut Body, mesh: Mesh, b: &mut Body) {
    if let Some((normal, depth)) = mesh_contact(a, &mesh, b) {
        solve_contact(a, b, normal, depth);
    }
}

fn mesh_contact(owner: &Body, mesh: &Mesh, other: &Body) -> Option<(Vec3, f32)> {
    let mut best: Option<(Vec3, f32)> = None;
    for piece in mesh.pieces() {
        let hit = match other.shape {
            Shape::Mesh(other_mesh) => {
                let mut piece_best: Option<(Vec3, f32)> = None;
                for other_piece in other_mesh.pieces() {
                    if let Some(hit) = piece_piece(owner, *piece, other, *other_piece) {
                        piece_best = deeper(piece_best, hit);
                    }
                }
                piece_best
            }
            _ => piece_body(owner, *piece, other),
        };
        if let Some(hit) = hit {
            best = deeper(best, hit);
        }
    }
    best
}

fn deeper(best: Option<(Vec3, f32)>, hit: (Vec3, f32)) -> Option<(Vec3, f32)> {
    match best {
        Some((normal, depth)) if depth >= hit.1 => Some((normal, depth)),
        _ => Some(hit),
    }
}

fn piece_body(owner: &Body, piece: Piece, other: &Body) -> Option<(Vec3, f32)> {
    match piece {
        Piece::Cuboid { center, half } => {
            let (position, orientation) = pose(owner, center);
            cuboid_body(position, orientation, half, other).map(|(normal, depth)| (-normal, depth))
        }
        Piece::Hull(hull) => hull_body(owner, &hull, other),
    }
}

fn piece_piece(a: &Body, piece_a: Piece, b: &Body, piece_b: Piece) -> Option<(Vec3, f32)> {
    match (piece_a, piece_b) {
        (
            Piece::Cuboid {
                center: ca,
                half: ha,
            },
            Piece::Cuboid {
                center: cb,
                half: hb,
            },
        ) => {
            let (pa, oa) = pose(a, ca);
            let (pb, ob) = pose(b, cb);
            box_box(pa, oa, ha, pb, ob, hb)
        }
        (Piece::Cuboid { center, half }, Piece::Hull(hull)) => {
            let (position, orientation) = pose(a, center);
            let probe = Body {
                position,
                orientation,
                shape: Shape::Box { half_extents: half },
                ..*a
            };
            hull_body(b, &hull, &probe).map(|(normal, depth)| (-normal, depth))
        }
        (Piece::Hull(hull), Piece::Cuboid { center, half }) => {
            let (position, orientation) = pose(b, center);
            let probe = Body {
                position,
                orientation,
                shape: Shape::Box { half_extents: half },
                ..*b
            };
            hull_body(a, &hull, &probe)
        }
        (Piece::Hull(left), Piece::Hull(right)) => hull_hull(a, &left, b, &right),
    }
}

fn piece_plane(body: &Body, piece: Piece, normal: Vec3, offset: f32) -> Option<(Vec3, f32)> {
    let point = match piece {
        Piece::Cuboid { center, half } => {
            let (position, orientation) = pose(body, center);
            let local = conjugate(orientation).rotate(-normal);
            let support = Vec3::new(
                half.x.copysign(local.x),
                half.y.copysign(local.y),
                half.z.copysign(local.z),
            );
            position + orientation.rotate(support)
        }
        Piece::Hull(hull) => hull_support(body, &hull, -normal),
    };
    let dist = normal.dot(point) - offset;
    if dist < 0.0 {
        Some((normal, -dist))
    } else {
        None
    }
}

fn primitive_contact(a: &Body, b: &Body) -> Option<(Vec3, f32)> {
    match (a.shape, b.shape) {
        (Shape::Sphere { radius: ra }, Shape::Sphere { radius: rb }) => {
            spheres(a.position, ra, b.position, rb)
        }
        (
            Shape::Sphere { radius },
            Shape::Capsule {
                radius: rc,
                half_height,
            },
        ) => capsule_sphere(b, rc, half_height, a.position, radius)
            .map(|(normal, depth)| (-normal, depth)),
        (
            Shape::Capsule {
                radius: rc,
                half_height,
            },
            Shape::Sphere { radius },
        ) => capsule_sphere(a, rc, half_height, b.position, radius),
        (
            Shape::Capsule {
                radius: ra,
                half_height: ha,
            },
            Shape::Capsule {
                radius: rb,
                half_height: hb,
            },
        ) => capsule_capsule(a, ra, ha, b, rb, hb),
        (Shape::Sphere { radius }, Shape::Box { half_extents }) => {
            sphere_box(a.position, radius, b.position, b.orientation, half_extents)
        }
        (Shape::Box { half_extents }, Shape::Sphere { radius }) => {
            sphere_box(b.position, radius, a.position, a.orientation, half_extents)
                .map(|(normal, depth)| (-normal, depth))
        }
        (
            Shape::Capsule {
                radius,
                half_height,
            },
            Shape::Box { half_extents },
        ) => capsule_box(
            a,
            radius,
            half_height,
            b.position,
            b.orientation,
            half_extents,
        ),
        (
            Shape::Box { half_extents },
            Shape::Capsule {
                radius,
                half_height,
            },
        ) => capsule_box(
            b,
            radius,
            half_height,
            a.position,
            a.orientation,
            half_extents,
        )
        .map(|(normal, depth)| (-normal, depth)),
        (Shape::Box { half_extents: ha }, Shape::Box { half_extents: hb }) => {
            box_box(a.position, a.orientation, ha, b.position, b.orientation, hb)
        }
        _ => None,
    }
}

fn cuboid_body(position: Vec3, orientation: Quat, half: Vec3, other: &Body) -> Option<(Vec3, f32)> {
    match other.shape {
        Shape::Sphere { radius } => sphere_box(other.position, radius, position, orientation, half),
        Shape::Capsule {
            radius,
            half_height,
        } => capsule_box(other, radius, half_height, position, orientation, half),
        Shape::Box { half_extents } => box_box(
            other.position,
            other.orientation,
            half_extents,
            position,
            orientation,
            half,
        ),
        Shape::Plane { .. } | Shape::Mesh(_) => None,
    }
}

fn spheres(a: Vec3, ra: f32, b: Vec3, rb: f32) -> Option<(Vec3, f32)> {
    let delta = a - b;
    let radius = ra + rb;
    let dist_sq = delta.length_squared();
    if dist_sq > radius * radius {
        return None;
    }
    let dist = dist_sq.sqrt();
    let normal = if dist > 1.0e-8 { delta / dist } else { Vec3::Y };
    Some((normal, radius - dist))
}

fn capsule_sphere(
    capsule: &Body,
    radius: f32,
    half_height: f32,
    center: Vec3,
    other_radius: f32,
) -> Option<(Vec3, f32)> {
    let (start, end) = segment(capsule, half_height);
    let point = closest_on_segment(center, start, end);
    spheres(point, radius, center, other_radius)
}

fn capsule_capsule(a: &Body, ra: f32, ha: f32, b: &Body, rb: f32, hb: f32) -> Option<(Vec3, f32)> {
    let (a0, a1) = segment(a, ha);
    let (b0, b1) = segment(b, hb);
    let (pa, pb) = closest_segments(a0, a1, b0, b1);
    spheres(pa, ra, pb, rb)
}

fn sphere_box(
    center: Vec3,
    radius: f32,
    box_pos: Vec3,
    box_orn: Quat,
    half: Vec3,
) -> Option<(Vec3, f32)> {
    let local = conjugate(box_orn).rotate(center - box_pos);
    let clamped = Vec3::new(
        local.x.clamp(-half.x, half.x),
        local.y.clamp(-half.y, half.y),
        local.z.clamp(-half.z, half.z),
    );
    let delta = local - clamped;
    let dist_sq = delta.length_squared();
    if dist_sq > 1.0e-10 {
        let dist = dist_sq.sqrt();
        if dist > radius {
            return None;
        }
        let normal = box_orn.rotate(delta / dist);
        return Some((normal, radius - dist));
    }
    let (axis, sign, depth) = nearest_face(local, half);
    let normal = box_orn.rotate(axis_vector(axis) * sign);
    Some((normal, depth + radius))
}

fn capsule_box(
    capsule: &Body,
    radius: f32,
    half_height: f32,
    box_pos: Vec3,
    box_orn: Quat,
    half: Vec3,
) -> Option<(Vec3, f32)> {
    let (start, end) = segment(capsule, half_height);
    let inv = conjugate(box_orn);
    let a = inv.rotate(start - box_pos);
    let b = inv.rotate(end - box_pos);
    let (on_seg, on_box, dist) = closest_segment_aabb(a, b, half);
    if dist > radius {
        return None;
    }
    if dist > 1.0e-5 {
        let normal = box_orn.rotate((on_seg - on_box) / dist);
        return Some((normal, radius - dist));
    }
    let (local_normal, depth) = segment_exit(a, b, half);
    Some((box_orn.rotate(local_normal), depth + radius))
}

fn box_box(
    a_pos: Vec3,
    a_orn: Quat,
    a_half: Vec3,
    b_pos: Vec3,
    b_orn: Quat,
    b_half: Vec3,
) -> Option<(Vec3, f32)> {
    let axes_a = [
        a_orn.rotate(Vec3::X),
        a_orn.rotate(Vec3::Y),
        a_orn.rotate(Vec3::Z),
    ];
    let axes_b = [
        b_orn.rotate(Vec3::X),
        b_orn.rotate(Vec3::Y),
        b_orn.rotate(Vec3::Z),
    ];
    let delta = a_pos - b_pos;
    let mut best_pen = f32::MAX;
    let mut best_axis = Vec3::Y;
    let mut test = |axis: Vec3| -> bool {
        let axis = unit(axis);
        if axis.length_squared() == 0.0 {
            return true;
        }
        let dist = delta.dot(axis).abs();
        let span = project_box(axis, &axes_a, a_half) + project_box(axis, &axes_b, b_half);
        if dist > span {
            return false;
        }
        let pen = span - dist;
        if pen < best_pen {
            best_pen = pen;
            best_axis = if delta.dot(axis) < 0.0 { -axis } else { axis };
        }
        true
    };
    for axis in axes_a {
        if !test(axis) {
            return None;
        }
    }
    for axis in axes_b {
        if !test(axis) {
            return None;
        }
    }
    for axis_a in axes_a {
        for axis_b in axes_b {
            if !test(axis_a.cross(axis_b)) {
                return None;
            }
        }
    }
    if best_pen == f32::MAX {
        return None;
    }
    Some((best_axis, best_pen))
}

fn hull_body(owner: &Body, hull: &Hull, other: &Body) -> Option<(Vec3, f32)> {
    match other.shape {
        Shape::Sphere { radius } => {
            let local = conjugate(owner.orientation).rotate(other.position - owner.position);
            sphere_hull(local, radius, hull)
                .map(|(normal, depth)| (owner.orientation.rotate(normal), depth))
                .map(|(normal, depth)| (-normal, depth))
        }
        Shape::Capsule {
            radius,
            half_height,
        } => {
            let (start, end) = segment(other, half_height);
            let inv = conjugate(owner.orientation);
            let a = inv.rotate(start - owner.position);
            let b = inv.rotate(end - owner.position);
            capsule_hull(a, b, radius, hull)
                .map(|(normal, depth)| (owner.orientation.rotate(normal), depth))
                .map(|(normal, depth)| (-normal, depth))
        }
        Shape::Box { half_extents } => {
            let center = conjugate(owner.orientation).rotate(other.position - owner.position);
            let orientation = quat_mul(conjugate(owner.orientation), other.orientation);
            box_hull(center, orientation, half_extents, hull)
                .map(|(normal, depth)| (owner.orientation.rotate(normal), depth))
                .map(|(normal, depth)| (-normal, depth))
        }
        Shape::Plane { .. } | Shape::Mesh(_) => None,
    }
}

fn hull_hull(a: &Body, left: &Hull, b: &Body, right: &Hull) -> Option<(Vec3, f32)> {
    let mut best: Option<(Vec3, f32)> = None;
    for vertex in right.verts.iter().take(right.vert_count as usize) {
        let point = b.position + b.orientation.rotate(*vertex);
        let local = conjugate(a.orientation).rotate(point - a.position);
        if let Some((normal, depth)) = sphere_hull(local, 0.0, left) {
            let world = a.orientation.rotate(normal);
            best = deeper(best, (world, depth));
        }
    }
    best
}

fn sphere_hull(center: Vec3, radius: f32, hull: &Hull) -> Option<(Vec3, f32)> {
    if hull.vert_count == 0 || hull.face_count == 0 {
        return None;
    }
    if let Some((normal, depth)) = hull_exit_point(center, hull) {
        return Some((normal, depth + radius));
    }
    let closest = closest_on_hull(center, hull);
    let delta = center - closest;
    let dist = delta.length();
    if dist > radius {
        return None;
    }
    let normal = if dist > 1.0e-6 { delta / dist } else { Vec3::Y };
    Some((normal, radius - dist))
}

fn capsule_hull(a: Vec3, b: Vec3, radius: f32, hull: &Hull) -> Option<(Vec3, f32)> {
    if hull.vert_count == 0 {
        return None;
    }
    let (on_seg, on_hull, dist) = closest_segment_hull(a, b, hull);
    if dist > radius && !segment_hits_hull(a, b, hull) {
        return None;
    }
    if dist > 1.0e-4 && !segment_hits_hull(a, b, hull) {
        let normal = (on_seg - on_hull) / dist;
        return Some((normal, radius - dist));
    }
    let (normal, depth) = segment_hull_exit(a, b, hull)?;
    Some((normal, depth + radius))
}

fn box_hull(center: Vec3, orientation: Quat, half: Vec3, hull: &Hull) -> Option<(Vec3, f32)> {
    let mut best: Option<(Vec3, f32)> = None;
    for vert in box_corners(center, orientation, half) {
        if let Some(hit) = sphere_hull(vert, 0.0, hull) {
            best = deeper(best, hit);
        }
    }
    best
}

fn hull_exit_point(point: Vec3, hull: &Hull) -> Option<(Vec3, f32)> {
    let mut exit = f32::MIN;
    let mut normal = Vec3::Y;
    for face in hull.faces.iter().take(hull.face_count as usize) {
        let dist = face.normal.dot(point) - face.offset;
        if dist > 1.0e-5 {
            return None;
        }
        if dist > exit {
            exit = dist;
            normal = face.normal;
        }
    }
    if exit == f32::MIN {
        return None;
    }
    Some((normal, -exit))
}

fn segment_hits_hull(a: Vec3, b: Vec3, hull: &Hull) -> bool {
    if hull_exit_point(a, hull).is_some() || hull_exit_point(b, hull).is_some() {
        return true;
    }
    for face in hull.faces.iter().take(hull.face_count as usize) {
        let da = face.normal.dot(a) - face.offset;
        let db = face.normal.dot(b) - face.offset;
        if da > 0.0 && db > 0.0 {
            return false;
        }
    }
    closest_segment_hull(a, b, hull).2 < 1.0e-3
}

fn segment_hull_exit(a: Vec3, b: Vec3, hull: &Hull) -> Option<(Vec3, f32)> {
    let mut best = f32::MAX;
    let mut normal = Vec3::Y;
    for face in hull.faces.iter().take(hull.face_count as usize) {
        let da = face.normal.dot(a) - face.offset;
        let db = face.normal.dot(b) - face.offset;
        let pen = -da.min(db);
        if pen < best {
            best = pen;
            normal = face.normal;
        }
    }
    if best == f32::MAX {
        None
    } else {
        Some((normal, best.max(0.0)))
    }
}

fn closest_segment_hull(a: Vec3, b: Vec3, hull: &Hull) -> (Vec3, Vec3, f32) {
    let mut point = a;
    let mut surface = closest_on_hull(point, hull);
    for _ in 0..16 {
        point = closest_on_segment(surface, a, b);
        surface = closest_on_hull(point, hull);
    }
    point = closest_on_segment(surface, a, b);
    let dist = (point - surface).length();
    (point, surface, dist)
}

fn closest_on_hull(point: Vec3, hull: &Hull) -> Vec3 {
    let verts = &hull.verts[..hull.vert_count as usize];
    let mut best = verts[0];
    let mut best_dist = (point - best).length_squared();
    for vertex in verts.iter().skip(1) {
        let dist = (point - *vertex).length_squared();
        if dist < best_dist {
            best = *vertex;
            best_dist = dist;
        }
    }
    for face in hull.faces.iter().take(hull.face_count as usize) {
        let a = hull.verts[face.index[0] as usize];
        let b = hull.verts[face.index[1] as usize];
        let c = hull.verts[face.index[2] as usize];
        for (p0, p1) in [(a, b), (b, c), (c, a)] {
            let edge = closest_on_segment(point, p0, p1);
            let dist = (point - edge).length_squared();
            if dist < best_dist {
                best = edge;
                best_dist = dist;
            }
        }
        let signed = face.normal.dot(point) - face.offset;
        let projected = point - face.normal * signed;
        if point_in_triangle(projected, a, b, c, face.normal) {
            let dist = signed * signed;
            if dist < best_dist {
                best = projected;
                best_dist = dist;
            }
        }
    }
    best
}

fn point_in_triangle(point: Vec3, a: Vec3, b: Vec3, c: Vec3, normal: Vec3) -> bool {
    let ab = (b - a).cross(point - a).dot(normal);
    let bc = (c - b).cross(point - b).dot(normal);
    let ca = (a - c).cross(point - c).dot(normal);
    ab >= -1.0e-4 && bc >= -1.0e-4 && ca >= -1.0e-4
}

fn closest_segment_aabb(a: Vec3, b: Vec3, half: Vec3) -> (Vec3, Vec3, f32) {
    let mut point = a;
    let mut surface = clamp_aabb(point, half);
    for _ in 0..12 {
        point = closest_on_segment(surface, a, b);
        surface = clamp_aabb(point, half);
    }
    point = closest_on_segment(surface, a, b);
    surface = clamp_aabb(point, half);
    let dist = (point - surface).length();
    (point, surface, dist)
}

fn segment_exit(a: Vec3, b: Vec3, half: Vec3) -> (Vec3, f32) {
    let mut best = f32::MAX;
    let mut normal = Vec3::Y;
    for axis in 0..3 {
        let (a_axis, b_axis, h) = match axis {
            0 => (a.x, b.x, half.x),
            1 => (a.y, b.y, half.y),
            _ => (a.z, b.z, half.z),
        };
        let min_c = a_axis.min(b_axis);
        let max_c = a_axis.max(b_axis);
        let pen_pos = h - min_c;
        let pen_neg = max_c + h;
        if pen_pos < best {
            best = pen_pos;
            normal = axis_vector(axis);
        }
        if pen_neg < best {
            best = pen_neg;
            normal = axis_vector(axis) * -1.0;
        }
    }
    (normal, best.max(0.0))
}

fn nearest_face(local: Vec3, half: Vec3) -> (usize, f32, f32) {
    let mut best_axis = 1;
    let mut best_sign = 1.0;
    let mut best_depth = f32::MAX;
    let coords = [local.x, local.y, local.z];
    let extents = [half.x, half.y, half.z];
    for axis in 0..3 {
        let pen_pos = extents[axis] - coords[axis];
        let pen_neg = extents[axis] + coords[axis];
        if pen_pos < best_depth {
            best_depth = pen_pos;
            best_axis = axis;
            best_sign = 1.0;
        }
        if pen_neg < best_depth {
            best_depth = pen_neg;
            best_axis = axis;
            best_sign = -1.0;
        }
    }
    (best_axis, best_sign, best_depth.max(0.0))
}

fn clamp_aabb(point: Vec3, half: Vec3) -> Vec3 {
    Vec3::new(
        point.x.clamp(-half.x, half.x),
        point.y.clamp(-half.y, half.y),
        point.z.clamp(-half.z, half.z),
    )
}

fn closest_on_segment(point: Vec3, start: Vec3, end: Vec3) -> Vec3 {
    let edge = end - start;
    let length_sq = edge.length_squared();
    if length_sq < 1.0e-12 {
        return start;
    }
    let t = (point - start).dot(edge) / length_sq;
    start + edge * t.clamp(0.0, 1.0)
}

fn closest_segments(a0: Vec3, a1: Vec3, b0: Vec3, b1: Vec3) -> (Vec3, Vec3) {
    let mut on_a = a0;
    let mut on_b = b0;
    for _ in 0..8 {
        on_a = closest_on_segment(on_b, a0, a1);
        on_b = closest_on_segment(on_a, b0, b1);
    }
    (on_a, on_b)
}

fn segment(body: &Body, half_height: f32) -> (Vec3, Vec3) {
    let axis = body.orientation.rotate(Vec3::Y) * half_height;
    (body.position - axis, body.position + axis)
}

fn support_point(body: &Body, dir: Vec3, piece: Option<&Hull>) -> Vec3 {
    let dir = unit(dir);
    if let Some(hull) = piece {
        return hull_support(body, hull, dir);
    }
    match body.shape {
        Shape::Sphere { radius } => body.position + dir * radius,
        Shape::Capsule {
            radius,
            half_height,
        } => {
            let axis = body.orientation.rotate(Vec3::Y);
            let end = if axis.dot(dir) >= 0.0 {
                half_height
            } else {
                -half_height
            };
            body.position + axis * end + dir * radius
        }
        Shape::Box { half_extents } => {
            let local = conjugate(body.orientation).rotate(dir);
            let support = Vec3::new(
                half_extents.x.copysign(local.x),
                half_extents.y.copysign(local.y),
                half_extents.z.copysign(local.z),
            );
            body.position + body.orientation.rotate(support)
        }
        Shape::Plane { .. } | Shape::Mesh(_) => body.position,
    }
}

fn hull_support(body: &Body, hull: &Hull, dir: Vec3) -> Vec3 {
    let local = conjugate(body.orientation).rotate(dir);
    let mut best = hull.verts[0];
    let mut best_dot = best.dot(local);
    for vertex in hull.verts.iter().take(hull.vert_count as usize).skip(1) {
        let dot = vertex.dot(local);
        if dot > best_dot {
            best = *vertex;
            best_dot = dot;
        }
    }
    body.position + body.orientation.rotate(best)
}

fn pose(body: &Body, local_center: Vec3) -> (Vec3, Quat) {
    (
        body.position + body.orientation.rotate(local_center),
        body.orientation,
    )
}

fn box_corners(center: Vec3, orientation: Quat, half: Vec3) -> [Vec3; 8] {
    let mut corners = [Vec3::ZERO; 8];
    let mut index = 0;
    for x in [-half.x, half.x] {
        for y in [-half.y, half.y] {
            for z in [-half.z, half.z] {
                corners[index] = center + orientation.rotate(Vec3::new(x, y, z));
                index += 1;
            }
        }
    }
    corners
}

fn project_box(axis: Vec3, axes: &[Vec3; 3], half: Vec3) -> f32 {
    axis.dot(axes[0]).abs() * half.x
        + axis.dot(axes[1]).abs() * half.y
        + axis.dot(axes[2]).abs() * half.z
}

fn axis_vector(axis: usize) -> Vec3 {
    match axis {
        0 => Vec3::X,
        1 => Vec3::Y,
        _ => Vec3::Z,
    }
}

fn unit(v: Vec3) -> Vec3 {
    let n = v.normalize();
    if n.length_squared() == 0.0 {
        Vec3::Y
    } else {
        n
    }
}

fn conjugate(q: Quat) -> Quat {
    Quat::new(-q.x, -q.y, -q.z, q.w)
}

fn quat_mul(lhs: Quat, rhs: Quat) -> Quat {
    Quat::new(
        lhs.w * rhs.x + lhs.x * rhs.w + lhs.y * rhs.z - lhs.z * rhs.y,
        lhs.w * rhs.y - lhs.x * rhs.z + lhs.y * rhs.w + lhs.z * rhs.x,
        lhs.w * rhs.z + lhs.x * rhs.y - lhs.y * rhs.x + lhs.z * rhs.w,
        lhs.w * rhs.w - lhs.x * rhs.x - lhs.y * rhs.y - lhs.z * rhs.z,
    )
}

/// Contact restitution is the larger of the two bodies. Friction is the larger too.
pub(crate) fn solve_contact(a: &mut Body, b: &mut Body, normal: Vec3, penetration: f32) {
    let normal = unit(normal);
    let inv = a.inverse_mass + b.inverse_mass;
    if inv <= 0.0 || !penetration.is_finite() || !normal.x.is_finite() {
        return;
    }
    let rel = a.velocity - b.velocity;
    let vn = rel.dot(normal);
    if vn < 0.0 {
        let restitution = a.restitution.max(b.restitution).clamp(0.0, 1.0);
        let j = -(1.0 + restitution) * vn / inv;
        let impulse = normal * j;
        a.velocity += impulse * a.inverse_mass;
        b.velocity -= impulse * b.inverse_mass;

        let tangent_vec = rel - normal * vn;
        let vt = tangent_vec.length();
        if vt > 1.0e-6 {
            let friction = a.friction.max(b.friction).max(0.0);
            if friction > 0.0 {
                let tangent = tangent_vec / vt;
                let jt = (-vt / inv).clamp(-friction * j, friction * j);
                let kick = tangent * jt;
                a.velocity += kick * a.inverse_mass;
                b.velocity -= kick * b.inverse_mass;
            }
        }
    }
    let depth = penetration - SLOP;
    if depth > 0.0 {
        let correction = normal * (depth * PERCENT / inv);
        a.position += correction * a.inverse_mass;
        b.position -= correction * b.inverse_mass;
    }
}
