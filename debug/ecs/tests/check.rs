//! Argument parsing and the planted-bug checks.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

use std::path::PathBuf;

use genos_ecs_debug::{
    execute, metrics_json, output_dir, parse_args, run_check, write_metrics, Options,
};

#[test]
fn help_parses_and_check_is_required() {
    let options = parse_args(["--help".to_string()]).expect("parse");
    assert!(options.help);
    let missing = execute(&Options {
        check: false,
        out: None,
        mutate: None,
        help: false,
    });
    assert!(missing.is_err());
}

#[test]
fn the_check_passes_and_the_mutations_fail() {
    let stats = run_check(None).expect("check");
    assert_eq!(stats.stale_rejects, 1);
    assert!(run_check(Some("stale_accept")).is_err());
    assert!(run_check(Some("skip_tick")).is_err());
}

#[test]
fn metrics_land_under_the_chosen_directory() {
    let dir = PathBuf::from("genos-debug/ecs-test");
    let stats = run_check(None).expect("check");
    let path = write_metrics(&dir, &metrics_json(&stats, "ok")).expect("write");
    assert!(path.ends_with("metrics.json"));
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.contains("\"stale_rejects\": 1"));
    let options =
        parse_args(["--out".to_string(), "genos-debug/ecs-custom".to_string()]).expect("parse");
    assert_eq!(
        output_dir(&options),
        PathBuf::from("genos-debug/ecs-custom")
    );
}
