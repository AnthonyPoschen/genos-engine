//! Argument parsing and the planted-bug checks.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]

use std::path::PathBuf;

use genos_jobs::Order;
use genos_jobs_debug::{execute, output_dir, parse_args, run_check, Options};

#[test]
fn help_parses_and_check_is_required() {
    let options = parse_args(["--help".to_string()]).expect("parse");
    assert!(options.help);
    let missing = execute(&Options {
        check: false,
        out: None,
        mutate: None,
        workers: None,
        order: None,
        budget_ms: None,
        help: false,
    });
    assert!(missing.is_err());
}

#[test]
fn the_check_passes_and_the_mutations_fail() {
    let options = Options {
        check: true,
        out: None,
        mutate: None,
        workers: Some(2),
        order: Some(Order::Parallel),
        budget_ms: Some(6.0),
        help: false,
    };
    let stats = run_check(&options).expect("check");
    assert!(stats.physics_ms > 0.0);
    assert!(stats.must_wall_ms > 0.0);
    assert_eq!(stats.rejected, 1);
    assert_eq!(stats.slack_during_fence, 0);
    for name in [
        "unordered_slots",
        "server_runs_parallel",
        "nested_wait",
        "slack_in_fence",
        "mislabeled",
    ] {
        let mut broken = options.clone();
        broken.mutate = Some(name.to_string());
        assert!(run_check(&broken).is_err(), "{name}");
    }
}

#[test]
fn the_output_directory_follows_out() {
    let options = parse_args([
        "--out".to_string(),
        "genos-debug/jobs-custom".to_string(),
        "--jobs-workers".to_string(),
        "3".to_string(),
        "--jobs-order".to_string(),
        "serial".to_string(),
    ])
    .expect("parse");
    assert_eq!(
        output_dir(&options),
        PathBuf::from("genos-debug/jobs-custom")
    );
    assert_eq!(options.workers, Some(3));
    assert_eq!(options.order, Some(Order::Serial));
}
