//! A disk-fed clip plays from the ring. An empty ring counts `audio_underrun`.

#![cfg_attr(
    test,
    expect(
        clippy::unwrap_used,
        reason = "a test aborts when the stream or the cook fails"
    )
)]

use genos_audio::{start_disk, Command, FakeDevice, Listener, Vec3};
use genos_load::{cook_ima, resident_clip, Pcm};

fn long_clip() -> genos_load::CookedAudio {
    let mut samples = Vec::with_capacity(140_000);
    for index in 0..140_000u32 {
        let sample = if index % 64 < 32 { 8_000 } else { -8_000 };
        samples.push(sample);
    }
    let pcm = Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples,
    };
    assert!(!resident_clip(pcm.sample_rate, pcm.channels, 140_000));
    let bytes = cook_ima(&pcm, 0, 140_000).unwrap();
    genos_load::parse_cooked(&bytes).unwrap()
}

#[test]
fn a_filled_ring_writes_samples() {
    let (device, capture) = FakeDevice::new(64, 256);
    capture.hold();
    let cooked = long_clip();
    let stream = start_disk(device, cooked, &[], true).unwrap();
    let began = std::time::Instant::now();
    while capture.parked() == 0 && began.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::yield_now();
    }
    let began = std::time::Instant::now();
    while stream.stats().disk_queued < 128 && began.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::yield_now();
    }
    assert!(
        stream.stats().disk_queued >= 128,
        "the io thread did not fill"
    );
    stream.set_listener(Listener {
        position: Vec3::ZERO,
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    });
    assert!(stream.push(Command::Play {
        voice: 0,
        clip: 0,
        positional: false,
        looping: true,
        gain: 1.0,
        position: Vec3::ZERO,
        velocity: Vec3::ZERO,
    }));
    capture.release(1);
    let began = std::time::Instant::now();
    while capture.filled() < 128 && began.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::yield_now();
    }
    capture.release(usize::MAX);
    stream.stop();
    assert!(capture.samples().iter().any(|sample| *sample != 0));
}

#[test]
fn an_empty_ring_counts_an_underrun() {
    let (device, capture) = FakeDevice::new(64, 256);
    capture.hold();
    let cooked = long_clip();
    let stream = start_disk(device, cooked, &[], false).unwrap();
    let began = std::time::Instant::now();
    while capture.parked() == 0 && began.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::yield_now();
    }
    assert_eq!(stream.stats().disk_queued, 0);
    stream.set_listener(Listener {
        position: Vec3::ZERO,
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    });
    assert!(stream.push(Command::Play {
        voice: 0,
        clip: 0,
        positional: false,
        looping: true,
        gain: 1.0,
        position: Vec3::ZERO,
        velocity: Vec3::ZERO,
    }));
    capture.release(1);
    let began = std::time::Instant::now();
    while capture.filled() < 128 && began.elapsed() < std::time::Duration::from_secs(2) {
        std::thread::yield_now();
    }
    let underruns = stream.stats().audio_underrun;
    capture.release(usize::MAX);
    stream.stop();
    assert!(underruns >= 1, "audio_underrun stayed {underruns}");
}
