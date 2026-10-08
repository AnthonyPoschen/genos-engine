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
}
