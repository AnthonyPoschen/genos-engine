//! Public mix from a package that does not own the mixer.

use genos_audio::{mix, Barrier, Listener, Source, Vec3};
use genos_load::{load_pcm, MemorySource, Pcm};

#[test]
fn approaching_left_source_behind_a_wall_and_a_dry_loop() {
    let positional = load_tone(48_000, 1, 660.0, 48_000, 12_000);
    let music = load_tone(48_000, 1, 220.0, 4_000, 2_000);
    let listener = Listener {
        position: Vec3::new(0.0, 1.7, 0.0),
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    };
    let voice = Source {
        pcm: &positional,
        position: Vec3::new(-2.0, 1.7, -3.0),
        velocity: Vec3::new(0.0, 0.0, 25.0),
        looping: true,
        positional: true,
        transmission: None,
    };
    let song = Source {
        pcm: &music,
        position: Vec3::new(4.0, 1.0, 2.0),
        velocity: Vec3::new(0.0, 0.0, 40.0),
        looping: true,
        positional: false,
        transmission: None,
    };
    let wall = Barrier {
        min: Vec3::new(-4.0, 0.0, -2.2),
        max: Vec3::new(1.0, 3.0, -0.8),
        absorption: 20.0,
    };
    let blocked = mix(&[voice, song], &listener, &[wall], 48_000, 8_192);
    let blocked_again = mix(&[voice, song], &listener, &[wall], 48_000, 8_192);
    assert_eq!(blocked, blocked_again);

    let clear = mix(&[voice, song], &listener, &[], 48_000, 8_192);
    let song_open = mix(std::slice::from_ref(&song), &listener, &[], 48_000, 8_192);
    let song_walled = mix(
        std::slice::from_ref(&song),
        &listener,
        &[wall],
        48_000,
        8_192,
    );
    assert_eq!(song_open, song_walled);

    let voice_only = Source {
        velocity: Vec3::ZERO,
        ..voice
    };
    let resting = mix(
        std::slice::from_ref(&voice_only),
        &listener,
        &[],
        48_000,
        8_192,
    );
    let moving = mix(std::slice::from_ref(&voice), &listener, &[], 48_000, 8_192);
    let rest_hz = hz(&resting);
    let move_ratio = hz(&moving) / rest_hz;
    let along = listener.position - voice.position;
    let closing = 25.0 * (along.z / along.length());
    let expected = 343.0 / (343.0 - closing);
    assert!(
        (move_ratio - expected).abs() < 0.025,
        "doppler ratio {move_ratio} rest {rest_hz}"
    );
    assert!(
        rms(&moving, 0) > rms(&moving, 1) * 2.0,
        "left of the listener"
    );
    assert!(
        level(&clear) > level(&blocked) * 1.5,
        "the wall cut the voice"
    );
    assert!(power(&blocked, 660.0) < power(&clear, 660.0) * 0.15);
    assert!(power(&blocked, 220.0) > power(&blocked, 660.0) * 4.0);
    println!("consumer_fnv={}", fnv(&blocked));
}

fn load_tone(rate: u32, channels: u16, freq: f32, frames: usize, amp: i16) -> Pcm {
    let mut samples = Vec::with_capacity(frames * channels as usize);
    for frame in 0..frames {
        let t = frame as f32 / rate as f32;
        let value = ((t * freq * std::f32::consts::TAU).sin() * f32::from(amp)).round() as i16;
        for _ in 0..channels {
            samples.push(value);
        }
    }
    load_pcm(&MemorySource::new(&wav(rate, channels, &samples))).expect("pcm")
}

fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_bytes = samples.len() * 2;
    let align = channels * 2;
    let mut bytes = Vec::with_capacity(44 + data_bytes);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    bytes.extend_from_slice(&align.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn hz(interleaved: &[i16]) -> f32 {
    let channel = if rms(interleaved, 0) >= rms(interleaved, 1) {
        0
    } else {
        1
    };
    let frames = interleaved.len() / 2;
    let mut previous = 0i32;
    let mut crossings = 0u32;
    for frame in 0..frames {
        let sample = i32::from(interleaved[frame * 2 + channel]);
        if previous <= 0 && sample > 0 {
            crossings += 1;
        }
        previous = sample;
    }
    crossings as f32 * 48_000.0 / frames as f32
}

fn rms(interleaved: &[i16], channel: usize) -> f32 {
    let frames = interleaved.len() / 2;
    let mut acc = 0.0f64;
    for frame in 0..frames {
        let sample = f64::from(interleaved[frame * 2 + channel]);
        acc += sample * sample;
    }
    (acc / frames as f64).sqrt() as f32
}

fn level(interleaved: &[i16]) -> f32 {
    rms(interleaved, 0) + rms(interleaved, 1)
}

fn power(interleaved: &[i16], freq: f32) -> f32 {
    let frames = interleaved.len() / 2;
    let coeff = 2.0 * (std::f32::consts::TAU * freq / 48_000.0).cos();
    let mut left = 0.0f32;
    let mut right = 0.0f32;
    for channel in 0..2 {
        let mut s1 = 0.0f32;
        let mut s2 = 0.0f32;
        for frame in 0..frames {
            let sample = f32::from(interleaved[frame * 2 + channel]);
            let s0 = sample + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let band = s1 * s1 + s2 * s2 - coeff * s1 * s2;
        if channel == 0 {
            left = band;
        } else {
            right = band;
        }
    }
    left + right
}

fn fnv(samples: &[i16]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in samples {
        hash ^= *sample as u16 as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
