//! Lavapipe dispatch of `lighting_world_irradiance_in` for ADR 0039 stage 3.
//!
//! The shader is the production cache read compose calls. It is not a copied
//! estimator. Software Vulkan only: the numbers are not a 4070 time and not
//! an accuracy gate.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

use genos_render::{GpuKind, Renderer};
use genos_window::Window;

const STRIDE: usize = 24;
const ROWS: usize = 10;
const ABS: f32 = 1.0e-3;

struct Row {
    shown: [f32; 4],
    mean_before: [f32; 4],
    mean_after: [f32; 4],
    stamp_before: u32,
    stamp_after: u32,
    hash_before: u32,
    hash_after: u32,
    conf: f32,
    own: f32,
    detail_on: f32,
    seed: [f32; 3],
    denoise: f32,
}

fn row(words: &[f32], id: usize) -> Row {
    let at = 4 + id * STRIDE;
    let w = &words[at..at + STRIDE];
    Row {
        shown: [w[0], w[1], w[2], w[3]],
        mean_before: [w[4], w[5], w[6], w[7]],
        mean_after: [w[8], w[9], w[10], w[11]],
        stamp_before: w[12].to_bits(),
        stamp_after: w[13].to_bits(),
        hash_before: w[14].to_bits(),
        hash_after: w[15].to_bits(),
        conf: w[16],
        own: w[17],
        detail_on: w[18],
        seed: [w[20], w[21], w[22]],
        denoise: w[23],
    }
}

fn near(got: [f32; 3], want: [f32; 3]) -> bool {
    got.iter()
        .zip(want)
        .all(|(a, b)| (a - b).abs() <= ABS && a.is_finite())
}

fn not_near(got: [f32; 3], want: [f32; 3]) -> bool {
    !near(got, want)
}

fn cell_held(row: &Row) {
    assert_eq!(
        row.stamp_before, row.stamp_after,
        "the owning cell was stamped"
    );
    assert_eq!(row.hash_before, row.hash_after, "the harmonics were written");
    assert!(
        near(
            [row.mean_before[0], row.mean_before[1], row.mean_before[2]],
            [row.mean_after[0], row.mean_after[1], row.mean_after[2]]
        ),
        "the cell mean changed"
    );
    assert!((row.mean_before[3] - row.mean_after[3]).abs() <= ABS);
}

fn check_samples_and_gaps(rows: &[Row], red: [f32; 3], own: [f32; 3]) {
    let sampled = &rows[4];
    assert!((sampled.own - 1.0).abs() <= ABS, "own {}", sampled.own);
    assert!(
        near([sampled.shown[0], sampled.shown[1], sampled.shown[2]], own),
        "own samples showed {:?} instead of the texel mean",
        sampled.shown
    );
    assert!(sampled.conf.abs() <= ABS, "a sampled texel stored a seed");
    cell_held(sampled);

    let mover = &rows[5];
    assert!(
        mover.shown[3] < 0.5,
        "a mover was treated as covered, trust {}",
        mover.shown[3]
    );

    let slant = &rows[6];
    assert!(
        near([slant.shown[0], slant.shown[1], slant.shown[2]], [0.0; 3]),
        "slanted pixel {:?}",
        slant.shown
    );
    cell_held(slant);
    assert_eq!(slant.stamp_before, 3);

    let bare = &rows[7];
    assert!((bare.own - 1.0).abs() <= ABS);
    assert!(
        near([bare.shown[0], bare.shown[1], bare.shown[2]], own),
        "texel with no cell {:?}",
        bare.shown
    );

    let quiet = &rows[8];
    assert!(quiet.denoise.abs() <= ABS);
    assert!(
        near([quiet.shown[0], quiet.shown[1], quiet.shown[2]], red),
        "denoise off hid the cell {:?}",
        quiet.shown
    );
    assert!((quiet.shown[3] - 1.0).abs() <= ABS);
    assert!(near(quiet.seed, red), "denoise off seed {:?}", quiet.seed);
    cell_held(quiet);

    let quiet_off = &rows[9];
    assert!(quiet_off.detail_on.abs() <= ABS);
    assert!(quiet_off.denoise.abs() <= ABS);
    assert!(
        not_near(
            [quiet_off.shown[0], quiet_off.shown[1], quiet_off.shown[2]],
            red
        ),
        "denoise off with detail off showed the cell {:?}",
        quiet_off.shown
    );
}

fn open() -> Result<genos_window::Lifecycle<Renderer>, String> {
    let mut window = Window::open_proof(64, 64)?;
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok(genos_window::Lifecycle::new(window, renderer, permit))
}

#[test]
fn the_production_read_shows_the_owning_cell_and_not_the_other_seeds() {
    let mut owner = open().expect("lavapipe device");
    let renderer = &mut *owner.device;
    assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    let words = renderer
        .texel_detail_prod()
        .expect("production irradiance dispatch");
    assert_eq!(words.len(), 4 + ROWS * STRIDE);
    let rows: Vec<Row> = (0..ROWS).map(|id| row(&words, id)).collect();

    let red = [8.0, 0.0, 0.0];
    let blue = [0.0, 0.0, 7.0];
    let own = [4.0, 2.0, 0.0];
    let probe = [1.0, 2.0, 3.0];

    let covered = &rows[0];
    assert!((covered.detail_on - 1.0).abs() <= ABS);
    assert!(
        near([covered.shown[0], covered.shown[1], covered.shown[2]], red),
        "covered pixel {:?} wanted the owning cell, not the probe {probe:?} or the green corners",
        covered.shown
    );
    assert!((covered.shown[3] - 1.0).abs() <= ABS, "trust {}", covered.shown[3]);
    assert!((covered.conf - 0.25).abs() <= ABS, "conf {}", covered.conf);
    assert!(covered.own.abs() <= ABS, "own samples {}", covered.own);
    assert!(near(covered.seed, red), "stored seed {:?}", covered.seed);
    assert!(not_near(covered.seed, probe));
    cell_held(covered);
    assert_eq!(covered.stamp_before, 3);

    let miss = &rows[1];
    assert!(
        not_near([miss.shown[0], miss.shown[1], miss.shown[2]], red),
        "an unlit owner must not show the red cell {:?}",
        miss.shown
    );
    assert!(miss.mean_after[3].abs() <= ABS, "unlit mean {}", miss.mean_after[3]);

    let off = &rows[2];
    assert!(off.detail_on.abs() <= ABS);
    assert!(
        not_near([off.shown[0], off.shown[1], off.shown[2]], red),
        "detail off showed the cell {:?}",
        off.shown
    );

    let lamp = &rows[3];
    assert!(
        near([lamp.shown[0], lamp.shown[1], lamp.shown[2]], blue),
        "lamp clear left {:?} wanted blue",
        lamp.shown
    );
    assert!(near(lamp.seed, blue), "seed after the lamp {:?}", lamp.seed);
    assert!((lamp.conf - 0.25).abs() <= ABS);
    assert!((lamp.shown[3] - 1.0).abs() <= ABS);
    cell_held(lamp);
    check_samples_and_gaps(&rows, red, own);
}
