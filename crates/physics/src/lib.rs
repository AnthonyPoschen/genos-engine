#![expect(
    clippy::as_conversions,
    reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
)]
#![expect(
    clippy::large_enum_variant,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::large_types_passed_by_value,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::match_same_arms,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]

//! One collision step for dynamic bodies.
//!
//! `step` advances the world in place. Gravity, a move wish, spring forces, and
//! contact run in that call. Contact tests the pairs the static tree and the
//! dynamic tree report. A static body is not in the dynamic tree, and two
//! statics are not a pair.
//! Gravity is along -Y. There is no air drag. Shapes are a sphere, a capsule,
//! a box, a plane, and a mesh reduced to a few convex pieces.

mod broadphase;
mod ccd;
mod contact;
mod destruction;
mod island;
mod mesh;
mod solver;

pub use destruction::{
    allows_slice, apply_damage, chunk_body, destruction_report, gi_cache_dirty, intact_body,
    spawn_breakable, ChunkChange, ChunkChangeKind, DestructionReport,
    CHANGE_RECORDS_AWAIT_LIGHTING,
};
pub use genos_fracture::{cook, Asset, CookRequest};

use genos_math::{Mat3, Quat, Vec3};

/// Acceleration along -Y, in meters per second squared.
pub const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

/// Vertices stored on one convex hull piece.
pub const MAX_HULL_VERTS: usize = 12;

/// Triangle faces stored on one convex hull piece.
pub const MAX_HULL_FACES: usize = 20;
/// Feature-edge directions kept per hull for edge-edge separating axes.
pub const MAX_HULL_EDGES: usize = 3 * MAX_HULL_FACES / 2;

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
    /// Unit feature-edge directions, one per parallel family, built with the
    /// faces so the step does not rebuild them per contact.
    pub edges: [Vec3; MAX_HULL_EDGES],
    pub edge_count: u8,
}

impl Hull {
    const EMPTY: Self = Self {
        verts: [Vec3::ZERO; MAX_HULL_VERTS],
        vert_count: 0,
        faces: [HullFace::EMPTY; MAX_HULL_FACES],
        face_count: 0,
        edges: [Vec3::ZERO; MAX_HULL_EDGES],
        edge_count: 0,
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
    /// Inverse of the unit-mass inertia about the local origin, from the
    /// pieces' volume. A zero matrix means the build stored no tensor yet.
    pub unit_inverse_inertia: Mat3,
}

impl Mesh {
    const EMPTY: Self = Self {
        pieces: [Piece::EMPTY; MAX_MESH_PIECES],
        count: 0,
        unit_inverse_inertia: Mat3::IDENTITY,
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

/// How far one body's collision looks ahead in a single step.
///
/// Whether the step still moves a body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    /// Forces, contact, and integration run.
    Live,
    /// Debris that has slept. The body stays in the list.
    Asleep,
    /// Removed from the step. The index stays so other bodies do not move.
    Culled,
}

/// `Discrete` uses the bounds at the current pose. `Speculative` grows the
/// contact distance by `min(speed * dt, speculative margin)` and lets the
/// solver stop the body. `Swept` advances that body's pairs to the time of
/// impact. Bodies that are not in a swept pair keep the one integration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionQuality {
    /// Overlap that already exists.
    Discrete,
    /// A margin on the approach. The bullet path.
    Speculative,
    /// Time of impact on this body's pairs. Other bodies keep the one integration.
    Swept,
}

/// Counters from the last `step`. A later step replaces them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepStats {
    /// A body asked for speculative or swept collision.
    pub ccd_applied: bool,
    /// Swept pairs clamped because 16 advances left a gap above 1 mm.
    pub toi_failed: u32,
    /// Swept pairs that used the speculative margin because the pair cap was full.
    pub toi_deferred: u32,
    /// Contacts between two dynamic bodies that stored a normal impulse.
    pub dynamic_pair_impulses: u32,
    /// Advances taken by fast pairs whose travel exceeded the smaller radius.
    pub pair_slices: u32,
    /// Contact points solved on the last narrow-phase pass.
    pub contact_points: u32,
    /// Dynamic bodies asleep at the end of the step.
    pub sleeping_bodies: u32,
    /// Pairs that reached the narrow phase.
    pub pair_tests: u32,
    /// Times the broadphase built or refit a tree this step.
    pub broadphase_builds: u32,
    /// Nodes in the static tree plus the dynamic tree.
    pub tree_nodes: u32,
    /// Contact islands that still have a dynamic body.
    pub islands: u32,
    /// Dynamic bodies that are not sleeping.
    pub awake_bodies: u32,
    /// Dynamic bodies the narrow phase handed to the solver.
    pub solver_bodies: u32,
}

