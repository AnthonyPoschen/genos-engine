//! Lavapipe spatial reuse against `restir-ref run --spatial 4`.
//! Reservoirs are the gate. The multi-seed `grid_worst` of that run is not.
//! A reversed neighbour list must fail the diff.

#![expect(clippy::expect_used, reason = "a failed fixture aborts the test")]

use std::path::Path;
use std::process::Command;

fn workspace() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../..")
}

fn restir_ref(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(workspace())
        .args([
            "run",
            "-q",
            "-p",
            "genos-restir-ref",
            "--bin",
            "restir-ref",
            "--",
        ])
        .args(args)
        .env("VK_DRIVER_FILES", "/usr/share/vulkan/icd.d/lvp_icd.json");
    command
}

fn restir_gpu(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_restir-gpu"));
    command
        .current_dir(workspace())
        .args(args)
        .env("VK_DRIVER_FILES", "/usr/share/vulkan/icd.d/lvp_icd.json")
        .env_remove("GENOS_RESTIR_INITIAL")
        .env_remove("GENOS_RESTIR_TEMPORAL")
        .env_remove("GENOS_RESTIR_SPATIAL")
        .env_remove("GENOS_RESTIR_OUT");
    command
}

fn output(mut command: Command) -> (i32, String) {
    let ran = command.output().expect("spawn");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    (ran.status.code().unwrap_or(1), text)
}

fn diff_frames(frames: &str) -> (String, String) {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("restir-c3-{frames}"));
    let _ = std::fs::remove_dir_all(&root);
    let reference = root.join("reference");
    let candidate = root.join("gpu");
    let diff = root.join("diff");
    let (code, text) = output(restir_ref(&[
        "run",
        "--spatial",
        "4",
        "--frames",
        frames,
        "--seeds",
        "1",
        "--out",
        reference.to_str().expect("utf8"),
    ]));
    assert!(
        reference.join("reference.json").is_file(),
        "reference missing ({code}): {text}"
    );
    let (code, gpu) = output(restir_gpu(&[
        "--spatial",
        "4",
        "--frames",
        frames,
        "--out",
        candidate.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{gpu}");
    assert!(gpu.to_ascii_lowercase().contains("llvmpipe"), "{gpu}");
    let (_code, diff_text) = output(restir_ref(&[
        "diff",
        "--reference",
        reference.join("reference.json").to_str().expect("utf8"),
        "--candidate",
        candidate.join("candidate.json").to_str().expect("utf8"),
        "--out",
        diff.to_str().expect("utf8"),
    ]));
    (gpu, diff_text)
}

fn assert_reservoirs(diff: &str) {
    assert!(diff.contains("mismatches 0"), "{diff}");
    let max_w = diff
        .lines()
        .find_map(|line| line.split_whitespace().nth(1).and_then(|_| {
            line.split("max_w_rel ").nth(1).map(|rest| rest.split_whitespace().next())
        }))
        .flatten()
        .expect("max_w_rel")
        .parse::<f32>()
        .expect("max_w number");
    assert!(
        max_w <= 1.0e-3,
        "reservoir W relative error {max_w} above 1e-3\n{diff}"
    );
}

#[test]
fn spatial_reservoirs_match_after_one_frame() {
    let (gpu, diff) = diff_frames("1");
    assert_reservoirs(&diff);
    assert!(gpu.contains("dispatch_ms"), "{gpu}");
}

#[test]
fn spatial_reservoirs_match_after_eight_frames() {
    let (_gpu, diff) = diff_frames("8");
    assert_reservoirs(&diff);
}

#[test]
fn a_reversed_neighbour_list_fails_the_reservoir_diff() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("restir-c3-reverse");
    let _ = std::fs::remove_dir_all(&root);
    let reference = root.join("reference");
    let candidate = root.join("gpu");
    let diff_dir = root.join("diff");
    let (code, text) = output(restir_ref(&[
        "run",
        "--spatial",
        "4",
        "--frames",
        "1",
        "--seeds",
        "1",
        "--out",
        reference.to_str().expect("utf8"),
    ]));
    assert!(
        reference.join("reference.json").is_file(),
        "reference missing ({code}): {text}"
    );
    let (code, text) = output(restir_gpu(&[
        "--spatial",
        "4",
        "--frames",
        "1",
        "--reverse-neighbours",
        "--out",
        candidate.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    let (code, text) = output(restir_ref(&[
        "diff",
        "--reference",
        reference.join("reference.json").to_str().expect("utf8"),
        "--candidate",
        candidate.join("candidate.json").to_str().expect("utf8"),
        "--out",
        diff_dir.to_str().expect("utf8"),
    ]));
    assert_ne!(code, 0, "reversed neighbours passed: {text}");
    assert!(text.contains("fail"), "{text}");
}
