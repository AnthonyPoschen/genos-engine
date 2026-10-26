//! Measures CCD metrics by running `genos_physics::step`.
//!
//! `good` and `bad` are the same 3 000 m/s scene. `good` asks for protection:
//! the bullet is speculative and both ships are swept, so `tunnel_count` is 0.
//! `bad` leaves protection off and still tunnels. `discrete` is the 30 m/s run,
//! scored on free-flight steps. `loss` scores that run from start to end, so
//! the inelastic drop fails `|energy_drift| <= 0.01`.
//!
//! ```text
//! --out DIR
//! --fixture proof|good|bad|discrete|loss
//! ```
//!
//! `--out` defaults to `./genos-debug/physics`. `GENOS_PHYSICS_METRICS_OUT`
//! and `GENOS_PHYSICS_METRICS_FIXTURE` mirror those arguments and apply only
//! when the argument was omitted.

use std::path::{Path, PathBuf};
use std::process::Command;

use genos_physics::{Body, MotionQuality, Shape, World};

const DEFAULT_OUT: &str = "genos-debug/physics";
const DT: f32 = 1.0 / 60.0;
const STEPS: u32 = 8;
/// Ships at 30 m/s still overlap on the step they meet, so the discrete solver stops them.
const CATCH_SHIP: f32 = 30.0;
/// A 6 m/s bullet overlaps the wall instead of leaping it.
const CATCH_BULLET: f32 = 6.0;
const TUNNEL_SPEED: f32 = 3000.0;
const PENETRATION_LIMIT: f64 = 0.01;

#[derive(Clone, Copy)]
enum Bound {
    AtMost,
    AtLeast,
    AbsAtMost,
}

struct Spec {
    name: &'static str,
    bound: Bound,
    threshold: f64,
    threshold_text: &'static str,
}

const METRICS: &[Spec] = &[
    Spec {
        name: "tunnel_count",
        bound: Bound::AtMost,
        threshold: 0.0,
        threshold_text: "<=0",
    },
    Spec {
        name: "penetration_m",
        bound: Bound::AtMost,
        threshold: PENETRATION_LIMIT,
        threshold_text: "<=0.01",
    },
    Spec {
        name: "energy_drift",
        bound: Bound::AbsAtMost,
        threshold: 0.01,
        threshold_text: "|drift|<=0.01",
    },
    Spec {
        name: "determinism_hash_match",
        bound: Bound::AtLeast,
        threshold: 1.0,
        threshold_text: ">=1",
    },
    Spec {
        name: "contacts_stored",
        bound: Bound::AtLeast,
        threshold: 1.0,
        threshold_text: ">=1",
    },
    Spec {
        name: "toi_deferred",
        bound: Bound::AtMost,
        threshold: 0.0,
        threshold_text: "<=0",
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
        }
    }
}

struct Row {
    name: &'static str,
    value: f64,
    value_text: String,
    threshold_text: &'static str,
    status: Status,
}

enum Fixture {
    Proof,
    Good,
    Bad,
    Discrete,
    Loss,
}

impl Fixture {
    fn name(&self) -> &'static str {
        match self {
            Self::Proof => "proof",
            Self::Good => "good",
            Self::Bad => "bad",
            Self::Discrete => "discrete",
            Self::Loss => "loss",
        }
    }
}

/// How a fixture is built. `good` and `bad` share the 3 000 m/s speeds.
#[derive(Clone, Copy)]
struct Setup {
    ship_speed: f32,
    bullet_speed: f32,
    protect: bool,
    /// Score `(E_end - E_start)`, which keeps the inelastic drop.
    /// Otherwise sum only the steps whose velocity change is gravity.
    full_energy: bool,
}

struct Scene {
    world: World,
    left: usize,
    right: usize,
    bullet: usize,
    wall: usize,
    ship_speed: f32,
    bullet_speed: f32,
}

