//! Speculative contacts and swept time of impact.
//!
//! The gap is a GJK distance for two convex shapes. A positive gap is
//! separation. A negative gap is penetration from the discrete contact.
//!
//! A swept pair advances by `gap / (|v_a - v_b| + |ω_a| r_a + |ω_b| r_b)`.
//! Angular velocity is not on `Body` yet, so both ω terms are 0. The advance
//! has no scale factor. A pair whose relative travel over the tick exceeds the
//! smaller bounding radius is sliced so each advance travels at most that
//! radius. The loop stops at a 1 mm gap, when the tick is consumed, or after
//! 16 advances. Hitting the cap with gap still above 1 mm clamps the pair and
//! counts `toi_failed`. This is not closest-point velocity, not a 0.8 factor,
//! and not a `1e-4` s minimum step.

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

fn solve_indexed(
    bodies: &mut [Body],
    index: usize,
    other: usize,
    normal: Vec3,
    penetration: f32,
) -> bool {
    if index < other {
        let (left, right) = bodies.split_at_mut(other);
        let Some(first) = left.get_mut(index) else {
            return false;
        };
        let Some(second) = right.first_mut() else {
            return false;
        };
        return solve_contact(first, second, normal, penetration);
    }
    let (left, right) = bodies.split_at_mut(index);
    let Some(first) = right.first_mut() else {
        return false;
    };
    let Some(second) = left.get_mut(other) else {
        return false;
    };
    solve_contact(first, second, normal, penetration)
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
    // Touching without a discrete overlap. The center delta still points from
    // the second body toward the first, which is the separating direction.
    let delta = first.position - second.position;
    let distance = delta.length();
    if distance > DISTANCE_EPSILON {
        return (delta / distance, 0.0);
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

const GAP_TOLERANCE: f32 = 0.001;
const MAX_ADVANCES: u32 = 16;
const TIME_EPSILON: f32 = 1.0e-8;

/// Pair list and coverage flags reused across steps so a discrete step allocates nothing.
#[derive(Clone, Debug)]
pub(crate) struct CcdScratch {
    pairs: Vec<(usize, usize)>,
    covered: Vec<u8>,
}

impl CcdScratch {
    pub(crate) const fn new() -> Self {
        Self {
            pairs: Vec::new(),
            covered: Vec::new(),
        }
    }

    pub(crate) fn covers(&self, index: usize) -> bool {
        self.covered.get(index).copied() == Some(1)
    }
}

/// Collect swept pairs whose expanded bounds meet. Pairs past `cap` increment `toi_deferred`.
pub(crate) fn prepare_swept(
    bodies: &[Body],
    scratch: &mut CcdScratch,
    cap: u32,
    dt: f32,
    stats: &mut crate::StepStats,
) {
    scratch.pairs.clear();
    let any_swept = bodies
        .iter()
        .any(|body| body.motion == MotionQuality::Swept);
    if !any_swept {
        if !scratch.covered.is_empty() {
            scratch.covered.fill(0);
        }
        return;
    }
    if scratch.covered.len() < bodies.len() {
        scratch.covered.resize(bodies.len(), 0);
    }
    scratch.covered.fill(0);
    collect_pairs(bodies, scratch, dt);
    let cap_len = usize::try_from(cap).unwrap_or(usize::MAX);
    if scratch.pairs.len() > cap_len {
        let dropped = scratch.pairs.len() - cap_len;
        scratch.pairs.truncate(cap_len);
        stats.toi_deferred = u32::try_from(dropped).unwrap_or(u32::MAX);
    }
    for &(first, second) in &scratch.pairs {
        if let Some(flag) = scratch.covered.get_mut(first) {
            *flag = 1;
        }
        if let Some(flag) = scratch.covered.get_mut(second) {
            *flag = 1;
        }
    }
}

fn collect_pairs(bodies: &[Body], scratch: &mut CcdScratch, dt: f32) {
    let count = bodies.len();
    let mut first = 0;
    while first < count {
        let mut second = first + 1;
        while second < count {
            if swept_pair(bodies, first, second, dt) {
                scratch.pairs.push((first, second));
            }
            second += 1;
        }
        first += 1;
    }
}

fn swept_pair(bodies: &[Body], first: usize, second: usize, dt: f32) -> bool {
    let (Some(a), Some(b)) = (bodies.get(first), bodies.get(second)) else {
        return false;
    };
    let swept = a.motion == MotionQuality::Swept || b.motion == MotionQuality::Swept;
    let dynamic = a.inverse_mass != 0.0 || b.inverse_mass != 0.0;
    swept && dynamic && bounds_meet(a, b, dt)
}

fn bounds_meet(a: &Body, b: &Body, dt: f32) -> bool {
    match (expanded_bounds(a, dt), expanded_bounds(b, dt)) {
        (None, _) | (_, None) => true,
        (Some((lo, hi)), Some((lo_b, hi_b))) => {
            lo_b.x <= hi.x
                && lo.x <= hi_b.x
                && lo_b.y <= hi.y
                && lo.y <= hi_b.y
                && lo_b.z <= hi.z
                && lo.z <= hi_b.z
        }
    }
}

fn expanded_bounds(body: &Body, dt: f32) -> Option<(Vec3, Vec3)> {
    let (lo, hi) = contact::bounds(body)?;
    let travel = body.velocity * dt;
    Some((
        Vec3::new(
            lo.x.min(lo.x + travel.x),
            lo.y.min(lo.y + travel.y),
            lo.z.min(lo.z + travel.z),
        ),
        Vec3::new(
            hi.x.max(hi.x + travel.x),
            hi.y.max(hi.y + travel.y),
            hi.z.max(hi.z + travel.z),
        ),
    ))
}

/// Advance accepted swept pairs. A swept body left out of those pairs uses the speculative margin.
pub(crate) fn resolve_swept(
    bodies: &mut [Body],
    scratch: &CcdScratch,
    dt: f32,
    margin: f32,
    stats: &mut crate::StepStats,
) {
    for slot in 0..scratch.pairs.len() {
        let Some(&(first, second)) = scratch.pairs.get(slot) else {
            continue;
        };
        advance_pair(bodies, first, second, dt, stats);
    }
    let count = bodies.len();
    let mut index = 0;
    while index < count {
        let deferred = bodies
            .get(index)
            .is_some_and(|body| body.motion == MotionQuality::Swept && !scratch.covers(index));
        if deferred {
            integrate_deferred(bodies, index, count, dt, margin);
        }
        index += 1;
    }
}

fn integrate_deferred(bodies: &mut [Body], index: usize, count: usize, dt: f32, margin: f32) {
    if let Some(body) = bodies.get_mut(index) {
        body.motion = MotionQuality::Speculative;
    }
    let mut other = 0;
    while other < count {
        if other != index {
            speculative_pair(bodies, index, other, dt, margin);
        }
        other += 1;
    }
    let Some(body) = bodies.get_mut(index) else {
        return;
    };
    body.motion = MotionQuality::Swept;
    if body.inverse_mass != 0.0 {
        body.position += body.velocity * dt;
    }
}

fn advance_pair(
    bodies: &mut [Body],
    first: usize,
    second: usize,
    dt: f32,
    stats: &mut crate::StepStats,
) {
    let Some(a) = bodies.get(first).copied() else {
        return;
    };
    let Some(b) = bodies.get(second).copied() else {
        return;
    };
    if a.inverse_mass == 0.0 && b.inverse_mass == 0.0 {
        return;
    }
    let fast = fast_pair(&a, &b, dt);
    let mut remaining = dt;
    let mut advances = 0_u32;
    loop {
        if remaining <= TIME_EPSILON {
            return;
        }
        let Some((normal, gap)) = gap_of(bodies, first, second) else {
            integrate_index(bodies, first, remaining);
            integrate_index(bodies, second, remaining);
            return;
        };
        if gap <= GAP_TOLERANCE {
            settle(bodies, first, second, normal, gap, remaining, stats);
            return;
        }
        if advances >= MAX_ADVANCES {
            stats.toi_failed = stats.toi_failed.saturating_add(1);
            return;
        }
        let Some(step_dt) = next_advance(bodies, first, second, gap, remaining) else {
            stats.toi_failed = stats.toi_failed.saturating_add(1);
            return;
        };
        integrate_index(bodies, first, step_dt);
        integrate_index(bodies, second, step_dt);
        remaining -= step_dt;
        advances = advances.saturating_add(1);
        if fast {
            stats.pair_slices = stats.pair_slices.saturating_add(1);
        }
    }
}

fn fast_pair(a: &Body, b: &Body, dt: f32) -> bool {
    let radius = bounding_radius(a).min(bounding_radius(b));
    let travel = (a.velocity - b.velocity).length() * dt;
    radius.is_finite() && travel > radius
}

fn gap_of(bodies: &[Body], first: usize, second: usize) -> Option<(Vec3, f32)> {
    let a = bodies.get(first)?;
    let b = bodies.get(second)?;
    signed_gap(a, b)
}

fn next_advance(
    bodies: &[Body],
    first: usize,
    second: usize,
    gap: f32,
    remaining: f32,
) -> Option<f32> {
    let a = bodies.get(first)?;
    let b = bodies.get(second)?;
    let bound = closing_bound(a, b);
    if bound <= 0.0 || !bound.is_finite() {
        return Some(remaining);
    }
    let mut advance = (gap / bound).min(remaining);
    let slice = radius_slice(a, b);
    if slice < advance {
        advance = slice;
    }
    if advance > TIME_EPSILON {
        Some(advance)
    } else {
        None
    }
}

fn settle(
    bodies: &mut [Body],
    first: usize,
    second: usize,
    normal: Vec3,
    gap: f32,
    remaining: f32,
    stats: &mut crate::StepStats,
) {
    let penetration = if gap < 0.0 { -gap } else { 0.0 };
    if solve_indexed(bodies, first, second, normal, penetration) {
        stats.dynamic_pair_impulses = stats.dynamic_pair_impulses.saturating_add(1);
    }
    integrate_index(bodies, first, remaining);
    integrate_index(bodies, second, remaining);
}

/// `|v_a - v_b| + |ω_a| r_a + |ω_b| r_b`. Both ω are 0 until `Body` stores angular velocity.
fn closing_bound(a: &Body, b: &Body) -> f32 {
    (a.velocity - b.velocity).length()
}

fn radius_slice(a: &Body, b: &Body) -> f32 {
    let radius = bounding_radius(a).min(bounding_radius(b));
    let rel = (a.velocity - b.velocity).length();
    if rel > 0.0 && radius.is_finite() {
        radius / rel
    } else {
        f32::INFINITY
    }
}

fn bounding_radius(body: &Body) -> f32 {
    match body.shape {
        Shape::Sphere { radius } => radius.max(0.0),
        Shape::Capsule {
            radius,
            half_height,
        } => (radius + half_height).max(0.0),
        Shape::Box { half_extents } => half_extents.length(),
        Shape::Plane { .. } => f32::INFINITY,
        Shape::Mesh(mesh) => mesh_radius(&mesh),
    }
}

fn mesh_radius(mesh: &Mesh) -> f32 {
    let mut radius = 0.0_f32;
    for piece in mesh.pieces() {
        let piece_radius = match piece {
            Piece::Cuboid { center, half } => center.length() + half.length(),
            Piece::Hull(hull) => hull_radius(hull),
        };
        radius = radius.max(piece_radius);
    }
    radius
}

fn hull_radius(hull: &crate::Hull) -> f32 {
    let mut far = 0.0_f32;
    for vertex in hull.verts.iter().take(usize::from(hull.vert_count)) {
        far = far.max(vertex.length());
    }
    far
}

fn integrate_index(bodies: &mut [Body], index: usize, dt: f32) {
    let Some(body) = bodies.get_mut(index) else {
        return;
    };
    if body.inverse_mass == 0.0 || !(dt > 0.0) {
        return;
    }
    body.position += body.velocity * dt;
}

#[cfg(test)]
mod tests {
    use genos_math::Vec3;

    use super::signed_gap;
    use crate::{step, Body, MotionQuality, World};

    const DT: f32 = 1.0 / 60.0;

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

    fn ship(x: f32, speed: f32) -> Body {
        let mut body = Body::cuboid(Vec3::new(x, 2.0, 0.0), 1_000.0, Vec3::new(2.0, 1.0, 1.0));
        body.velocity = Vec3::new(speed, 0.0, 0.0);
        body.motion = MotionQuality::Swept;
        body
    }

    fn overlap(left: &Body, right: &Body) -> f32 {
        let gap = (left.position.x - right.position.x).abs() - 4.0;
        if gap < 0.0 {
            -gap
        } else {
            0.0
        }
    }

    #[test]
    fn swept_ships_at_the_harness_gap_meet_without_exchanging() {
        let mut world = World::new(Vec3::ZERO);
        world.insert(ship(0.0, 3_000.0));
        world.insert(ship(6.0, -3_000.0));
        step(&mut world, DT);
        let left = world.bodies[0];
        let right = world.bodies[1];
        assert!(left.position.x < right.position.x, "centers exchanged");
        let depth = overlap(&left, &right);
        assert!(depth <= 0.01, "penetration {depth}");
        assert_eq!(world.step_stats.toi_failed, 0);
        assert!(left.normal_impulse > 0.0 && right.normal_impulse > 0.0);
        assert!(world.step_stats.pair_slices >= 1);
    }

    #[test]
    fn swept_ships_forty_metres_apart_need_more_than_one_slice() {
        let mut world = World::new(Vec3::ZERO);
        world.insert(ship(0.0, 3_000.0));
        world.insert(ship(40.0, -3_000.0));
        step(&mut world, DT);
        let left = world.bodies[0];
        let right = world.bodies[1];
        assert!(left.position.x < right.position.x, "centers exchanged");
        let depth = overlap(&left, &right);
        assert!(depth <= 0.01, "penetration {depth}");
        assert_eq!(world.step_stats.toi_failed, 0);
        assert!(world.step_stats.pair_slices > 1);
        assert!(left.normal_impulse > 0.0 && right.normal_impulse > 0.0);
    }

    #[test]
    fn a_zero_pair_cap_defers_the_sweep() {
        let mut world = World::new(Vec3::ZERO);
        world.swept_pair_cap = 0;
        world.insert(ship(0.0, 3_000.0));
        world.insert(ship(6.0, -3_000.0));
        step(&mut world, DT);
        assert!(world.step_stats.toi_deferred >= 1);
        assert!(
            world.bodies[0].position.x > world.bodies[1].position.x,
            "deferred ships should still tunnel"
        );
    }

    #[test]
    fn sixteen_advances_clamp_without_tunneling() {
        let mut left = Body::sphere(Vec3::ZERO, 1.0, 0.01);
        left.velocity = Vec3::new(3_000.0, 0.0, 0.0);
        left.motion = MotionQuality::Swept;
        let mut right = Body::sphere(Vec3::new(5.0, 0.0, 0.0), 1.0, 0.01);
        right.velocity = Vec3::new(-3_000.0, 0.0, 0.0);
        right.motion = MotionQuality::Swept;
        let mut world = World::new(Vec3::ZERO);
        world.insert(left);
        world.insert(right);
        step(&mut world, DT);
        assert_eq!(world.step_stats.toi_failed, 1);
        let trailing = world.bodies[0].position.x + 0.01;
        let leading = world.bodies[1].position.x - 0.01;
        assert!(
            trailing < leading,
            "clamped spheres overlapped: {trailing} vs {leading}"
        );
    }
}
