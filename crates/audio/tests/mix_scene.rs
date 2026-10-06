//! The shipped mix, fed by loaded PCM. Pitch, image, and level are measured on the buffer.

use genos_audio::{mix, Barrier, Listener, Source, Vec3};
use genos_load::{load_pcm, MemorySource, Pcm};

const RATE: u32 = 48_000;
const FRAMES: usize = 16_384;
const TONE: f32 = 480.0;

#[test]
fn positional_pitch_follows_airborne_doppler() {
    let pcm = tone(RATE, 1, TONE, RATE as usize, 12_000);
    let listener = hear(
        Vec3::new(0.0, 1.7, 0.0),
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::ZERO,
    );
    let place = Vec3::new(0.0, 1.7, -8.0);
    let rest = play(&pcm, place, Vec3::ZERO, &listener, &[]);
    let approach = play(&pcm, place, Vec3::new(0.0, 0.0, 20.0), &listener, &[]);
    let faster = play(&pcm, place, Vec3::new(0.0, 0.0, 40.0), &listener, &[]);
    let retreat = play(&pcm, place, Vec3::new(0.0, 0.0, -20.0), &listener, &[]);
    let sideways = play(&pcm, place, Vec3::new(20.0, 0.0, 0.0), &listener, &[]);
    let moving_listener = hear(
        Vec3::new(0.0, 1.7, 0.0),
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(0.0, 0.0, -20.0),
    );
    let listener_approach = play(&pcm, place, Vec3::ZERO, &moving_listener, &[]);
    let listener_retreat = play(
        &pcm,
        place,
        Vec3::ZERO,
        &hear(
            Vec3::new(0.0, 1.7, 0.0),
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, 20.0),
        ),
        &[],
    );

    let rest_hz = hz(&rest);
    assert!(rest_hz > 400.0, "rest tone was not measurable: {rest_hz}");
    let approach_ratio = hz(&approach) / rest_hz;
    let faster_ratio = hz(&faster) / rest_hz;
    let retreat_ratio = hz(&retreat) / rest_hz;
    let side_ratio = hz(&sideways) / rest_hz;
    let listener_ratio = hz(&listener_approach) / rest_hz;
    let listener_away = hz(&listener_retreat) / rest_hz;

    assert!(
        (approach_ratio - airborne(20.0, 0.0)).abs() < 0.02,
        "approach ratio {approach_ratio}"
    );
    assert!(
        (faster_ratio - airborne(40.0, 0.0)).abs() < 0.02,
        "faster ratio {faster_ratio}"
    );
    assert!(faster_ratio > approach_ratio + 0.03);
    assert!(
        (retreat_ratio - airborne(-20.0, 0.0)).abs() < 0.02,
        "retreat ratio {retreat_ratio}"
    );
    assert!(retreat_ratio < 0.99);
    assert!(
        (side_ratio - 1.0).abs() < 0.02,
        "sideways ratio {side_ratio}"
    );
    assert!(
        (listener_ratio - airborne(0.0, 20.0)).abs() < 0.02,
        "listener approach ratio {listener_ratio}"
    );
    assert!(
        (listener_away - airborne(0.0, -20.0)).abs() < 0.02,
        "listener retreat ratio {listener_away}"
    );
}

#[test]
fn positional_image_and_distance() {
    let pcm = tone(RATE, 1, TONE, RATE as usize, 12_000);
    let at = Vec3::new(0.0, 1.7, 0.0);
    let front_facing = Vec3::new(0.0, 0.0, -1.0);
    let listener = hear(at, front_facing, Vec3::ZERO);
    let front = play(&pcm, Vec3::new(0.0, 1.7, -4.0), Vec3::ZERO, &listener, &[]);
    let left = play(&pcm, Vec3::new(-4.0, 1.7, 0.0), Vec3::ZERO, &listener, &[]);
    let behind_left = play(&pcm, Vec3::new(-3.0, 1.7, 4.0), Vec3::ZERO, &listener, &[]);
    let turned = hear(at, Vec3::new(0.0, 0.0, 1.0), Vec3::ZERO);
    let swapped = play(&pcm, Vec3::new(-4.0, 1.7, 0.0), Vec3::ZERO, &turned, &[]);
    let near = play(&pcm, Vec3::new(0.0, 1.7, -2.0), Vec3::ZERO, &listener, &[]);
    let far = play(&pcm, Vec3::new(0.0, 1.7, -4.0), Vec3::ZERO, &listener, &[]);
    let very_close = play(&pcm, Vec3::new(0.0, 1.7, -0.05), Vec3::ZERO, &listener, &[]);
    let one_meter = play(&pcm, Vec3::new(0.0, 1.7, -1.0), Vec3::ZERO, &listener, &[]);

    let front_ratio = rms(&front, 0) / rms(&front, 1);
    assert!(
        (front_ratio - 1.0).abs() < 0.08,
        "front balance {front_ratio}"
    );
    assert!(rms(&left, 0) > rms(&left, 1) * 8.0, "left image");
    assert!(
        rms(&behind_left, 0) > rms(&behind_left, 1) * 2.0,
        "behind-left image"
    );
    assert!(rms(&swapped, 1) > rms(&swapped, 0) * 8.0, "turned image");
    let distance_ratio = rms(&far, 0) / rms(&near, 0);
    assert!(
        (distance_ratio - 0.5).abs() < 0.08,
        "distance ratio {distance_ratio}"
    );
    let close_ratio = rms(&very_close, 0) / rms(&one_meter, 0);
    assert!(
        close_ratio.is_finite() && (close_ratio - 1.0).abs() < 0.12,
        "close ratio {close_ratio}"
    );
}

