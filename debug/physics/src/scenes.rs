#![expect(
    clippy::as_conversions,
    reason = "grid coordinates stay small integers"
)]
#![expect(
    clippy::cast_precision_loss,
    reason = "grid coordinates stay small integers"
)]
//! The physics debug scenes. Each one is a fixture a physics test also runs,
//! built here in code because the shared `debug/scenes/*.scene` files describe
//! lighting geometry, not bodies.

use genos_math::{Quat, Vec3};
use genos_physics::{Body, MotionQuality, World};

/// One entry in the scene picker.
pub struct SceneInfo {
    pub name: &'static str,
    pub summary: &'static str,
    /// Steps a headless run takes when `--steps` is omitted.
    pub steps: u32,
    build: fn() -> World,
}

impl SceneInfo {
    #[must_use]
    pub fn build(&self) -> World {
        (self.build)()
    }
}

pub const SCENES: [SceneInfo; 7] = [
    SceneInfo {
        name: "ship_lanes",
        summary: "four swept wedge-hull ship pairs head on at 30, 120, 600 and 3000 m/s, 36 m apart (tests/ship_impact.rs)",
        steps: 60,
        build: ship_lanes,
    },
    SceneInfo {
        name: "hull_edges",
        summary: "static crossed hull cubes: 1 cm apart, 5 cm overlapped, and an edge falling onto a static edge (tests/hull_edges.rs)",
        steps: 120,
        build: hull_edges,
    },
    SceneInfo {
        name: "resting_pile",
        summary: "16 turned hull cubes dropped onto a static hull slab under gravity",
        steps: 240,
        build: resting_pile,
    },
    SceneInfo {
        name: "static_grid_10k",
        summary: "10 000 static boxes on a 100 x 100 grid with 8 spheres falling onto it",
        steps: 60,
        build: static_grid_10k,
    },
    SceneInfo {
        name: "resting_1025",
        summary: "1 024 resting spheres on a plane (scale bench)",
        steps: 30,
        build: resting_1025,
    },
    SceneInfo {
        name: "layer_filter",
        summary: "two overlapping debris spheres; the layer table drops the pair",
        steps: 1,
        build: layer_filter,
    },
    SceneInfo {
        name: "sleep_island",
        summary: "two stacks of boxes that sleep apart; a displaced sphere wakes one island",
        steps: 180,
        build: sleep_island,
    },
];

#[must_use]
pub fn find(name: &str) -> Option<&'static SceneInfo> {
    SCENES.iter().find(|scene| scene.name == name)
}

pub const LANE_SPEEDS: [f32; 4] = [30.0, 120.0, 600.0, 3_000.0];
const LANE_GAP: f32 = 36.0;
const SHIP_HALF_LENGTH: f32 = 2.0;
const LANE_SPACING: f32 = 20.0;

/// A wedge hull: a 4 m long box whose nose tapers to a 1 m wide face.
fn wedge(nose: f32) -> Vec<[Vec3; 3]> {
    let l = SHIP_HALF_LENGTH;
    let p = [
        Vec3::new(-l * nose, -1.0, -1.0),
        Vec3::new(-l * nose, 1.0, -1.0),
        Vec3::new(-l * nose, 1.0, 1.0),
        Vec3::new(-l * nose, -1.0, 1.0),
        Vec3::new(l * nose, -0.5, -0.5),
        Vec3::new(l * nose, 0.5, -0.5),
        Vec3::new(l * nose, 0.5, 0.5),
        Vec3::new(l * nose, -0.5, 0.5),
    ];
    let quads = [
        [0, 1, 2, 3],
        [4, 7, 6, 5],
        [0, 4, 5, 1],
        [3, 2, 6, 7],
        [0, 3, 7, 4],
        [1, 5, 6, 2],
    ];
    let mut tris = Vec::new();
    for q in quads {
        // Mirroring x flips the winding; keep the faces outward.
        if nose > 0.0 {
            tris.push([p[q[0]], p[q[2]], p[q[1]]]);
            tris.push([p[q[0]], p[q[3]], p[q[2]]]);
        } else {
            tris.push([p[q[0]], p[q[1]], p[q[2]]]);
            tris.push([p[q[0]], p[q[2]], p[q[3]]]);
        }
    }
    tris
}