impl StepStats {
    const fn zero() -> Self {
        Self {
            ccd_applied: false,
            toi_failed: 0,
            toi_deferred: 0,
            dynamic_pair_impulses: 0,
            pair_slices: 0,
            contact_points: 0,
            sleeping_bodies: 0,
            pair_tests: 0,
            broadphase_builds: 0,
            tree_nodes: 0,
            islands: 0,
            awake_bodies: 0,
            solver_bodies: 0,
        }
    }
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
    /// `Discrete` unless a caller asks for a bullet margin or a swept pair.
    pub motion: MotionQuality,
    /// Normal impulse stored on this body during the last `step`. The next step replaces it.
    pub normal_impulse: f32,
    /// World-space angular velocity, radians per second.
    pub angular_velocity: Vec3,
    /// `Live` until debris sleeps or a culled chunk is tombstoned.
    pub presence: Presence,
    /// Spawned chunk under the debris volume. Two debris bodies do not collide.
    pub debris: bool,
    /// True after the body has been still long enough to leave the solver.
    /// A contact from a moving body clears it. Static bodies stay false.
    pub sleeping: bool,
    /// Seconds spent under the sleep speed. Cleared when the body wakes.
    pub sleep_time: f32,
    /// `u16::MAX` until a caller picks an object layer. Then 0 static, 1 dynamic, 2 debris.
    pub object_layer: u16,
    /// 0 lives in the static tree. Anything else lives in the dynamic tree.
    pub broad_phase_layer: u8,
    /// Island id. Bodies that share an id sleep and wake together.
    pub island: u32,
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
            motion: MotionQuality::Discrete,
            normal_impulse: 0.0,
            angular_velocity: Vec3::ZERO,
            presence: Presence::Live,
            debris: false,
            sleeping: false,
            sleep_time: 0.0,
            object_layer: u16::MAX,
            broad_phase_layer: 0,
            island: 0,
        }
    }

    /// Pin this body to an object layer. Debris stays in the dynamic tree.
    #[must_use]
    pub const fn with_layer(mut self, layer: u16) -> Self {
        self.object_layer = layer;
        self.broad_phase_layer = if layer == 0 { 0 } else { 1 };
        self
    }

    /// `½ ωᵀ I ω` in joules. `I` is diagonal in the body frame. Zero when `ω` is zero.
    pub fn rotational_energy(self) -> f32 {
        if self.inverse_mass == 0.0 || self.angular_velocity.length_squared() <= 0.0 {
            return 0.0;
        }
        let local = conjugate(self.orientation).rotate(self.angular_velocity);
        let inertia = inertia_diagonal(self);
        0.5 * (inertia.x * local.x * local.x
            + inertia.y * local.y * local.y
            + inertia.z * local.z * local.z)
    }

    pub fn mass(self) -> f32 {
        if self.inverse_mass == 0.0 {
            f32::INFINITY
        } else {
            1.0 / self.inverse_mass
        }
    }
}

