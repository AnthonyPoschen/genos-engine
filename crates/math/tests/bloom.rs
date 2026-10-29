//! Energy and the comparison harness for the proposed bloom chain (ADR 0028).

use genos_math::bloom::{
    bloom, blur, compare, downsample_weight_sum, filter_gain, fixture_input, fixture_output,
    fixture_params, from_dump, pyramid_traffic_bytes, to_dump, total, traffic_ms,
    upsample_weight_sum, Image, Params, BUDGET_1060_MS, BUDGET_4070_MS, GTX_1060_BYTES_PER_SEC,
    HIGH, LOW, MEDIUM, RTX_4070_BYTES_PER_SEC, ULTRA,
};

fn flat(width: usize, height: usize, rgb: [f64; 3]) -> Image {
    let pixels = width.saturating_mul(height);
    let mut samples = Vec::with_capacity(pixels.saturating_mul(3));
    for _ in 0..pixels {
        samples.push(rgb[0]);
        samples.push(rgb[1]);
        samples.push(rgb[2]);
    }
    Image {
        width,
        height,
        rgb: samples,
    }
}

fn rel(before: &Image, after: &Image) -> f64 {
    let src = total(before);
    (total(after) - src).abs() / src.abs().max(1.0e-6)
}

fn gradient(width: usize, height: usize) -> Image {
    let mut rgb = Vec::new();
    for y in 0..height {
        for x in 0..width {
            rgb.push(0.02 + 0.01 * f64::from(u32::try_from(x).unwrap_or(0)));
            rgb.push(if x == width / 2 && y == height / 2 {
                30.0
            } else {
                0.05
            });
            rgb.push(0.01 * f64::from(u32::try_from(y).unwrap_or(0)));
        }
    }
    Image { width, height, rgb }
}

#[test]
fn weights_sum_to_one() {
    assert!((downsample_weight_sum() - 1.0).abs() < 1.0e-15);
    assert!((upsample_weight_sum() - 1.0).abs() < 1.0e-15);
}

#[test]
fn a_flat_field_is_unchanged_at_every_preset() {
    let image = flat(32, 16, [0.25, 0.5, 1.25]);
    for params in [LOW, MEDIUM, HIGH, ULTRA] {
        let out = bloom(&image, params);
        assert!(
            (filter_gain(&image, params) - 1.0).abs() < 1.0e-9,
            "gain {}",
            filter_gain(&image, params)
        );
        for (got, want) in out.rgb.iter().zip(image.rgb.iter()) {
            assert!((got - want).abs() < 1.0e-9);
        }
        assert!(rel(&image, &out) < 1.0e-12);
    }
}

#[test]
fn scatter_zero_and_zero_levels_copy_the_image() {
    let image = gradient(16, 16);
    let held = bloom(&image, Params::new(6, 0.0));
    assert_eq!(held.rgb, image.rgb);
    let off = bloom(&image, Params::new(0, 0.5));
    assert_eq!(off.rgb, image.rgb);
}

#[test]
fn a_varying_field_keeps_its_sum() {
    let image = gradient(32, 32);
    for params in [LOW, MEDIUM, HIGH, ULTRA] {
        let gain = filter_gain(&image, params);
        assert!(
            (gain - 1.0).abs() < 0.05,
            "raw filter drifted: gain {gain} levels {}",
            params.levels
        );
        let out = bloom(&image, params);
        assert!(rel(&image, &out) < 1.0e-9, "rel {}", rel(&image, &out));
    }
}

#[test]
fn a_spike_moves_energy_off_the_pixel() {
    let mut image = flat(16, 16, [0.0, 0.0, 0.0]);
    let center = (8 * 16 + 8) * 3;
    if let Some(slot) = image.rgb.get_mut(center) {
        *slot = 10.0;
    }
    if let Some(slot) = image.rgb.get_mut(center + 1) {
        *slot = 10.0;
    }
    if let Some(slot) = image.rgb.get_mut(center + 2) {
        *slot = 10.0;
    }
    let out = bloom(&image, Params::new(4, 1.0));
    let before = image.rgb.get(center).copied().unwrap_or(0.0);
    let after = out.rgb.get(center).copied().unwrap_or(0.0);
    assert!(after < before);
    let neighbor = out.rgb.get(center + 3).copied().unwrap_or(0.0);
    assert!(neighbor > 0.0);
    assert!(rel(&image, &out) < 1.0e-9);
}

