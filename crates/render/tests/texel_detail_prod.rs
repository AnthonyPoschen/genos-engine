//! Lavapipe dispatch of `lighting_screen_probe_indirect` for ADR 0039.
//!
//! That is the function `lighting_compose.comp` calls. The change-block head
//! is `lighting_change_head`, the packer `lighting_passes` uploads. The
//! shader copies those words. It does not build bit 17. Software Vulkan only:
//! the numbers are not a 4070 time and not an accuracy gate. This dispatch
//! does not present a swapchain image. The production work buffer is
//! device-local, so a presented frame is not how this fixture reads the head.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

use genos_render::{lighting_change_head, GpuKind, LightingPreset, LightingSettings, Renderer};
use genos_window::Window;

const STRIDE: usize = 16;
const ROWS: usize = 10;
const ABS: f32 = 1.0e-3;
const DETAIL_BIT: u32 = 1 << 17;

struct Row {
    shown: [f32; 3],
    world_w: f32,
    reads: f32,
    skipped: f32,
    seed: [f32; 3],
    conf: f32,
    cell_same: f32,
    own: f32,
    head_hi: u32,
    cell: [f32; 3],
}

fn row(words: &[f32], id: usize) -> Row {
    let at = 8 + id * STRIDE;
    let w = &words[at..at + STRIDE];
    Row {
        shown: [w[0], w[1], w[2]],
        world_w: w[3],
        reads: w[4],
        skipped: w[5],
        seed: [w[6], w[7], w[8]],
        conf: w[9],
        cell_same: w[10],
        own: w[11],
        head_hi: w[12].to_bits(),
        cell: [w[13], w[14], w[15]],
    }
}

fn near(got: [f32; 3], want: [f32; 3]) -> bool {
    got.iter()
        .zip(want)
        .all(|(a, b)| (a - b).abs() <= ABS && a.is_finite())
}

fn settings(detail: bool, low: bool) -> LightingSettings {
    let mut settings = LightingSettings::preset(LightingPreset::High);
    settings.surface_cache.sh_cells = true;
    settings.surface_cache.sh_texel_detail = detail;
    if low {
        settings.bounce.count = 0;
    }
    settings.sanitize();
    settings
}

fn open() -> Result<genos_window::Lifecycle<Renderer>, String> {
    let mut window = Window::open_proof(64, 64)?;
    let frame = window.pump();
    let (renderer, permit) =
        Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    Ok(genos_window::Lifecycle::new(window, renderer, permit))
}

#[test]
fn the_settings_path_sets_head_bit_17_only_when_texel_detail_is_on() {
    let on = lighting_change_head(&settings(true, false));
    let off = lighting_change_head(&settings(false, false));
    let low = lighting_change_head(&settings(true, true));
    assert_ne!(on.head_hi & DETAIL_BIT, 0, "detail on packs bit 17");
    assert_eq!(
        off.head_hi & DETAIL_BIT,
        0,
        "detail off leaves bit 17 clear"
    );
    assert_ne!(
        low.head_hi & DETAIL_BIT,
        0,
        "the bit is packed even when the world mode is 0"
    );
    assert_eq!(on.mode_cap / 1000, 3, "high is the surface cache");
    assert_eq!(low.mode_cap / 1000, 0, "bounce 0 is probe GI");
    assert_ne!(on.flags & (1 << 20), 0, "the estimator bit stays on");
    assert_ne!(on.flags & (1 << 11), 0, "the default slant gate stays on");
}

