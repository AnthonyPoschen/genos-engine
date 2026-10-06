//! The seeded particle step, the card builder, and the fog integral. No window and no Vulkan.

use genos_load::{load_texture, load_texture_map, FileSource};
use genos_render::{
    build, card_quad, compose_fog, fire_radiance, fog_along, frame_at, illuminate_facing,
    image_from_map, image_from_texture, medium, object_cards, sample, shade_lit, DrawKind, Emitter,
    FireLight, FogLamp, Kind, Puff, Simulation, World, FIRE_COLOR, PROOF_STEPS, STEP_DT,
};
use genos_scene::{load_path, Floor, Light, Scene, Vec3};
use std::path::PathBuf;

fn scene_file() -> genos_scene::Scene {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/camera/scene.rhai");
    load_path(&path).expect("camera scene")
}

fn asset(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join(name)
}

fn room(lamp: [f32; 3]) -> Scene {
    Scene {
        floor: Floor {
            position: Vec3::new(0.0, 0.0, 0.0),
            half_x: 3.0,
            half_z: 3.0,
            color: [1.0, 1.0, 1.0],
        },
        walls: Vec::new(),
        solids: Vec::new(),
        lights: vec![Light {
            position: Vec3::new(0.0, 4.0, 0.0),
            color: lamp,

            direction: Vec3::ZERO,
        }],
    }
}

fn run(steps: u32) -> Simulation {
    run_sim(steps)
}

fn run_sim(steps: u32) -> Simulation {
    let mut sim = Simulation::from_scene(&scene_file());
    for _ in 0..steps {
        sim.step(STEP_DT);
    }
    sim
}

fn lit_color(scene: &Scene, albedo: [f32; 3]) -> [f32; 3] {
    let normal = [0.0, 1.0, 0.0];
    let direct = illuminate_facing(scene, 0.0, 1.0, 0.0, normal);
    let field = build(scene);
    let bounce = sample(&field, 0.0, 0.0);
    shade_lit(albedo, direct, bounce, normal)
}

fn brightness(color: [f32; 3]) -> f32 {
    color[0] + color[1] + color[2]
}

fn saturation(color: [f32; 3]) -> f32 {
    let max = color[0].max(color[1]).max(color[2]);
    let min = color[0].min(color[1]).min(color[2]);
    if max <= 1.0e-4 {
        0.0
    } else {
        (max - min) / max
    }
}

