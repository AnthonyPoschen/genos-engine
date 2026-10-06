use genos_math::Vec3;
use genos_physics::{step, Body, Piece, Shape, World};

fn cylinder(radius: f32, height: f32, sides: u32) -> Vec<[Vec3; 3]> {
    let mut triangles = Vec::new();
    for i in 0..sides {
        let a0 = i as f32 / sides as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / sides as f32 * std::f32::consts::TAU;
        let p0 = Vec3::new(a0.cos() * radius, 0.0, a0.sin() * radius);
        let p1 = Vec3::new(a1.cos() * radius, 0.0, a1.sin() * radius);
        let p2 = Vec3::new(p1.x, height, p1.z);
        let p3 = Vec3::new(p0.x, height, p0.z);
        let top = Vec3::new(0.0, height, 0.0);
        triangles.push([p0, p1, p2]);
        triangles.push([p0, p2, p3]);
        triangles.push([top, p3, p2]);
        triangles.push([Vec3::ZERO, p1, p0]);
    }
    triangles
}

#[test]
fn a_capsule_stops_against_a_round_mesh() {
    let triangles = cylinder(0.7, 1.2, 16);
    let shape = Shape::from_triangles(&triangles);
    assert!(
        shape.contact_faces() < triangles.len(),
        "faces {} from {}",
        shape.contact_faces(),
        triangles.len()
    );
    let mut capsule = Body::capsule(Vec3::new(0.0, 0.6, 1.4), 1.0, 0.3, 0.4);
    capsule.velocity = Vec3::new(0.0, 0.0, -3.0);
    let mut world = World::new(Vec3::ZERO);
    assert!(world.insert(capsule));
    assert!(world.insert(Body::mesh(Vec3::ZERO, 0.0, &triangles)));
    for _ in 0..60 {
        world = step(&world, 1.0 / 60.0);
    }
    let z = world.bodies[0].position.z;
    assert!(z > 0.9, "capsule entered the round mesh at z {z}");
    assert!(z < 1.45, "capsule never met the round mesh at z {z}");
}

fn within(got: f32, expected: f32, tol: f32) {
    let error = (got - expected).abs();
    assert!(error <= tol, "got {got} expected {expected} error {error}");
}

#[test]
fn an_overlapping_capsule_and_box_separate_along_the_contact() {
    let mut capsule = Body::capsule(Vec3::new(0.6, 0.0, 0.0), 1.0, 0.25, 0.4);
    capsule.velocity = Vec3::new(-1.5, 0.0, 0.0);
    let wall = Body::cuboid(Vec3::ZERO, 0.0, Vec3::new(0.5, 0.5, 0.5));
    let mut world = World::new(Vec3::ZERO);
    assert!(world.insert(capsule));
    assert!(world.insert(wall));
    for _ in 0..30 {
        world = step(&world, 1.0 / 60.0);
    }
    let center = world.bodies[0].position.x;
    assert!(
        center > 0.7,
        "capsule passed into the box, center x {center}"
    );
    assert!(center < 1.2, "capsule left the contact, center x {center}");
    assert!(
        world.bodies[0].velocity.x > -0.05,
        "still moving through the box at {}",
        world.bodies[0].velocity.x
    );
}

#[test]
fn a_capsule_rests_on_a_static_box() {
    let capsule = Body::capsule(Vec3::new(0.0, 2.2, 0.0), 1.0, 0.25, 0.4);
    let stand = Body::cuboid(Vec3::new(0.0, 0.5, 0.0), 0.0, Vec3::new(1.0, 0.5, 1.0));
    let mut world = World::new(genos_physics::GRAVITY);
    assert!(world.insert(capsule));
    assert!(world.insert(stand));
    for _ in 0..240 {
        world = step(&world, 1.0 / 60.0);
    }
    let bottom = world.bodies[0].position.y - 0.65;
    within(bottom, 1.0, 0.05);
    assert!(
        world.bodies[0].velocity.y.abs() < 0.2,
        "still falling at {}",
        world.bodies[0].velocity.y
    );
}

#[test]
fn a_box_rests_on_a_static_plane() {
    let block = Body::cuboid(Vec3::new(0.2, 2.0, -0.4), 2.0, Vec3::new(0.3, 0.5, 0.4));
    let floor = Body::plane(Vec3::Y, 0.0);
    let mut world = World::new(genos_physics::GRAVITY);
    assert!(world.insert(block));
    assert!(world.insert(floor));
    for _ in 0..240 {
        world = step(&world, 1.0 / 60.0);
    }
    let bottom = world.bodies[0].position.y - 0.5;
    within(bottom, 0.0, 0.05);
    assert!(world.bodies[0].position.y > 0.4, "box passed the plane");
}

