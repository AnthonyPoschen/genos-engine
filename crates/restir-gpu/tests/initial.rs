//! Lavapipe initial candidates against `restir-ref run --no-temporal --spatial 0 --frames 1`.
//! A perturbed seed must fail `restir-ref diff`.

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
        .args(["run", "-q", "-p", "genos-restir-ref", "--bin", "restir-ref", "--"])
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

#[test]
fn a_bare_run_writes_nothing() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("restir-off");
    let _ = std::fs::remove_dir_all(&dir);
    let (code, text) = output(restir_gpu(&["--out", dir.to_str().expect("utf8")]));
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("off"), "{text}");
    assert!(!dir.join("candidate.json").exists());
}

#[test]
fn initial_candidates_match_the_cpu_reference_and_a_perturbed_seed_fails() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("restir-c1");
    let _ = std::fs::remove_dir_all(&root);
    let reference = root.join("reference");
    let candidate = root.join("gpu");
    let perturbed = root.join("perturbed");
    let diff_ok = root.join("diff-ok");
    let diff_bad = root.join("diff-bad");
    let (code, text) = output(restir_ref(&[
        "run",
        "--no-temporal",
        "--spatial",
        "0",
        "--frames",
        "1",
        "--out",
        reference.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    let (code, text) = output(restir_gpu(&[
        "--initial",
        "--out",
        candidate.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    assert!(text.to_ascii_lowercase().contains("llvmpipe"), "{text}");
    let (code, text) = output(restir_ref(&[
        "diff",
        "--reference",
        reference.join("reference.json").to_str().expect("utf8"),
        "--candidate",
        candidate.join("candidate.json").to_str().expect("utf8"),
        "--out",
        diff_ok.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("mismatches 0"), "{text}");
    let (code, text) = output(restir_gpu(&[
        "--initial",
        "--seed",
        "2",
        "--out",
        perturbed.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    let (code, text) = output(restir_ref(&[
        "diff",
        "--reference",
        reference.join("reference.json").to_str().expect("utf8"),
        "--candidate",
        perturbed.join("candidate.json").to_str().expect("utf8"),
        "--out",
        diff_bad.to_str().expect("utf8"),
    ]));
    assert_ne!(code, 0, "perturbed seed passed: {text}");
    assert!(text.contains("fail"), "{text}");
}
