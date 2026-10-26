//! The good fixture passes, the broken fixture fails, and a missing feature
//! does not become a pass.

#![expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")]

use genos_fog_check::{
    evaluate, exit_code, format_lines, judge, resolve, write_metrics, Fixture, Status, Suite,
};

#[test]
fn analytic_good_passes() {
    let report = evaluate(Suite::Analytic, Fixture::Good);
    assert_eq!(exit_code(&report), 0, "{}", format_lines(&report));
    assert_eq!(report.metrics.len(), 7);
    for metric in &report.metrics {
        assert_eq!(metric.status, Status::Pass, "{metric:?}");
        assert!(metric.value.is_some(), "{}", metric.name);
    }
}

#[test]
fn analytic_broken_fails_each_metric() {
    let report = evaluate(Suite::Analytic, Fixture::Broken);
    assert_eq!(exit_code(&report), 1, "{}", format_lines(&report));
    assert_eq!(report.metrics.len(), 7);
    for metric in &report.metrics {
        assert_eq!(metric.status, Status::Fail, "{metric:?}");
        let value = metric.value.unwrap();
        assert!(
            value > metric.threshold,
            "{} value {value} threshold {}",
            metric.name,
            metric.threshold
        );
    }
    let beer = metric(&report, "beer_lambert_max_abs");
    assert!(beer.value.unwrap() > 0.3, "{beer:?}");
}

#[test]
fn feature_metrics_keep_absent_rows_and_grade_the_live_ones() {
    let report = evaluate(Suite::Feature, Fixture::Good);
    let lines = format_lines(&report);
    assert_eq!(exit_code(&report), 0, "{lines}");
    assert!(report.metrics.len() > 5);
    let leak = metric(&report, "local_volume_leak");
    assert_eq!(leak.status, Status::Pass, "{leak:?}");
    assert!(leak.value.unwrap() <= leak.threshold);
    let gi = metric(&report, "gi_inscatter_blur_rel");
    assert_eq!(gi.value, None);
    assert_eq!(gi.status, Status::NotImplemented);
    for metric in &report.metrics {
        if metric.status == Status::NotImplemented {
            assert_eq!(metric.value, None, "{}", metric.name);
            assert!(!metric.note.is_empty(), "{}", metric.name);
        }
    }
}

#[test]
fn an_unshadowed_shaft_fails_the_shadow_grade() {
    let reference = genos_render::shaft_image(true);
    let candidate = genos_render::shaft_image(false);
    let reference = genos_lighting_ref::Picture::new(
        reference.width,
        reference.height,
        reference.linear,
    )
    .unwrap();
    let candidate = genos_lighting_ref::Picture::new(
        candidate.width,
        candidate.height,
        candidate.linear,
    )
    .unwrap();
    let grade = genos_lighting_ref::blur_relative(
        &reference,
        &candidate,
        &genos_lighting_ref::MetricParams::default(),
    )
    .unwrap();
    assert!(grade > 0.12, "{grade}");
}

#[test]
fn held_history_fails_smear_and_a_scaled_yaw_fails_pop() {
    assert!(genos_render::smear_broken() > 0.50);
    assert!(f64::from(genos_render::yaw_pop_broken()) > 0.05);
}

#[test]
fn a_scaled_still_frame_fails_the_flicker_gate() {
    let broken = f64::from(genos_render::still_flicker_broken());
    assert!(broken > 0.02, "{broken}");
    assert_eq!(judge(true, Some(broken), 0.02), Status::Fail);
}

#[test]
fn a_leaking_volume_fails_the_leak_gate() {
    let broken = f64::from(genos_render::leak_broken());
    assert!(broken > 1.0e-4, "{broken}");
    assert_eq!(judge(true, Some(broken), 1.0e-4), Status::Fail);
}

#[test]
fn a_zero_does_not_pass_a_missing_feature() {
    assert_eq!(judge(false, Some(0.0), 0.5), Status::NotImplemented);
    assert_eq!(judge(true, Some(0.0), 0.5), Status::Pass);
    assert_eq!(judge(true, Some(0.5), 0.5), Status::Pass);
    assert_eq!(judge(true, Some(0.51), 0.5), Status::Fail);
    assert_eq!(judge(true, Some(-0.1), 0.5), Status::Fail);
    assert_eq!(judge(true, None, 0.5), Status::NotImplemented);
}

