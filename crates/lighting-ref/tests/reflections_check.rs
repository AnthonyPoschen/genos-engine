//! The reflections-check binary: known-good passes, broken input fails, live does not fake a pass.

#![expect(clippy::panic, reason = "a failed process aborts the test")]
#![expect(clippy::unwrap_used, reason = "a failed process aborts the test")]

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reflections-check"))
}

fn run(fixture: &str, out: &std::path::Path) -> (u8, String) {
    let output = bin()
        .args(["--fixture", fixture, "--out"])
        .arg(out)
        .output()
        .unwrap_or_else(|err| panic!("{err}"));
    let code = output.status.code().unwrap_or_else(|| panic!("no status"));
    let text = String::from_utf8(output.stdout).unwrap_or_else(|err| panic!("{err}"));
    let err = String::from_utf8(output.stderr).unwrap_or_else(|err| panic!("{err}"));
    assert!(err.is_empty(), "{err}");
    (u8::try_from(code).unwrap_or(255), text)
}

#[test]
fn reflections_good_passes_bad_fails_and_live_is_unimplemented() {
    let root = std::env::temp_dir().join(format!("genos-reflections-bin-{}", std::process::id()));
    let good_dir = root.join("good");
    let bad_dir = root.join("bad");
    let live_dir = root.join("live");

    let (good_code, good_text) = run("good", &good_dir);
    assert_eq!(good_code, 0, "{good_text}");
    assert_eq!(good_text.lines().count(), 18);
    assert!(good_text.lines().all(|line| line.ends_with(" PASS")));
    let good_json = std::fs::read_to_string(good_dir.join("metrics.json")).unwrap();
    assert!(good_json.contains("\"fixture\": \"good\""));
    assert!(!good_json.contains("FAIL"));

    let (bad_code, bad_text) = run("bad", &bad_dir);
    assert_eq!(bad_code, 1, "{bad_text}");
    assert_eq!(bad_text.lines().count(), 18);
    assert!(bad_text.lines().all(|line| line.ends_with(" FAIL")));
    let bad_json = std::fs::read_to_string(bad_dir.join("metrics.json")).unwrap();
    assert!(bad_json.contains("\"fixture\": \"bad\""));
    assert!(!bad_json.contains("PASS"));

    let (live_code, live_text) = run("live", &live_dir);
    assert_eq!(live_code, 0, "{live_text}");
    assert_eq!(live_text.lines().count(), 18);
    assert!(live_text.lines().all(|line| line.ends_with(" NOT-IMPLEMENTED")));
    assert!(live_text.contains("value=na"));
    let live_json = std::fs::read_to_string(live_dir.join("metrics.json")).unwrap();
    assert!(live_json.contains("\"value\": null"));
    assert!(live_json.contains("\"reference\": "));
    assert!(!live_json.contains("\"status\": \"PASS\""));

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn reflections_out_wins_over_the_environment_mirror() {
    let root = std::env::temp_dir().join(format!(
        "genos-reflections-out-{}",
        std::process::id()
    ));
    let env_dir = root.join("env");
    let cli_dir = root.join("cli");
    let output = bin()
        .args(["--fixture", "live", "--out"])
        .arg(&cli_dir)
        .env("GENOS_REFLECTIONS_OUT", &env_dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(cli_dir.join("metrics.json").is_file());
    assert!(!env_dir.join("metrics.json").is_file());

    let mirrored = bin()
        .args(["--fixture", "live"])
        .env("GENOS_REFLECTIONS_OUT", &env_dir)
        .output()
        .unwrap();
    assert!(mirrored.status.success());
    assert!(env_dir.join("metrics.json").is_file());
    std::fs::remove_dir_all(&root).unwrap();
}
