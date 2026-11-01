#![expect(
    clippy::as_conversions,
    reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
)]
#![expect(
    clippy::float_cmp,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::large_types_passed_by_value,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::similar_names,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::single_match_else,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::struct_field_names,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::unnecessary_wraps,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::wildcard_enum_match_arm,
    reason = "TODO(lint-2026-10-14): pre-existing wildcard match. docs/LINT_RULES.md"
)]

//! Contact between the shapes on two bodies.
//!
//! Primitives use closest points. A mesh tests each convex piece. The normal
//! points from the second body toward the first, matching [`solve_contact`].

use genos_math::{Quat, Vec3};

use crate::solver::{ContactPoint, Manifold};
use crate::{Body, Hull, Mesh, Piece, Shape, MAX_HULL_EDGES, MAX_HULL_VERTS};

/// Scratch lists for [`collide`], kept across passes and steps.
#[derive(Clone, Debug)]
pub(crate) struct Pairs {
    planes: Vec<u32>,
    pairs: Vec<(u32, u32)>,
    manifolds: Vec<Manifold>,
    /// Last step's manifolds, impulses included, for the warm start.
    warm: Vec<Manifold>,
    wake: Vec<bool>,
}

impl Pairs {
    pub(crate) fn contact_point_count(&self) -> u32 {
        self.manifolds.iter().fold(0_u32, |sum, manifold| {
            sum.saturating_add(u32::from(manifold.count))
        })
    }

    pub(crate) fn pair_list(&self) -> &[(u32, u32)] {
        &self.pairs
    }

    pub(crate) fn solver_bodies(&mut self, bodies: &[crate::Body]) -> u32 {
        self.wake.resize(bodies.len(), false);
        self.wake.fill(false);
        let mut count = 0_u32;
        for &(left, right) in &self.pairs {
            for index in [left, right] {
                let Some(slot) = usize::try_from(index).ok() else {
                    continue;
                };
                let Some(flag) = self.wake.get_mut(slot) else {
                    continue;
                };
                if *flag {
                    continue;
                }
                let Some(body) = bodies.get(slot) else {
                    continue;
                };
                if body.inverse_mass > 0.0 && !body.sleeping {
                    *flag = true;
                    count = count.saturating_add(1);
                }
            }
        }
        count
    }
}

impl Pairs {
    pub(crate) const fn new() -> Self {
        Self {
            planes: Vec::new(),
            pairs: Vec::new(),
            manifolds: Vec::new(),
            warm: Vec::new(),
            wake: Vec::new(),
        }
    }
}

/// Bounds grow this much, so a pair a contact earlier in the same pass pushed
/// together is still tested.
pub(crate) const BOUNDS_MARGIN: f32 = 0.02;

/// One contact pass over every pair that can touch: bounds that meet (a sweep along
/// X), and every bounded body against every plane. Two static bodies never move, so
/// their pair is skipped. The pairs run in index order, the order a pass over every
/// pair took.
pub(crate) struct PairQuery<'a> {
    pub layers: &'a crate::broadphase::LayerPairs,
    pub trees: &'a mut crate::broadphase::Broadphase,
    pub islands: &'a mut crate::island::Islands,
}

pub(crate) fn collide(
    bodies: &mut [Body],
    scratch: &mut Pairs,
    query: &mut PairQuery<'_>,
    warm_start: bool,
    velocity: bool,
    rebuild: bool,
) -> u32 {
    if rebuild {
        query.trees.fill(
            bodies,
            query.layers,
            &mut scratch.pairs,
            &mut scratch.planes,
        );
        let displaced = std::mem::take(&mut query.trees.displaced);
        query
            .islands
            .wake_from_pairs(bodies, &scratch.pairs, &displaced);
        query.trees.displaced = displaced;
        scratch.pairs.retain(|(left, right)| {
            let live = |index: u32| {
                bodies
                    .get(usize::try_from(index).unwrap_or(0))
                    .is_some_and(|body| body.presence == crate::Presence::Live && !body.sleeping)
            };
            // A sleeper stays in the manifold as infinite mass. Dropping the
            // pair lets an awake neighbour fall through it.
            live(*left) || live(*right)
        });
    }
    if rebuild {
        scratch.manifolds.clear();
        let pairs = std::mem::take(&mut scratch.pairs);
        for (i, j) in &pairs {
            append_manifolds(*i, *j, bodies, &mut scratch.manifolds);
        }
        scratch.pairs = pairs;
    }
    let warm = if warm_start {
        std::mem::take(&mut scratch.warm)
    } else {
        Vec::new()
    };
    let hits = crate::solver::solve(
        bodies,
        &mut scratch.manifolds,
        &warm,
        &mut scratch.wake,
        velocity,
    );
    if warm_start {
        scratch.warm = warm;
        scratch.warm.clone_from(&scratch.manifolds);
    }
    hits
}

/// World bounds of a body: low and high corners. None for a plane, which is
/// unbounded.
pub(crate) fn bounds(body: &Body) -> Option<(Vec3, Vec3)> {
    let mut lo = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut hi = -lo;
    let mut add = |p: Vec3| {
        lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    };
    match body.shape {
        Shape::Plane { .. } => return None,
        Shape::Mesh(mesh) => {
            for piece in mesh.pieces() {
                match piece {
                    Piece::Cuboid { center, half } => {
                        for corner in 0..8 {
                            let local = Vec3::new(
                                if corner & 1 == 0 { -half.x } else { half.x },
                                if corner & 2 == 0 { -half.y } else { half.y },
                                if corner & 4 == 0 { -half.z } else { half.z },
                            );
                            add(body.position + body.orientation.rotate(*center + local));
                        }
                    }
                    Piece::Hull(hull) => {
                        for vert in &hull.verts[..hull.vert_count as usize] {
                            add(body.position + body.orientation.rotate(*vert));
                        }
                    }
                }
            }
        }
        _ => {
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                add(support_point(body, axis, None));
                add(support_point(body, -axis, None));
            }
        }
    }
    if lo.x > hi.x {
        // A mesh with no pieces: the position alone.
        return Some((body.position, body.position));
    }
    Some((lo, hi))
}

