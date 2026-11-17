//! Low and a listener outside a zone match the dry mix. Inside, the tail changes it.

use genos_audio::{
    apply_reverb, mix_into, zone_index, Listener, Quality, Reverb, Source, Vec3, Zone,
};
use genos_load::Pcm;

fn tone() -> Pcm {
    Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..256)
            .map(|index| if index % 16 < 8 { 12_000 } else { -12_000 })
            .collect(),
    }
}

fn dry(pcm: &Pcm) -> Vec<f32> {
    let source = Source {
        pcm,
        position: Vec3::new(0.0, 1.0, -2.0),
        velocity: Vec3::ZERO,
        looping: true,
        positional: false,
        transmission: None,
    };
    let listener = Listener {
        position: Vec3::new(0.0, 1.0, 0.0),
        facing: Vec3::new(0.0, 0.0, -1.0),
        velocity: Vec3::ZERO,
    };
    let mut out = vec![0.0; 64 * 2];
    mix_into(
        &mut out,
        std::slice::from_ref(&source),
        &listener,
        &[],
        48_000,
    );
    out
}

fn room(send: f32, tail: u8, min: Vec3, max: Vec3) -> Zone {
    Zone {
        min,
        max,
        send,
        tail,
    }
}

#[test]
fn low_and_outside_match_the_dry_buffer() {
    let pcm = tone();
    let original = dry(&pcm);
    let zone = room(1.0, 0, Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 3.0, 1.0));
    let mut low = original.clone();
    apply_reverb(
        &mut low,
        Vec3::new(0.0, 1.0, 0.0),
        std::slice::from_ref(&zone),
        Quality::Low,
        None,
    );
    assert_eq!(low, original);

    let mut outside = original.clone();
    let mut state = Reverb::new();
    apply_reverb(
        &mut outside,
        Vec3::new(8.0, 1.0, 8.0),
        std::slice::from_ref(&zone),
        Quality::Medium,
        Some(&mut state),
    );
    assert_eq!(outside, original);
}

#[test]
fn a_listener_inside_a_zone_changes_the_buffer() {
    let pcm = tone();
    let original = dry(&pcm);
    let zone = room(1.0, 0, Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 3.0, 2.0));
    let mut wet = original.clone();
    let mut state = Reverb::new();
    apply_reverb(
        &mut wet,
        Vec3::new(0.0, 1.0, 0.0),
        std::slice::from_ref(&zone),
        Quality::Medium,
        Some(&mut state),
    );
    assert_ne!(wet, original);
}

#[test]
fn the_smallest_box_wins_then_the_lower_tail() {
    let outer = room(0.2, 0, Vec3::new(-4.0, 0.0, -4.0), Vec3::new(4.0, 4.0, 4.0));
    let inner = room(0.9, 3, Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0));
    let point = Vec3::new(0.0, 1.0, 0.0);
    assert_eq!(zone_index(&[outer, inner], point), Some(1));

    let twin_a = room(0.5, 2, Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
    let twin_b = room(0.4, 1, Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
    assert_eq!(zone_index(&[twin_a, twin_b], point), Some(1));

    let same = room(0.4, 1, Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 2.0));
    assert_eq!(zone_index(&[same, same], point), Some(0));
    assert_eq!(zone_index(&[outer], Vec3::new(20.0, 0.0, 0.0)), None);
}