#[test]
fn a_fresh_emitter_keeps_each_option_on_its_own_result() {
    let origin = [1.0, 2.0, -0.5];
    let mut radius = Emitter::at(origin);
    radius.spawn_radius = 0.45;
    radius.rise = 0.0;
    radius.drift = 0.0;
    radius.interval = 0.02;
    radius.capacity = 8;
    radius.angle = 0.2;
    let mut angled = radius.clone();
    angled.face_camera = false;
    angled.angle = 1.15;
    let mut sim_radius = Simulation::fresh(radius);
    let mut sim_angle = Simulation::fresh(angled);
    for _ in 0..24 {
        sim_radius.step(STEP_DT);
        sim_angle.step(STEP_DT);
    }
    let born = sim_radius.live();
    assert!(born.len() >= 3, "the radius emitter did not birth a spread");
    let mut separated = 0.0_f32;
    for particle in &born {
        let dx = particle.position[0] - origin[0];
        let dz = particle.position[2] - origin[2];
        let reach = (dx * dx + dz * dz).sqrt();
        assert!(
            reach <= 0.45 + 1.0e-4,
            "a birth left the spawn radius: {reach}"
        );
        assert!((particle.position[1] - origin[1]).abs() < 1.0e-4);
        separated = separated.max(reach);
    }
    assert!(separated > 0.05, "every birth sat on one point");
    assert_eq!(
        sim_radius
            .live()
            .iter()
            .map(|particle| particle.position)
            .collect::<Vec<_>>(),
        sim_angle
            .live()
            .iter()
            .map(|particle| particle.position)
            .collect::<Vec<_>>(),
        "the off angle moved the births"
    );

    let eye = [-4.0, 1.6, 3.0];
    let center = [0.0, 1.4, 0.0];
    let (facing_normal, _) = card_quad(center, eye, 0.4, true, 1.15);
    let (off_normal, _) = card_quad(center, eye, 0.4, false, 1.15);
    let to_eye = normalize([eye[0] - center[0], eye[1] - center[1], eye[2] - center[2]]);
    assert!(
        dot(facing_normal, to_eye) > 0.98,
        "the camera-facing normal missed the eye: {facing_normal:?}"
    );
    assert!(
        dot(off_normal, to_eye) < 0.55,
        "the off-angle normal still points at the eye: {off_normal:?}"
    );

    let mut facing = Emitter::at(center);
    facing.spawn_radius = 0.0;
    facing.rise = 0.0;
    facing.drift = 0.0;
    facing.interval = 10.0;
    facing.face_camera = true;
    facing.angle = 1.15;
    facing.albedo = [0.2, 0.5, 0.1];
    let mut off = facing.clone();
    off.face_camera = false;
    let mut world = World::from_scene(room([1.0, 1.0, 1.0]));
    let mut face_sim = Simulation::fresh(facing);
    face_sim.step(STEP_DT);
    face_sim.apply(&mut world, false, false);
    let face_cards = cards_of(&world, eye);
    world.objects.clear();
    let mut bare = World::from_scene(room([1.0, 1.0, 1.0]));
    let mut off_sim = Simulation::fresh(off);
    off_sim.step(STEP_DT);
    off_sim.apply(&mut bare, false, false);
    let off_cards = cards_of(&bare, eye);
    assert_eq!(face_cards.len(), 1);
    assert_eq!(off_cards.len(), 1);
    assert!(dot(face_cards[0].normal, to_eye) > 0.98);
    assert!(dot(off_cards[0].normal, to_eye) < 0.55);
    assert_eq!(face_cards[0].albedo, [0.2, 0.5, 0.1]);
    assert_ne!(face_cards[0].albedo, FIRE_COLOR);

    let bright = room([1.0, 1.0, 1.0]);
    let dim = room([0.04, 0.04, 0.04]);
    let albedo = [0.2, 0.5, 0.1];
    let lit = lit_color(&bright, albedo);
    let dark = lit_color(&dim, albedo);
    assert!(
        brightness(lit) > brightness(dark) + 0.02,
        "world light did not change the card: lit {lit:?} dark {dark:?}"
    );
    let black = lit_color(&bright, [0.0, 0.0, 0.0]);
    assert!(
        brightness(black) < 1.0e-5,
        "albedo zero did not stay black: {black:?}"
    );
    let once = lit_color(&bright, [0.02, 0.01, 0.004]);
    let twice = lit_color(&bright, [0.04, 0.02, 0.008]);
    for axis in 0..3 {
        assert!(
            (twice[axis] - once[axis] * 2.0).abs() < 1.0e-3,
            "the shade path did not multiply the albedo: once {once:?} twice {twice:?}"
        );
    }
    let custom = lit_color(&bright, albedo);
    let flame = lit_color(&bright, FIRE_COLOR);
    assert_ne!(
        custom, flame,
        "a custom albedo became the built-in flame color"
    );

    let mut glow = Emitter::at([0.3, 1.4, -0.2]);
    glow.spawn_radius = 0.0;
    glow.rise = 0.0;
    glow.drift = 0.0;
    glow.interval = 10.0;
    glow.emission = [0.9, 0.25, 0.04];
    glow.albedo = [0.0, 0.0, 0.0];
    let mut dark_emit = glow.clone();
    dark_emit.emission = [0.0, 0.0, 0.0];
    let mut world = World::from_scene(room([1.0, 1.0, 1.0]));
    let mut sim = Simulation::fresh(glow);
    sim.step(STEP_DT);
    sim.apply(&mut world, true, false);
    let (fire, _) = medium(&world, eye);
    let near = fire_radiance([0.3, -0.2], fire);
    let far = fire_radiance([4.0, 4.0], fire);
    assert!(
        near[0] > near[2] + 0.01,
        "the emitted light is not warm: {near:?}"
    );
    assert!(near[0] > far[0] * 2.0, "a far surface took the same light");
    let mut quiet = World::from_scene(room([1.0, 1.0, 1.0]));
    let mut off = Simulation::fresh(dark_emit);
    off.step(STEP_DT);
    off.apply(&mut quiet, true, false);
    let (none, _) = medium(&quiet, eye);
    assert_eq!(fire_radiance([0.3, -0.2], none), [0.0, 0.0, 0.0]);
    let black_card = lit_color(&bright, [0.0, 0.0, 0.0]);
    assert!(brightness(black_card) < 1.0e-5);
}