#[test]
fn covered_pixels_skip_screen_probe_indirect_and_the_rest_keep_it() {
    let mut owner = open().expect("lavapipe device");
    let renderer = &mut *owner.device;
    assert_eq!(renderer.gpu_kind(), GpuKind::Software);
    let words = renderer
        .texel_detail_prod()
        .expect("production compose indirect");
    assert_eq!(words.len(), 8 + ROWS * STRIDE);
    let rows: Vec<Row> = (0..ROWS).map(|id| row(&words, id)).collect();

    let on = lighting_change_head(&settings(true, false));
    let off = lighting_change_head(&settings(false, false));
    let low = lighting_change_head(&settings(true, true));
    assert_eq!(
        words[1].to_bits(),
        on.head_hi,
        "the shader read a head it did not pack"
    );
    assert_eq!(words[6].to_bits(), (on.head_hi as f32).to_bits());
    assert_eq!(words[7].to_bits(), (off.head_hi as f32).to_bits());

    let red = [8.0, 0.0, 0.0];
    let blue = [0.0, 0.0, 7.0];
    let own = [4.0, 2.0, 0.0];
    let probe = [1.0, 2.0, 3.0];
    let green = [0.0, 6.0, 0.0];

    let covered = &rows[0];
    assert_eq!(covered.head_hi, on.head_hi);
    assert!(
        near(covered.shown, red),
        "covered pixel {:?} wanted the owning cell, not the probe {probe:?} or the green corners",
        covered.shown
    );
    assert!(
        (covered.world_w - 1.0).abs() <= ABS,
        "trust {}",
        covered.world_w
    );
    assert!(covered.reads.abs() <= ABS, "probe reads {}", covered.reads);
    assert!((covered.skipped - 1.0).abs() <= ABS);
    assert!((covered.conf - 0.25).abs() <= ABS, "conf {}", covered.conf);
    assert!(near(covered.seed, red), "stored seed {:?}", covered.seed);
    assert!(covered.own.abs() <= ABS, "own samples {}", covered.own);
    assert!(
        (covered.cell_same - 1.0).abs() <= ABS,
        "the cell was written"
    );

    let miss = &rows[1];
    assert!(
        near(miss.shown, probe),
        "an empty owner must show the probe, got {:?}",
        miss.shown
    );
    assert!(miss.reads >= 1.0, "miss still reads probes {}", miss.reads);
    assert!(miss.skipped.abs() <= ABS);
    // A miss claims an empty texel and the first-fill path stores the probe
    // color as that texel's seed, then returns it with trust 1. The mix path
    // (trust 0) is the same color. Either way the shown value is the probe,
    // not black and not a cell.
    let miss_seeded = near(miss.seed, probe) && (miss.world_w - 1.0).abs() <= ABS;
    let miss_mixed = miss.world_w.abs() <= ABS;
    assert!(
        miss_seeded || miss_mixed,
        "miss fallback was not the probe seed and not a probe mix: seed {:?} trust {}",
        miss.seed,
        miss.world_w
    );

    let detail_off = &rows[2];
    assert_eq!(detail_off.head_hi, off.head_hi);
    assert!(
        near(detail_off.shown, probe),
        "detail off with an empty cache showed {:?}, wanted the probe",
        detail_off.shown
    );
    assert!(detail_off.reads >= 1.0);
    assert!(detail_off.skipped.abs() <= ABS);

    let lamp_empty = &rows[3];
    assert!(
        near(lamp_empty.shown, probe),
        "a lamp change with an empty owner showed {:?}, wanted the probe, not the old green seed",
        lamp_empty.shown
    );
    assert!(
        lamp_empty.conf.abs() <= ABS || near(lamp_empty.seed, probe),
        "the empty owner left {:?}, conf {}",
        lamp_empty.seed,
        lamp_empty.conf
    );
    assert!(
        !near(lamp_empty.seed, green),
        "stale green seed {:?}",
        lamp_empty.seed
    );
    assert!(
        lamp_empty.reads >= 1.0,
        "the empty owner still needs probes"
    );
    assert!(lamp_empty.skipped.abs() <= ABS);
    assert!(
        near(lamp_empty.cell, [0.0; 3]),
        "the empty cell was filled {:?}",
        lamp_empty.cell
    );
    // The fallback read may stamp the cell's read time. It must not store light.

    let sampled = &rows[4];
    assert!(
        near(sampled.shown, own),
        "own samples showed {:?}, wanted the texel mean",
        sampled.shown
    );
    assert!(
        (sampled.world_w - 1.0).abs() <= ABS,
        "own trust {}",
        sampled.world_w
    );
    assert!(
        sampled.reads >= 1.0,
        "trust 1 from own samples is not coverage, reads {}",
        sampled.reads
    );
    assert!(sampled.skipped.abs() <= ABS);
    assert!((sampled.own - 1.0).abs() <= ABS);
    assert!((sampled.cell_same - 1.0).abs() <= ABS);

    let slant = &rows[5];
    assert!(
        near(slant.shown, probe),
        "a slanted pixel showed {:?}, wanted the probe. The axis SH read is empty and compose keeps the probes",
        slant.shown
    );
    assert!(slant.world_w.abs() <= ABS, "slant trust {}", slant.world_w);
    assert!(slant.reads >= 1.0);
    assert!(slant.skipped.abs() <= ABS);
    assert!(
        slant.conf.abs() <= ABS,
        "a slant stored a texel seed, conf {}",
        slant.conf
    );
    assert!((slant.cell_same - 1.0).abs() <= ABS);

    let mover = &rows[6];
    assert!(
        near(mover.shown, probe),
        "a mover showed {:?}, wanted the probe, not the cell",
        mover.shown
    );
    assert!(mover.reads >= 1.0);
    assert!(mover.skipped.abs() <= ABS);
    assert!(
        mover.conf.abs() <= ABS,
        "a mover stored the cell seed, conf {} seed {:?}",
        mover.conf,
        mover.seed
    );
    assert!(near(mover.cell, red), "mover cell {:?}", mover.cell);
    assert!((mover.cell_same - 1.0).abs() <= ABS);

    let quiet = &rows[7];
    assert!(
        near(quiet.shown, red),
        "denoise off hid the cell {:?}",
        quiet.shown
    );
    assert!((quiet.world_w - 1.0).abs() <= ABS);
    assert!(quiet.reads.abs() <= ABS, "denoise off still read probes");
    assert!((quiet.skipped - 1.0).abs() <= ABS);
    assert!(near(quiet.seed, red), "denoise off seed {:?}", quiet.seed);
    assert!((quiet.cell_same - 1.0).abs() <= ABS);

    let mode0 = &rows[8];
    assert_eq!(mode0.head_hi, low.head_hi);
    assert!(
        near(mode0.shown, probe),
        "mode 0 showed {:?}, wanted the probe. The detail bit does not retire probes without the cache",
        mode0.shown
    );
    assert!(mode0.reads >= 1.0);
    assert!(mode0.skipped.abs() <= ABS);

    let lamp_blue = &rows[9];
    assert!(
        near(lamp_blue.shown, blue),
        "a live owner after the lamp showed {:?}, wanted blue",
        lamp_blue.shown
    );
    assert!(
        near(lamp_blue.seed, blue),
        "seed after the lamp {:?}",
        lamp_blue.seed
    );
    assert!((lamp_blue.conf - 0.25).abs() <= ABS);
    assert!(lamp_blue.reads.abs() <= ABS, "the blue owner is covered");
    assert!((lamp_blue.skipped - 1.0).abs() <= ABS);
    assert!(near(lamp_blue.cell, blue));
    assert!((lamp_blue.cell_same - 1.0).abs() <= ABS);

    let skip = words[2].to_bits();
    let eval = words[3].to_bits();
    let reads = words[4].to_bits();
    assert_eq!(skip, 3, "covered, denoise-off, and the blue owner skip");
    assert_eq!(eval, 7, "the other rows still evaluate probe indirect");
    assert_eq!(reads, 7, "one probe irradiance read on each evaluated row");
}
