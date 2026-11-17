//! Low-preset mix cost. 32 voices, one 480-frame quantum, at 48 kHz.

use std::time::Instant;

use genos_audio::{mix_into, Listener, Source, Vec3};
use genos_load::Pcm;

const PLACES: [Vec3; 32] = [
    Vec3::new(-8.0, 1.2, -2.0),
    Vec3::new(-7.5, 1.2, -2.2),
    Vec3::new(-7.0, 1.2, -2.4),
    Vec3::new(-6.5, 1.2, -2.6),
    Vec3::new(-6.0, 1.2, -2.8),
    Vec3::new(-5.5, 1.2, -3.0),
    Vec3::new(-5.0, 1.2, -3.2),
    Vec3::new(-4.5, 1.2, -3.4),
    Vec3::new(-4.0, 1.2, -3.6),
    Vec3::new(-3.5, 1.2, -3.8),
    Vec3::new(-3.0, 1.2, -4.0),
    Vec3::new(-2.5, 1.2, -4.2),
    Vec3::new(-2.0, 1.2, -4.4),
    Vec3::new(-1.5, 1.2, -4.6),
    Vec3::new(-1.0, 1.2, -4.8),
    Vec3::new(-0.5, 1.2, -5.0),
    Vec3::new(0.0, 1.2, -5.2),
    Vec3::new(0.5, 1.2, -5.4),
    Vec3::new(1.0, 1.2, -5.6),
    Vec3::new(1.5, 1.2, -5.8),
    Vec3::new(2.0, 1.2, -6.0),
    Vec3::new(2.5, 1.2, -6.2),
    Vec3::new(3.0, 1.2, -6.4),
    Vec3::new(3.5, 1.2, -6.6),
    Vec3::new(4.0, 1.2, -6.8),
    Vec3::new(4.5, 1.2, -7.0),
    Vec3::new(5.0, 1.2, -7.2),
    Vec3::new(5.5, 1.2, -7.4),
    Vec3::new(6.0, 1.2, -7.6),
    Vec3::new(6.5, 1.2, -7.8),
    Vec3::new(7.0, 1.2, -8.0),
    Vec3::new(7.5, 1.2, -8.2),
];

#[test]
fn low_quantum_stays_under_one_millisecond() {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..4_096)
            .map(|index: i32| {
                let wrapped = index.rem_euclid(200);
                i16::try_from(wrapped * 40).unwrap_or(0)
            })
            .collect(),
    };
    let listener = Listener {
        position: Vec3::new(0.0, 1.7, 0.0),
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    };
    let sources: Vec<Source<'_>> = PLACES
        .iter()
        .map(|position| Source {
            pcm: &pcm,
            position: *position,
            velocity: Vec3::new(0.0, 0.0, 4.0),
            looping: true,
            positional: true,
            transmission: None,
        })
        .collect();
    let mut out = vec![0.0f32; 480 * 2];
    for _ in 0..4 {
        mix_into(&mut out, &sources, &listener, &[], 48_000);
    }
    let mut samples = [0u128; 20];
    for sample in &mut samples {
        let start = Instant::now();
        mix_into(&mut out, &sources, &listener, &[], 48_000);
        *sample = start.elapsed().as_micros();
    }
    samples.sort_unstable();
    let best = samples[0];
    let median = samples[10];
    eprintln!("low_mix_us_min={best} low_mix_us_median={median}");
    let ceiling = if cfg!(debug_assertions) { 5_000 } else { 1_000 };
    assert!(
        median < ceiling,
        "32 voices x 480 frames median {median} us (min {best})"
    );
}
