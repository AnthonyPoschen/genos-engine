#![expect(
    clippy::as_conversions,
    reason = "TODO(lint-2026-10-14): pre-existing cast. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::cast_precision_loss,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::expect_used,
    reason = "TODO(lint-2026-10-14): pre-existing. docs/LINT_RULES.md"
)]

//! Submit one mixed buffer through the platform device.

use genos_audio::{mix, start_with, AlsaDevice, Barrier, Command, Listener, Source, Vec3};
use genos_load::{load_pcm, MemorySource};

fn main() {
    let mut dry = Vec::with_capacity(4_800);
    for frame in 0..4_800 {
        let t = frame as f32 / 48_000.0;
        dry.push(
            (t * 440.0 * std::f32::consts::TAU)
                .sin()
                .mul_add(3_000.0, 0.0)
                .round() as i16,
        );
    }
    let pcm = load_pcm(&MemorySource::new(&wav(&dry))).expect("pcm");
    let listener = Listener {
        position: Vec3::new(0.0, 1.7, 0.0),
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    };
    let source = Source {
        pcm: &pcm,
        position: Vec3::new(-2.0, 1.7, -4.0),
        velocity: Vec3::new(0.0, 0.0, 15.0),
        looping: true,
        positional: true,
        transmission: None,
    };
    let wall = Barrier {
        min: Vec3::new(-1.0, 0.0, -2.3),
        max: Vec3::new(1.0, 3.0, -2.0),
        absorption: 1.2,
    };
    let mixed = mix(
        std::slice::from_ref(&source),
        &listener,
        &[wall],
        48_000,
        2_400,
    );
    let dry_fnv = fnv(&pcm.samples);
    let mix_fnv = fnv(&mixed);
    println!("dry_fnv={dry_fnv}");
    println!("mix_fnv={mix_fnv}");
    println!("samples={}", mixed.len());
    println!("head={},{},{},{}", mixed[0], mixed[1], mixed[2], mixed[3]);
    if mixed.iter().all(|sample| *sample == 0) || mix_fnv == dry_fnv {
        eprintln!("mix was silence or the dry clip");
        genos_window::exit(1);
    }
    let stream = match start_with(AlsaDevice::new(), vec![pcm], &[wall]) {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("alsa open failed: {err}");
            genos_window::exit(2);
        }
    };
    stream.set_listener(listener);
    let playing = stream.push(Command::Play {
        voice: 0,
        clip: 0,
        positional: true,
        looping: true,
        gain: 1.0,
        position: Vec3::new(-2.0, 1.7, -4.0),
        velocity: Vec3::new(0.0, 0.0, 15.0),
    });
    if !playing {
        eprintln!("command ring rejected play");
        stream.stop();
        genos_window::exit(1);
    }
    let started = std::time::Instant::now();
    while stream.stats().periods_written < 2
        && started.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::yield_now();
    }
    let periods = stream.stats().periods_written;
    println!("periods={periods}");
    stream.stop();
    if periods == 0 {
        eprintln!("the device wrote no quantum");
        genos_window::exit(1);
    }
}

fn wav(samples: &[i16]) -> Vec<u8> {
    let data_bytes = samples.len() * 2;
    let mut bytes = Vec::with_capacity(44 + data_bytes);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&(96_000u32).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn fnv(samples: &[i16]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in samples {
        for byte in sample.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}