#[test]
fn the_linear_chain_commutes_with_scale() {
    let image = gradient(16, 16);
    let params = Params::new(3, 0.4);
    let plain = bloom(&image, params);
    let mut scaled = image.clone();
    for sample in &mut scaled.rgb {
        *sample *= 2.0;
    }
    let got = bloom(&scaled, params);
    for (left, right) in plain.rgb.iter().zip(got.rgb.iter()) {
        assert!((right - left * 2.0).abs() < 1.0e-8);
    }
}

#[test]
fn karis_removes_firefly_energy() {
    let mut image = flat(16, 16, [0.0, 0.0, 0.0]);
    let center = (4 * 16 + 4) * 3;
    for channel in 0..3 {
        if let Some(slot) = image.rgb.get_mut(center + channel) {
            *slot = 1_000.0;
        }
    }
    let params = Params::new(2, 1.0);
    let linear = total(&blur(&image, params));
    let softened = total(&blur(&image, params.with_karis()));
    assert!(softened < linear * 0.5);
    let scene = total(&image);
    assert!((linear - scene).abs() / scene < 0.05);
}

#[test]
fn compare_matches_an_f32_readback_and_rejects_a_scaled_buffer() {
    // Values that are exact in both f32 and f64, so the promotion adds nothing.
    let image = Image {
        width: 2,
        height: 1,
        rgb: vec![0.5, 0.25, 0.125, 2.0, 4.0, 0.0],
    };
    let rgb = [0.5_f32, 0.25, 0.125, 2.0, 4.0, 0.0];
    let same = compare(&image, &rgb);
    assert!(!same.size_mismatch);
    assert_eq!(same.max_abs.to_bits(), 0);
    assert_eq!(same.mean_abs.to_bits(), 0);
    assert_eq!(same.energy_rel.to_bits(), 0);
    let rgba = [0.5_f32, 0.25, 0.125, 1.0, 2.0, 4.0, 0.0, 1.0];
    let rgba_same = compare(&image, &rgba);
    assert!(!rgba_same.size_mismatch);
    assert_eq!(rgba_same.max_abs.to_bits(), 0);
    let doubled = [1.0_f32, 0.5, 0.25, 4.0, 8.0, 0.0];
    let bad = compare(&image, &doubled);
    assert!(bad.energy_rel > 0.5);
    assert!(bad.max_abs > 0.1);
    let wrong = compare(&image, &[0.0_f32]);
    assert!(wrong.size_mismatch);
    let fixture = compare(&fixture_output(), &[]);
    assert!(fixture.size_mismatch);
}

#[test]
fn dump_roundtrips_the_fixture() {
    let image = fixture_output();
    let parsed = from_dump(&to_dump(&image));
    assert!(parsed.is_some());
    if let Some(parsed) = parsed {
        assert_eq!(parsed.width, image.width);
        assert_eq!(parsed.height, image.height);
        assert_eq!(parsed.rgb.len(), image.rgb.len());
        for (got, want) in parsed.rgb.iter().zip(image.rgb.iter()) {
            assert!((got - want).abs() <= 0.0);
        }
    }
    assert!(from_dump("nope").is_none());
}

#[test]
fn fixture_is_stable() {
    let once = fixture_output();
    let twice = bloom(&fixture_input(), fixture_params());
    assert_eq!(once.rgb, twice.rgb);
}

#[test]
fn preset_budgets_hold_at_1440p() {
    assert_eq!(pyramid_traffic_bytes(2_560, 1_440, LOW.levels), 137_088_000);
    assert_eq!(
        pyramid_traffic_bytes(2_560, 1_440, MEDIUM.levels),
        137_491_200
    );
    assert_eq!(
        pyramid_traffic_bytes(2_560, 1_440, HIGH.levels),
        137_591_680
    );
    assert_eq!(
        pyramid_traffic_bytes(2_560, 1_440, ULTRA.levels),
        137_616_320
    );
    let low_ms = traffic_ms(
        pyramid_traffic_bytes(2_560, 1_440, LOW.levels),
        GTX_1060_BYTES_PER_SEC,
    );
    let high_ms = traffic_ms(
        pyramid_traffic_bytes(2_560, 1_440, HIGH.levels),
        RTX_4070_BYTES_PER_SEC,
    );
    assert!(low_ms < BUDGET_1060_MS, "1060 {low_ms}");
    assert!(high_ms < BUDGET_4070_MS, "4070 {high_ms}");
}
