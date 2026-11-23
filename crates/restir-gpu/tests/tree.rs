//! Lavapipe light-tree picks against `restir-ref --proposal tree`.
//! Reservoir lamps must match. `--tree-fault ignore-distance` must not.

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

fn diff(reference: &Path, candidate: &Path, out: &Path) -> (i32, String) {
    output(restir_ref(&[
        "diff",
        "--reference",
        reference.to_str().expect("utf8"),
        "--candidate",
        candidate.to_str().expect("utf8"),
        "--ref-err",
        "1000",
        "--grid-worst",
        "10",
        "--frame-ms",
        "60000",
        "--frames-to-ref",
        "8",
        "--out",
        out.to_str().expect("utf8"),
    ]))
}

#[test]
#[expect(clippy::too_many_lines, reason = "one lavapipe diff of the tree and the fault")]
#[expect(clippy::similar_names, reason = "the fault dumps are the cpu and gpu pair")]
fn tree_picks_match_the_cpu_and_ignore_distance_fails() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("restir-c4");
    let _ = std::fs::remove_dir_all(&root);
    let reference = root.join("reference");
    let gpu = root.join("gpu");
    let fault_cpu = root.join("fault-cpu");
    let fault_gpu = root.join("fault-gpu");
    let diff_ok = root.join("diff-ok");
    let diff_fault = root.join("diff-fault");
    let diff_bad = root.join("diff-bad");
    let (code, text) = output(restir_ref(&[
        "run",
        "--scene",
        "many",
        "--proposal",
        "tree",
        "--no-temporal",
        "--spatial",
        "0",
        "--frames",
        "1",
        "--seeds",
        "1",
        "--ref-err",
        "1000",
        "--grid-worst",
        "10",
        "--frame-ms",
        "60000",
        "--out",
        reference.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    let (code, text) = output(restir_gpu(&[
        "--initial",
        "--scene",
        "many",
        "--proposal",
        "tree",
        "--out",
        gpu.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    assert!(text.to_ascii_lowercase().contains("llvmpipe"), "{text}");
    assert!(text.contains("dispatch_ms"), "{text}");
    println!("lavapipe {text}");
    let (code, text) = diff(
        &reference.join("reference.json"),
        &gpu.join("candidate.json"),
        &diff_ok,
    );
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("mismatches 0"), "{text}");

    let (code, text) = output(restir_ref(&[
        "run",
        "--scene",
        "many",
        "--proposal",
        "tree",
        "--tree-fault",
        "ignore-distance",
        "--no-temporal",
        "--spatial",
        "0",
        "--frames",
        "1",
        "--seeds",
        "1",
        "--ref-err",
        "1000",
        "--grid-worst",
        "10",
        "--frame-ms",
        "60000",
        "--out",
        fault_cpu.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    let (code, text) = output(restir_gpu(&[
        "--initial",
        "--scene",
        "many",
        "--proposal",
        "tree",
        "--tree-fault",
        "ignore-distance",
        "--out",
        fault_gpu.to_str().expect("utf8"),
    ]));
    assert_eq!(code, 0, "{text}");
    let (code, text) = diff(
        &fault_cpu.join("reference.json"),
        &fault_gpu.join("candidate.json"),
        &diff_fault,
    );
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("mismatches 0"), "{text}");

    let (code, text) = diff(
        &reference.join("reference.json"),
        &fault_gpu.join("candidate.json"),
        &diff_bad,
    );
    assert_ne!(code, 0, "ignore-distance matched the tree: {text}");
    assert!(text.contains("reservoir lamps differ"), "{text}");
}