/// Bodies plus a gravity acceleration.
#[derive(Clone, Debug)]
pub struct World {
    pub bodies: Vec<Body>,
    pub gravity: Vec3,
    /// Cap on `min(speed * dt, margin)` for `MotionQuality::Speculative`. Default 0.25 m.
    pub speculative_margin: f32,
    /// Swept pairs past this count use the speculative margin for that step. Default 8.
    pub swept_pair_cap: u32,
    /// Fracture assets. Empty for a world that does not break.
    pub(crate) breakables: Vec<destruction::Breakable>,
    /// Chunk bounds records for this step, in `(breakable, chunk)` order.
    /// Cleared at the start of the next `step`. Lighting does not read them.
    pub change_records: Vec<destruction::ChunkChange>,
    /// A client emits [`change_records`](Self::change_records). A headless server sets this false.
    pub emit_change_records: bool,
    /// Replaced at the start of each `step`.
    pub step_stats: StepStats,
    /// Linear speed under which a body can sleep. Default 0.12 m/s.
    pub sleep_linear: f32,
    /// Angular speed under which a body can sleep. Default 0.25 rad/s.
    pub sleep_angular: f32,
    /// How long every member of an island must stay calm. Default 0.5 s.
    pub sleep_time: f32,
    /// Which object layers produce pairs.
    pub layer_pairs: broadphase::LayerPairs,
    /// The contact pass's lists, kept so a step after the first does not allocate.
    pairs: contact::Pairs,
    /// Swept-pair scratch, kept so a later step does not allocate.
    ccd_scratch: ccd::CcdScratch,
    broadphase: broadphase::Broadphase,
    islands: island::Islands,
}

/// Two worlds are equal when their bodies and gravity are.
impl PartialEq for World {
    fn eq(&self, other: &Self) -> bool {
        self.bodies == other.bodies && self.gravity == other.gravity
    }
}

impl World {
    pub const fn new(gravity: Vec3) -> Self {
        Self {
            bodies: Vec::new(),
            gravity,
            speculative_margin: 0.25,
            swept_pair_cap: 8,
            breakables: Vec::new(),
            change_records: Vec::new(),
            emit_change_records: true,
            step_stats: StepStats::zero(),
            sleep_linear: 0.12,
            sleep_angular: 0.25,
            sleep_time: 0.5,
            layer_pairs: broadphase::LayerPairs::standard(),
            pairs: contact::Pairs::new(),
            ccd_scratch: ccd::CcdScratch::new(),
            broadphase: broadphase::Broadphase::new(),
            islands: island::Islands::new(),
        }
    }

    /// Store one body. Returns its index in `bodies`.
    pub fn insert(&mut self, mut body: Body) -> usize {
        broadphase::sync_layer(&mut body);
        let index = self.bodies.len();
        body.island = u32::try_from(index).unwrap_or(0);
        self.bodies.push(body);
        self.broadphase.insert_one(&self.bodies, index);
        index
    }

    /// Build the trees for this batch in one pass.
    pub fn insert_many(&mut self, bodies: impl IntoIterator<Item = Body>) -> Vec<usize> {
        let mut ids = Vec::new();
        for mut body in bodies {
            broadphase::sync_layer(&mut body);
            let index = self.bodies.len();
            body.island = u32::try_from(index).unwrap_or(0);
            self.bodies.push(body);
            ids.push(index);
        }
        self.broadphase.insert_batch(&self.bodies);
        ids
    }

    /// Replace both trees with a tight median build.
    pub fn refit(&mut self) {
        for body in &mut self.bodies {
            broadphase::sync_layer(body);
        }
        self.broadphase.rebuild(&self.bodies);
    }

    /// Move a body and wake its island. The leaf grows until [`refit`](Self::refit).
    pub fn set_position(&mut self, index: usize, position: Vec3) {
        let Some(body) = self.bodies.get_mut(index) else {
            return;
        };
        body.position = position;
        let id = u32::try_from(index).unwrap_or(0);
        if !self.broadphase.displaced.contains(&id) {
            self.broadphase.displaced.push(id);
        }
        self.wake(index);
        self.broadphase.expand(&self.bodies, index);
    }

    /// Wake every dynamic body that shares this body's island.
    pub fn wake(&mut self, index: usize) {
        let Some(island) = self.bodies.get(index).map(|body| body.island) else {
            return;
        };
        for body in &mut self.bodies {
            if body.island == island && body.inverse_mass > 0.0 {
                body.sleeping = false;
                body.sleep_time = 0.0;
            }
        }
    }

    /// A body changed mass or layer outside `insert`. Puts it in the matching tree.
    pub fn reindex(&mut self, index: usize) {
        let Some(body) = self.bodies.get_mut(index) else {
            return;
        };
        broadphase::sync_layer(body);
        self.broadphase.retarget(&self.bodies, index);
    }