struct Reading {
    tunnel_count: f64,
    penetration_m: f64,
    energy_drift: f64,
    determinism_hash_match: f64,
    contacts_stored: f64,
    toi_deferred: f64,
    ship_speed: f32,
    bullet_speed: f32,
    protect: bool,
    full_energy: bool,
    ccd_in_step: bool,
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("metrics: {err}");
            2
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32, String> {
    let args = parse_args()?;
    if args.help {
        print_help();
        return Ok(0);
    }
    if args.list {
        println!("metrics: test");
        return Ok(0);
    }
    if args.skip {
        return Ok(0);
    }
    match args.fixture {
        Fixture::Proof => prove(&args.out),
        Fixture::Good | Fixture::Bad | Fixture::Discrete | Fixture::Loss => {
            let reading = measure(setup(&args.fixture)?)?;
            let rows = judge_all(&reading);
            write_report(&args.out, args.fixture.name(), &reading, &rows)?;
            print_scene(&reading);
            print_rows(&rows);
            Ok(exit_code(&rows))
        }
    }
}

struct Args {
    out: PathBuf,
    fixture: Fixture,
    help: bool,
    list: bool,
    skip: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut out_arg = None;
    let mut fixture_arg = None;
    let mut help = false;
    let mut list = false;
    let mut exact = false;
    let mut filters = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => help = true,
            "--list" => list = true,
            "--exact" => exact = true,
            "--quiet" | "--nocapture" | "--show-output" | "--ignored" | "--include-ignored" => {}
            "--test-threads" | "--skip" | "--color" | "--format" => {
                let _ = args.next();
            }
            "--out" => out_arg = Some(need(&mut args, "--out")?),
            "--fixture" => fixture_arg = Some(need(&mut args, "--fixture")?),
            other
                if other.starts_with("--test-threads=")
                    || other.starts_with("--skip=")
                    || other.starts_with("--color=")
                    || other.starts_with("--format=") => {}
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => filters.push(other.to_string()),
        }
    }
    let skip = cargo_filter_skips(&filters, exact);
    let out = if let Some(dir) = out_arg {
        dir
    } else if let Some(dir) = env_text("GENOS_PHYSICS_METRICS_OUT") {
        dir
    } else {
        default_out()
    };
    if out.is_empty() || out.chars().all(char::is_whitespace) {
        return Err("--out is empty".into());
    }
    let fixture = if let Some(name) = fixture_arg {
        parse_fixture(&name)?
    } else if let Some(name) = env_text("GENOS_PHYSICS_METRICS_FIXTURE") {
        parse_fixture(&name)?
    } else {
        Fixture::Proof
    };
    Ok(Args {
        out: PathBuf::from(out),
        fixture,
        help,
        list,
        skip,
    })
}

/// `cargo test <filter>` forwards the filter to this binary. A filter that does
/// not name `metrics` skips the run so the rest of the package can still be selected.
fn cargo_filter_skips(filters: &[String], exact: bool) -> bool {
    if filters.is_empty() {
        return false;
    }
    let named = if exact {
        filters.iter().any(|filter| filter == "metrics")
    } else {
        filters
            .iter()
            .any(|filter| "metrics".contains(filter.as_str()))
    };
    !named
}