#[test]
fn barriers_change_positional_level_only() {
    let pcm = tone(RATE, 1, TONE, RATE as usize, 12_000);
    let listener = hear(
        Vec3::new(0.0, 1.7, 0.0),
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::ZERO,
    );
    let place = Vec3::new(0.0, 1.7, -8.0);
    let clear = play(&pcm, place, Vec3::ZERO, &listener, &[]);
    let thin = slab(-4.2, -4.0, 2.0);
    let thick = slab(-5.0, -4.0, 2.0);
    let transmitting = slab(-4.3, -4.0, 0.1);
    let blocking = slab(-4.4, -4.0, 25.0);
    let first = slab(-3.25, -3.0, 2.0);
    let second = slab(-5.25, -5.0, 2.0);
    let through_thin = play(&pcm, place, Vec3::ZERO, &listener, &[thin]);
    let through_thick = play(&pcm, place, Vec3::ZERO, &listener, &[thick]);
    let through_soft = play(&pcm, place, Vec3::ZERO, &listener, &[transmitting]);
    let through_hard = play(&pcm, place, Vec3::ZERO, &listener, &[blocking]);
    let through_one = play(&pcm, place, Vec3::ZERO, &listener, &[first]);
    let through_two = play(&pcm, place, Vec3::ZERO, &listener, &[first, second]);

    let clear_level = level(&clear);
    assert!(clear_level > level(&through_thin) * 1.2);
    assert!(level(&through_thick) < level(&through_thin) * 0.5);
    assert!(level(&through_soft) > clear_level * 0.85);
    assert!(level(&through_hard) < clear_level * 0.05);
    assert!(level(&through_two) < level(&through_one) * 0.85);
    assert!(
        level(&through_two) < level(&play(&pcm, place, Vec3::ZERO, &listener, &[second])) * 0.85
    );

    let music = tone(RATE, 1, 220.0, 2_000, 8_000);
    let dry = Source {
        pcm: &music,
        position: place,
        velocity: Vec3::new(0.0, 0.0, 40.0),
        looping: true,
        positional: false,
        transmission: Some(0.0),
    };
    let open = mix(std::slice::from_ref(&dry), &listener, &[], RATE, 4_000);
    let walled = mix(
        std::slice::from_ref(&dry),
        &listener,
        &[blocking, thick],
        RATE,
        4_000,
    );
    assert_eq!(open, walled);

    let muted = Source {
        pcm: &pcm,
        position: place,
        velocity: Vec3::ZERO,
        looping: true,
        positional: true,
        transmission: Some(0.0),
    };
    let silent = mix(std::slice::from_ref(&muted), &listener, &[], RATE, 2_000);
    assert!(level(&silent) < 1.0);
}

