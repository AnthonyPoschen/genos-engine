#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

//! Production `lighting_world_irradiance_in` on lavapipe.
//!
//! The dispatch is `texel_seed_prod.comp`, which includes the production
//! cache shader and calls that function. Default surface-cache flags, the
//! high preset's denoise bit, SH cells, and the stage 3 seed bit. It is not
//! the `texel_seed_check` helper.

use genos_render::{GpuKind, Renderer};
use genos_window::Window;

const HEAD: usize = 4;
const STRIDE: usize = 20;
const NAMES: [&str; 8] = [
    "compete",
    "miss",
    "off",
    "lamp",
    "own",
    "mover",
    "slant",
    "own_empty",
];

fn open() -> Result<genos_window::Lifecycle<Renderer>, String> {
    let mut window = Window::open_proof(64, 64)?;
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok(genos_window::Lifecycle::new(window, renderer, permit))
}

fn bits(words: &[f32], index: usize) -> u32 {
    words[index].to_bits()
}

struct Record {
    shown: [f32; 3],
    weight: f32,
    before: [f32; 4],
    after: [f32; 4],
    stamp_before: u32,
    stamp_after: u32,
    hash_before: u32,
    hash_after: u32,
    confidence: f32,
    own: f32,
    seed_on: f32,
    flags: u32,
}

fn record(words: &[f32], id: usize) -> Record {
    let at = HEAD + id * STRIDE;
    Record {
        shown: [words[at], words[at + 1], words[at + 2]],
        weight: words[at + 3],
        before: [words[at + 4], words[at + 5], words[at + 6], words[at + 7]],
        after: [words[at + 8], words[at + 9], words[at + 10], words[at + 11]],
        stamp_before: bits(words, at + 12),
        stamp_after: bits(words, at + 13),
        hash_before: bits(words, at + 14),
        hash_after: bits(words, at + 15),
        confidence: words[at + 16],
        own: words[at + 17],
        seed_on: words[at + 18],
        flags: bits(words, at + 19),
    }
}

fn cell_held(row: &Record) {
    assert_eq!(
        row.stamp_before, row.stamp_after,
        "owning cell stamp changed"
    );
    assert_eq!(row.stamp_after, 3, "owning cell stamp was rewritten");
    for (before, after) in row.before.iter().zip(row.after) {
        assert!(
            (before - after).abs() < 1.0e-5,
            "owning cell mean or count changed: {:?} -> {:?}",
            row.before,
            row.after
        );
    }
    assert_eq!(
        row.hash_before, row.hash_after,
        "owning cell harmonics changed"
    );
}

fn expect_header(flags: u32, head_w: u32, ctrl: u32, mode: u32) {
    assert_ne!(flags & (1 << 19), 0, "default flags include the resample bit");
    assert_eq!(flags & (1 << 29), 0, "layer 2 is on when its off bit is clear");
    assert_eq!(
        flags & (1 << 31),
        0,
        "SVGF is on when its off bit is clear"
    );
    assert_ne!(head_w & (1 << 17), 0, "the stage 3 seed bit is set");
    assert_ne!(head_w & 1, 0, "SH cells are on");
    assert_ne!(ctrl & 64, 0, "the high preset denoise bit is set");
    assert_eq!(mode, 3, "world mode is the surface cache");
}

fn dump(words: &[f32]) -> Vec<Record> {
    let flags = bits(words, 0);
    let head_w = bits(words, 1);
    let ctrl = bits(words, 2);
    let mode = bits(words, 3);
    println!("header flags={flags:#x} head_w={head_w:#x} ctrl={ctrl:#x} mode={mode}");
    expect_header(flags, head_w, ctrl, mode);
    let rows: Vec<Record> = (0..8).map(|id| record(words, id)).collect();
    for (name, row) in NAMES.iter().zip(&rows) {
        println!(
            "{name} shown={:?} w={} before={:?} after={:?} stamp={}→{} hash {:#x} {:#x} conf={} own={} seed_on={} flags={:#x}",
            row.shown,
            row.weight,
            row.before,
            row.after,
            row.stamp_before,
            row.stamp_after,
            row.hash_before,
            row.hash_after,
            row.confidence,
            row.own,
            row.seed_on,
            row.flags
        );
    }
    rows
}

