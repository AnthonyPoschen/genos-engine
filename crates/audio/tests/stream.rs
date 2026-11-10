#![cfg_attr(
    test,
    expect(clippy::panic, reason = "a test aborts when the stream fails to start")
)]

//! Held device, command ring, and the quantum callback.

use std::time::{Duration, Instant};

use genos_audio::{
    allocating_mix_entries, mix, start_with, Barrier, Command, FakeDevice, Listener, Source, Vec3,
};
use genos_load::{hex, sha256, Pcm};

const TAPE: &[u8] = include_bytes!("golden/tape.pcm");

#[test]
fn the_callback_writes_the_mix_and_does_not_call_mix() {
    let pcm = tape();
    let (device, capture) = FakeDevice::new(64, 64);
    capture.hold();
    let before = allocating_mix_entries();
    let stream = match start_with(device, vec![pcm.clone()], &[wall()]) {
        Ok(stream) => stream,
        Err(err) => panic!("stream did not start: {err}"),
    };
    assert!(wait_parked(&capture));
    stream.set_listener(listener());
    assert!(stream.push(play_command()));
    capture.release(1);
    assert!(wait_until(&capture, 128));
    assert_eq!(allocating_mix_entries(), before);
    let stats = stream.stats();
    assert_eq!(stats.device_period_frames, 64);
    stream.stop();
    let offline = mix(
        std::slice::from_ref(&scene(&pcm)),
        &listener(),
        &[wall()],
        48_000,
        64,
    );
    assert_eq!(&capture.samples()[..128], &offline[..128]);
    let mut bytes = Vec::with_capacity(offline.len() * 2);
    for sample in &offline {
        bytes.extend(sample.to_le_bytes());
    }
    let expected = include_str!("golden/positional.sha256").trim();
    assert_eq!(hex(&sha256(&bytes)), expected);
}

#[test]
fn a_full_ring_drops_the_newest_command_and_the_listener_still_updates() {
    let pcm = tape();
    let (device, capture) = FakeDevice::new(64, 256);
    capture.hold();
    let stream = match start_with(device, vec![pcm.clone()], &[]) {
        Ok(stream) => stream,
        Err(err) => panic!("stream did not start: {err}"),
    };
    assert!(wait_parked(&capture));
    assert!(stream.push(play_command()));
    let ear = Listener {
        position: Vec3::new(4.0, 1.0, 2.0),
        facing: Vec3::new(1.0, 0.0, 0.0),
        velocity: Vec3::ZERO,
    };
    stream.set_listener(ear);
    let mut dropped = false;
    for _ in 0..400 {
        if !stream.push(Command::Stop { voice: 1 }) {
            dropped = true;
            break;
        }
    }
    assert!(dropped);
    assert!(stream.stats().audio_command_drop >= 1);
    capture.release(1);
    assert!(wait_until(&capture, 128));
    stream.stop();
    let offline = mix(std::slice::from_ref(&scene(&pcm)), &ear, &[], 48_000, 64);
    assert_eq!(&capture.samples()[..128], &offline[..128]);
}

#[test]
fn a_rejected_write_counts_a_device_xrun() {
    let pcm = tape();
    let (mut device, _capture) = FakeDevice::new(32, 32);
    device.fail_next(1);
    let stream = match start_with(device, vec![pcm], &[]) {
        Ok(stream) => stream,
        Err(err) => panic!("stream did not start: {err}"),
    };
    let started = Instant::now();
    while stream.stats().device_xrun == 0 && started.elapsed() < Duration::from_secs(2) {
        std::thread::yield_now();
    }
    let stats = stream.stats();
    stream.stop();
    assert!(stats.device_xrun >= 1);
    assert_eq!(stats.device_period_frames, 32);
}

fn wait_parked(capture: &genos_audio::FakeCapture) -> bool {
    let started = Instant::now();
    while capture.parked() == 0 && started.elapsed() < Duration::from_secs(2) {
        std::thread::yield_now();
    }
    capture.parked() > 0
}

fn wait_until(capture: &genos_audio::FakeCapture, samples: usize) -> bool {
    let started = Instant::now();
    while capture.filled() < samples && started.elapsed() < Duration::from_secs(2) {
        std::thread::yield_now();
    }
    capture.filled() >= samples
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

fn play_command() -> Command {
    Command::Play {
        voice: 0,
        clip: 0,
        positional: true,
        looping: true,
        gain: 1.0,
        position: Vec3::new(-1.5, 1.2, -4.0),
        velocity: Vec3::new(3.0, 0.0, 12.0),
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
