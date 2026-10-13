//! `--frames 5 --wake-log --shot` has to return. A proof window does not close,
//! and the wake log takes the terminal, so a loop that waits for settle or for
//! a fence forever looks like a hang from the first second.

use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn five_frames_with_wake_log_and_shot_exit() {
    let bin = env!("CARGO_BIN_EXE_genos-stress");
    let root = std::env::temp_dir().join(format!("genos-wake-shot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let log = root.join("log");
    let shot = root.join("shot.png");
    let mut child = Command::new(bin);
    child
        .args([
            "--proof",
            "--no-panel",
            "--size",
            "320x180",
            "--frames",
            "5",
            "--dynamic",
            "0",
            "--still-boxes",
            "--freeze-sun",
            "--time",
            "0.25",
            "--wake",
            "legacy",
            "--wake-log",
        ])
        .arg(&log)
        .arg("--shot")
        .arg(&shot)
        .env("VK_DRIVER_FILES", "/usr/share/vulkan/icd.d/lvp_icd.json")
        .env("VK_ICD_FILENAMES", "/usr/share/vulkan/icd.d/lvp_icd.json")
        .env("XDG_RUNTIME_DIR", "/tmp/weston-runtime")
        .env("WAYLAND_DISPLAY", "genos-proof")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = child.spawn().expect("spawn genos-stress");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if started.elapsed() > Duration::from_secs(90) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("5-frame --wake-log --shot did not exit in 90s");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(status.success(), "exit {status}");
    let text = std::fs::read_to_string(log.join("run.txt")).expect("wake log");
    assert!(text.contains("frames=5"), "{text}");
    let png = std::fs::metadata(&shot).expect("shot png");
    assert!(png.len() > 32, "png {}", png.len());
    let _ = std::fs::remove_dir_all(&root);
}