#[test]
fn one_shot_loop_and_simultaneous_sources() {
    let shot = tone(RATE, 1, TONE, 200, 10_000);
    let listener = hear(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO);
    let once = Source {
        pcm: &shot,
        position: Vec3::ZERO,
        velocity: Vec3::ZERO,
        looping: false,
        positional: false,
        transmission: None,
    };
    let ended = mix(std::slice::from_ref(&once), &listener, &[], RATE, 800);
    assert!(level(&ended[..400]) > 100.0);
    assert!(ended[400..].iter().all(|sample| *sample == 0));

    let again = Source {
        looping: true,
        ..once
    };
    let looped = mix(std::slice::from_ref(&again), &listener, &[], RATE, 800);
    assert!(level(&looped[600..]) > 100.0);
    assert_eq!(looped[0], looped[400]);
    assert_eq!(looped[1], looped[401]);

    let low = tone(24_000, 1, 300.0, 24_000, 4_000);
    let high = stereo_right(RATE, 900.0, RATE as usize, 4_000);
    let low_source = Source {
        pcm: &low,
        position: Vec3::new(-3.0, 1.0, -3.0),
        velocity: Vec3::new(10.0, 60.0, 10.0),
        looping: true,
        positional: true,
        transmission: None,
    };
    let high_source = Source {
        pcm: &high,
        position: Vec3::ZERO,
        velocity: Vec3::new(0.0, 0.0, 50.0),
        looping: false,
        positional: false,
        transmission: None,
    };
    let both = mix(&[low_source, high_source], &listener, &[], RATE, FRAMES);
    let only_low = mix(
        std::slice::from_ref(&low_source),
        &listener,
        &[],
        RATE,
        FRAMES,
    );
    let only_high = mix(
        std::slice::from_ref(&high_source),
        &listener,
        &[],
        RATE,
        FRAMES,
    );
    assert_ne!(both, only_low);
    assert_ne!(both, only_high);
    assert!(power(&both, 300.0, 0) > power(&both, 1500.0, 0) * 8.0);
    assert!(power(&only_high, 900.0, 1) > power(&only_high, 900.0, 0) * 8.0);
    assert!(
        (hz(&only_low) - 300.0).abs() < 8.0,
        "resampled {}",
        hz(&only_low)
    );
    let low_rest = Source {
        velocity: Vec3::ZERO,
        ..low_source
    };
    let sideways = mix(
        std::slice::from_ref(&low_source),
        &listener,
        &[],
        RATE,
        FRAMES,
    );
    let still = mix(
        std::slice::from_ref(&low_rest),
        &listener,
        &[],
        RATE,
        FRAMES,
    );
    assert!((hz(&sideways) - hz(&still)).abs() < 8.0);
}

fn airborne(source_closing: f32, listener_closing: f32) -> f32 {
    (343.0 + listener_closing) / (343.0 - source_closing)
}

fn hear(position: Vec3, facing: Vec3, velocity: Vec3) -> Listener {
    Listener {
        position,
        facing,
        velocity,
    }
}

fn play(
    pcm: &Pcm,
    position: Vec3,
    velocity: Vec3,
    listener: &Listener,
    barriers: &[Barrier],
) -> Vec<i16> {
    let source = Source {
        pcm,
        position,
        velocity,
        looping: true,
        positional: true,
        transmission: None,
    };
    mix(
        std::slice::from_ref(&source),
        listener,
        barriers,
        RATE,
        FRAMES,
    )
}

fn slab(z0: f32, z1: f32, absorption: f32) -> Barrier {
    Barrier {
        min: Vec3::new(-2.0, 0.0, z0),
        max: Vec3::new(2.0, 3.0, z1),
        absorption,
    }
}

fn tone(rate: u32, channels: u16, freq: f32, frames: usize, amp: i16) -> Pcm {
    let mut samples = Vec::with_capacity(frames * channels as usize);
    for frame in 0..frames {
        let t = frame as f32 / rate as f32;
        let sample = (t * freq * std::f32::consts::TAU).sin();
        let value = (sample * f32::from(amp)).round() as i16;
        for _ in 0..channels {
            samples.push(value);
        }
    }
    load_pcm(&MemorySource::new(&wav(rate, channels, &samples))).expect("pcm")
}

fn stereo_right(rate: u32, freq: f32, frames: usize, amp: i16) -> Pcm {
    let mut samples = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        let t = frame as f32 / rate as f32;
        let sample = (t * freq * std::f32::consts::TAU).sin();
        let value = (sample * f32::from(amp)).round() as i16;
        samples.push(0);
        samples.push(value);
    }
    load_pcm(&MemorySource::new(&wav(rate, 2, &samples))).expect("pcm")
}

fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_bytes = samples.len() * 2;
    let align = channels * 2;
    let mut bytes = Vec::with_capacity(44 + data_bytes);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
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
    crossings as f32 * RATE as f32 / frames as f32
}

fn rms(interleaved: &[i16], channel: usize) -> f32 {
    let frames = interleaved.len() / 2;
    if frames == 0 {
        return 0.0;
    }
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

fn power(interleaved: &[i16], freq: f32, channel: usize) -> f32 {
    let frames = interleaved.len() / 2;
    let omega = std::f32::consts::TAU * freq / RATE as f32;
    let coeff = 2.0 * omega.cos();
    let mut s1 = 0.0f32;
    let mut s2 = 0.0f32;
    for frame in 0..frames {
        let sample = f32::from(interleaved[frame * 2 + channel]);
        let s0 = sample + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / frames as f32
}
