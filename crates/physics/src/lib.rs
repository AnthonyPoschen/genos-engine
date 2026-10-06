//! One collision step for dynamic bodies.
//!
//! `step` copies the world and returns the next world. Gravity, a move wish,
//! spring forces, and contact run in that call. The call does not allocate.
//! Gravity is along -Y. There is no air drag. Shapes are a sphere, a capsule,
//! a box, a plane, and a mesh reduced to a few convex pieces.

mod contact;
mod mesh;

use genos_math::{Quat, Vec3};

/// Acceleration along -Y, in meters per second squared.
pub const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

/// Bodies stored in one world. A step does not grow this set.
pub const MAX_BODIES: usize = 32;

/// Vertices stored on one convex hull piece.
pub const MAX_HULL_VERTS: usize = 12;

/// Triangle faces stored on one convex hull piece.
pub const MAX_HULL_FACES: usize = 20;

/// Convex pieces stored on one mesh collider.
pub const MAX_MESH_PIECES: usize = 6;

/// One outward triangle of a convex hull, in the body's local space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullFace {
    pub normal: Vec3,
    pub offset: f32,
    pub index: [u8; 3],
}

impl HullFace {
    const EMPTY: Self = Self {
        normal: Vec3::ZERO,
        offset: 0.0,
        index: [0, 0, 0],
    };
}

/// A convex polyhedron in the body's local space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    pub verts: [Vec3; MAX_HULL_VERTS],
    pub vert_count: u8,
    pub faces: [HullFace; MAX_HULL_FACES],
    pub face_count: u8,
}

impl Hull {
    const EMPTY: Self = Self {
        verts: [Vec3::ZERO; MAX_HULL_VERTS],
        vert_count: 0,
        faces: [HullFace::EMPTY; MAX_HULL_FACES],
        face_count: 0,
    };
}

/// One convex piece of a simplified mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Piece {
    /// Axis-aligned box in the body's local space.
    Cuboid {
        center: Vec3,
        half: Vec3,
    },
    Hull(Hull),
}

impl Piece {
    const EMPTY: Self = Self::Cuboid {
        center: Vec3::ZERO,
        half: Vec3::ZERO,
    };

    fn faces(self) -> usize {
        match self {
            Self::Cuboid { .. } => 6,
            Self::Hull(hull) => hull.face_count as usize,
        }
    }
}

/// A mesh reduced to a handful of convex pieces before contact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mesh {
    pub pieces: [Piece; MAX_MESH_PIECES],
    pub count: u8,
}

impl Mesh {
    const EMPTY: Self = Self {
        pieces: [Piece::EMPTY; MAX_MESH_PIECES],
        count: 0,
    };

    fn pieces(&self) -> &[Piece] {
        &self.pieces[..self.count as usize]
    }
}

/// A sphere, capsule, box, static plane, or simplified mesh.
/// The plane normal points into the free half.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sphere {
        radius: f32,
    },
    /// Segment along local Y. `half_height` is the segment half-length.
    Capsule {
        radius: f32,
        half_height: f32,
    },
    /// Oriented box. `half_extents` is half the size on each local axis.
    Box {
        half_extents: Vec3,
    },
    /// `normal · point = offset`.
    Plane {
        normal: Vec3,
        offset: f32,
    },
    Mesh(Mesh),
}

impl Shape {
    /// Faces the contact step uses. A mesh reports the reduced piece faces.
    pub fn contact_faces(self) -> usize {
        match self {
            Self::Mesh(mesh) => mesh.pieces().iter().map(|piece| piece.faces()).sum(),
            Self::Box { .. } => 6,
            Self::Plane { .. } => 1,
            Self::Sphere { .. } | Self::Capsule { .. } => 1,
        }
    }

    /// Convex pieces the contact step tests. A primitive is one piece.
    pub fn contact_pieces(self) -> usize {
        match self {
            Self::Mesh(mesh) => mesh.count as usize,
            Self::Plane { .. } => 0,
            Self::Sphere { .. } | Self::Capsule { .. } | Self::Box { .. } => 1,
        }
    }

    /// Build a collider from a triangle list. A complex mesh is reduced first.
    pub fn from_triangles(triangles: &[[Vec3; 3]]) -> Self {
        Self::Mesh(mesh::simplify(triangles))
    }
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
    /// When set, the step copies `wish.x` and `wish.z` into the velocity before contact.
    pub motor: bool,
    /// Desired horizontal velocity. Contact can remove the blocked part.
    pub wish: Vec3,
}

impl Body {
    pub const fn sphere(position: Vec3, mass: f32, radius: f32) -> Self {
        Self::new(position, mass, Shape::Sphere { radius })
    }

    /// Upright when `orientation` is identity. The segment follows local Y.
    pub const fn capsule(position: Vec3, mass: f32, radius: f32, half_height: f32) -> Self {
        Self::new(
            position,
            mass,
            Shape::Capsule {
                radius,
                half_height,
            },
        )
    }

    /// Box centered on `position`. Axes follow `orientation`.
    pub const fn cuboid(position: Vec3, mass: f32, half_extents: Vec3) -> Self {
        Self::new(position, mass, Shape::Box { half_extents })
    }

    pub fn plane(normal: Vec3, offset: f32) -> Self {
        let normal = normal.normalize();
        let normal = if normal.length_squared() == 0.0 {
            Vec3::Y
        } else {
            normal
        };
        Self::new(Vec3::ZERO, 0.0, Shape::Plane { normal, offset })
    }

    /// Static when `mass` is 0. Vertices stay in the body's local space.
    pub fn mesh(position: Vec3, mass: f32, triangles: &[[Vec3; 3]]) -> Self {
        Self::new(position, mass, Shape::from_triangles(triangles))
    }

    const fn new(position: Vec3, mass: f32, shape: Shape) -> Self {
        Self {
            position,
            orientation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            inverse_mass: if mass > 0.0 { 1.0 / mass } else { 0.0 },
            restitution: 0.0,
            friction: 0.0,
            shape,
            spring: None,
            motor: false,
            wish: Vec3::ZERO,
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
        if body.motor {
            body.velocity.x = body.wish.x;
            body.velocity.z = body.wish.z;
        }
    }
    for body in &mut next.bodies[..live] {
        if body.inverse_mass == 0.0 {
            continue;
        }
        body.position += body.velocity * dt;
    }
    for _ in 0..4 {
        contact::collide(&mut next.bodies[..live]);
    }
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
