//! Sequential impulses at a contact point (ADR 0034).
//!
//! Each point stores a world position. The normal row uses
//! `v + ω × r` and the inverse inertia at that point. Friction is two
//! tangent rows clamped to a Coulomb box. Position correction is a separate
//! displacement, so it does not add kinetic energy.

use genos_math::{Quat, Vec3};

use crate::{inertia_diagonal, Body, Presence};

const SLOP: f32 = 0.001;
const PERCENT: f32 = 0.8;
const RESTITUTION_SPEED: f32 = 1.0;
const WARM_DISTANCE_SQ: f32 = 0.05 * 0.05;
const VELOCITY_ITERATIONS: u32 = 8;
const POSITION_ITERATIONS: u32 = 1;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ContactPoint {
    pub position: Vec3,
    pub depth: f32,
    pub normal_impulse: f32,
    pub tangent: [f32; 2],
    /// Target separating speed from restitution, captured before iterations.
    pub bias: f32,
}

impl ContactPoint {
    pub(crate) const fn new(position: Vec3, depth: f32) -> Self {
        Self {
            position,
            depth,
            normal_impulse: 0.0,
            tangent: [0.0, 0.0],
            bias: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Manifold {
    pub a: u32,
    pub b: u32,
    pub normal: Vec3,
    /// Separating-axis overlap. Position correction uses this, not a vertex
    /// depth, so a far vertex cannot throw the body.
    pub depth: f32,
    pub points: [ContactPoint; 4],
    pub count: u8,
    /// Friction axes from the first frame of this contact, kept while the
    /// normal stays aligned so a warm impulse does not swap tangent.
    pub tangent_axis: [Vec3; 2],
}

/// Velocity iterations, then a positional pass. Returns how many manifolds
/// joined two dynamic bodies with a normal impulse.
pub(crate) fn solve(
    bodies: &mut [Body],
    manifolds: &mut [Manifold],
    warm: &[Manifold],
    wake: &mut Vec<bool>,
    velocity: bool,
) -> u32 {
    wake_touched(bodies, manifolds, wake);
    prepare_bias(bodies, manifolds);
    if velocity {
        warm_start(bodies, manifolds, warm);
        let iterations = VELOCITY_ITERATIONS;
        for _ in 0..iterations {
            for manifold in manifolds.iter_mut() {
                solve_velocity(bodies, manifold);
            }
        }
    }
    for _ in 0..POSITION_ITERATIONS {
        for manifold in manifolds.iter_mut() {
            solve_position(bodies, manifold);
        }
    }
    let mut hits = 0_u32;
    for manifold in manifolds.iter() {
        let Some(a) = bodies.get(usize_from(manifold.a)) else {
            continue;
        };
        let Some(b) = bodies.get(usize_from(manifold.b)) else {
            continue;
        };
        let dynamic = a.inverse_mass > 0.0 && b.inverse_mass > 0.0;
        let pushed = manifold.points[..usize_from_u8(manifold.count)]
            .iter()
            .any(|point| point.normal_impulse > 0.0);
        if dynamic && pushed {
            hits = hits.saturating_add(1);
        }
    }
    hits
}

/// One CCD contact. Same rows as [`solve`], one velocity pass and one
/// positional pass, no warm start.
pub(crate) fn solve_pair(
    a: &mut Body,
    b: &mut Body,
    normal: Vec3,
    points: &mut [ContactPoint],
) -> bool {
    if points.is_empty() {
        return false;
    }
    let normal = unit(normal);
    let restitution = a.restitution.max(b.restitution).clamp(0.0, 1.0);
    for point in points.iter_mut() {
        let ra = point.position - a.position;
        let rb = point.position - b.position;
        let vn = (point_velocity(a, ra) - point_velocity(b, rb)).dot(normal);
        point.bias = if vn < -RESTITUTION_SPEED {
            -restitution * vn
        } else {
            0.0
        };
    }
    let (t1, t2) = tangents(normal);
    for _ in 0..32 {
        for point in points.iter_mut() {
            velocity_point(a, b, normal, t1, t2, true, point);
        }
    }
    let mut depth = points.iter().map(|point| point.depth).fold(0.0, f32::max);
    for _ in 0..POSITION_ITERATIONS {
        let extra = depth - SLOP;
        if extra <= 0.0 {
            break;
        }
        let inv_a = inv_mass(a);
        let inv_b = inv_mass(b);
        let inv = inv_a + inv_b;
        if inv <= 1.0e-8 {
            break;
        }
        let lambda = extra * PERCENT / inv;
        let shift_a = normal * (lambda * inv_a);
        let shift_b = normal * (lambda * inv_b);
        a.position += shift_a;
        b.position -= shift_b;
        depth -= (shift_a + shift_b).dot(normal);
    }
    let dynamic = a.inverse_mass > 0.0 && b.inverse_mass > 0.0;
    dynamic && points.iter().any(|point| point.normal_impulse > 0.0)
}

fn wake_touched(bodies: &mut [Body], manifolds: &[Manifold], wake: &mut Vec<bool>) {
    wake.clear();
    wake.resize(bodies.len(), false);
    for manifold in manifolds {
        let ia = usize_from(manifold.a);
        let ib = usize_from(manifold.b);
        let (Some(a), Some(b)) = (bodies.get(ia), bodies.get(ib)) else {
            continue;
        };
        if a.sleeping && moving(b) {
            wake[ia] = true;
        }
        if b.sleeping && moving(a) {
            wake[ib] = true;
        }
    }
    for (body, flagged) in bodies.iter_mut().zip(wake.iter()) {
        if *flagged {
            body.sleeping = false;
            body.sleep_time = 0.0;
        }
    }
}

fn moving(body: &Body) -> bool {
    !body.sleeping
        && body.inverse_mass > 0.0
        && (body.velocity.length() > 0.25 || body.angular_velocity.length() > 0.4)
}

fn prepare_bias(bodies: &[Body], manifolds: &mut [Manifold]) {
    for manifold in manifolds.iter_mut() {
        let (Some(a), Some(b)) = (
            bodies.get(usize_from(manifold.a)),
            bodies.get(usize_from(manifold.b)),
        ) else {
            continue;
        };
        let restitution = if a.inverse_mass > 0.0 || b.inverse_mass > 0.0 {
            a.restitution.max(b.restitution).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let count = usize_from_u8(manifold.count);
        for point in &mut manifold.points[..count] {
            let ra = point.position - a.position;
            let rb = point.position - b.position;
            let vn = (point_velocity(a, ra) - point_velocity(b, rb)).dot(manifold.normal);
            point.bias = if vn < -RESTITUTION_SPEED {
                -restitution * vn
            } else {
                0.0
            };
        }
    }
}

fn warm_start(bodies: &mut [Body], manifolds: &mut [Manifold], warm: &[Manifold]) {
    for manifold in manifolds.iter_mut() {
        let Some(previous) = warm
            .iter()
            .find(|old| old.a == manifold.a && old.b == manifold.b)
        else {
            continue;
        };
        if previous.normal.dot(manifold.normal) > 0.9 {
            manifold.tangent_axis = previous.tangent_axis;
        }
        let count = usize_from_u8(manifold.count);
        let prev_count = usize_from_u8(previous.count);
        for point in &mut manifold.points[..count] {
            let Some(cached) = previous.points[..prev_count].iter().find(|cached| {
                (cached.position - point.position).length_squared() < WARM_DISTANCE_SQ
            }) else {
                continue;
            };
            point.normal_impulse = cached.normal_impulse;
            point.tangent = cached.tangent;
            let Some((a, b)) = pair_mut(bodies, manifold.a, manifold.b) else {
                continue;
            };
            let ra = point.position - a.position;
            let rb = point.position - b.position;
            apply(a, b, ra, rb, manifold.normal * point.normal_impulse, true);
            let (t1, t2) = (manifold.tangent_axis[0], manifold.tangent_axis[1]);
            apply(a, b, ra, rb, t1 * point.tangent[0], true);
            apply(a, b, ra, rb, t2 * point.tangent[1], true);
        }
    }
}

fn solve_velocity(bodies: &mut [Body], manifold: &mut Manifold) {
    let count = usize_from_u8(manifold.count);
    let (t1, t2) = (manifold.tangent_axis[0], manifold.tangent_axis[1]);
    for index in 0..count {
        let position = manifold.points[index].position;
        let depth = manifold.points[index].depth;
        let normal_impulse = manifold.points[index].normal_impulse;
        let tangent = manifold.points[index].tangent;
        let Some((a, b)) = pair_mut(bodies, manifold.a, manifold.b) else {
            return;
        };
        if both_inactive(a, b) {
            return;
        }
        let mut point = ContactPoint {
            position,
            depth,
            normal_impulse,
            tangent,
            bias: manifold.points[index].bias,
        };
        velocity_point(a, b, manifold.normal, t1, t2, true, &mut point);
        manifold.points[index].normal_impulse = point.normal_impulse;
        manifold.points[index].tangent = point.tangent;
    }
}

fn solve_position(bodies: &mut [Body], manifold: &mut Manifold) {
    let Some((a, b)) = pair_mut(bodies, manifold.a, manifold.b) else {
        return;
    };
    if both_inactive(a, b) {
        return;
    }
    let extra = manifold.depth - SLOP;
    if extra <= 0.0 {
        return;
    }
    let inv_a = inv_mass(a);
    let inv_b = inv_mass(b);
    let inv = inv_a + inv_b;
    if inv <= 1.0e-8 {
        return;
    }
    let lambda = extra * PERCENT / inv;
    let shift_a = manifold.normal * (lambda * inv_a);
    let shift_b = manifold.normal * (lambda * inv_b);
    a.position += shift_a;
    b.position -= shift_b;
    // `b` moves by `-shift_b`, so the gap along the normal grows by both shifts.
    manifold.depth -= (shift_a + shift_b).dot(manifold.normal);
}

fn velocity_point(
    a: &mut Body,
    b: &mut Body,
    normal: Vec3,
    t1: Vec3,
    t2: Vec3,
    normal_torque: bool,
    point: &mut ContactPoint,
) {
    let ra = point.position - a.position;
    let rb = point.position - b.position;
    let normal_impulse = normal_delta(a, b, normal, ra, rb, point);
    apply(a, b, ra, rb, normal_impulse, normal_torque);
    if normal_impulse != Vec3::ZERO {
        let magnitude = normal_impulse.length();
        a.normal_impulse += magnitude;
        b.normal_impulse += magnitude;
    }
    let friction = friction_delta(a, b, t1, t2, ra, rb, point);
    apply(a, b, ra, rb, friction, true);
}

fn normal_delta(
    a: &Body,
    b: &Body,
    normal: Vec3,
    ra: Vec3,
    rb: Vec3,
    point: &mut ContactPoint,
) -> Vec3 {
    let rel = point_velocity(a, ra) - point_velocity(b, rb);
    let vn = rel.dot(normal);
    let k = mass_along(a, b, ra, rb, normal);
    if k <= 1.0e-8 {
        return Vec3::ZERO;
    }
    let j = -(vn - point.bias) / k;
    let old = point.normal_impulse;
    point.normal_impulse = (old + j).max(0.0);
    normal * (point.normal_impulse - old)
}

fn friction_delta(
    a: &Body,
    b: &Body,
    t1: Vec3,
    t2: Vec3,
    ra: Vec3,
    rb: Vec3,
    point: &mut ContactPoint,
) -> Vec3 {
    let friction = a.friction.max(b.friction).max(0.0);
    let limit = friction * point.normal_impulse;
    let mut impulse = Vec3::ZERO;
    for slot in 0..2 {
        let axis = if slot == 0 { t1 } else { t2 };
        let rel = point_velocity(a, ra) - point_velocity(b, rb);
        let vt = rel.dot(axis);
        let kt = mass_along(a, b, ra, rb, axis);
        if kt <= 1.0e-8 {
            continue;
        }
        let jt = -vt / kt;
        let old = point.tangent[slot];
        point.tangent[slot] = (old + jt).clamp(-limit, limit);
        impulse += axis * (point.tangent[slot] - old);
    }
    impulse
}

fn apply(a: &mut Body, b: &mut Body, ra: Vec3, rb: Vec3, impulse: Vec3, torque: bool) {
    a.velocity += impulse * inv_mass(a);
    b.velocity -= impulse * inv_mass(b);
    if torque {
        a.angular_velocity += inv_inertia(a, ra.cross(impulse));
        b.angular_velocity -= inv_inertia(b, rb.cross(impulse));
    }
}

fn point_velocity(body: &Body, r: Vec3) -> Vec3 {
    if inv_mass(body) == 0.0 {
        Vec3::ZERO
    } else {
        body.velocity + body.angular_velocity.cross(r)
    }
}

fn mass_along(a: &Body, b: &Body, ra: Vec3, rb: Vec3, dir: Vec3) -> f32 {
    let mut mass = inv_mass(a) + inv_mass(b);
    let crossed_a = ra.cross(dir);
    if crossed_a.length_squared() > 1.0e-12 {
        mass += crossed_a.dot(inv_inertia(a, crossed_a));
    }
    let crossed_b = rb.cross(dir);
    if crossed_b.length_squared() > 1.0e-12 {
        mass += crossed_b.dot(inv_inertia(b, crossed_b));
    }
    mass
}

fn inv_mass(body: &Body) -> f32 {
    if body.sleeping || body.presence != Presence::Live {
        0.0
    } else {
        body.inverse_mass
    }
}

fn inv_inertia(body: &Body, vector: Vec3) -> Vec3 {
    if inv_mass(body) == 0.0 {
        return Vec3::ZERO;
    }
    let local = conjugate(body.orientation).rotate(vector);
    let scaled = if let crate::Shape::Mesh(mesh) = body.shape {
        mesh.unit_inverse_inertia.transform(local) * body.inverse_mass
    } else {
        let inertia = inertia_diagonal(*body);
        Vec3::new(
            recip(inertia.x) * local.x,
            recip(inertia.y) * local.y,
            recip(inertia.z) * local.z,
        )
    };
    body.orientation.rotate(scaled)
}

fn both_inactive(a: &Body, b: &Body) -> bool {
    (a.sleeping || a.inverse_mass == 0.0) && (b.sleeping || b.inverse_mass == 0.0)
}

pub(crate) fn tangent_axes(normal: Vec3) -> (Vec3, Vec3) {
    tangents(normal)
}

fn tangents(normal: Vec3) -> (Vec3, Vec3) {
    let helper = if normal.y.abs() < 0.707 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let t1 = unit(normal.cross(helper));
    (t1, normal.cross(t1))
}

fn recip(value: f32) -> f32 {
    if value > 1.0e-8 {
        1.0 / value
    } else {
        0.0
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

pub(crate) fn integrate_orientation(orientation: Quat, spin: Vec3, dt: f32) -> Quat {
    if spin.length_squared() <= 0.0 || !(dt > 0.0) {
        return orientation;
    }
    let omega = Quat::new(spin.x, spin.y, spin.z, 0.0);
    let delta = quat_mul(omega, orientation) * (0.5 * dt);
    (orientation + delta).normalize()
}

fn quat_mul(lhs: Quat, rhs: Quat) -> Quat {
    Quat::new(
        lhs.w * rhs.x + lhs.x * rhs.w + lhs.y * rhs.z - lhs.z * rhs.y,
        lhs.w * rhs.y - lhs.x * rhs.z + lhs.y * rhs.w + lhs.z * rhs.x,
        lhs.w * rhs.z + lhs.x * rhs.y - lhs.y * rhs.x + lhs.z * rhs.w,
        lhs.w * rhs.w - lhs.x * rhs.x - lhs.y * rhs.y - lhs.z * rhs.z,
    )
}

fn conjugate(q: Quat) -> Quat {
    Quat::new(-q.x, -q.y, -q.z, q.w)
}

fn pair_mut(bodies: &mut [Body], a: u32, b: u32) -> Option<(&mut Body, &mut Body)> {
    let ia = usize_from(a);
    let ib = usize_from(b);
    if ia == ib || ia >= bodies.len() || ib >= bodies.len() {
        return None;
    }
    if ia < ib {
        let (left, right) = bodies.split_at_mut(ib);
        Some((&mut left[ia], &mut right[0]))
    } else {
        let (left, right) = bodies.split_at_mut(ia);
        Some((&mut right[0], &mut left[ib]))
    }
}

fn usize_from(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
}

fn usize_from_u8(value: u8) -> usize {
    usize::from(value)
}

/// Implicit Euler on `I ω̇ + ω × Iω = 0` in the body frame (Catto 2015).
/// One Newton loop. A free spin does not gain rotational energy.
pub(crate) fn implicit_gyroscopic(w0: Vec3, inertia: Vec3, dt: f32) -> Vec3 {
    if w0.length_squared() <= 1.0e-12 || !(dt > 0.0) {
        return w0;
    }
    let mut w = w0;
    for _ in 0..4 {
        let f = gyro_residual(w, w0, inertia, dt);
        if f.length_squared() < 1.0e-18 {
            break;
        }
        let jacobian = gyro_jacobian(w, inertia, dt);
        let Some(dw) = solve3(jacobian, Vec3::new(-f.x, -f.y, -f.z)) else {
            return w0;
        };
        if !dw.x.is_finite() || !dw.y.is_finite() || !dw.z.is_finite() {
            return w0;
        }
        w += dw;
    }
    if w.x.is_finite() && w.y.is_finite() && w.z.is_finite() {
        w
    } else {
        w0
    }
}

fn gyro_residual(w: Vec3, w0: Vec3, inertia: Vec3, dt: f32) -> Vec3 {
    let iw = mul_diag(inertia, w);
    mul_diag(inertia, w - w0) * (1.0 / dt) - w.cross(iw)
}

fn gyro_jacobian(w: Vec3, inertia: Vec3, dt: f32) -> [f32; 9] {
    let (x, y, z) = (w.x, w.y, w.z);
    let (ix, iy, iz) = (inertia.x, inertia.y, inertia.z);
    // Column-major Jacobian of I*(w−w0)/dt − (w × Iw).
    let scale = |axis: f32| if dt > 0.0 { axis / dt } else { 0.0 };
    [
        scale(ix),
        -z * (ix - iz),
        -y * (iy - ix),
        -z * (iz - iy),
        scale(iy),
        -x * (iy - ix),
        -y * (iz - iy),
        -x * (ix - iz),
        scale(iz),
    ]
}

fn mul_diag(inertia: Vec3, w: Vec3) -> Vec3 {
    Vec3::new(inertia.x * w.x, inertia.y * w.y, inertia.z * w.z)
}

/// Column-major 3×3. Returns `None` when the matrix is singular.
#[expect(
    clippy::needless_range_loop,
    reason = "Gaussian elimination indexes the augmented row and column together"
)]
fn solve3(m: [f32; 9], b: Vec3) -> Option<Vec3> {
    let mut a = [
        [m[0], m[3], m[6], b.x],
        [m[1], m[4], m[7], b.y],
        [m[2], m[5], m[8], b.z],
    ];
    for col in 0..3 {
        let mut pivot = col;
        let mut best = a[col][col].abs();
        for row in (col + 1)..3 {
            let value = a[row][col].abs();
            if value > best {
                best = value;
                pivot = row;
            }
        }
        if best < 1.0e-12 {
            return None;
        }
        if pivot != col {
            a.swap(col, pivot);
        }
        let div = a[col][col];
        for k in col..4 {
            a[col][k] /= div;
        }
        for row in 0..3 {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            for k in col..4 {
                a[row][k] -= factor * a[col][k];
            }
        }
    }
    Some(Vec3::new(a[0][3], a[1][3], a[2][3]))
}