/// An axis-aligned box hull with outward winding.
#[must_use]
pub fn box_hull(h: Vec3) -> Vec<[Vec3; 3]> {
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

fn ship_lanes() -> World {
    let mut world = World::new(Vec3::ZERO);
    let mut z = 0.0;
    for speed in LANE_SPEEDS {
        for (x, v, nose) in [
            (0.0, speed, 1.0),
            (LANE_GAP + 2.0 * SHIP_HALF_LENGTH, -speed, -1.0),
        ] {
            let mut ship = Body::mesh(Vec3::new(x, 0.0, z), 1_000.0, &wedge(nose));
            ship.velocity = Vec3::new(v, 0.0, 0.0);
            ship.motion = MotionQuality::Swept;
            world.insert(ship);
        }
        z += LANE_SPACING;
    }
    world
}

fn hull_edges() -> World {
    let mut world = World::new(Vec3::new(0.0, -9.81, 0.0));
    let cube = box_hull(Vec3::new(0.5, 0.5, 0.5));
    let reach = 0.5 * std::f32::consts::SQRT_2;
    let quarter = std::f32::consts::FRAC_PI_4;
    let about_z = Quat::from_axis_angle(Vec3::Z, quarter);
    let about_y = Quat::from_axis_angle(Vec3::Y, quarter);
    // Two static pairs, crossed edges 1 cm apart and 5 cm overlapped: the
    // contact query runs every step with nothing moving.
    for (z, gap) in [(0.0_f32, 0.01_f32), (4.0, -0.05)] {
        let mut a = Body::mesh(Vec3::new(0.0, 0.0, z), 0.0, &cube);
        a.orientation = about_z;
        let mut b = Body::mesh(Vec3::new(2.0 * reach + gap, 0.0, z), 0.0, &cube);
        b.orientation = about_y;
        world.insert(a);
        world.insert(b);
    }
    // A dynamic cube, edge down along x, falls onto a static cube whose top
    // edge runs along z.
    let mut base = Body::mesh(Vec3::new(0.0, 0.0, 8.0), 0.0, &cube);
    base.orientation = about_z;
    let mut top = Body::mesh(Vec3::new(0.0, 2.0 * reach + 0.5, 8.0), 1.0, &cube);
    top.orientation = Quat::from_axis_angle(Vec3::X, quarter);
    world.insert(base);
    world.insert(top);
    world
}

fn resting_pile() -> World {
    let mut world = World::new(Vec3::new(0.0, -9.81, 0.0));
    world.insert(Body::mesh(
        Vec3::new(0.0, -0.5, 0.0),
        0.0,
        &box_hull(Vec3::new(4.0, 0.5, 4.0)),
    ));
    let cube = box_hull(Vec3::new(0.5, 0.5, 0.5));
    let mut angle = 0.0_f32;
    for layer in [0.6_f32, 1.8, 3.0, 4.2] {
        for (x, z) in [(-0.6_f32, -0.6_f32), (0.6, -0.6), (-0.6, 0.6), (0.6, 0.6)] {
            let mut body = Body::mesh(Vec3::new(x, layer, z), 1.0, &cube);
            body.orientation = Quat::from_axis_angle(Vec3::Y, angle);
            angle += 0.21;
            world.insert(body);
        }
    }
    world
}

fn static_grid_10k() -> World {
    let mut world = World::new(Vec3::new(0.0, -9.81, 0.0));
    let mut boxes = Vec::with_capacity(10_000);
    let mut x = -99.0;
    for _ in 0..100 {
        let mut z = -99.0;
        for _ in 0..100 {
            boxes.push(Body::cuboid(
                Vec3::new(x, 0.0, z),
                0.0,
                Vec3::new(0.9, 0.5, 0.9),
            ));
            z += 2.0;
        }
        x += 2.0;
    }
    world.insert_many(boxes);
    let mut x = -7.0;
    for _ in 0..8 {
        world.insert(Body::sphere(Vec3::new(x, 4.0, 1.0), 1.0, 0.5));
        x += 2.0;
    }
    world
}

fn resting_1025() -> World {
    let mut world = World::new(Vec3::new(0.0, -9.81, 0.0));
    world.insert(Body::plane(Vec3::Y, 0.0));
    for x in 0..32 {
        for z in 0..32 {
            world.insert(Body::sphere(
                Vec3::new(x as f32 * 1.2, 0.5, z as f32 * 1.2),
                1.0,
                0.5,
            ));
        }
    }
    world
}

fn layer_filter() -> World {
    let mut world = World::new(Vec3::ZERO);
    world.insert(Body::sphere(Vec3::new(0.0, 1.0, 0.0), 1.0, 0.5).with_layer(2));
    world.insert(Body::sphere(Vec3::new(0.2, 1.0, 0.0), 1.0, 0.5).with_layer(2));
    world
}

fn sleep_island() -> World {
    let mut world = World::new(Vec3::new(0.0, -9.81, 0.0));
    world.insert(Body::plane(Vec3::Y, 0.0));
    for x in [0.0_f32, 6.0] {
        for level in 0..2 {
            let mut body = Body::cuboid(
                Vec3::new(x, 0.5 + level as f32, 0.0),
                1.0,
                Vec3::new(0.5, 0.5, 0.5),
            );
            body.friction = 0.8;
            world.insert(body);
        }
    }
    world
}