#[test]
fn a_loaded_texture_map_changes_frame_when_the_age_crosses_a_boundary() {
    let png = asset("ember.png");
    let map_file = asset("ember.map");
    let map = load_texture_map(&FileSource::new(&png), &FileSource::new(&map_file))
        .expect("ember texture map");
    let image = image_from_map(&map, "burn").expect("burn clip");
    let early = map.sample("burn", 0.0).expect("first frame");
    let later = map
        .sample("burn", image.frame_seconds + 0.01)
        .expect("next frame");
    assert_ne!(early, later, "the clip did not change frame");
    assert_eq!(frame_at(&image, 0.0).x, early.x);
    assert_eq!(frame_at(&image, 0.0).y, early.y);
    assert_eq!(frame_at(&image, image.frame_seconds + 0.01).x, later.x);

    let texture = load_texture(&FileSource::new(&png), "ember").expect("ember image");
    let whole = image_from_texture(&texture);
    assert_eq!(whole.pixels, texture.image.pixels);

    let mut emitter = Emitter::at([0.0, 1.2, 0.0]);
    emitter.interval = 10.0;
    emitter.rise = 0.0;
    emitter.drift = 0.0;
    emitter.image = Some(image.clone());
    let mut sim = Simulation::fresh(emitter);
    sim.step(0.001);
    let mut world = World::from_scene(room([1.0, 1.0, 1.0]));
    sim.apply(&mut world, false, false);
    let young = cards_of(&world, [0.0, 2.0, 4.0]);
    assert_eq!(young.len(), 1);
    assert!(young[0].textured);
    assert_eq!(young[0].frame.x, early.x);
    assert_eq!(young[0].frame.width, early.width);

    sim.step(image.frame_seconds + 0.01);
    sim.apply(&mut world, false, false);
    let aged = cards_of(&world, [0.0, 2.0, 4.0]);
    let oldest = aged
        .iter()
        .max_by(|a, b| a.frame.x.cmp(&b.frame.x))
        .expect("aged card");
    assert_eq!(oldest.frame.x, later.x);
    assert_ne!(young[0].frame, oldest.frame);

    let mut plain = Emitter::at([0.2, 1.0, 0.2]);
    plain.interval = 10.0;
    plain.image = Some(whole);
    let mut sim = Simulation::fresh(plain);
    sim.step(0.001);
    let mut world = World::from_scene(room([1.0, 1.0, 1.0]));
    sim.apply(&mut world, false, false);
    let stored = particle_image(&world).expect("loaded image");
    assert_eq!(stored, texture.image.pixels);
}

