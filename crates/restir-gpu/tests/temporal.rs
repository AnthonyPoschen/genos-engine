//! Lavapipe temporal reuse against `restir-ref run --spatial 0`.
//! A history `M` cap must fail the diff. A reset drops `M` to the candidate count.

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

fn diff_frames(frames: &str) -> (i32, String, String) {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("restir-c2-{frames}"));
    let _ = std::fs::remove_dir_all(&root);
    let reference = root.join("reference");
    let candidate = root.join("gpu");
    let diff = root.join("diff");
    let (code, text) = output(restir_ref(&[
        "run",
        "--spatial",
        "0",
        "--frames",
        frames,
        "--out",
        reference.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    let (code, text) = output(restir_gpu(&[
        "--temporal",
        "--frames",
        frames,
        "--out",
        candidate.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    assert!(text.to_ascii_lowercase().contains("llvmpipe"), "{text}");
    let (code, diff_text) = output(restir_ref(&[
        "diff",
        "--reference",
        reference.join("reference.json").to_str().expect("utf8"),
        "--candidate",
        candidate.join("candidate.json").to_str().expect("utf8"),
        "--out",
        diff.to_str().expect("utf8"),
    ]));
    (code, text, diff_text)
}

#[test]
fn temporal_reservoirs_match_the_cpu_reference_after_eight_frames() {
    let (code, gpu, diff) = diff_frames("8");
    assert_eq!(code, 0, "gpu {gpu}\ndiff {diff}");
    assert!(diff.contains("mismatches 0"), "{diff}");
}

#[test]
fn a_history_m_cap_fails_the_reservoir_diff() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("restir-c2-cap");
    let _ = std::fs::remove_dir_all(&root);
    let reference = root.join("reference");
    let candidate = root.join("gpu");
    let diff_dir = root.join("diff");
    let (code, text) = output(restir_ref(&[
        "run",
        "--spatial",
        "0",
        "--frames",
        "8",
        "--out",
        reference.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    let (code, text) = output(restir_gpu(&[
        "--temporal",
        "--frames",
        "8",
        "--m-cap",
        "1",
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
    assert_ne!(code, 0, "clamped M passed: {text}");
    assert!(text.contains("fail"), "{text}");
}

#[test]
fn reset_after_drops_m_to_the_candidate_count() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("restir-c2-reset");
    let _ = std::fs::remove_dir_all(&root);
    let cleared = root.join("cleared");
    let kept = root.join("kept");
    let (code, text) = output(restir_gpu(&[
        "--temporal",
        "--scene",
        "open-row",
        "--proposal",
        "uniform",
        "--candidates",
        "2",
        "--seed",
        "3",
        "--frames",
        "11",
        "--reset-after",
        "10",
        "--out",
        cleared.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    assert_eq!(history_m(&text), 2, "{text}");
    let (code, kept_text) = output(restir_gpu(&[
        "--temporal",
        "--scene",
        "open-row",
        "--proposal",
        "uniform",
        "--candidates",
        "2",
        "--seed",
        "3",
        "--frames",
        "11",
        "--out",
        kept.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{kept_text}");
    let grown = history_m(&kept_text);
    assert!(
        grown > 2,
        "skipping the reset left M at {grown}: {kept_text}"
    );
}

fn history_m(text: &str) -> u32 {
    text.lines()
        .find_map(|line| line.strip_prefix("history_m "))
        .expect("history_m line")
        .trim()
        .parse()
        .expect("history m")
}
