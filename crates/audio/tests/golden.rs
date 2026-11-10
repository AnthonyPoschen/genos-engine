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

#[test]
fn covered_paths_match_one_hash_and_each_control_moves_it() {
    let looped = loop_mix(true);
    let stopped = loop_mix(false);
    assert_ne!(digest(&looped), digest(&stopped), "loop wrap");
    assert!(looped[60..].iter().any(|sample| *sample != 0));
    assert!(stopped[20..].iter().all(|sample| *sample == 0));

    let doppler = doppler_mix(true, true);
    let doppler_once = doppler_mix(true, false);
    let still = doppler_mix(false, true);
    let still_once = doppler_mix(false, false);
    assert_ne!(digest(&doppler), digest(&doppler_once), "doppler wrap");
    assert_eq!(digest(&still), digest(&still_once), "a still cursor does not wrap");

    let absorbed = exp_mix(2.0);
    let open = exp_mix(0.0);
    let peak = |buffer: &[i16]| {
        let mut loudest = 0u16;
        for sample in buffer {
            loudest = loudest.max(sample.unsigned_abs());
        }
        loudest
    };
    assert!(peak(&absorbed) > 0, "scaled exp must stay above the noise floor");
    assert!(
        u32::from(peak(&absorbed)) * 20 < u32::from(peak(&open)),
        "a 2 m wall at 2 nepers needs a non-zero power of two"
    );
    assert_ne!(digest(&absorbed), digest(&open), "exp reduction");

    let clipped = clip_mix(20_000);
    let quiet = clip_mix(4_000);
    let solo = clip_one(20_000);
    assert!(clipped.contains(&i16::MAX), "two loud voices must clip");
    assert!(!quiet.contains(&i16::MAX));
    assert_ne!(digest(&clipped), digest(&quiet), "i16 clip");
    assert_ne!(digest(&clipped), digest(&solo), "two voices");

    let mut bytes = Vec::new();
    for buffer in [&looped, &doppler, &absorbed, &clipped] {
        for sample in buffer {
            bytes.extend(sample.to_le_bytes());
        }
    }
    assert_eq!(
        hex(&sha256(&bytes)),
        include_str!("golden/paths.sha256").trim()
    );
}

fn digest(samples: &[i16]) -> String {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend(sample.to_le_bytes());
    }
    hex(&sha256(&bytes))
}

fn loop_mix(looping: bool) -> Vec<i16> {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![0, 8_000, -8_000, 4_000],
    };
    let source = Source {
        pcm: &pcm,
        position: Vec3::ZERO,
        velocity: Vec3::ZERO,
        looping,
        positional: false,
        transmission: None,
    };
    mix(
        std::slice::from_ref(&source),
        &listener(),
        &[],
        48_000,
        32,
    )
}

fn doppler_mix(moving: bool, looping: bool) -> Vec<i16> {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..80).map(|index| i16::try_from(index * 200).unwrap_or(0)).collect(),
    };
    let source = Source {
        pcm: &pcm,
        position: Vec3::new(0.0, 1.0, -4.0),
        velocity: if moving {
            Vec3::new(0.0, 0.0, 120.0)
        } else {
            Vec3::ZERO
        },
        looping,
        positional: true,
        transmission: None,
    };
    let ear = Listener {
        position: Vec3::new(0.0, 1.0, 0.0),
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    };
    mix(std::slice::from_ref(&source), &ear, &[], 48_000, 64)
}

fn exp_mix(absorption: f32) -> Vec<i16> {
    let pcm = tape();
    let source = Source {
        pcm: &pcm,
        position: Vec3::new(0.0, 1.0, -5.0),
        velocity: Vec3::ZERO,
        looping: false,
        positional: true,
        transmission: None,
    };
    let ear = Listener {
        position: Vec3::new(0.0, 1.0, 0.0),
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    };
    let wall = Barrier {
        min: Vec3::new(-1.0, 0.0, -3.0),
        max: Vec3::new(1.0, 3.0, -1.0),
        absorption,
    };
    mix(std::slice::from_ref(&source), &ear, &[wall], 48_000, 32)
}

fn clip_mix(amplitude: i16) -> Vec<i16> {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![amplitude; 8],
    };
    let voice = Source {
        pcm: &pcm,
        position: Vec3::ZERO,
        velocity: Vec3::ZERO,
        looping: false,
        positional: false,
        transmission: None,
    };
    mix(&[voice, voice], &listener(), &[], 48_000, 8)
}

fn clip_one(amplitude: i16) -> Vec<i16> {
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: vec![amplitude; 8],
    };
    let source = Source {
        pcm: &pcm,
        position: Vec3::ZERO,
        velocity: Vec3::ZERO,
        looping: false,
        positional: false,
        transmission: None,
    };
    mix(std::slice::from_ref(&source), &listener(), &[], 48_000, 8)
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