fn l_triangles() -> Vec<[Vec3; 3]> {
    let mut triangles = Vec::new();
    add_grid(
        &mut triangles,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::X * 2.0,
        Vec3::Z,
        8,
        false,
    );
    add_grid(
        &mut triangles,
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::X * 2.0,
        Vec3::Z,
        8,
        true,
    );
    add_grid(
        &mut triangles,
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::X,
        Vec3::Z,
        8,
        false,
    );
    add_grid(
        &mut triangles,
        Vec3::new(0.0, 1.0, 1.0),
        Vec3::X,
        Vec3::Z,
        8,
        true,
    );
    add_wall(
        &mut triangles,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::X * 2.0,
        Vec3::Y,
        8,
    );
    add_wall(
        &mut triangles,
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::Z,
        Vec3::Y,
        8,
    );
    add_wall(
        &mut triangles,
        Vec3::new(2.0, 0.0, 1.0),
        Vec3::X * -1.0,
        Vec3::Y,
        8,
    );
    add_wall(
        &mut triangles,
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::Z,
        Vec3::Y,
        8,
    );
    add_wall(
        &mut triangles,
        Vec3::new(1.0, 0.0, 2.0),
        Vec3::X * -1.0,
        Vec3::Y,
        8,
    );
    add_wall(
        &mut triangles,
        Vec3::new(0.0, 0.0, 2.0),
        Vec3::Z * -2.0,
        Vec3::Y,
        8,
    );
    triangles
}

fn add_grid(
    out: &mut Vec<[Vec3; 3]>,
    origin: Vec3,
    axis_u: Vec3,
    axis_v: Vec3,
    cells: i32,
    flip: bool,
) {
    for v in 0..cells {
        for u in 0..cells {
            let p00 =
                origin + axis_u * (u as f32 / cells as f32) + axis_v * (v as f32 / cells as f32);
            let p10 = origin
                + axis_u * ((u + 1) as f32 / cells as f32)
                + axis_v * (v as f32 / cells as f32);
            let p11 = origin
                + axis_u * ((u + 1) as f32 / cells as f32)
                + axis_v * ((v + 1) as f32 / cells as f32);
            let p01 = origin
                + axis_u * (u as f32 / cells as f32)
                + axis_v * ((v + 1) as f32 / cells as f32);
            if flip {
                out.push([p00, p01, p11]);
                out.push([p00, p11, p10]);
            } else {
                out.push([p00, p11, p01]);
                out.push([p00, p10, p11]);
            }
        }
    }
}

fn add_wall(out: &mut Vec<[Vec3; 3]>, origin: Vec3, across: Vec3, up: Vec3, cells: i32) {
    add_grid(out, origin, across, up, cells, true);
}

#[test]
fn a_dense_concave_mesh_blocks_only_its_volume() {
    let triangles = l_triangles();
    assert!(
        triangles.len() > 200,
        "mesh was not dense: {}",
        triangles.len()
    );
    let shape = Shape::from_triangles(&triangles);
    assert!(
        shape.contact_faces() * 4 < triangles.len(),
        "reduced faces {} from {}",
        shape.contact_faces(),
        triangles.len()
    );
    assert!(
        shape.contact_pieces() < 6,
        "pieces {}",
        shape.contact_pieces()
    );

    let mut mover = Body::sphere(Vec3::new(1.0, 0.5, -0.8), 1.0, 0.15);
    mover.velocity = Vec3::new(0.0, 0.0, 3.0);
    let mut world = World::new(Vec3::ZERO);
    assert!(world.insert(mover));
    assert!(world.insert(Body::mesh(Vec3::ZERO, 0.0, &triangles)));
    for _ in 0..90 {
        world = step(&world, 1.0 / 60.0);
    }
    let z = world.bodies[0].position.z;
    assert!(z < 0.25, "sphere passed through the solid, z {z}");
    assert!(z > -0.5, "sphere never reached the solid, z {z}");

    let mut clear = Body::sphere(Vec3::new(1.6, 0.5, 1.6), 1.0, 0.1);
    clear.velocity = Vec3::new(3.0, 0.0, 0.0);
    let mut open = World::new(Vec3::ZERO);
    assert!(open.insert(clear));
    assert!(open.insert(Body::mesh(Vec3::ZERO, 0.0, &triangles)));
    for _ in 0..60 {
        open = step(&open, 1.0 / 60.0);
    }
    let x = open.bodies[0].position.x;
    assert!(
        x > 2.4,
        "empty notch blocked the sphere, x {x} pieces {} faces {}",
        shape.contact_pieces(),
        shape.contact_faces()
    );
}