fn append_manifolds(i: u32, j: u32, bodies: &[Body], out: &mut Vec<Manifold>) {
    let (Some(a), Some(b)) = (bodies.get(usize_index(i)), bodies.get(usize_index(j))) else {
        return;
    };
    match (a.shape, b.shape) {
        (Shape::Plane { .. }, Shape::Plane { .. }) => {}
        (Shape::Plane { normal, offset }, _) => {
            append_plane(j, i, b, a, normal, offset, out);
        }
        (_, Shape::Plane { normal, offset }) => append_plane(i, j, a, b, normal, offset, out),
        (Shape::Mesh(mesh), _) => append_mesh(i, j, a, mesh, b, false, out),
        (_, Shape::Mesh(mesh)) => append_mesh(j, i, b, mesh, a, true, out),
        _ => {
            if let Some((normal, depth)) = primitive_contact(a, b) {
                push_manifold(i, j, a, b, normal, depth, out);
            }
        }
    }
}

fn append_plane(
    body_index: u32,
    plane_index: u32,
    body: &Body,
    plane: &Body,
    normal: Vec3,
    offset: f32,
    out: &mut Vec<Manifold>,
) {
    match body.shape {
        Shape::Mesh(mesh) => {
            for piece in mesh.pieces() {
                if let Some((hit_normal, depth)) = piece_plane(body, *piece, normal, offset) {
                    push_manifold(body_index, plane_index, body, plane, hit_normal, depth, out);
                }
            }
        }
        _ => {
            let point = support_point(body, -normal, None);
            let dist = normal.dot(point) - offset;
            if dist < 0.0 {
                push_manifold(body_index, plane_index, body, plane, normal, -dist, out);
            }
        }
    }
}

fn append_mesh(
    owner_index: u32,
    other_index: u32,
    owner: &Body,
    mesh: Mesh,
    other: &Body,
    flip: bool,
    out: &mut Vec<Manifold>,
) {
    let Some((normal, depth)) = mesh_contact(owner, &mesh, other) else {
        return;
    };
    if flip {
        push_manifold(other_index, owner_index, other, owner, -normal, depth, out);
    } else {
        push_manifold(owner_index, other_index, owner, other, normal, depth, out);
    }
}

fn push_manifold(
    a_index: u32,
    b_index: u32,
    a: &Body,
    b: &Body,
    normal: Vec3,
    depth: f32,
    out: &mut Vec<Manifold>,
) {
    let normal = unit(normal);
    if !depth.is_finite() || depth <= 0.0 || !normal.x.is_finite() {
        return;
    }
    let (points, count) = contact_points(a, b, normal, depth);
    if count == 0 {
        return;
    }
    let (t1, t2) = crate::solver::tangent_axes(normal);
    let mut points = points;
    let anchored = usize::from(count);
    for point in &mut points[..anchored] {
        point.local_a = conjugate(a.orientation).rotate(point.position - a.position);
        point.local_b = conjugate(b.orientation).rotate(point.position - b.position);
        point.depth0 = point.depth;
    }
    out.push(Manifold {
        a: a_index,
        b: b_index,
        normal,
        depth,
        points,
        count,
        tangent_axis: [t1, t2],
        twist: 0.0,
    });
}