    /// Tombstone one body. Neighbours stay asleep.
    pub fn remove(&mut self, index: usize) {
        let Some(body) = self.bodies.get_mut(index) else {
            return;
        };
        body.presence = Presence::Culled;
        self.broadphase.forget(index);
    }

    #[must_use]
    pub fn in_static_tree(&self, index: usize) -> bool {
        self.broadphase.in_static(index)
    }

    #[must_use]
    pub fn in_dynamic_tree(&self, index: usize) -> bool {
        self.broadphase.in_dynamic(index)
    }

    #[must_use]
    pub fn tree_node_count(&self) -> u32 {
        self.broadphase.node_count()
    }

    #[must_use]
    pub fn static_rebuilds(&self) -> u32 {
        self.broadphase.static_rebuilds
    }

    /// Pairs the current trees report. Does not refit and does not step.
    #[must_use]
    pub fn overlapping_pairs(&mut self) -> Vec<(u32, u32)> {
        let mut pairs = Vec::new();
        let mut planes = Vec::new();
        self.broadphase
            .fill(&self.bodies, &self.layer_pairs, &mut pairs, &mut planes);
        pairs
    }

    pub fn set_layer_pair(&mut self, a: u16, b: u16, accept: bool) {
        self.layer_pairs.set(a, b, accept);
    }
}

/// Advance every body by `dt` seconds.
pub fn step(world: &mut World, dt: f32) {
    if !(dt > 0.0) {
        return;
    }
    world.change_records.clear();
    world.islands.bind(world.broadphase.dynamic_ids());
    let gravity = world.gravity;
    let dynamic_count = world.broadphase.dynamic_count();
    for slot in 0..dynamic_count {
        let id = world.broadphase.dynamic_at(slot);
        let Some(body) = world.bodies.get_mut(usize_index(id)) else {
            continue;
        };
        body.normal_impulse = 0.0;
        apply_forces(body, gravity, dt);
        if body.motor {
            body.velocity.x = body.wish.x;
            body.velocity.z = body.wish.z;
        }
        broadphase::sync_layer(body);
    }
    for item in &world.breakables {
        let Some(intact) = item.intact else {
            continue;
        };
        if let Some(body) = world.bodies.get_mut(intact) {
            body.normal_impulse = 0.0;
        }
    }
    world.step_stats = StepStats::zero();
    let (speculative, swept) = motion_flags(world);
    world.step_stats.ccd_applied = speculative || swept;
    ccd::prepare_swept(
        &world.bodies,
        &mut world.ccd_scratch,
        world.swept_pair_cap,
        dt,
        swept,
        &mut world.step_stats,
    );
    if speculative {
        ccd::speculative(&mut world.bodies, dt, world.speculative_margin);
    }
    let dynamic_count = world.broadphase.dynamic_count();
    for slot in 0..dynamic_count {
        let id = world.broadphase.dynamic_at(slot);
        let index = usize_index(id);
        let Some(body) = world.bodies.get_mut(index) else {
            continue;
        };
        if body.inverse_mass == 0.0 || world.ccd_scratch.covers(index) {
            continue;
        }
        if body.motion == MotionQuality::Swept {
            continue;
        }
        integrate_body(body, dt);
    }
    ccd::resolve_swept(
        &mut world.bodies,
        &world.ccd_scratch,
        dt,
        world.speculative_margin,
        &mut world.step_stats,
    );
    // The narrow phase is built once. Later passes move the stored local
    // points with the bodies and separate again, without another SAT.
    world.broadphase.prepare(&world.bodies);
    let mut hits = 0_u32;
    for pass in 0..4 {
        let mut query = contact::PairQuery {
            layers: &world.layer_pairs,
            trees: &mut world.broadphase,
            islands: &mut world.islands,
        };
        hits = hits.saturating_add(contact::collide(
            &mut world.bodies,
            &mut world.pairs,
            &mut query,
            pass == 0,
            pass == 0,
            pass == 0,
        ));
    }
    close_step(world, hits, dt);
    if !world.breakables.is_empty() {
        destruction::after_step(world, dt);
    }
}

