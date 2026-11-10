#![cfg_attr(
    test,
    expect(
        clippy::unwrap_used,
        reason = "a test aborts when a cook or a WAV fixture fails"
    )
)]

//! Two cooks of one clip share a subset hash. A missing cook falls back to WAV.

use genos_load::{
    audio_cook_fallbacks, cook_ima, hex, load_audio, parse_cooked, resident_clip, sha256, Pcm,
};

fn ramp() -> Pcm {
    Pcm {
        sample_rate: 48_000,
        channels: 1,
        samples: (0..32)
            .map(|index| i16::try_from(index * 500 - 4_000).unwrap_or(0))
            .collect(),
    }
}

#[test]
fn two_cooks_share_the_subset_hash() {
    let pcm = ramp();
    let first = cook_ima(&pcm, 0, 32).unwrap();
    let second = cook_ima(&pcm, 0, 32).unwrap();
    assert_eq!(first, second);
    assert_eq!(hex(&sha256(&first)), include_str!("adpcm.sha256").trim(),);
    let decoded = parse_cooked(&first).unwrap().decode_all();
    let mut peak = 0u32;
    for (got, want) in decoded.samples.iter().zip(pcm.samples.iter()) {
        peak = peak.max(i32::from(*got).abs_diff(i32::from(*want)));
    }
    let measured = include_str!("adpcm-error.txt").trim();
    assert_eq!(peak.to_string(), measured);
}

#[test]
fn a_short_clip_stays_resident_and_a_long_one_does_not() {
    assert!(resident_clip(48_000, 1, 48_000));
    assert!(resident_clip(48_000, 1, 96_000));
    assert!(resident_clip(48_000, 1, 131_072));
    assert!(!resident_clip(48_000, 1, 131_073));
    assert!(!resident_clip(48_000, 1, 140_000));
}

#[test]
fn a_missing_cook_decodes_wav_and_logs_one_line() {
    let before = audio_cook_fallbacks();
    let wav = tiny_wav();
    let loaded = load_audio(&wav).unwrap();
    assert_eq!(loaded.samples, vec![1_000, -1_000]);
    assert_eq!(audio_cook_fallbacks(), before + 1);
    let cooked = cook_ima(&ramp(), 0, 32).unwrap();
    let again = audio_cook_fallbacks();
    let _ = load_audio(&cooked).unwrap();
    assert_eq!(audio_cook_fallbacks(), again);
}

fn tiny_wav() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(b"fmt ");
    body.extend(16u32.to_le_bytes());
    body.extend(1u16.to_le_bytes());
    body.extend(1u16.to_le_bytes());
    body.extend(48_000u32.to_le_bytes());
    body.extend(96_000u32.to_le_bytes());
    body.extend(2u16.to_le_bytes());
    body.extend(16u16.to_le_bytes());
    body.extend(b"data");
    body.extend(4u32.to_le_bytes());
    body.extend(1_000i16.to_le_bytes());
    body.extend((-1_000i16).to_le_bytes());
    let mut file = Vec::new();
    file.extend(b"RIFF");
    file.extend(u32::try_from(4 + body.len()).unwrap_or(0).to_le_bytes());
    file.extend(b"WAVE");
    file.extend(body);
    file
}