fn outward_box(half: f32) -> Vec<[Vec3; 3]> {
    let h = half;
    let nnn = Vec3::new(-h, -h, -h);
    let pnn = Vec3::new(h, -h, -h);
    let ppn = Vec3::new(h, h, -h);
    let npn = Vec3::new(-h, h, -h);
    let nnp = Vec3::new(-h, -h, h);
    let pnp = Vec3::new(h, -h, h);
    let ppp = Vec3::new(h, h, h);
    let npp = Vec3::new(-h, h, h);
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
fn a_sphere_and_capsule_stop_outside_every_face_of_a_convex_box_mesh() {
    let half = 0.5;
    let radius = 0.2;
    let triangles = outward_box(half);
    let shape = Shape::from_triangles(&triangles);
    let Shape::Mesh(mesh) = shape else {
        panic!("convex box was not a mesh");
    };
    assert_eq!(mesh.count, 1, "convex box split into {}", mesh.count);
    let Piece::Hull(hull) = mesh.pieces[0] else {
        panic!("convex box was voxelized instead of a hull");
    };
    assert_eq!(
        hull.face_count, 12,
        "cube hull stored {} triangles",
        hull.face_count
    );
    for axis in [
        Vec3::X,
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::Y,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::Z,
        Vec3::new(0.0, 0.0, -1.0),
    ] {
        assert!(
            hull.faces[..hull.face_count as usize]
                .iter()
                .any(|face| face.normal.dot(axis) > 0.9),
            "hull dropped the {axis:?} plane"
        );
    }

    let approaches = [
        (Vec3::X, Vec3::new(2.0, 0.0, 0.0), Vec3::new(-3.0, 0.0, 0.0)),
        (
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(-2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
        ),
        (Vec3::Y, Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, -3.0, 0.0)),
        (
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.0, -2.0, 0.0),
            Vec3::new(0.0, 3.0, 0.0),
        ),
        (Vec3::Z, Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, -3.0)),
        (
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, -2.0),
            Vec3::new(0.0, 0.0, 3.0),
        ),
    ];
    for (outward, start, velocity) in approaches {
        rest_against_hull(&triangles, outward, start, velocity, radius, 0.0);
        rest_against_hull(&triangles, outward, start, velocity, radius, 0.25);
    }
}

fn rest_against_hull(
    triangles: &[[Vec3; 3]],
    outward: Vec3,
    start: Vec3,
    velocity: Vec3,
    radius: f32,
    half_height: f32,
) {
    let half = 0.5;
    let extent = if outward.y.abs() > 0.5 {
        radius + half_height
    } else {
        radius
    };
    let mut body = if half_height > 0.0 {
        Body::capsule(start, 1.0, radius, half_height)
    } else {
        Body::sphere(start, 1.0, radius)
    };
    body.velocity = velocity;
    let mut world = World::new(Vec3::ZERO);
    assert!(world.insert(body));
    assert!(world.insert(Body::mesh(Vec3::ZERO, 0.0, triangles)));
    for _ in 0..120 {
        world = step(&world, 1.0 / 60.0);
    }
    let pos = world.bodies[0].position;
    let along = pos.dot(outward);
    let name = if half_height > 0.0 {
        "capsule"
    } else {
        "sphere"
    };
    assert!(
        along > half + extent - 0.05,
        "{name} entered the {outward:?} face at {pos:?}"
    );
    assert!(
        along < half + extent + 0.15,
        "{name} never met the {outward:?} face at {pos:?}"
    );
    let side = pos - outward * along;
    assert!(
        side.length() < 0.15,
        "{name} left the {outward:?} face at {pos:?}"
    );
    assert!(
        world.bodies[0].velocity.dot(outward) > -0.05,
        "{name} still moves into the {outward:?} face at {:?}",
        world.bodies[0].velocity
    );
}
