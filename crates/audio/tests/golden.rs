//! The offline mix of a stored tape. The hash does not open a device.

use genos_audio::{mix, Barrier, Listener, Source, Vec3};
use genos_load::{hex, sha256, Pcm};

const TAPE: &[u8] = include_bytes!("golden/tape.pcm");

#[test]
fn stored_tape_mix_matches_the_golden_hash() {
    let pcm = tape();
    let mixed = mix(
        std::slice::from_ref(&scene(&pcm)),
        &listener(),
        &[wall()],
        48_000,
        64,
    );
    let mut bytes = Vec::with_capacity(mixed.len() * 2);
    for sample in &mixed {
        bytes.extend(sample.to_le_bytes());
    }
    let digest = hex(&sha256(&bytes));
    let expected = include_str!("golden/positional.sha256").trim();
    assert_eq!(digest, expected, "mixed samples {mixed:?}");
}

fn tape() -> Pcm {
    let mut samples = Vec::with_capacity(TAPE.len() / 2);
    for pair in TAPE.as_chunks::<2>().0 {
        samples.push(i16::from_le_bytes(*pair));
    }
    Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples,
    }
}

fn scene(pcm: &Pcm) -> Source<'_> {
    Source {
        pcm,
        position: Vec3::new(-1.5, 1.2, -4.0),
        velocity: Vec3::new(3.0, 0.0, 12.0),
        looping: true,
        positional: true,
        transmission: None,
    }
}

fn listener() -> Listener {
    Listener {
        position: Vec3::new(0.0, 1.7, 0.0),
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::new(0.0, 0.0, -2.0),
    }
}

fn wall() -> Barrier {
    Barrier {
        min: Vec3::new(-0.8, 0.0, -2.2),
        max: Vec3::new(0.8, 3.0, -2.0),
        absorption: 1.5,
    }
}
