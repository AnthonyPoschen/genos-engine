#![cfg_attr(
    test,
    expect(
        clippy::as_conversions,
        reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
    )
)]
#![cfg_attr(
    test,
    expect(
        clippy::cast_precision_loss,
        reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
    )
)]
#![cfg_attr(
    test,
    expect(
        clippy::undocumented_unsafe_blocks,
        reason = "TODO(lint-2026-10-14): Vulkan or OS FFI; add a SAFETY comment. docs/LINT_RULES.md"
    )
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use genos_math::{Mat4, Quat, Vec3};
use genos_physics::{step, Body, World, GRAVITY};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct CountAlloc;

unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountAlloc = CountAlloc;

#[test]
fn timed_math_and_physics_batch_does_not_allocate() {
    const BATCH: u32 = 10_000;
    let mut direction = Vec3::new(1.0, 2.0, 3.0);
    let spin = Mat4::from_axis_angle(Vec3::new(0.2, 0.9, 0.1).normalize(), 0.02);
    let mut matrix = Mat4::from_translation(Vec3::new(0.4, -0.2, 1.5));
    let turn = Quat::from_axis_angle(Vec3::Y, 0.35);
    let mut world = World::new(GRAVITY);
    for index in 0..32 {
        let x = index as f32 * 0.75;
        let mut body = Body::sphere(Vec3::new(x, 1.0 + (index % 3) as f32, 0.0), 1.0, 0.4);
        body.velocity = Vec3::new(0.2, -1.0, 0.1);
        world.insert(body);
    }

    // The first step sizes the contact lists the later ones reuse.
    step(&mut world, 1.0 / 60.0);

    let started = Instant::now();
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..BATCH {
        direction = (direction + Vec3::new(0.15, -0.05, 0.2)).normalize();
        matrix = spin * matrix;
        direction = turn.rotate(direction);
    }
    step(&mut world, 1.0 / 60.0);
    let allocs = ALLOCS.load(Ordering::Relaxed) - before;
    let elapsed = started.elapsed();
    std::hint::black_box((direction, matrix, &world));

    println!("normalizes={BATCH}");
    println!("matrix_multiplies={BATCH}");
    println!("quaternion_direction_rotations={BATCH}");
    println!("physics_step_bodies={}", world.bodies.len());
    println!("allocs={allocs}");
    println!("elapsed_ns={}", elapsed.as_nanos());
    println!(
        "checksum {:.6}",
        direction.x + matrix.cols[0] + world.bodies[0].position.y
    );
    assert_eq!(allocs, 0, "timed math and physics allocated {allocs} times");

    let mut aircraft = genos_physics::aero_trim_world();
    step(&mut aircraft, 1.0 / 60.0);
    let before_aircraft = ALLOCS.load(Ordering::Relaxed);
    step(&mut aircraft, 1.0 / 60.0);
    let aircraft_allocs = ALLOCS.load(Ordering::Relaxed) - before_aircraft;
    println!("aircraft_allocs={aircraft_allocs}");
    assert_eq!(
        aircraft_allocs, 0,
        "trimmed aircraft step allocated {aircraft_allocs} times"
    );

    let mut sleeper = genos_physics::aero_trim_world();
    sleeper.bodies[0].sleeping = true;
    sleeper.bodies[0].velocity = Vec3::ZERO;
    step(&mut sleeper, 1.0 / 60.0);
    let before_wake = ALLOCS.load(Ordering::Relaxed);
    step(&mut sleeper, 1.0 / 60.0);
    let wake_allocs = ALLOCS.load(Ordering::Relaxed) - before_wake;
    println!("aircraft_wake_allocs={wake_allocs}");
    assert!(
        !sleeper.bodies[0].sleeping,
        "the allocation sleeper stayed asleep"
    );
    assert_eq!(
        wake_allocs, 0,
        "aircraft wake step allocated {wake_allocs} times"
    );

    let mut landing = genos_physics::gear_plane_world();
    step(&mut landing, 1.0 / 60.0);
    let before_landing = ALLOCS.load(Ordering::Relaxed);
    step(&mut landing, 1.0 / 60.0);
    let landing_allocs = ALLOCS.load(Ordering::Relaxed) - before_landing;
    println!("landing_allocs={landing_allocs}");
    assert_eq!(
        landing_allocs, 0,
        "gear and wing step allocated {landing_allocs} times"
    );
}
