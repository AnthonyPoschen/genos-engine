//! One collision step for dynamic bodies.
//!
//! `step` copies the world and returns the next world. Gravity, spring forces,
//! and contact impulses run in that call. The call does not allocate.
//! Gravity is along -Y. There is no air drag.

use genos_math::{Quat, Vec3};

/// Acceleration along -Y, in meters per second squared.
pub const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

/// Bodies stored in one world. A step does not grow this set.
pub const MAX_BODIES: usize = 32;

/// A sphere, or a static plane. The plane normal points into the free half.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sphere {
        radius: f32,
    },
    /// `normal · point = offset`.
    Plane {
        normal: Vec3,
        offset: f32,
    },
}

/// A support spring. It pushes while the body is closer than `rest_length`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub anchor: Vec3,
    pub rest_length: f32,
    pub stiffness: f32,
    pub damping: f32,
}

/// A simulated mass. Position and orientation use the math types.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub position: Vec3,
    pub orientation: Quat,
    pub velocity: Vec3,
    pub inverse_mass: f32,
    pub restitution: f32,
    pub friction: f32,
    pub shape: Shape,
    pub spring: Option<Spring>,
}

impl Body {
    pub const fn sphere(position: Vec3, mass: f32, radius: f32) -> Self {
        Self {
            position,
            orientation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            inverse_mass: if mass > 0.0 { 1.0 / mass } else { 0.0 },
            restitution: 0.0,
            friction: 0.0,
            shape: Shape::Sphere { radius },
            spring: None,
        }
    }

    pub fn plane(normal: Vec3, offset: f32) -> Self {
        let normal = normal.normalize();
        let normal = if normal.length_squared() == 0.0 {
            Vec3::Y
        } else {
            normal
        };
        Self {
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            inverse_mass: 0.0,
            restitution: 0.0,
            friction: 0.0,
            shape: Shape::Plane { normal, offset },
            spring: None,
        }
    }

    pub fn mass(self) -> f32 {
        if self.inverse_mass == 0.0 {
            f32::INFINITY
        } else {
            1.0 / self.inverse_mass
        }
    }
}

const fn resting() -> Body {
    Body::sphere(Vec3::ZERO, 0.0, 0.0)
}

/// Bodies plus a gravity acceleration. Only `bodies[..count]` is live.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct World {
    pub bodies: [Body; MAX_BODIES],
    pub count: usize,
    pub gravity: Vec3,
}

impl World {
    pub const fn new(gravity: Vec3) -> Self {
        Self {
            bodies: [resting(); MAX_BODIES],
            count: 0,
            gravity,
        }
    }

    /// Store one body. Returns false when the world is already full.
    pub fn insert(&mut self, body: Body) -> bool {
        if self.count >= MAX_BODIES {
            return false;
        }
        self.bodies[self.count] = body;
        self.count += 1;
        true
    }
}

/// Advance every live body by `dt` seconds.
#[must_use]
pub fn step(world: &World, dt: f32) -> World {
    let mut next = *world;
    if !(dt > 0.0) {
        return next;
    }
    let live = next.count;
    for body in &mut next.bodies[..live] {
        apply_forces(body, next.gravity, dt);
    }
    for body in &mut next.bodies[..live] {
        if body.inverse_mass == 0.0 {
            continue;
        }
        body.position += body.velocity * dt;
    }
    collide(&mut next.bodies[..live]);
    next
}

fn apply_forces(body: &mut Body, gravity: Vec3, dt: f32) {
    if body.inverse_mass == 0.0 {
        return;
    }
    let mut acceleration = gravity;
    if let Some(spring) = body.spring {
        let delta = body.position - spring.anchor;
        let dist_sq = delta.length_squared();
        let (dir, dist) = if dist_sq > 1.0e-12 {
            let dist = dist_sq.sqrt();
            (delta * (1.0 / dist), dist)
        } else {
            (Vec3::Y, 0.0)
        };
        let compression = spring.rest_length - dist;
        if compression > 0.0 {
            let speed_along = dir.dot(body.velocity);
            let magnitude = spring.stiffness * compression - spring.damping * speed_along;
            acceleration += dir * (magnitude * body.inverse_mass);
        }
    }
    body.velocity += acceleration * dt;
}

fn collide(bodies: &mut [Body]) {
    for i in 0..bodies.len() {
        for j in (i + 1)..bodies.len() {
            let (left, right) = bodies.split_at_mut(j);
            collide_pair(&mut left[i], &mut right[0]);
        }
    }
}

fn collide_pair(a: &mut Body, b: &mut Body) {
    match (a.shape, b.shape) {
        (Shape::Sphere { radius: ra }, Shape::Sphere { radius: rb }) => solve_spheres(a, b, ra, rb),
        (Shape::Sphere { radius }, Shape::Plane { normal, offset }) => {
            solve_sphere_plane(a, b, radius, normal, offset);
        }
        (Shape::Plane { normal, offset }, Shape::Sphere { radius }) => {
            solve_sphere_plane(b, a, radius, normal, offset);
        }
        (Shape::Plane { .. }, Shape::Plane { .. }) => {}
    }
}

fn solve_spheres(a: &mut Body, b: &mut Body, ra: f32, rb: f32) {
    let delta = a.position - b.position;
    let radius = ra + rb;
    let dist_sq = delta.length_squared();
    if dist_sq > radius * radius {
        return;
    }
    let dist = dist_sq.sqrt();
    let normal = if dist > 1.0e-8 {
        delta * (1.0 / dist)
    } else {
        Vec3::Y
    };
    solve_contact(a, b, normal, radius - dist);
}

fn solve_sphere_plane(sphere: &mut Body, plane: &mut Body, radius: f32, normal: Vec3, offset: f32) {
    let dist = normal.dot(sphere.position) - offset;
    if dist > radius {
        return;
    }
    solve_contact(sphere, plane, normal, radius - dist);
}

/// Contact restitution is the larger of the two bodies. Friction is the larger too.
fn solve_contact(a: &mut Body, b: &mut Body, normal: Vec3, penetration: f32) {
    let inv = a.inverse_mass + b.inverse_mass;
    if inv <= 0.0 {
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
                let tangent = tangent_vec * (1.0 / vt);
                let jt = (-vt / inv).clamp(-friction * j, friction * j);
                let kick = tangent * jt;
                a.velocity += kick * a.inverse_mass;
                b.velocity -= kick * b.inverse_mass;
            }
        }
    }
    const SLOP: f32 = 0.001;
    const PERCENT: f32 = 0.8;
    let depth = penetration - SLOP;
    if depth > 0.0 {
        let correction = normal * (depth * PERCENT / inv);
        a.position += correction * a.inverse_mass;
        b.position -= correction * b.inverse_mass;
    }
}