fn usize_index(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
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

pub(crate) fn primitive_contact(a: &Body, b: &Body) -> Option<(Vec3, f32)> {
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

/// Separating-axis test on both hulls' face normals and on the cross product
/// of every pair of feature-edge directions. The axis with the least overlap
/// is the contact normal, pointing from `b` toward `a`. Face-only SAT misses
/// the edge-edge axis, so crossed edges read as touching when apart and as
/// deep face overlap when they barely cross.
fn hull_hull(a: &Body, left: &Hull, b: &Body, right: &Hull) -> Option<(Vec3, f32)> {
    hull_hull_contact(a, left, b, right).map(|hit| (hit.normal, hit.depth))
}

/// One hull-hull hit: normal from `b` toward `a`, depth, and a world point
/// midway between the two surfaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HullHit {
    pub normal: Vec3,
    pub depth: f32,
    pub point: Vec3,
}

/// Edge axes win only when clearly shallower than the best face axis, so a
/// face-to-face rest does not flicker onto a near-tied edge axis.
const EDGE_AXIS_BIAS: f32 = 1.0e-3;
/// `|ea x eb|` below this (unit edges) is treated as parallel and skipped.
const PARALLEL_SINE: f32 = 1.0e-3;

pub(crate) fn hull_hull_contact(a: &Body, left: &Hull, b: &Body, right: &Hull) -> Option<HullHit> {
    if left.vert_count == 0 || right.vert_count == 0 {
        return None;
    }
    // World-space vertices once per pair; every axis below projects them.
    let world_a = world_verts(a, left);
    let world_b = world_verts(b, right);
    let (verts_a, verts_b) = (
        &world_a[..usize::from(left.vert_count)],
        &world_b[..usize::from(right.vert_count)],
    );
    let mut best: Option<(Vec3, f32)> = None;
    for (owner, hull) in [(a, left), (b, right)] {
        for face in hull.faces.iter().take(hull.face_count as usize) {
            let axis = owner.orientation.rotate(face.normal);
            let (normal, depth) = axis_overlap(verts_a, verts_b, axis);
            if depth < -1.0e-5 {
                return None;
            }
            best = match best {
                Some((_, kept)) if kept <= depth => best,
                _ => Some((normal, depth.max(0.0))),
            };
        }
    }
    let (face_normal, face_depth) = best?;
    let (left_dirs, left_count) = (&left.edges, usize::from(left.edge_count));
    let (right_dirs, right_count) = (&right.edges, usize::from(right.edge_count));
    let mut edge: Option<(Vec3, f32, usize, usize)> = None;
    for (i, local_a) in left_dirs.iter().take(left_count).enumerate() {
        let ea = a.orientation.rotate(*local_a);
        for (j, local_b) in right_dirs.iter().take(right_count).enumerate() {
            let eb = b.orientation.rotate(*local_b);
            let cross = ea.cross(eb);
            let sine = cross.length();
            if sine < PARALLEL_SINE || !sine.is_finite() {
                continue;
            }
            let axis = cross / sine;
            let (normal, depth) = axis_overlap(verts_a, verts_b, axis);
            if depth < -1.0e-5 {
                return None;
            }
            edge = match edge {
                Some((_, kept, _, _)) if kept <= depth => edge,
                _ => Some((normal, depth.max(0.0), i, j)),
            };
        }
    }
    if let Some((normal, depth, i, j)) = edge {
        if depth + EDGE_AXIS_BIAS < face_depth {
            let ea = a.orientation.rotate(left_dirs[i]);
            let eb = b.orientation.rotate(right_dirs[j]);
            // a's edge facing b (support along -normal), b's along +normal.
            let (a0, a1) = support_edge(a, left, -normal, ea);
            let (b0, b1) = support_edge(b, right, normal, eb);
            let (on_a, on_b) = closest_between_segments(a0, a1, b0, b1);
            return Some(HullHit {
                normal,
                depth,
                point: (on_a + on_b) * 0.5,
            });
        }
    }
    let deepest_b = support_world(verts_b, face_normal);
    Some(HullHit {
        normal: face_normal,
        depth: face_depth,
        point: deepest_b - face_normal * (face_depth * 0.5),
    })
}

/// Overlap of the two hulls on `axis`, and the normal (from `b` toward `a`)
/// that resolves it. Negative depth means `axis` separates them.
fn axis_overlap(verts_a: &[Vec3], verts_b: &[Vec3], axis: Vec3) -> (Vec3, f32) {
    let (min_a, max_a) = extent(verts_a, axis);
    let (min_b, max_b) = extent(verts_b, axis);
    let ahead = max_a - min_b;
    let behind = max_b - min_a;
    if ahead <= behind {
        (-axis, ahead)
    } else {
        (axis, behind)
    }
}

/// Unit directions of the hull's feature edges in local space, one per
/// parallel family. Triangle diagonals inside a flat face are skipped.
/// Runs when the hull is built, not in the step.
pub(crate) fn edge_directions(hull: &Hull) -> ([Vec3; MAX_HULL_EDGES], usize) {
    let mut dirs = [Vec3::ZERO; MAX_HULL_EDGES];
    let mut count = 0;
    let faces = &hull.faces[..hull.face_count as usize];
    for (f, face) in faces.iter().enumerate() {
        for k in 0..3 {
            let i0 = face.index[k];
            let i1 = face.index[(k + 1) % 3];
            let neighbour = faces.iter().enumerate().find(|(g, other)| {
                *g != f && (0..3).any(|m| other.index[m] == i1 && other.index[(m + 1) % 3] == i0)
            });
            if let Some((_, other)) = neighbour {
                if other.normal.dot(face.normal) > 1.0 - 1.0e-4 {
                    continue;
                }
            }
            let edge = hull.verts[usize::from(i1)] - hull.verts[usize::from(i0)];
            let length = edge.length();
            if length < 1.0e-6 {
                continue;
            }
            let dir = edge / length;
            let known = dirs[..count]
                .iter()
                .any(|kept: &Vec3| kept.cross(dir).length() < PARALLEL_SINE);
            if !known && count < MAX_HULL_EDGES {
                dirs[count] = dir;
                count += 1;
            }
        }
    }
    (dirs, count)
}

fn world_verts(body: &Body, hull: &Hull) -> [Vec3; crate::MAX_HULL_VERTS] {
    let mut out = [body.position; crate::MAX_HULL_VERTS];
    for (slot, vertex) in out
        .iter_mut()
        .zip(hull.verts.iter().take(usize::from(hull.vert_count)))
    {
        *slot = body.position + body.orientation.rotate(*vertex);
    }
    out
}

fn extent(verts: &[Vec3], axis: Vec3) -> (f32, f32) {
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for vertex in verts {
        let along = axis.dot(*vertex);
        low = low.min(along);
        high = high.max(along);
    }
    (low, high)
}

fn support_world(verts: &[Vec3], direction: Vec3) -> Vec3 {
    let mut best = Vec3::ZERO;
    let mut best_dot = f32::NEG_INFINITY;
    for world in verts.iter().copied() {
        let along = direction.dot(world);
        if along > best_dot {
            best_dot = along;
            best = world;
        }
    }
    best
}

/// The world-space hull edge parallel to `dir` that reaches furthest along
/// `direction`.
fn support_edge(body: &Body, hull: &Hull, direction: Vec3, dir: Vec3) -> (Vec3, Vec3) {
    let mut best = (body.position, body.position);
    let mut best_dot = f32::NEG_INFINITY;
    for face in hull.faces.iter().take(hull.face_count as usize) {
        for k in 0..3 {
            let p0 = body.position
                + body
                    .orientation
                    .rotate(hull.verts[usize::from(face.index[k])]);
            let p1 = body.position
                + body
                    .orientation
                    .rotate(hull.verts[usize::from(face.index[(k + 1) % 3])]);
            let edge = p1 - p0;
            let length = edge.length();
            if length < 1.0e-6 || (edge / length).cross(dir).length() >= PARALLEL_SINE {
                continue;
            }
            let along = direction.dot(p0 + p1);
            if along > best_dot {
                best_dot = along;
                best = (p0, p1);
            }
        }
    }
    best
}

/// Closest points between segments `p0-p1` and `q0-q1`.
fn closest_between_segments(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> (Vec3, Vec3) {
    let d1 = p1 - p0;
    let d2 = q1 - q0;
    let offset = p0 - q0;
    let len_p = d1.dot(d1);
    let len_q = d2.dot(d2);
    let q_off = d2.dot(offset);
    if len_p <= 1.0e-12 && len_q <= 1.0e-12 {
        return (p0, q0);
    }
    let (sp, tq) = if len_p <= 1.0e-12 {
        (0.0, (q_off / len_q).clamp(0.0, 1.0))
    } else {
        let p_off = d1.dot(offset);
        if len_q <= 1.0e-12 {
            ((-p_off / len_p).clamp(0.0, 1.0), 0.0)
        } else {
            let mix = d1.dot(d2);
            let denom = len_p * len_q - mix * mix;
            let sp = if denom > 1.0e-12 {
                ((mix * q_off - p_off * len_q) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let tq = (mix * sp + q_off) / len_q;
            if tq < 0.0 {
                ((-p_off / len_p).clamp(0.0, 1.0), 0.0)
            } else if tq > 1.0 {
                (((mix - p_off) / len_p).clamp(0.0, 1.0), 1.0)
            } else {
                (sp, tq)
            }
        }
    };
    (p0 + d1 * sp, q0 + d2 * tq)
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

pub(crate) fn shape_support(body: &Body, dir: Vec3) -> Vec3 {
    match body.shape {
        Shape::Mesh(mesh) => {
            let mut best = body.position;
            let mut best_dot = f32::NEG_INFINITY;
            let mut found = false;
            for piece in mesh.pieces() {
                let point = piece_support(body, piece, dir);
                let score = point.dot(dir);
                if !found || score > best_dot {
                    best = point;
                    best_dot = score;
                    found = true;
                }
            }
            best
        }
        Shape::Plane { .. } => body.position,
        Shape::Sphere { .. } | Shape::Capsule { .. } | Shape::Box { .. } => {
            support_point(body, dir, None)
        }
    }
}

fn piece_support(body: &Body, piece: &Piece, dir: Vec3) -> Vec3 {
    let dir = unit(dir);
    match piece {
        Piece::Cuboid { center, half } => {
            let local = conjugate(body.orientation).rotate(dir);
            let support = Vec3::new(
                half.x.copysign(local.x),
                half.y.copysign(local.y),
                half.z.copysign(local.z),
            );
            body.position + body.orientation.rotate(*center + support)
        }
        Piece::Hull(hull) => hull_support(body, hull, dir),
    }
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
///
/// Returns whether both bodies were dynamic and a closing impulse was stored.
pub(crate) fn solve_contact(a: &mut Body, b: &mut Body, normal: Vec3, penetration: f32) -> bool {
    let normal = unit(normal);
    if a.inverse_mass + b.inverse_mass <= 0.0 || !penetration.is_finite() || !normal.x.is_finite() {
        return false;
    }
    let (mut points, count) = contact_points(a, b, normal, penetration.max(0.0));
    crate::solver::solve_pair(a, b, normal, &mut points[..usize::from(count)])
}

/// Incident face of `a` clipped to the reference face of `b`.
///
/// `normal` points out of `b` into `a`. The polygon is the face of `a` that
/// looks back along that normal, cut by the side planes of `b`'s facing
/// face, then reduced to at most four points of the overlap.
fn face_clip(a: &Body, b: &Body, normal: Vec3) -> Option<([ContactPoint; 4], u8)> {
    let mut incident = [Vec3::ZERO; MAX_HULL_VERTS];
    let count = face_polygon(a, -normal, &mut incident)?;
    if count < 2 {
        return None;
    }
    let mut clipped = [Vec3::ZERO; 16];
    let kept = clip_to_body(&incident[..count], b, normal, &mut clipped);
    if kept == 0 {
        return None;
    }
    let mut chosen = [Vec3::ZERO; 4];
    let used = reduce_polygon(&clipped[..kept], &mut chosen);
    let support = support_of(b, normal);
    let plane = support.dot(normal);
    let mut points = [ContactPoint::new(Vec3::ZERO, 0.0); 4];
    for index in 0..used {
        let position = chosen[index];
        let depth = plane - position.dot(normal);
        points[index] = ContactPoint::new(position, depth.max(0.0));
    }
    let count = u8::try_from(used).unwrap_or(u8::MAX);
    Some((points, count))
}

fn face_polygon(body: &Body, direction: Vec3, out: &mut [Vec3]) -> Option<usize> {
    match body.shape {
        Shape::Box { half_extents } => Some(box_face(body, half_extents, direction, out)),
        Shape::Mesh(mesh) => mesh_face(body, &mesh, direction, out),
        Shape::Sphere { .. } | Shape::Capsule { .. } | Shape::Plane { .. } => None,
    }
}

fn box_face(body: &Body, half: Vec3, direction: Vec3, out: &mut [Vec3]) -> usize {
    let local = conjugate(body.orientation).rotate(direction);
    let (axis, sign) = dominant_axis(local);
    let mut extent = Vec3::ZERO;
    match axis {
        0 => extent.x = sign * half.x,
        1 => extent.y = sign * half.y,
        _ => extent.z = sign * half.z,
    }
    let mut corners = [Vec3::ZERO; 4];
    for (index, (u, v)) in [(1.0, 1.0), (1.0, -1.0), (-1.0, -1.0), (-1.0, 1.0)]
        .into_iter()
        .enumerate()
    {
        let mut corner = extent;
        match axis {
            0 => {
                corner.y = u * half.y;
                corner.z = v * half.z;
            }
            1 => {
                corner.x = u * half.x;
                corner.z = v * half.z;
            }
            _ => {
                corner.x = u * half.x;
                corner.y = v * half.y;
            }
        }
        corners[index] = body.position + body.orientation.rotate(corner);
    }
    out[..4].copy_from_slice(&corners);
    4
}

fn dominant_axis(local: Vec3) -> (usize, f32) {
    let ax = local.x.abs();
    let ay = local.y.abs();
    let az = local.z.abs();
    if ax >= ay && ax >= az {
        (0, if local.x >= 0.0 { 1.0 } else { -1.0 })
    } else if ay >= az {
        (1, if local.y >= 0.0 { 1.0 } else { -1.0 })
    } else {
        (2, if local.z >= 0.0 { 1.0 } else { -1.0 })
    }
}

fn mesh_face(body: &Body, mesh: &Mesh, direction: Vec3, out: &mut [Vec3]) -> Option<usize> {
    let mut best = f32::NEG_INFINITY;
    let mut best_piece: Option<Piece> = None;
    for piece in mesh.pieces() {
        match piece {
            Piece::Cuboid { half, .. } => {
                let local = conjugate(body.orientation).rotate(direction);
                let (axis, sign) = dominant_axis(local);
                let mut offset = Vec3::ZERO;
                match axis {
                    0 => offset.x = sign * half.x,
                    1 => offset.y = sign * half.y,
                    _ => offset.z = sign * half.z,
                }
                let normal = body.orientation.rotate(offset).normalize_or(direction);
                let score = normal.dot(direction);
                if score > best {
                    best = score;
                    best_piece = Some(*piece);
                }
            }
            Piece::Hull(hull) => {
                let faces = hull.face_count as usize;
                for face in hull.faces.iter().take(faces) {
                    let normal = body.orientation.rotate(face.normal);
                    let score = normal.dot(direction);
                    if score > best {
                        best = score;
                        best_piece = Some(*piece);
                    }
                }
            }
        }
    }
    let piece = best_piece?;
    match piece {
        Piece::Cuboid { center, half } => {
            let shifted = Body {
                position: body.position + body.orientation.rotate(center),
                ..*body
            };
            Some(box_face(&shifted, half, direction, out))
        }
        Piece::Hull(hull) => Some(hull_face_verts(body, &hull, direction, best, out)),
    }
}

trait NormalizeOr {
    fn normalize_or(self, fallback: Vec3) -> Vec3;
}

impl NormalizeOr for Vec3 {
    fn normalize_or(self, fallback: Vec3) -> Vec3 {
        let length = self.length();
        if length < 1.0e-8 {
            fallback
        } else {
            self / length
        }
    }
}

fn hull_face_verts(
    body: &Body,
    hull: &Hull,
    direction: Vec3,
    best: f32,
    out: &mut [Vec3],
) -> usize {
    let mut verts = [Vec3::ZERO; MAX_HULL_VERTS];
    let mut count = 0_usize;
    let mut face_normal = direction;
    let faces = hull.face_count as usize;
    for face in hull.faces.iter().take(faces) {
        let normal = body.orientation.rotate(face.normal);
        if normal.dot(direction) < best - 0.05 {
            continue;
        }
        face_normal = normal;
        for slot in face.index {
            let local = hull.verts[slot as usize];
            let world = body.position + body.orientation.rotate(local);
            let mut seen = false;
            for other in verts.iter().take(count) {
                if (*other - world).length_squared() < 1.0e-8 {
                    seen = true;
                    break;
                }
            }
            if seen || count >= verts.len() {
                continue;
            }
            verts[count] = world;
            count += 1;
        }
    }
    if count < 3 {
        return count.min(out.len());
    }
    order_polygon(&mut verts[..count], face_normal);
    let n = count.min(out.len());
    out[..n].copy_from_slice(&verts[..n]);
    n
}

fn order_polygon(verts: &mut [Vec3], normal: Vec3) {
    let mut origin = Vec3::ZERO;
    for vertex in verts.iter() {
        origin += *vertex;
    }
    origin = origin / f32::from(u8::try_from(verts.len()).unwrap_or(1));
    let tangent = perpendicular_axis(normal);
    let bitangent = normal.cross(tangent);
    verts.sort_by(|left, right| {
        let left_offset = *left - origin;
        let right_offset = *right - origin;
        let left_angle = bitangent.dot(left_offset).atan2(tangent.dot(left_offset));
        let right_angle = bitangent.dot(right_offset).atan2(tangent.dot(right_offset));
        left_angle
            .partial_cmp(&right_angle)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn perpendicular_axis(normal: Vec3) -> Vec3 {
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

fn clip_to_body(incident: &[Vec3], reference: &Body, normal: Vec3, out: &mut [Vec3]) -> usize {
    match reference.shape {
        Shape::Plane { .. } | Shape::Sphere { .. } | Shape::Capsule { .. } => {
            let n = incident.len().min(out.len());
            out[..n].copy_from_slice(&incident[..n]);
            n
        }
        Shape::Box { .. } | Shape::Mesh(_) => {
            let mut planes = [(Vec3::ZERO, 0.0_f32); 8];
            let plane_count = reference_planes(reference, normal, &mut planes);
            if plane_count == 0 {
                let n = incident.len().min(out.len());
                out[..n].copy_from_slice(&incident[..n]);
                return n;
            }
            let mut current = [Vec3::ZERO; 16];
            let mut n = incident.len().min(current.len());
            current[..n].copy_from_slice(&incident[..n]);
            let mut other = [Vec3::ZERO; 16];
            for (plane_n, offset) in planes.iter().take(plane_count) {
                n = clip_halfspace(&current[..n], *plane_n, *offset, &mut other);
                current[..n].copy_from_slice(&other[..n]);
            }
            let kept = n.min(out.len());
            out[..kept].copy_from_slice(&current[..kept]);
            kept
        }
    }
}

/// Inward side planes of the face of `body` that faces `outward`.
fn reference_planes(body: &Body, outward: Vec3, planes: &mut [(Vec3, f32); 8]) -> usize {
    let mut polygon = [Vec3::ZERO; MAX_HULL_VERTS];
    let Some(count) = face_polygon(body, outward, &mut polygon) else {
        return 0;
    };
    if count < 3 {
        return 0;
    }
    let mut center = Vec3::ZERO;
    for vertex in polygon.iter().take(count) {
        center += *vertex;
    }
    center = center / f32::from(u8::try_from(count).unwrap_or(1));
    let mut n = 0_usize;
    for index in 0..count {
        if n >= planes.len() {
            break;
        }
        let start = polygon[index];
        let end = polygon[(index + 1) % count];
        let edge = end - start;
        if edge.length_squared() < 1.0e-10 {
            continue;
        }
        let mut inward = outward.cross(edge);
        let length = inward.length();
        if length < 1.0e-8 {
            continue;
        }
        inward = inward / length;
        if inward.dot(center - start) < 0.0 {
            inward = -inward;
        }
        planes[n] = (inward, inward.dot(start));
        n += 1;
    }
    n
}

fn clip_halfspace(input: &[Vec3], normal: Vec3, offset: f32, output: &mut [Vec3]) -> usize {
    if input.is_empty() {
        return 0;
    }
    let inside = |point: Vec3| point.dot(normal) >= offset - 1.0e-5;
    let mut count = 0_usize;
    let mut previous = input[input.len() - 1];
    let mut previous_in = inside(previous);
    for &point in input {
        let point_in = inside(point);
        if point_in {
            if !previous_in && count < output.len() {
                output[count] = intersect_plane(previous, point, normal, offset);
                count += 1;
            }
            if count < output.len() {
                output[count] = point;
                count += 1;
            }
        } else if previous_in && count < output.len() {
            output[count] = intersect_plane(previous, point, normal, offset);
            count += 1;
        }
        previous = point;
        previous_in = point_in;
    }
    count
}

fn intersect_plane(from: Vec3, to: Vec3, normal: Vec3, offset: f32) -> Vec3 {
    let start = from.dot(normal);
    let end = to.dot(normal);
    let denom = end - start;
    if denom.abs() < 1.0e-8 {
        return from;
    }
    let t = ((offset - start) / denom).clamp(0.0, 1.0);
    from + (to - from) * t
}

/// Keep at most four points. The first is an extreme, then the furthest,
/// then the two that add the most area, so a face does not collapse onto
/// one corner.
fn reduce_polygon(points: &[Vec3], out: &mut [Vec3; 4]) -> usize {
    if points.len() <= 4 {
        for (index, point) in points.iter().enumerate() {
            out[index] = *point;
        }
        return points.len();
    }
    out[0] = points[0];
    let mut far = 0_usize;
    let mut far_d = -1.0_f32;
    for (index, point) in points.iter().enumerate() {
        let distance = (*point - out[0]).length_squared();
        if distance > far_d {
            far_d = distance;
            far = index;
        }
    }
    out[1] = points[far];
    let mut best = 0_usize;
    let mut best_area = -1.0_f32;
    for (index, point) in points.iter().enumerate() {
        let area = (out[1] - out[0]).cross(*point - out[0]).length_squared();
        if area > best_area {
            best_area = area;
            best = index;
        }
    }
    out[2] = points[best];
    let mut fourth = 0_usize;
    let mut fourth_area = -1.0_f32;
    for (index, point) in points.iter().enumerate() {
        let area = (out[1] - out[0]).cross(*point - out[0]).length_squared()
            + (out[2] - out[1]).cross(*point - out[1]).length_squared();
        if area > fourth_area {
            fourth_area = area;
            fourth = index;
        }
    }
    out[3] = points[fourth];
    4
}

fn contact_points(a: &Body, b: &Body, normal: Vec3, depth: f32) -> ([ContactPoint; 4], u8) {
    if let Some(point) = round_surface(a, b, normal) {
        return (
            [
                ContactPoint::new(point, depth),
                ContactPoint::new(Vec3::ZERO, 0.0),
                ContactPoint::new(Vec3::ZERO, 0.0),
                ContactPoint::new(Vec3::ZERO, 0.0),
            ],
            1,
        );
    }
    if let Some(point) = round_surface(b, a, -normal) {
        return (
            [
                ContactPoint::new(point, depth),
                ContactPoint::new(Vec3::ZERO, 0.0),
                ContactPoint::new(Vec3::ZERO, 0.0),
                ContactPoint::new(Vec3::ZERO, 0.0),
            ],
            1,
        );
    }
    if let Some(clipped) = face_clip(a, b, normal) {
        return clipped;
    }
    let mut found = [(f32::NEG_INFINITY, Vec3::ZERO); 4];
    let mut count = 0_u8;
    // `normal` points from `b` toward `a`. Keep the vertices of either body
    // that actually sit in the overlap, measured against the facing plane.
    collect_face(a, b, normal, depth, &mut found, &mut count);
    if count == 0 {
        collect_face(b, a, -normal, depth, &mut found, &mut count);
    }
    if count == 0 {
        let point = support_point(a, -normal, None);
        return (
            [
                ContactPoint::new(point, depth),
                ContactPoint::new(Vec3::ZERO, 0.0),
                ContactPoint::new(Vec3::ZERO, 0.0),
                ContactPoint::new(Vec3::ZERO, 0.0),
            ],
            1,
        );
    }
    let mut points = [ContactPoint::new(Vec3::ZERO, 0.0); 4];
    for (index, (point_depth, position)) in found.into_iter().take(usize::from(count)).enumerate() {
        points[index] = ContactPoint::new(position, point_depth);
    }
    (points, count)
}

/// Surface point of a sphere or capsule. `outward` points from the other body
/// toward `body`, so the contact sits on the near side and `r` stays along
/// the normal. A segment endpoint would torque a capsule that is only translating.
fn round_surface(body: &Body, _other: &Body, outward: Vec3) -> Option<Vec3> {
    match body.shape {
        Shape::Sphere { radius } => Some(body.position - outward * radius),
        Shape::Capsule {
            radius,
            half_height,
        } => {
            let (start, end) = segment(body, half_height);
            let on_axis = closest_on_segment(body.position - outward, start, end);
            Some(on_axis - outward * radius)
        }
        Shape::Box { .. } | Shape::Plane { .. } | Shape::Mesh(_) => None,
    }
}

/// Vertices of `owner` that lie inside `other`. `outward` points out of `other`
/// toward `owner`, so the facing plane is `other`'s support along `outward`.
fn collect_face(
    owner: &Body,
    other: &Body,
    outward: Vec3,
    sat_depth: f32,
    found: &mut [(f32, Vec3); 4],
    count: &mut u8,
) {
    let mut verts = [Vec3::ZERO; MAX_HULL_VERTS];
    let n = write_vertices(owner, &mut verts);
    let band = sat_depth + 0.02;
    // The centre projected onto the facing plane. When that point lies in
    // both shapes the normal force can sit under the centre, so a body
    // whose centre is over the support does not tip off a corner.
    let support = support_of(other, outward);
    let gap = owner.position.dot(outward) - support.dot(outward);
    let projected = owner.position - outward * gap;
    if contains_point(other, projected, 0.001) && contains_point(owner, projected, 0.02) {
        let depth = point_depth(other, projected, outward);
        if (-0.002..=band).contains(&depth) {
            insert_point(found, count, depth, projected);
        }
    }
    for vertex in verts.iter().take(n) {
        if !contains_point(other, *vertex, 0.02) {
            continue;
        }
        let depth = point_depth(other, *vertex, outward);
        if !(-0.002..=band).contains(&depth) {
            continue;
        }
        insert_point(found, count, depth, *vertex);
    }
}

fn insert_point(found: &mut [(f32, Vec3); 4], count: &mut u8, depth: f32, position: Vec3) {
    if *count < 4 {
        found[usize::from(*count)] = (depth, position);
        *count += 1;
        return;
    }
    let mut low = 0_usize;
    for index in 1..4 {
        if found[index].0 < found[low].0 {
            low = index;
        }
    }
    if depth > found[low].0 {
        found[low] = (depth, position);
    }
}

fn write_vertices(body: &Body, out: &mut [Vec3; MAX_HULL_VERTS]) -> usize {
    match body.shape {
        Shape::Sphere { .. } => {
            out[0] = body.position;
            1
        }
        Shape::Capsule { half_height, .. } => {
            let (start, end) = segment(body, half_height);
            out[0] = start;
            out[1] = end;
            2
        }
        Shape::Box { half_extents } => write_box(
            body.position,
            body.orientation,
            Vec3::ZERO,
            half_extents,
            out,
        ),
        Shape::Plane { .. } => 0,
        Shape::Mesh(mesh) => {
            let mut n = 0_usize;
            for piece in mesh.pieces() {
                if n >= MAX_HULL_VERTS {
                    break;
                }
                match piece {
                    Piece::Cuboid { center, half } => {
                        let mut corners = [Vec3::ZERO; 8];
                        let written = write_box(
                            body.position,
                            body.orientation,
                            *center,
                            *half,
                            &mut corners,
                        );
                        for corner in corners.iter().take(written) {
                            if n >= MAX_HULL_VERTS {
                                break;
                            }
                            out[n] = *corner;
                            n += 1;
                        }
                    }
                    Piece::Hull(hull) => {
                        for vert in hull.verts.iter().take(usize::from(hull.vert_count)) {
                            if n >= MAX_HULL_VERTS {
                                break;
                            }
                            out[n] = body.position + body.orientation.rotate(*vert);
                            n += 1;
                        }
                    }
                }
            }
            n
        }
    }
}

fn write_box(
    position: Vec3,
    orientation: Quat,
    center: Vec3,
    half: Vec3,
    out: &mut [Vec3],
) -> usize {
    let mut n = 0_usize;
    for x in [-half.x, half.x] {
        for y in [-half.y, half.y] {
            for z in [-half.z, half.z] {
                if n >= out.len() {
                    return n;
                }
                out[n] = position + orientation.rotate(center + Vec3::new(x, y, z));
                n += 1;
            }
        }
    }
    n
}

fn contains_point(body: &Body, point: Vec3, slop: f32) -> bool {
    match body.shape {
        Shape::Plane { normal, offset } => normal.dot(point) - offset <= slop,
        Shape::Sphere { radius } => (point - body.position).length() <= radius + slop,
        Shape::Capsule {
            radius,
            half_height,
        } => {
            let (start, end) = segment(body, half_height);
            (point - closest_on_segment(point, start, end)).length() <= radius + slop
        }
        Shape::Box { half_extents } => {
            let local = conjugate(body.orientation).rotate(point - body.position);
            local.x.abs() <= half_extents.x + slop
                && local.y.abs() <= half_extents.y + slop
                && local.z.abs() <= half_extents.z + slop
        }
        Shape::Mesh(mesh) => mesh.pieces().iter().any(|piece| match piece {
            Piece::Cuboid { center, half } => {
                let origin = body.position + body.orientation.rotate(*center);
                let local = conjugate(body.orientation).rotate(point - origin);
                local.x.abs() <= half.x + slop
                    && local.y.abs() <= half.y + slop
                    && local.z.abs() <= half.z + slop
            }
            Piece::Hull(hull) => hull
                .faces
                .iter()
                .take(usize::from(hull.face_count))
                .all(|face| {
                    let normal = body.orientation.rotate(face.normal);
                    let plane = body.position + body.orientation.rotate(face.normal * face.offset);
                    normal.dot(point - plane) <= slop
                }),
        }),
    }
}

fn point_depth(body: &Body, point: Vec3, normal: Vec3) -> f32 {
    match body.shape {
        Shape::Plane {
            normal: plane_n,
            offset,
        } => {
            let axis = if plane_n.dot(normal) >= 0.0 {
                plane_n
            } else {
                -plane_n
            };
            let plane_offset = if plane_n.dot(normal) >= 0.0 {
                offset
            } else {
                -offset
            };
            plane_offset - axis.dot(point)
        }
        _ => support_of(body, normal).dot(normal) - point.dot(normal),
    }
}

fn support_of(body: &Body, dir: Vec3) -> Vec3 {
    match body.shape {
        Shape::Mesh(mesh) => {
            let mut best = body.position;
            let mut best_dot = f32::NEG_INFINITY;
            let mut found = false;
            for piece in mesh.pieces() {
                let point = piece_support(body, piece, dir);
                let score = point.dot(dir);
                if !found || score > best_dot {
                    best = point;
                    best_dot = score;
                    found = true;
                }
            }
            best
        }
        Shape::Plane { normal, offset } => normal * offset,
        _ => support_point(body, dir, None),
    }
}

#[cfg(test)]
mod edge_tests {
    use genos_math::{Quat, Vec3};

    use super::hull_hull_contact;
    use crate::{Body, Piece, Shape};

    fn cube(position: Vec3, axis: Vec3) -> Body {
        let h = 0.5;
        let v = |x: f32, y: f32, z: f32| Vec3::new(x * h, y * h, z * h);
        let (nnn, pnn, ppn, npn) = (
            v(-1., -1., -1.),
            v(1., -1., -1.),
            v(1., 1., -1.),
            v(-1., 1., -1.),
        );
        let (nnp, pnp, ppp, npp) = (
            v(-1., -1., 1.),
            v(1., -1., 1.),
            v(1., 1., 1.),
            v(-1., 1., 1.),
        );
        let triangles = [
            [pnn, ppn, ppp],
            [pnn, ppp, pnp],
            [nnn, nnp, npp],
            [nnn, npp, npn],
            [npn, npp, ppp],
            [npn, ppp, ppn],
            [nnn, pnn, pnp],
            [nnn, pnp, nnp],
            [nnp, pnp, ppp],
            [nnp, ppp, npp],
            [nnn, npn, ppn],
            [nnn, ppn, pnn],
        ];
        let mut body = Body::mesh(position, 1.0, &triangles);
        body.orientation = Quat::from_axis_angle(axis, std::f32::consts::FRAC_PI_4);
        body
    }

    fn hull(body: &Body) -> crate::Hull {
        let Shape::Mesh(mesh) = body.shape else {
            unreachable!()
        };
        let Piece::Hull(hull) = mesh.pieces[0] else {
            unreachable!()
        };
        hull
    }

    #[test]
    fn crossed_edges_give_the_edge_axis_depth_and_point() {
        let reach = 0.5 * std::f32::consts::SQRT_2;
        let a = cube(Vec3::ZERO, Vec3::Z);
        let b = cube(Vec3::new(2.0 * reach - 0.05, 0.0, 0.0), Vec3::Y);
        let hit = hull_hull_contact(&a, &hull(&a), &b, &hull(&b)).unwrap_or_else(|| unreachable!());
        assert!(
            (hit.normal - Vec3::new(-1.0, 0.0, 0.0)).length() < 1.0e-4,
            "{hit:?}"
        );
        assert!((hit.depth - 0.05).abs() < 1.0e-4, "{hit:?}");
        assert!(
            (hit.point - Vec3::new(reach - 0.025, 0.0, 0.0)).length() < 1.0e-4,
            "{hit:?}"
        );
    }

    #[test]
    fn crossed_edges_apart_have_no_hit() {
        let reach = 0.5 * std::f32::consts::SQRT_2;
        let a = cube(Vec3::ZERO, Vec3::Z);
        let b = cube(Vec3::new(2.0 * reach + 0.01, 0.0, 0.0), Vec3::Y);
        assert!(hull_hull_contact(&a, &hull(&a), &b, &hull(&b)).is_none());
    }

    #[test]
    fn aligned_cubes_keep_the_face_axis() {
        let mut a = cube(Vec3::ZERO, Vec3::Z);
        let mut b = cube(Vec3::new(0.95, 0.0, 0.0), Vec3::Z);
        a.orientation = Quat::IDENTITY;
        b.orientation = Quat::IDENTITY;
        let hit = hull_hull_contact(&a, &hull(&a), &b, &hull(&b)).unwrap_or_else(|| unreachable!());
        assert!(
            (hit.normal - Vec3::new(-1.0, 0.0, 0.0)).length() < 1.0e-6,
            "{hit:?}"
        );
        assert!((hit.depth - 0.05).abs() < 1.0e-5, "{hit:?}");
    }
}