#[test]
fn all_good_exits_0_with_not_implemented_rows() {
    let report = evaluate(Suite::All, Fixture::Good);
    assert_eq!(exit_code(&report), 0);
    assert!(report
        .metrics
        .iter()
        .any(|metric| metric.status == Status::Pass));
    assert!(report
        .metrics
        .iter()
        .all(|metric| metric.status != Status::Fail));
}

#[test]
fn all_broken_exits_on_failure() {
    let report = evaluate(Suite::All, Fixture::Broken);
    assert_eq!(exit_code(&report), 1);
}

#[test]
fn argument_wins_over_environment() {
    let args = vec!["--suite".to_string(), "analytic".to_string()];
    let config = resolve(
        &args,
        Some("feature"),
        Some("broken"),
        Some("genos-debug/other"),
    )
    .unwrap();
    assert_eq!(config.suite, Suite::Analytic);
    assert_eq!(config.fixture, Fixture::Broken);
    assert_eq!(config.out.as_os_str(), "genos-debug/other");

    let from_env = resolve(&[], Some("feature"), None, None).unwrap();
    assert_eq!(from_env.suite, Suite::Feature);
    assert_eq!(from_env.fixture, Fixture::Good);
    assert_eq!(from_env.out.as_os_str(), "genos-debug/fog");
}

#[test]
fn summary_line_has_four_fields() {
    let report = evaluate(Suite::Analytic, Fixture::Good);
    for line in format_lines(&report).lines() {
        assert_eq!(line.split_whitespace().count(), 4, "{line}");
    }
    let feature = evaluate(Suite::Feature, Fixture::Good);
    let line = format_lines(&feature).lines().next().unwrap().to_string();
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(fields.len(), 4, "{line}");
    assert_eq!(fields[0], "froxel_temporal_smear_px");
    assert_eq!(fields[3], "PASS", "{line}");
}

#[test]
fn json_records_pass_and_fail() {
    let dir = std::env::temp_dir().join(format!("genos-fog-check-{}", std::process::id()));
    let good_dir = dir.join("good");
    let bad_dir = dir.join("broken");
    let good = evaluate(Suite::Analytic, Fixture::Good);
    let bad = evaluate(Suite::Analytic, Fixture::Broken);
    let good_path = write_metrics(&good_dir, &good).unwrap();
    let bad_path = write_metrics(&bad_dir, &bad).unwrap();
    let good_text = std::fs::read_to_string(&good_path).unwrap();
    let bad_text = std::fs::read_to_string(&bad_path).unwrap();
    assert!(good_text.contains("\"status\": \"PASS\""), "{good_text}");
    assert!(!good_text.contains("\"status\": \"FAIL\""), "{good_text}");
    assert!(bad_text.contains("\"status\": \"FAIL\""), "{bad_text}");
    assert!(!bad_text.contains("\"status\": \"PASS\""), "{bad_text}");
    let feature = evaluate(Suite::Feature, Fixture::Good);
    let feature_path = write_metrics(dir.join("feature").as_path(), &feature).unwrap();
    let feature_text = std::fs::read_to_string(&feature_path).unwrap();
    assert!(feature_text.contains("\"value\": null"), "{feature_text}");
    assert!(
        feature_text.contains("\"status\": \"NOT-IMPLEMENTED\""),
        "{feature_text}"
    );
}

#[test]
fn a_bad_argument_exits_2() {
    let exe = env!("CARGO_BIN_EXE_fog-check");
    let bad_suite = std::process::Command::new(exe)
        .arg("--suite")
        .arg("nope")
        .output()
        .unwrap();
    assert_eq!(bad_suite.status.code(), Some(2), "{bad_suite:?}");
    let missing = std::process::Command::new(exe)
        .arg("--out")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2), "{missing:?}");
    let help = std::process::Command::new(exe)
        .arg("--help")
        .output()
        .unwrap();
    assert_eq!(help.status.code(), Some(0));
}

fn metric<'a>(report: &'a genos_fog_check::Report, name: &str) -> &'a genos_fog_check::Metric {
    report
        .metrics
        .iter()
        .find(|metric| metric.name == name)
        .unwrap()
}