#[test]
fn a_seeded_fire_steps_retires_and_gives_off_smoke() {
    let once = run(1);
    let born = once.live();
    assert!(
        born.iter().any(|particle| particle.kind == Kind::Flame),
        "the stationary flame is missing"
    );
    assert!(
        born.iter().any(|particle| particle.kind == Kind::Fire),
        "the first step births no embers"
    );
    assert!(born.iter().all(|particle| particle.age < particle.life));

    let scene = scene_file();
    let red = scene.solid_by_color([1.0, 0.0, 0.0]).expect("red solid");
    for particle in &born {
        assert!(particle.position[1] > red.height);
    }

    let steady = run(PROOF_STEPS);
    assert!(steady.retired() > 0, "nothing retired");
    assert!(steady.born() > steady.live().len() as u32);
    assert!(steady
        .live()
        .iter()
        .all(|particle| particle.age < particle.life));
    assert!(steady
        .live()
        .iter()
        .any(|particle| particle.kind == Kind::Fire));
    assert!(steady
        .live()
        .iter()
        .any(|particle| particle.kind == Kind::Smoke));

    let again = run(PROOF_STEPS);
    assert_eq!(steady.live(), again.live());
    assert_eq!(steady.retired(), again.retired());
}

#[test]
fn the_camera_scene_puts_embers_and_a_smoke_trail_above_the_red_box() {
    let scene = scene_file();
    let mut world = World::from_scene(scene);
    let mut sim = Simulation::from_scene(&world.scene);
    for _ in 0..PROOF_STEPS {
        sim.advance(&mut world, STEP_DT);
    }
    let red = world
        .scene
        .solid_by_color([1.0, 0.0, 0.0])
        .expect("red solid");
    let (embers, smoke) = split(&world);
    assert!(!embers.is_empty() && !smoke.is_empty());
    let mut spread = 0.0_f32;
    for point in &embers {
        assert!(
            point[1] > red.height,
            "an ember is not above the top: {point:?}"
        );
        spread = spread.max((point[0] - red.position.x).abs());
        spread = spread.max((point[2] - red.position.z).abs());
    }
    assert!(spread > 0.04, "the embers sit on one point");
    let flames: Vec<_> = sim
        .live()
        .into_iter()
        .filter(|particle| particle.kind == Kind::Flame)
        .collect();
    assert_eq!(flames.len(), 3, "the stationary fire is missing");
    let flame_y = flames
        .iter()
        .map(|particle| particle.position[1])
        .sum::<f32>()
        / flames.len() as f32;
    let (fire, _) = medium(&world, [-6.0, 1.7, 6.0]);
    assert!(
        (fire.position[1] - flame_y).abs() < 0.05,
        "the light is not at the flame center: light {:?} center {flame_y}",
        fire.position
    );
    assert!(
        fire.position[1] > red.height + 0.3,
        "the light sits on the base of the flame: {:?}",
        fire.position
    );
    let early = run_sim(8);
    let late = run_sim(PROOF_STEPS);
    let early_flames: Vec<_> = early
        .live()
        .into_iter()
        .filter(|particle| particle.kind == Kind::Flame)
        .collect();
    let late_flames: Vec<_> = late
        .live()
        .into_iter()
        .filter(|particle| particle.kind == Kind::Flame)
        .collect();
    for (start, end) in early_flames.iter().zip(&late_flames) {
        assert_eq!(start.position, end.position, "the stationary flame moved");
        assert!(
            end.age > start.age + 0.5,
            "the stationary flame did not animate"
        );
    }
    let max_ember = embers.iter().map(|point| point[1]).fold(0.0_f32, f32::max);
    let max_smoke = smoke.iter().map(|point| point[1]).fold(0.0_f32, f32::max);
    assert!(
        max_smoke > max_ember + 0.05,
        "the smoke trail does not continue above the embers: smoke {max_smoke} embers {max_ember}"
    );
    let green = world.scene.solid_by_color([0.0, 1.0, 0.0]).expect("green");
    let blue = world.scene.solid_by_color([0.0, 0.0, 1.0]).expect("blue");
    for point in embers.iter().chain(smoke.iter()) {
        let near_green = (point[0] - green.position.x).hypot(point[2] - green.position.z);
        let near_blue = (point[0] - blue.position.x).hypot(point[2] - blue.position.z);
        assert!(
            near_green > 1.0 && near_blue > 1.0,
            "a particle landed on another solid: {point:?}"
        );
    }
    assert_eq!(world.light_scene().solids.len(), 3);

    let mut again = World::from_scene(world.scene.clone());
    let mut second = Simulation::from_scene(&again.scene);
    for _ in 0..PROOF_STEPS {
        second.advance(&mut again, STEP_DT);
    }
    assert_eq!(split(&world), split(&again));
    assert_eq!(sim.live(), second.live());
}

