#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

//! `texel_seed_check.comp` on lavapipe against `lighting_sh`.
//!
//! The shader runs the same tilt the production `lighting_sh_tilt` calls, then
//! the seed, the lamp-change clear, and the must-fail seed-as-samples control.
//! A software device proves the arithmetic. It does not time a 4070.

use genos_render::lighting_sh::{texel_seed_check_expected, texel_seed_check_words, ShL1};
use genos_render::{GpuKind, Renderer};
use genos_window::Window;

fn open() -> Result<genos_window::Lifecycle<Renderer>, String> {
    let mut window = Window::open_proof(64, 64)?;
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok(genos_window::Lifecycle::new(window, renderer, permit))
}

#[test]
fn lavapipe_texel_seed_matches_the_owning_cell_and_a_clear_drops_it() {
    let axis = [0.0_f32, 1.0, 0.0];
    let normal = {
        let yaw = 0.4_f32;
        let pitch = 0.5_f32;
        [
            pitch.sin() * yaw.cos(),
            pitch.cos(),
            pitch.sin() * yaw.sin(),
        ]
    };
    let (e_axis, cell) = {
        let mut cell = ShL1::default();
        cell.c[0] = [0.4, 0.2, 0.1];
        cell.c[1] = [0.3, 0.0, -0.1];
        cell.c[2] = [0.05, 0.2, 0.0];
        cell.c[3] = [-0.2, 0.1, 0.15];
        let e_axis = [0.8, 0.35, 0.2];
        (e_axis, cell)
    };
    let own = [8.0_f32, 0.25, 3.0];
    let words = texel_seed_check_words(e_axis, axis, normal, &cell, own);
    let expected = texel_seed_check_expected(&cell, e_axis, axis, normal, own);
    assert!(
        (expected[0] - e_axis[0]).abs() > 1.0e-3,
        "the fixture tilt must leave the axis mean"
    );
    assert_eq!(expected[5], 0.0, "the reference writes no cell");
    assert!(
        expected[8] < 1.0e-6,
        "a clear with no own sample shows black"
    );
    assert!((expected[16] - own[0]).abs() < 1.0e-5);
    assert!(expected[19] < 1.0e-6, "the clear drops confidence");
    assert!((expected[24] - expected[0]).abs() < 1.0e-5);
    assert!(
        (expected[28] - 8.0).abs() < 1.0e-5,
        "the fault keeps eight samples"
    );

    let mut owner = open().expect("lavapipe device");
    let renderer = &mut *owner.device;
    assert_eq!(
        renderer.gpu_kind(),
        GpuKind::Software,
        "this parity is a lavapipe run, not a 4070 timing"
    );
    let gpu = renderer
        .texel_seed_check(&words)
        .expect("texel seed dispatch");
    for id in 0..4 {
        let at = 32 + id * 8;
        for lane in 0..6 {
            let got = gpu[at + lane];
            let want = expected[id * 8 + lane];
            assert!(
                (got - want).abs() <= 1.0e-5,
                "record {id} lane {lane}: gpu {got} cpu {want}"
            );
        }
    }
}