fn need(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "these variables only mirror --out and --fixture, and only when that argument was omitted"
)]
fn env_text(key: &str) -> Option<String> {
    let Ok(value) = std::env::var(key) else {
        return None;
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// `cargo test` starts in the package directory. The default still writes to
/// the repository `genos-debug/physics`. An explicit `--out` or env path stays
/// relative to the process directory.
fn default_out() -> String {
    let Ok(here) = std::env::current_dir() else {
        return DEFAULT_OUT.to_string();
    };
    let Ok(text) = std::fs::read_to_string(here.join("Cargo.toml")) else {
        return DEFAULT_OUT.to_string();
    };
    if text
        .lines()
        .any(|line| line.trim() == "name = \"genos-physics\"")
    {
        here.join("../../genos-debug/physics")
            .to_string_lossy()
            .into_owned()
    } else {
        DEFAULT_OUT.to_string()
    }
}

fn parse_fixture(text: &str) -> Result<Fixture, String> {
    match text {
        "proof" => Ok(Fixture::Proof),
        "good" => Ok(Fixture::Good),
        "bad" => Ok(Fixture::Bad),
        "discrete" => Ok(Fixture::Discrete),
        "loss" => Ok(Fixture::Loss),
        other => Err(format!("unknown fixture {other}")),
    }
}

fn setup(fixture: &Fixture) -> Result<Setup, String> {
    match fixture {
        Fixture::Good => Ok(Setup {
            ship_speed: TUNNEL_SPEED,
            bullet_speed: TUNNEL_SPEED,
            protect: true,
            full_energy: false,
        }),
        Fixture::Bad => Ok(Setup {
            ship_speed: TUNNEL_SPEED,
            bullet_speed: TUNNEL_SPEED,
            protect: false,
            full_energy: false,
        }),
        Fixture::Discrete => Ok(Setup {
            ship_speed: CATCH_SHIP,
            bullet_speed: CATCH_BULLET,
            protect: false,
            full_energy: false,
        }),
        Fixture::Loss => Ok(Setup {
            ship_speed: CATCH_SHIP,
            bullet_speed: CATCH_BULLET,
            protect: false,
            full_energy: true,
        }),
        Fixture::Proof => Err("proof does not score itself".into()),
    }
}

fn print_help() {
    println!(
        "\
--out DIR
    JSON directory. Default ./genos-debug/physics.
    GENOS_PHYSICS_METRICS_OUT applies only when --out is omitted.
--fixture proof|good|bad|discrete|loss
    proof runs all four. Default proof.
    GENOS_PHYSICS_METRICS_FIXTURE applies only when --fixture is omitted.
good and bad both run ships and bullet at {TUNNEL_SPEED} m/s.
    good marks the bullet speculative and both ships swept. bad leaves protection off, so the ships tunnel.
discrete runs ships at {CATCH_SHIP} m/s and the bullet at {CATCH_BULLET} m/s.
    energy_drift on good, bad, and discrete sums free-flight steps only.
loss is the discrete scene scored from start to end, so an inelastic drop fails.
Each fixture calls step {STEPS} times at 1/60 s.
Exit 0 when no metric is FAIL. Exit 1 when a metric is FAIL. Exit 2 on a bad argument."
    );
}

fn measure(setup: Setup) -> Result<Reading, String> {
    let first = simulate(&setup)?;
    let second = simulate(&setup)?;
    let match_bit = u8::from(hash_state(&first.world) == hash_state(&second.world));
    Ok(Reading {
        tunnel_count: first.tunnel_count,
        penetration_m: first.penetration_m,
        energy_drift: first.energy_drift,
        determinism_hash_match: f64::from(match_bit),
        contacts_stored: first.contacts_stored,
        toi_deferred: first.toi_deferred,
        ship_speed: first.ship_speed,
        bullet_speed: first.bullet_speed,
        protect: setup.protect,
        full_energy: setup.full_energy,
        ccd_in_step: first.ccd_in_step,
    })
}

struct Simulated {
    world: World,
    tunnel_count: f64,
    penetration_m: f64,
    energy_drift: f64,
    contacts_stored: f64,
    toi_deferred: f64,
    ship_speed: f32,
    bullet_speed: f32,
    ccd_in_step: bool,
}

fn simulate(setup: &Setup) -> Result<Simulated, String> {
    let mut scene = build(setup);
    let start = energy(&scene.world);
    let mut ships_tunneled = false;
    let mut bullet_tunneled = false;
    let mut deepest = 0.0_f32;
    let mut free_delta = 0.0;
    let mut toi_failed = 0_u32;
    let mut toi_deferred = 0_u32;
    let mut contacts_stored = false;
    for _ in 0..STEPS {
        let before = velocities(&scene.world);
        let energy_before = energy(&scene.world);
        genos_physics::step(&mut scene.world, DT);
        if free_flight(&before, &scene.world) {
            free_delta += energy(&scene.world) - energy_before;
        }
        ships_tunneled |= ships_exchanged(&scene)?;
        bullet_tunneled |= bullet_past_wall(&scene)?;
        deepest = deepest.max(penetration(&scene)?);
        toi_failed = toi_failed.saturating_add(scene.world.step_stats.toi_failed);
        toi_deferred = toi_deferred.saturating_add(scene.world.step_stats.toi_deferred);
        contacts_stored |= ships_stored_contact(&scene)?;
    }
    let end = energy(&scene.world);
    let scale = start.abs().max(1.0);
    let drift = if setup.full_energy {
        (end - start) / scale
    } else {
        free_delta / scale
    };
    let tunnels = u32::from(ships_tunneled) + u32::from(bullet_tunneled) + toi_failed;
    Ok(Simulated {
        ship_speed: scene.ship_speed,
        bullet_speed: scene.bullet_speed,
        ccd_in_step: scene.world.step_stats.ccd_applied,
        world: scene.world,
        tunnel_count: f64::from(tunnels),
        penetration_m: f64::from(deepest),
        energy_drift: drift,
        contacts_stored: f64::from(u8::from(contacts_stored)),
        toi_deferred: f64::from(toi_deferred),
    })
}

fn ships_stored_contact(scene: &Scene) -> Result<bool, String> {
    let left = body(scene, scene.left)?;
    let right = body(scene, scene.right)?;
    Ok(left.normal_impulse > 0.0 && right.normal_impulse > 0.0)
}

struct VelocitySample {
    velocity: genos_math::Vec3,
    inverse_mass: f32,
}

fn velocities(world: &World) -> Vec<VelocitySample> {
    world
        .bodies
        .iter()
        .map(|body| VelocitySample {
            velocity: body.velocity,
            inverse_mass: body.inverse_mass,
        })
        .collect()
}

/// A step is free flight when every dynamic velocity changed only by gravity.
/// An inelastic impulse is not free flight, so it stays out of `energy_drift`.
fn free_flight(before: &[VelocitySample], world: &World) -> bool {
    if before.len() != world.bodies.len() {
        return false;
    }
    before
        .iter()
        .zip(&world.bodies)
        .all(|(sample, body)| gravity_only(sample, body.velocity, world.gravity))
}

fn gravity_only(
    sample: &VelocitySample,
    after: genos_math::Vec3,
    gravity: genos_math::Vec3,
) -> bool {
    let predicted = if sample.inverse_mass == 0.0 {
        sample.velocity
    } else {
        sample.velocity + gravity * DT
    };
    let delta = after - predicted;
    delta.length_squared() <= 1.0e-8
}

/// Ships face each other on X. The bullet and the wall share a Z the ships do not reach.
/// `good` and `bad` both use 3 000 m/s. The speed is not the protection switch.
fn build(setup: &Setup) -> Scene {
    let ship_speed = setup.ship_speed;
    let bullet_speed = setup.bullet_speed;
    let mut world = World::new(genos_physics::GRAVITY);
    let mut left = Body::cuboid(
        genos_math::Vec3::new(0.0, 2.0, 0.0),
        1_000.0,
        genos_math::Vec3::new(2.0, 1.0, 1.0),
    );
    left.velocity = genos_math::Vec3::new(ship_speed, 0.0, 0.0);
    let mut right = Body::cuboid(
        genos_math::Vec3::new(6.0, 2.0, 0.0),
        1_000.0,
        genos_math::Vec3::new(2.0, 1.0, 1.0),
    );
    right.velocity = genos_math::Vec3::new(-ship_speed, 0.0, 0.0);
    let mut bullet = Body::sphere(genos_math::Vec3::new(0.0, 2.0, 4.0), 0.02, 0.05);
    bullet.velocity = genos_math::Vec3::new(bullet_speed, 0.0, 0.0);
    if setup.protect {
        bullet.motion = MotionQuality::Speculative;
        left.motion = MotionQuality::Swept;
        right.motion = MotionQuality::Swept;
    }
    let wall = Body::cuboid(
        genos_math::Vec3::new(0.3, 2.0, 4.0),
        0.0,
        genos_math::Vec3::new(0.15, 5.0, 1.0),
    );
    let left_index = world.insert(left);
    let right_index = world.insert(right);
    let bullet_index = world.insert(bullet);
    let wall_index = world.insert(wall);
    Scene {
        world,
        left: left_index,
        right: right_index,
        bullet: bullet_index,
        wall: wall_index,
        ship_speed,
        bullet_speed,
    }
}

fn body(scene: &Scene, index: usize) -> Result<&Body, String> {
    scene
        .world
        .bodies
        .get(index)
        .ok_or_else(|| format!("body {index} missing"))
}

fn ships_exchanged(scene: &Scene) -> Result<bool, String> {
    let left = body(scene, scene.left)?;
    let right = body(scene, scene.right)?;
    Ok(left.position.x > right.position.x)
}

/// The bullet started on the near side. It has tunneled when its trailing point
/// is past the wall's far face.
fn bullet_past_wall(scene: &Scene) -> Result<bool, String> {
    let bullet = body(scene, scene.bullet)?;
    let wall = body(scene, scene.wall)?;
    let radius = sphere_radius(bullet)?;
    let half = box_half(wall)?;
    let trailing = bullet.position.x - radius;
    let far_face = wall.position.x + half.x;
    Ok(trailing > far_face)
}

fn penetration(scene: &Scene) -> Result<f32, String> {
    let left = body(scene, scene.left)?;
    let right = body(scene, scene.right)?;
    let bullet = body(scene, scene.bullet)?;
    let wall = body(scene, scene.wall)?;
    let ships = box_overlap(left, right)?;
    let shot = sphere_box_depth(bullet, wall)?;
    Ok(ships.max(shot))
}

fn box_half(body: &Body) -> Result<genos_math::Vec3, String> {
    match body.shape {
        Shape::Box { half_extents } => Ok(half_extents),
        Shape::Sphere { .. } | Shape::Capsule { .. } | Shape::Plane { .. } | Shape::Mesh(_) => {
            Err("expected a box".into())
        }
    }
}

fn sphere_radius(body: &Body) -> Result<f32, String> {
    match body.shape {
        Shape::Sphere { radius } => Ok(radius),
        Shape::Capsule { .. } | Shape::Box { .. } | Shape::Plane { .. } | Shape::Mesh(_) => {
            Err("expected a sphere".into())
        }
    }
}

/// Positive overlap along the smallest axis. Zero when the boxes are apart.
fn box_overlap(a: &Body, b: &Body) -> Result<f32, String> {
    let a_half = box_half(a)?;
    let b_half = box_half(b)?;
    let overlap_x = (a_half.x + b_half.x) - (a.position.x - b.position.x).abs();
    let overlap_y = (a_half.y + b_half.y) - (a.position.y - b.position.y).abs();
    let overlap_z = (a_half.z + b_half.z) - (a.position.z - b.position.z).abs();
    if overlap_x > 0.0 && overlap_y > 0.0 && overlap_z > 0.0 {
        Ok(overlap_x.min(overlap_y).min(overlap_z))
    } else {
        Ok(0.0)
    }
}

/// Positive sphere-box depth. Zero when the sphere is outside.
fn sphere_box_depth(sphere: &Body, box_body: &Body) -> Result<f32, String> {
    let radius = sphere_radius(sphere)?;
    let half = box_half(box_body)?;
    let local = sphere.position - box_body.position;
    let clamped = genos_math::Vec3::new(
        local.x.clamp(-half.x, half.x),
        local.y.clamp(-half.y, half.y),
        local.z.clamp(-half.z, half.z),
    );
    let delta = local - clamped;
    let dist_sq = delta.length_squared();
    if dist_sq > 1.0e-12 {
        let dist = dist_sq.sqrt();
        Ok((radius - dist).max(0.0))
    } else {
        let face = (half.x - local.x.abs())
            .min(half.y - local.y.abs())
            .min(half.z - local.z.abs());
        Ok(face + radius)
    }
}

/// `1/2 m |v|^2 + 1/2 ω^T I ω + m g h` over dynamic bodies.
fn energy(world: &World) -> f64 {
    let gravity = f64::from(-world.gravity.y);
    let mut total = 0.0;
    for body in &world.bodies {
        if body.inverse_mass == 0.0 {
            continue;
        }
        let mass = f64::from(body.mass());
        let speed_sq = f64::from(body.velocity.x) * f64::from(body.velocity.x)
            + f64::from(body.velocity.y) * f64::from(body.velocity.y)
            + f64::from(body.velocity.z) * f64::from(body.velocity.z);
        total += 0.5 * mass * speed_sq
            + rotational_joules(body)
            + mass * gravity * f64::from(body.position.y);
    }
    total
}

fn rotational_joules(body: &Body) -> f64 {
    f64::from(body.rotational_energy())
}

fn hash_state(world: &World) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for body in &world.bodies {
        for bits in [
            body.position.x.to_bits(),
            body.position.y.to_bits(),
            body.position.z.to_bits(),
            body.velocity.x.to_bits(),
            body.velocity.y.to_bits(),
            body.velocity.z.to_bits(),
            body.angular_velocity.x.to_bits(),
            body.angular_velocity.y.to_bits(),
            body.angular_velocity.z.to_bits(),
            body.orientation.x.to_bits(),
            body.orientation.y.to_bits(),
            body.orientation.z.to_bits(),
            body.orientation.w.to_bits(),
        ] {
            hash = fnv_u32(hash, bits);
        }
    }
    hash
}

fn fnv_u32(mut hash: u64, bits: u32) -> u64 {
    for shift in [0, 8, 16, 24] {
        hash ^= u64::from((bits >> shift) & 0xff);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn judge_all(reading: &Reading) -> Vec<Row> {
    let values = [
        reading.tunnel_count,
        reading.penetration_m,
        reading.energy_drift,
        reading.determinism_hash_match,
        reading.contacts_stored,
        reading.toi_deferred,
    ];
    METRICS
        .iter()
        .zip(values)
        .map(|(spec, value)| judge(spec, value))
        .collect()
}

fn judge(spec: &Spec, value: f64) -> Row {
    let finite = value.is_finite();
    let inside = finite
        && match spec.bound {
            Bound::AtMost => value <= spec.threshold,
            Bound::AtLeast => value >= spec.threshold,
            Bound::AbsAtMost => value.abs() <= spec.threshold,
        };
    Row {
        name: spec.name,
        value,
        value_text: if finite {
            fmt_num(value)
        } else {
            "nan".to_string()
        },
        threshold_text: spec.threshold_text,
        status: if inside { Status::Pass } else { Status::Fail },
    }
}

fn fmt_num(value: f64) -> String {
    let text = format!("{value}");
    if text == "-0" {
        "0".to_string()
    } else {
        text
    }
}

fn print_scene(reading: &Reading) {
    let protection = if reading.protect { "true" } else { "false" };
    let energy = if reading.full_energy {
        "full"
    } else {
        "free-flight"
    };
    let ccd = if reading.ccd_in_step { "true" } else { "false" };
    println!(
        "scene protection={protection} ships_m_s={} bullet_m_s={} steps={STEPS} energy={energy} ccd_in_step={ccd}",
        fmt_num(f64::from(reading.ship_speed)),
        fmt_num(f64::from(reading.bullet_speed)),
    );
}

fn line(row: &Row) -> String {
    format!(
        "{} {} {} {}",
        row.name,
        row.value_text,
        row.threshold_text,
        row.status.as_str()
    )
}

fn print_rows(rows: &[Row]) {
    for row in rows {
        println!("{}", line(row));
    }
}

fn exit_code(rows: &[Row]) -> i32 {
    i32::from(rows.iter().any(|row| row.status == Status::Fail))
}

fn json(fixture: &str, reading: &Reading, rows: &[Row]) -> String {
    let failed = rows.iter().filter(|row| row.status == Status::Fail).count();
    let protection = if reading.protect { "true" } else { "false" };
    let energy = if reading.full_energy {
        "full"
    } else {
        "free-flight"
    };
    let ccd = if reading.ccd_in_step { "true" } else { "false" };
    let mut out = format!(
        "{{\n  \"fixture\": \"{fixture}\",\n  \"protection\": {protection},\n  \"ccd_in_step\": {ccd},\n  \"energy\": \"{energy}\",\n  \"ship_speed_m_s\": {},\n  \"bullet_speed_m_s\": {},\n  \"steps\": {STEPS},\n  \"failed\": {failed},\n  \"metrics\": [\n",
        fmt_num(f64::from(reading.ship_speed)),
        fmt_num(f64::from(reading.bullet_speed)),
    );
    for (index, row) in rows.iter().enumerate() {
        let comma = if index + 1 == rows.len() { "" } else { "," };
        let value = if row.value.is_finite() {
            fmt_num(row.value)
        } else {
            "null".to_string()
        };
        let row_line = format!(
            "    {{\"name\": \"{}\", \"value\": {value}, \"threshold\": \"{}\", \"status\": \"{}\"}}{comma}\n",
            row.name, row.threshold_text, row.status.as_str()
        );
        out.push_str(&row_line);
    }
    out.push_str("  ]\n}\n");
    out
}

fn write_report(dir: &Path, fixture: &str, reading: &Reading, rows: &[Row]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let path = dir.join(format!("{fixture}.json"));
    std::fs::write(&path, json(fixture, reading, rows))
        .map_err(|err| format!("{}: {err}", path.display()))
}

struct Child {
    code: i32,
    stdout: String,
}

fn prove(out: &Path) -> Result<i32, String> {
    std::fs::create_dir_all(out).map_err(|err| format!("{}: {err}", out.display()))?;
    let exe = std::env::current_exe().map_err(|err| format!("current exe: {err}"))?;
    let good = run_child(&exe, out, "good")?;
    let bad = run_child(&exe, out, "bad")?;
    let discrete = run_child(&exe, out, "discrete")?;
    let loss = run_child(&exe, out, "loss")?;
    println!("# good");
    print!("{}", good.stdout);
    println!("# bad");
    print!("{}", bad.stdout);
    println!("# discrete");
    print!("{}", discrete.stdout);
    println!("# loss");
    print!("{}", loss.stdout);
    if good.code != 0 || bad.code != 1 || discrete.code != 0 || loss.code != 1 {
        return Err(format!(
            "proof exits good {} bad {} discrete {} loss {}",
            good.code, bad.code, discrete.code, loss.code
        ));
    }
    expect_protected(&good.stdout)?;
    expect_unprotected(&bad.stdout)?;
    expect_discrete(&discrete.stdout)?;
    expect_loss(&loss.stdout)?;
    expect_file(out, "good", true, 0, "free-flight", true)?;
    expect_file(out, "bad", false, 2, "free-flight", false)?;
    expect_file(out, "discrete", false, 0, "free-flight", false)?;
    expect_file(out, "loss", false, 1, "full", false)?;
    Ok(0)
}

fn run_child(exe: &Path, out: &Path, fixture: &str) -> Result<Child, String> {
    let output = Command::new(exe)
        .arg("--fixture")
        .arg(fixture)
        .arg("--out")
        .arg(out)
        .output()
        .map_err(|err| format!("spawn {}: {err}", exe.display()))?;
    let stdout =
        String::from_utf8(output.stdout).map_err(|_| "child stdout is not utf-8".to_string())?;
    let code = output
        .status
        .code()
        .ok_or_else(|| format!("{fixture} did not exit"))?;
    if code == 2 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{fixture}: {stderr}{stdout}"));
    }
    Ok(Child { code, stdout })
}

struct Printed {
    name: String,
    value: f64,
    threshold: String,
    status: String,
}

fn printed_rows(text: &str) -> Result<Vec<Printed>, String> {
    let mut rows = Vec::new();
    for row in text.lines() {
        if row.is_empty() || row.starts_with("scene ") || row.starts_with("# ") {
            continue;
        }
        let mut parts = row.split(' ');
        let name = parts
            .next()
            .ok_or_else(|| format!("short line {row}"))?
            .to_string();
        let value_text = parts
            .next()
            .ok_or_else(|| format!("short line {row}"))?
            .to_string();
        let threshold = parts
            .next()
            .ok_or_else(|| format!("short line {row}"))?
            .to_string();
        let status = parts
            .next()
            .ok_or_else(|| format!("short line {row}"))?
            .to_string();
        if parts.next().is_some() {
            return Err(format!("extra field on {row}"));
        }
        let value = value_text
            .parse::<f64>()
            .map_err(|_| format!("bad number on {row}"))?;
        rows.push(Printed {
            name,
            value,
            threshold,
            status,
        });
    }
    if rows.len() != METRICS.len() {
        return Err(format!(
            "{} metric lines, wanted {}",
            rows.len(),
            METRICS.len()
        ));
    }
    for (row, spec) in rows.iter().zip(METRICS) {
        if row.name != spec.name || row.threshold != spec.threshold_text {
            return Err(format!("{} does not match the metric table", row.name));
        }
    }
    Ok(rows)
}

fn expect_speed(text: &str) -> Result<(), String> {
    if !text.contains(&format!("ships_m_s={}", fmt_num(f64::from(TUNNEL_SPEED))))
        || !text.contains(&format!("bullet_m_s={}", fmt_num(f64::from(TUNNEL_SPEED))))
    {
        return Err("high-speed scene did not use 3000 m/s for both bodies".into());
    }
    if !text.contains("energy=free-flight") {
        return Err("high-speed scene did not record free-flight energy".into());
    }
    Ok(())
}

fn expect_protected(text: &str) -> Result<(), String> {
    if !text.contains("protection=true") || !text.contains("ccd_in_step=true") {
        return Err("protected scene missing protection or ccd_in_step".into());
    }
    expect_speed(text)?;
    for row in printed_rows(text)? {
        if row.status != "PASS" {
            return Err(format!("{} wanted PASS with protection on", row.name));
        }
        if row.name == "tunnel_count" && row.value > 0.0 {
            return Err(format!("tunnel_count {} wanted 0", row.value));
        }
    }
    Ok(())
}

fn expect_unprotected(text: &str) -> Result<(), String> {
    if !text.contains("protection=false") || !text.contains("ccd_in_step=false") {
        return Err("unprotected scene recorded protection".into());
    }
    expect_speed(text)?;
    let mut tunnel_failed = false;
    let mut contacts_failed = false;
    for row in printed_rows(text)? {
        if row.name == "tunnel_count" {
            if row.status != "FAIL" || row.value <= 0.0 {
                return Err(format!(
                    "tunnel_count {} {} wanted FAIL above 0",
                    row.value, row.status
                ));
            }
            tunnel_failed = true;
        } else if row.name == "contacts_stored" {
            if row.status != "FAIL" || row.value >= 1.0 {
                return Err(format!(
                    "contacts_stored {} {} wanted FAIL below 1",
                    row.value, row.status
                ));
            }
            contacts_failed = true;
        } else if row.status != "PASS" {
            return Err(format!("{} wanted PASS beside the tunnel", row.name));
        }
    }
    if !tunnel_failed || !contacts_failed {
        return Err("unprotected run did not fail tunnel and contacts".into());
    }
    Ok(())
}

fn expect_discrete(text: &str) -> Result<(), String> {
    if !text.contains(&format!("ships_m_s={}", fmt_num(f64::from(CATCH_SHIP))))
        || !text.contains(&format!("bullet_m_s={}", fmt_num(f64::from(CATCH_BULLET))))
    {
        return Err("discrete scene changed the catching speeds".into());
    }
    if !text.contains("energy=free-flight") {
        return Err("discrete scene scored the inelastic drop".into());
    }
    for row in printed_rows(text)? {
        if row.status != "PASS" {
            return Err(format!("{} wanted PASS", row.name));
        }
    }
    Ok(())
}

fn expect_loss(text: &str) -> Result<(), String> {
    if !text.contains("energy=full") {
        return Err("loss scene hid the inelastic drop".into());
    }
    if !text.contains(&format!("ships_m_s={}", fmt_num(f64::from(CATCH_SHIP)))) {
        return Err("loss scene is not the discrete impact".into());
    }
    let mut energy_failed = false;
    for row in printed_rows(text)? {
        if row.name == "energy_drift" {
            if row.status != "FAIL" || row.value.abs() < 0.9 {
                return Err(format!(
                    "energy_drift {} {} wanted a FAIL past 90%",
                    row.value, row.status
                ));
            }
            energy_failed = true;
        } else if row.status != "PASS" {
            return Err(format!("{} wanted PASS on the loss scene", row.name));
        }
    }
    if !energy_failed {
        return Err("loss run has no energy_drift".into());
    }
    Ok(())
}

fn expect_file(
    dir: &Path,
    fixture: &str,
    protect: bool,
    failed: i32,
    energy: &str,
    ccd: bool,
) -> Result<(), String> {
    let path = dir.join(format!("{fixture}.json"));
    let text =
        std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let protection = if protect { "true" } else { "false" };
    if !text.contains(&format!("\"fixture\": \"{fixture}\"")) {
        return Err(format!("{fixture} json has the wrong fixture"));
    }
    if !text.contains(&format!("\"protection\": {protection}")) {
        return Err(format!("{fixture} json has the wrong protection flag"));
    }
    let ccd_text = if ccd { "true" } else { "false" };
    if !text.contains(&format!("\"ccd_in_step\": {ccd_text}")) {
        return Err(format!("{fixture} json has the wrong ccd_in_step flag"));
    }
    if !text.contains(&format!("\"energy\": \"{energy}\"")) {
        return Err(format!("{fixture} json has the wrong energy account"));
    }
    if !text.contains(&format!("\"failed\": {failed}")) {
        return Err(format!("{fixture} json missing failed {failed}"));
    }
    Ok(())
}