fn close_step(world: &mut World, hits: u32, dt: f32) {
    world.step_stats.dynamic_pair_impulses =
        world.step_stats.dynamic_pair_impulses.saturating_add(hits);
    world.step_stats.contact_points = world.pairs.contact_point_count();
    world.islands.store_pairs(world.pairs.pair_list());
    world.islands.assign_and_sleep(
        &mut world.bodies,
        dt,
        world.sleep_linear,
        world.sleep_angular,
        world.sleep_time,
    );
    world.step_stats.sleeping_bodies = sleeping_dynamics(world);
    world.step_stats.pair_tests = u32::try_from(world.pairs.pair_list().len()).unwrap_or(u32::MAX);
    world.step_stats.broadphase_builds = 1;
    world.step_stats.tree_nodes = world.broadphase.node_count();
    world.step_stats.islands = world.islands.count(&world.bodies);
    world.step_stats.awake_bodies = awake_dynamics(world);
    world.step_stats.solver_bodies = world.pairs.solver_bodies(&world.bodies);
    world.broadphase.displaced.clear();
}

fn usize_index(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

fn motion_flags(world: &World) -> (bool, bool) {
    let mut speculative = false;
    let mut swept = false;
    for slot in 0..world.broadphase.dynamic_count() {
        let id = world.broadphase.dynamic_at(slot);
        let Some(body) = world.bodies.get(usize_index(id)) else {
            continue;
        };
        speculative |= body.motion == MotionQuality::Speculative;
        swept |= body.motion == MotionQuality::Swept;
        if speculative && swept {
            break;
        }
    }
    (speculative, swept)
}

fn sleeping_dynamics(world: &World) -> u32 {
    let mut count = 0_u32;
    for slot in 0..world.broadphase.dynamic_count() {
        let id = world.broadphase.dynamic_at(slot);
        let asleep = world
            .bodies
            .get(usize_index(id))
            .is_some_and(|body| body.sleeping);
        if asleep {
            count = count.saturating_add(1);
        }
    }
    count
}

fn awake_dynamics(world: &World) -> u32 {
    let mut count = 0_u32;
    for slot in 0..world.broadphase.dynamic_count() {
        let id = world.broadphase.dynamic_at(slot);
        let awake = world.bodies.get(usize_index(id)).is_some_and(|body| {
            body.inverse_mass > 0.0 && !body.sleeping && body.presence == Presence::Live
        });
        if awake {
            count = count.saturating_add(1);
        }
    }
    count
}

pub(crate) fn integrate_body(body: &mut Body, dt: f32) {
    if body.presence != Presence::Live || body.inverse_mass == 0.0 || body.sleeping || !(dt > 0.0) {
        return;
    }
    body.position += body.velocity * dt;
    if body.angular_velocity.length_squared() <= 0.0 {
        return;
    }
    let local = conjugate(body.orientation).rotate(body.angular_velocity);
    let stepped = solver::implicit_gyroscopic(local, inertia_diagonal(*body), dt);
    body.angular_velocity = body.orientation.rotate(stepped);
    body.orientation = solver::integrate_orientation(body.orientation, body.angular_velocity, dt);
}

fn conjugate(q: Quat) -> Quat {
    Quat::new(-q.x, -q.y, -q.z, q.w)
}

/// Body-frame principal moments `(Ixx, Iyy, Izz)`.
pub(crate) fn inertia_diagonal(body: Body) -> Vec3 {
    let mass = body.mass();
    if !mass.is_finite() {
        return Vec3::ZERO;
    }
    match body.shape {
        Shape::Sphere { radius } => {
            let moment = 0.4 * mass * radius * radius;
            Vec3::new(moment, moment, moment)
        }
        Shape::Box { half_extents } => box_inertia(mass, half_extents),
        Shape::Capsule {
            radius,
            half_height,
        } => capsule_inertia(mass, radius, half_height),
        Shape::Plane { .. } => Vec3::ZERO,
        Shape::Mesh(mesh) => mesh_principal(mass, &mesh),
    }
}

/// Diagonal of the volume-integral inertia, for the gyroscopic step and energy.
fn mesh_principal(mass: f32, mesh: &Mesh) -> Vec3 {
    let inverse = mesh.unit_inverse_inertia;
    let unit = invert_mat3(inverse).unwrap_or_else(|| {
        let radius = mesh_extent(mesh);
        let moment = 0.4 * radius * radius;
        Mat3::from_scale(Vec3::new(moment, moment, moment))
    });
    Vec3::new(unit.cols[0], unit.cols[4], unit.cols[8]) * mass
}

/// Unit-mass inertia inverse about the local origin.
pub(crate) fn mesh_unit_inverse(mesh: &Mesh) -> Mat3 {
    let mut volume = 0.0_f32;
    let mut ixx = 0.0_f32;
    let mut iyy = 0.0_f32;
    let mut izz = 0.0_f32;
    let mut ixy = 0.0_f32;
    let mut iyz = 0.0_f32;
    let mut izx = 0.0_f32;
    for piece in mesh.pieces() {
        match piece {
            Piece::Cuboid { center, half } => {
                let vol = 8.0 * half.x * half.y * half.z;
                if vol <= 1.0e-12 {
                    continue;
                }
                let local = box_inertia(vol, *half);
                volume += vol;
                let (xx, yy, zz, xy, yz, zx) = parallel_axis(local, vol, *center);
                ixx += xx;
                iyy += yy;
                izz += zz;
                ixy += xy;
                iyz += yz;
                izx += zx;
            }
            Piece::Hull(hull) => {
                let (vol, xx, yy, zz, xy, yz, zx) = hull_moments(hull);
                volume += vol;
                ixx += xx;
                iyy += yy;
                izz += zz;
                ixy += xy;
                iyz += yz;
                izx += zx;
            }
        }
    }
    if volume <= 1.0e-8 {
        let radius = mesh_extent(mesh).max(1.0e-3);
        let moment = 0.4 * radius * radius;
        return Mat3::from_scale(Vec3::new(1.0 / moment, 1.0 / moment, 1.0 / moment));
    }
    let scale = 1.0 / volume;
    let inertia = Mat3::from_cols([
        ixx * scale,
        ixy * scale,
        izx * scale,
        ixy * scale,
        iyy * scale,
        iyz * scale,
        izx * scale,
        iyz * scale,
        izz * scale,
    ]);
    invert_mat3(inertia).unwrap_or_else(|| {
        let radius = mesh_extent(mesh).max(1.0e-3);
        let moment = 0.4 * radius * radius;
        Mat3::from_scale(Vec3::new(1.0 / moment, 1.0 / moment, 1.0 / moment))
    })
}

/// `I_origin = I_center + m (|c|² I − c cᵀ)`.
fn parallel_axis(local: Vec3, mass: f32, center: Vec3) -> (f32, f32, f32, f32, f32, f32) {
    let r2 = center.length_squared();
    (
        local.x + mass * (r2 - center.x * center.x),
        local.y + mass * (r2 - center.y * center.y),
        local.z + mass * (r2 - center.z * center.z),
        -mass * center.x * center.y,
        -mass * center.y * center.z,
        -mass * center.z * center.x,
    )
}

/// Volume integrals of one hull about the local origin (unit density).
///
/// Each face triangle is a tetrahedron with the origin. The second moments
/// follow Eberly's closed form for a tetrahedron with one vertex at the origin.
fn hull_moments(hull: &Hull) -> (f32, f32, f32, f32, f32, f32, f32) {
    let mut volume = 0.0_f32;
    let mut ixx = 0.0_f32;
    let mut iyy = 0.0_f32;
    let mut izz = 0.0_f32;
    let mut ixy = 0.0_f32;
    let mut iyz = 0.0_f32;
    let mut izx = 0.0_f32;
    let count = hull.face_count as usize;
    for face in hull.faces.iter().take(count) {
        let a = hull.verts[face.index[0] as usize];
        let b = hull.verts[face.index[1] as usize];
        let c = hull.verts[face.index[2] as usize];
        let vol = a.dot(b.cross(c)) / 6.0;
        if vol.abs() < 1.0e-14 {
            continue;
        }
        volume += vol;
        ixx += second_diag(a.y, b.y, c.y, a.z, b.z, c.z, vol);
        iyy += second_diag(a.x, b.x, c.x, a.z, b.z, c.z, vol);
        izz += second_diag(a.x, b.x, c.x, a.y, b.y, c.y, vol);
        ixy -= second_off(a.x, b.x, c.x, a.y, b.y, c.y, vol);
        iyz -= second_off(a.y, b.y, c.y, a.z, b.z, c.z, vol);
        izx -= second_off(a.z, b.z, c.z, a.x, b.x, c.x, vol);
    }
    (volume, ixx, iyy, izz, ixy, iyz, izx)
}

fn second_diag(y1: f32, y2: f32, y3: f32, z1: f32, z2: f32, z3: f32, volume: f32) -> f32 {
    let sy = y1 * y1 + y2 * y2 + y3 * y3 + y1 * y2 + y1 * y3 + y2 * y3;
    let sz = z1 * z1 + z2 * z2 + z3 * z3 + z1 * z2 + z1 * z3 + z2 * z3;
    volume * (sy + sz) / 10.0
}

fn second_off(x1: f32, x2: f32, x3: f32, y1: f32, y2: f32, y3: f32, volume: f32) -> f32 {
    let sum = 2.0 * (x1 * y1 + x2 * y2 + x3 * y3)
        + x1 * y2
        + x2 * y1
        + x1 * y3
        + x3 * y1
        + x2 * y3
        + x3 * y2;
    volume * sum / 20.0
}

fn invert_mat3(m: Mat3) -> Option<Mat3> {
    let c = m.cols;
    let (m00, m01, m02) = (c[0], c[3], c[6]);
    let (m10, m11, m12) = (c[1], c[4], c[7]);
    let (m20, m21, m22) = (c[2], c[5], c[8]);
    let c00 = m11 * m22 - m12 * m21;
    let c01 = m12 * m20 - m10 * m22;
    let c02 = m10 * m21 - m11 * m20;
    let det = m00 * c00 + m01 * c01 + m02 * c02;
    if det.abs() < 1.0e-14 {
        return None;
    }
    let inv_det = 1.0 / det;
    let c10 = m21 * m02 - m22 * m01;
    let c11 = m22 * m00 - m20 * m02;
    let c12 = m20 * m01 - m21 * m00;
    let c20 = m01 * m12 - m02 * m11;
    let c21 = m02 * m10 - m00 * m12;
    let c22 = m00 * m11 - m01 * m10;
    Some(Mat3::from_cols([
        c00 * inv_det,
        c01 * inv_det,
        c02 * inv_det,
        c10 * inv_det,
        c11 * inv_det,
        c12 * inv_det,
        c20 * inv_det,
        c21 * inv_det,
        c22 * inv_det,
    ]))
}

/// `Ixx = m/3 (hy² + hz²)` with half extents, which is `m/12` of the full side squares.
fn box_inertia(mass: f32, half: Vec3) -> Vec3 {
    let scale = mass / 3.0;
    Vec3::new(
        scale * (half.y * half.y + half.z * half.z),
        scale * (half.x * half.x + half.z * half.z),
        scale * (half.x * half.x + half.y * half.y),
    )
}

fn capsule_inertia(mass: f32, radius: f32, half_height: f32) -> Vec3 {
    let r = radius.max(0.0);
    let h = half_height.max(0.0) * 2.0;
    let cylinder = std::f32::consts::PI * r * r * h;
    let hemispheres = (4.0 / 3.0) * std::f32::consts::PI * r * r * r;
    let volume = cylinder + hemispheres;
    if volume <= f32::MIN_POSITIVE {
        return Vec3::ZERO;
    }
    let cylinder_mass = mass * (cylinder / volume);
    let sphere_mass = mass * (hemispheres / volume);
    let axial = 0.5 * cylinder_mass * r * r + 0.4 * sphere_mass * r * r;
    let perp = cylinder_mass * (0.25 * r * r + h * h / 12.0)
        + sphere_mass * (0.4 * r * r + 0.5 * h * h + 0.375 * h * r);
    Vec3::new(perp, axial, perp)
}

fn mesh_extent(mesh: &Mesh) -> f32 {
    let mut far = 0.0_f32;
    for piece in mesh.pieces() {
        let radius = match piece {
            Piece::Cuboid { center, half } => center.length() + half.length(),
            Piece::Hull(hull) => hull
                .verts
                .iter()
                .take(usize::from(hull.vert_count))
                .fold(0.0_f32, |best, vertex| best.max(vertex.length())),
        };
        far = far.max(radius);
    }
    far
}

fn apply_forces(body: &mut Body, gravity: Vec3, dt: f32) {
    if body.sleeping && (body.velocity.length() > 0.25 || body.angular_velocity.length() > 0.4) {
        body.sleeping = false;
        body.sleep_time = 0.0;
    }
    if body.presence != Presence::Live || body.inverse_mass == 0.0 || body.sleeping {
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

#[cfg(test)]
mod inertia_tests {
    use super::{box_inertia, inertia_diagonal, step, Body, World};
    use crate::contact;
    use genos_math::Vec3;

    fn cube(half: Vec3) -> Vec<[Vec3; 3]> {
        let v = |x: f32, y: f32, z: f32| Vec3::new(x * half.x, y * half.y, z * half.z);
        let (nnn, pnn, ppn, npn) = (
            v(-1.0, -1.0, -1.0),
            v(1.0, -1.0, -1.0),
            v(1.0, 1.0, -1.0),
            v(-1.0, 1.0, -1.0),
        );
        let (nnp, pnp, ppp, npp) = (
            v(-1.0, -1.0, 1.0),
            v(1.0, -1.0, 1.0),
            v(1.0, 1.0, 1.0),
            v(-1.0, 1.0, 1.0),
        );
        vec![
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
        ]
    }

    #[test]
    fn tree_pairs_match_a_sweep_after_forty_steps() {
        use genos_math::Quat;
        fn cube(h: Vec3) -> Vec<[Vec3; 3]> {
            let v = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
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
            vec![
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
            ]
        }
        let mut world = World::new(Vec3::new(0.0, -9.81, 0.0));
        world.insert(Body::mesh(
            Vec3::new(0.0, -0.5, 0.0),
            0.0,
            &cube(Vec3::new(4.0, 0.5, 4.0)),
        ));
        let block = cube(Vec3::new(0.5, 0.5, 0.5));
        let mut angle = 0.0_f32;
        for layer in [0.6_f32, 1.8, 3.0, 4.2] {
            for (x, z) in [(-0.6_f32, -0.6_f32), (0.6, -0.6), (-0.6, 0.6), (0.6, 0.6)] {
                let mut body = Body::mesh(Vec3::new(x, layer, z), 1.0, &block);
                body.orientation = Quat::from_axis_angle(Vec3::Y, angle);
                angle += 0.21;
                world.insert(body);
            }
        }
        for _ in 0..40 {
            step(&mut world, 1.0 / 60.0);
        }
        let tree = world.overlapping_pairs();
        let mut sweep = Vec::new();
        let margin = Vec3::new(0.02, 0.02, 0.02);
        let mut boxes = Vec::new();
        for (index, body) in world.bodies.iter().enumerate() {
            if let Some((lo, hi)) = contact::bounds(body) {
                boxes.push((lo - margin, hi + margin, u32::try_from(index).unwrap_or(0)));
            }
        }
        boxes.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
        for (at, &(lo, hi, i)) in boxes.iter().enumerate() {
            for &(lo2, hi2, j) in &boxes[at + 1..] {
                if lo2.x > hi.x {
                    break;
                }
                let meet = lo2.y <= hi.y && lo.y <= hi2.y && lo2.z <= hi.z && lo.z <= hi2.z;
                let moving = world.bodies[i as usize].inverse_mass != 0.0
                    || world.bodies[j as usize].inverse_mass != 0.0;
                if meet && moving {
                    sweep.push((i.min(j), i.max(j)));
                }
            }
        }
        sweep.sort_unstable();
        assert_eq!(tree, sweep, "tree {} sweep {}", tree.len(), sweep.len());
    }

    #[test]
    fn a_cube_hull_matches_the_box_moment() {
        let half = Vec3::new(0.5, 0.4, 0.3);
        let body = Body::mesh(Vec3::ZERO, 2.0, &cube(half));
        let got = inertia_diagonal(body);
        let expect = box_inertia(2.0, half);
        let err = (got - expect).length() / expect.length();
        assert!(
            err < 0.02,
            "hull inertia {got:?} vs box {expect:?}, relative {err}"
        );
    }
}
