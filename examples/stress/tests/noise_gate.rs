#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        reason = "the renderer reads GENOS_LIGHTING_LAMP_PICK_DIAG; this test is the argument that turns the counters on"
    )
)]
#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]

//! s7 on lavapipe: 96×54, 4200 settle frames, 64 still and 64 pan.
//!
//! Needs a Wayland socket (`WAYLAND_DISPLAY`) and lavapipe. Run with
//! `--test-threads=1`. Grade A exits through the assertion below.

use genos_stress::noise_gate::{self, Config};

#[test]
fn s7_noise_gate_grades_a_on_lavapipe() {
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let out = std::env::temp_dir().join("genos-noise-gate");
    let config = Config::lavapipe(out);
    let code = noise_gate::run(&config).expect("noise gate");
    let metrics = std::fs::read_to_string(config.out.join("metrics.json")).expect("metrics.json");
    assert!(
        metrics.contains("\"grade\": \"A\""),
        "s7 exited {code}\n{metrics}"
    );
    assert_eq!(code, 0, "{metrics}");
    assert!(
        metrics.contains("\"still_restir\":") && !metrics.contains("\"still_restir\": 0"),
        "the random path did not shade a reservoir sample\n{metrics}"
    );
}

#[test]
fn s7_noise_gate_grades_a_with_direct_specular_on_lavapipe() {
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let out = std::env::temp_dir().join("genos-noise-gate-specular");
    let config = Config {
        specular: true,
        ..Config::lavapipe(out)
    };
    let code = noise_gate::run(&config).expect("noise gate");
    let metrics = std::fs::read_to_string(config.out.join("metrics.json")).expect("metrics.json");
    assert!(
        metrics.contains("\"grade\": \"A\""),
        "s7 with the GGX lobe exited {code}\n{metrics}"
    );
    assert_eq!(code, 0, "{metrics}");
    assert!(
        metrics.contains("\"still_restir\":") && !metrics.contains("\"still_restir\": 0"),
        "the random path did not shade a reservoir sample\n{metrics}"
    );
}

#[test]
fn s7_broken_picker_fails_on_lavapipe() {
    std::env::set_var("GENOS_LIGHTING_LAMP_PICK_DIAG", "1");
    let out = std::env::temp_dir().join("genos-noise-gate-broken");
    let config = Config {
        settle: 40,
        frames: 16,
        broken: true,
        ..Config::lavapipe(out)
    };
    let code = noise_gate::run(&config).expect("noise gate");
    let metrics = std::fs::read_to_string(config.out.join("metrics.json")).expect("metrics.json");
    assert_ne!(code, 0, "a uniform pick with no MIS must fail\n{metrics}");
    assert!(
        !metrics.contains("\"grade\": \"A\"") && !metrics.contains("\"grade\": \"B\""),
        "broken picker graded a pass\n{metrics}"
    );
}