#[test]
fn the_owning_cell_seed_wins_a_competing_resample_on_the_production_path() {
    let mut life = open().expect("proof window");
    let renderer = &mut *life.device;
    assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    let rows = dump(&renderer.texel_seed_prod().expect("production seed dispatch"));

    let compete = &rows[0];
    assert!((compete.seed_on - 1.0).abs() < 1.0e-5);
    assert!(
        compete.shown[0] > 4.0 && compete.shown[1] < 1.0 && compete.shown[2] < 1.0,
        "the owning cell's red tilt must beat the green resample, got {:?}",
        compete.shown
    );
    assert!(compete.own < 0.5, "the seed must not be an own sample");
    cell_held(compete);

    let miss = &rows[1];
    assert!(
        miss.shown[1] > 5.0 && miss.shown[0] < 1.0,
        "an empty owning cell keeps the green resample, got {:?}",
        miss.shown
    );
    assert!(miss.before[3] < 0.5 && miss.after[3] < 0.5);
    assert_eq!(miss.hash_before, miss.hash_after);

    let off = &rows[2];
    assert!(off.seed_on.abs() < 1.0e-5, "the off case must clear the seed bit");
    assert!(
        (off.shown[0] - 4.5).abs() < 0.05 && (off.shown[1] - 2.625).abs() < 0.05,
        "the seed bit off must keep the resample mix, got {:?}",
        off.shown
    );

    let lamp = &rows[3];
    assert!(
        lamp.shown[2] > 6.0 && lamp.shown[1] < 0.5 && lamp.shown[0] < 0.5,
        "a lamp clear must refill from the current blue cell, got {:?}",
        lamp.shown
    );
    cell_held(lamp);

    // Default SVGF with no fresh field shows lighting_sc_wrc_now before the
    // own mean. The sample still owns the slot: count stays 1 and no seed is
    // stored over it, so the colour is the cell bilinear rather than the red
    // tilt. The empty-neighbour case below is the own-mean arm.
    let own = &rows[4];
    assert!(
        (own.own - 1.0).abs() < 1.0e-3,
        "the first own sample count must stay 1, got {}",
        own.own
    );
    assert!(
        own.confidence.abs() < 1.0e-5,
        "an own sample must not take a seed, confidence {}",
        own.confidence
    );
    assert!(
        (own.shown[0] - off.shown[0]).abs() < 0.05 && (own.shown[1] - off.shown[1]).abs() < 0.05,
        "an own sample must not switch the pixel onto the red seed, got {:?} vs {:?}",
        own.shown,
        off.shown
    );
    cell_held(own);

    let mover = &rows[5];
    assert!(
        (mover.shown[0] - off.shown[0]).abs() < 0.05
            && (mover.shown[1] - off.shown[1]).abs() < 0.05,
        "a mover stays on the resample, got {:?}",
        mover.shown
    );

    let slant = &rows[6];
    assert!(
        slant.shown[0].abs() < 1.0 && slant.shown[1].abs() < 1.0 && slant.shown[2].abs() < 1.0,
        "a slanted normal stays off the texel seed, got {:?}",
        slant.shown
    );
    cell_held(slant);

    let own_empty = &rows[7];
    assert!(
        (own_empty.own - 1.0).abs() < 1.0e-3,
        "the empty-neighbour sample count must stay 1, got {}",
        own_empty.own
    );
    assert!(own_empty.confidence.abs() < 1.0e-5);
    assert!(
        (own_empty.shown[0] - 4.0).abs() < 0.05
            && (own_empty.shown[1] - 2.0).abs() < 0.05
            && own_empty.shown[2].abs() < 0.05,
        "with no lit cell, one own sample shows that sample, got {:?}",
        own_empty.shown
    );
}
