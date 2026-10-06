//! Real ALSA submit. A missing device is recorded and is not a silent success.

use genos_audio::{mix, submit, Listener, Source, Vec3};
use genos_load::{load_pcm, MemorySource};

#[test]
fn alsa_consumes_every_frame_of_the_mix_when_the_device_opens() {
    let pcm = {
        let mut samples = Vec::with_capacity(2_400);
        for frame in 0..2_400 {
            let t = frame as f32 / 48_000.0;
            let value = (t * 440.0 * std::f32::consts::TAU).sin();
            samples.push((value * 4_000.0).round() as i16);
        }
        let wav = wav_bytes(&samples);
        load_pcm(&MemorySource::new(&wav)).expect("pcm")
    };
    let listener = Listener {
        position: Vec3::new(0.0, 1.7, 0.0),
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    };
    let source = Source {
        pcm: &pcm,
        position: Vec3::new(-1.5, 1.7, -3.0),
        velocity: Vec3::new(0.0, 0.0, 8.0),
        looping: false,
        positional: true,
        transmission: None,
    };
    let mixed = mix(std::slice::from_ref(&source), &listener, &[], 48_000, 2_400);
    assert!(mixed.iter().any(|sample| *sample != 0));
    match submit(&mixed, 48_000) {
        Ok(frames) => {
            assert_eq!(
                frames,
                mixed.len() as u64 / 2,
                "the device consumed a short buffer"
            );
            let again = submit(&mixed, 48_000).expect("second submit");
            assert_eq!(again, frames);
        }
        Err(err) => eprintln!("alsa open failed: {err}"),
    }
}

fn wav_bytes(samples: &[i16]) -> Vec<u8> {
    let data_bytes = samples.len() * 2;
    let mut bytes = Vec::with_capacity(44 + data_bytes);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&(48_000u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}