#[test]
fn fog_darkens_a_lit_ray_and_fire_warms_the_ground() {
    let eye = [-6.0, 1.7, 6.0];
    let puff = Puff {
        center: [0.0, 2.8, 0.0],
        radius: 0.42,
        density: 1.7,
    };
    let clear_puff = Puff {
        density: 0.0,
        ..puff
    };
    let lamps = [FogLamp {
        position: [0.0, 7.0, 0.0],
        color: [1.0, 1.0, 1.0],
    }];
    let fire = FireLight {
        position: [0.0, 1.8, 0.0],
        color: FIRE_COLOR,
        strength: 0.55,
    };
    let field = [0.22, 0.06, 0.02];
    let dir = [0.0 - eye[0], 2.8 - eye[1], 0.0 - eye[2]];
    let through = fog_along(eye, dir, &[puff], &lamps, fire, field);
    let removed = fog_along(eye, dir, &[clear_puff], &lamps, fire, field);
    let background = [0.75, 0.22, 0.16];
    let with_smoke = compose_fog(background, through);
    let without = compose_fog(background, removed);
    assert!(
        through.optical > 0.4 && removed.optical < 1.0e-4,
        "optical depth did not follow the puff: {through:?} {removed:?}"
    );
    assert!(
        brightness(with_smoke) + 0.02 < brightness(without)
            || saturation(with_smoke) + 0.05 < saturation(without),
        "smoke did not darken or gray the ray: with {with_smoke:?} without {without:?}"
    );
    let miss = fog_along(eye, [0.0, -1.0, 0.0], &[puff], &lamps, fire, field);
    let missed = compose_fog(background, miss);
    assert!(
        (missed[0] - background[0]).abs() < 1.0e-4
            && (missed[1] - background[1]).abs() < 1.0e-4
            && (missed[2] - background[2]).abs() < 1.0e-4,
        "a ray that misses the puff changed the background: {missed:?}"
    );
    let unlit = fog_along(eye, dir, &[puff], &[], FireLight::off(), [0.0; 3]);
    assert!(
        brightness(unlit.inscatter) + 0.05 < brightness(through.inscatter),
        "removing the lamp and the fire did not darken the smoke: lit {:?} dark {:?}",
        through.inscatter,
        unlit.inscatter
    );

    let near = fire_radiance([-1.1, 0.85], fire);
    let far = fire_radiance([-5.2, 4.2], fire);
    let off = fire_radiance([-1.1, 0.85], FireLight::off());
    assert!(near[0] > near[2] + 0.02, "fire is not warm: {near:?}");
    assert!(
        near[0] > far[0] * 4.0,
        "far floor took the same fire: near {near:?} far {far:?}"
    );
    assert_eq!(off, [0.0; 3]);
}

fn cards_of(world: &World, eye: [f32; 3]) -> Vec<genos_render::Card> {
    let mut cards = Vec::new();
    for object in &world.objects {
        cards.extend(object_cards(object, eye));
    }
    cards
}

fn particle_image(world: &World) -> Option<Vec<u8>> {
    for object in &world.objects {
        if let DrawKind::Particles {
            image: Some(image), ..
        } = &object.kind
        {
            return Some(image.pixels.clone());
        }
    }
    None
}

fn split(world: &World) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let mut embers = Vec::new();
    let mut smoke = Vec::new();
    for object in &world.objects {
        if let DrawKind::Particles { points, lit, .. } = &object.kind {
            if *lit {
                smoke.extend(points.iter().copied());
            } else {
                embers.extend(points.iter().copied());
            }
        }
    }
    (embers, smoke)
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1.0e-6);
    [v[0] / len, v[1] / len, v[2] / len]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
