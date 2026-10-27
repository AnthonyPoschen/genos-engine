#![expect(
    clippy::expect_used,
    reason = "the quit-gap harness aborts when weston or a binary fixture fails"
)]

//! Quit gap for every windowed binary, under Weston headless and lavapipe.
//!
//! Build the binaries first (`cargo test --workspace` does). This file does
//! not invoke Cargo. A nested build would wait on the same target lock.

use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use genos_window::{
    gap_within_budget, parse_quit_gap, parse_quit_phases, parse_quit_total, phases_within_budget,
    PhaseSample, QUIT_GAP_PREFIX, QUIT_TOTAL_PREFIX,
};

const ICD: &str = "/usr/share/vulkan/icd.d/lvp_icd.json";
const LIMIT: Duration = Duration::from_secs(600);

struct Spec {
    label: &'static str,
    name: &'static str,
    port: u16,
    args: &'static [&'static str],
    /// Rhai source written beside the run. The script thread is still in it at quit.
    script: Option<&'static str>,
    /// Process statuses that still mean the binary finished. A signal is never one of these.
    codes: &'static [i32],
}

struct Finished {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

struct Seat {
    runtime: PathBuf,
    socket: String,
    child: Child,
}

/// Phases a full device teardown must print, including the post-unmap tail.
const FULL_PHASES: &[&str] = &[
    "prepare",
    "stop_producers",
    "drain_gpu",
    "destroy_yield",
    "destroy_lighting",
    "destroy_swapchain",
    "destroy_objects",
    "destroy_device",
    "destroy_surface",
    "destroy_instance",
    "unmap",
    "device_drop",
    "exit_tail",
];

#[test]
fn every_binary_answers_pings_while_it_quits() {
    let seat = Seat::open("genos-quit").expect("weston");
    let mut measured = Vec::new();
    for spec in paths() {
        let run = launch(&seat, &spec, None).expect(spec.label);
        let gap = must_gap(spec.label, &run);
        let total = must_total(spec.label, &run);
        let phases = phases_of(&run);
        print_phases(spec.label, gap, total, &phases);
        measured.push((spec.label, gap, total));
        assert_finished(&spec, &run);
        assert_clean(spec.label, gap, total, &phases, &run);
    }
    for spec in plants() {
        let run = launch(&seat, &spec, Some("2000")).expect(spec.label);
        let gap = must_gap(spec.label, &run);
        let total = must_total(spec.label, &run);
        let phases = phases_of(&run);
        print_phases(spec.label, gap, total, &phases);
        assert_finished(&spec, &run);
        assert!(
            !gap_within_budget(gap),
            "{} planted stall must fail the budget, gap {gap:.1} ms; clean runs {measured:?}",
            spec.label
        );
        assert!(
            gap >= 1000.0,
            "{} planted gap was only {gap:.1} ms",
            spec.label
        );
        let drain = phase_ms(&phases, "drain_gpu");
        assert!(
            drain >= 1000.0,
            "{} planted drain_gpu was only {drain:.1} ms; phases {phases:?}",
            spec.label
        );
        assert!(
            !phases_within_budget(&phases, total),
            "{} planted phases stayed inside the budget",
            spec.label
        );
        for phase in &phases {
            if phase.name == "drain_gpu" {
                continue;
            }
            assert!(
                gap_within_budget(phase.ms),
                "{} phase {} took {:.1} ms during the plant; only drain_gpu may block",
                spec.label,
                phase.name,
                phase.ms
            );
        }
    }
    for spec in early_closes() {
        let run = launch(&seat, &spec, None).expect(spec.label);
        let gap = must_gap(spec.label, &run);
        let total = must_total(spec.label, &run);
        let phases = phases_of(&run);
        print_phases(spec.label, gap, total, &phases);
        assert_finished(&spec, &run);
        assert_clean(spec.label, gap, total, &phases, &run);
        assert!(
            !run.stdout.contains("frames=") && !run.stderr.contains("frames="),
            "{} printed a frame line, so close-early ran the frame loop\nstdout {}\nstderr {}",
            spec.label,
            run.stdout,
            run.stderr
        );
        assert!(
            !run.stderr.contains("genos-lighting: loading"),
            "{} loaded a scene before quit\nstderr {}",
            spec.label,
            run.stderr
        );
    }
}

fn assert_finished(spec: &Spec, run: &Finished) {
    assert!(
        run.code.is_some_and(|code| spec.codes.contains(&code)),
        "{} exit {:?} stderr {}",
        spec.label,
        run.code,
        run.stderr
    );
    assert!(
        !run.stderr.contains("corrupted size"),
        "{} stderr {}",
        spec.label,
        run.stderr
    );
}

fn paths() -> Vec<Spec> {
    let mut specs = first_paths();
    specs.extend(later_paths());
    specs
}

fn first_paths() -> Vec<Spec> {
    vec![
        spec(
            "camera_proof",
            "genos-camera",
            18765,
            &["--proof", "--frames", "3", "--size", "320x180"],
            None,
            &[0],
        ),
        spec(
            "camera_named",
            "genos-camera",
            18768,
            &["--frames", "3", "--size", "320x180"],
            None,
            &[0],
        ),
        spec(
            "stress_proof",
            "genos-stress",
            18766,
            &[
                "--proof",
                "--frames",
                "3",
                "--size",
                "320x180",
                "--preset",
                "potato",
                "--scale",
                "small",
                "--no-panel",
            ],
            None,
            &[0],
        ),
        spec(
            "stress_panel",
            "genos-stress",
            18769,
            &[
                "--frames",
                "3",
                "--size",
                "320x180",
                "--preset",
                "potato",
                "--scale",
                "small",
            ],
            None,
            &[0],
        ),
        spec(
            "lighting_headless",
            "genos-lighting",
            18767,
            &[
                "--headless",
                "--frames",
                "3",
                "--size",
                "320x180",
                "--preset",
                "potato",
                "--scene",
                "stress",
            ],
            None,
            &[0],
        ),
        spec(
            "lighting_panel",
            "genos-lighting",
            18770,
            &[
                "--frames",
                "3",
                "--size",
                "320x180",
                "--preset",
                "potato",
                "--scene",
                "stress",
            ],
            None,
            &[0],
        ),
    ]
}

fn later_paths() -> Vec<Spec> {
    vec![
        spec(
            "lighting_check",
            "genos-lighting",
            18771,
            &["--check", "--size", "320x180", "--preset", "potato"],
            None,
            &[0, 1],
        ),
        spec(
            "lighting_ref",
            "genos-stress",
            18772,
            &[
                "lighting-ref",
                "check",
                "--mode",
                "lavapipe-small",
                "--scene",
                "stress-still",
                "--checkpoint",
                "hall",
                "--warmup-frames",
                "0",
                "--settle-frames",
                "1",
                "--capture-frames",
                "1",
            ],
            None,
            &[0, 1, 2],
        ),
        spec(
            "lighting_script",
            "genos-lighting",
            18773,
            &[
                "--headless",
                "--frames",
                "3",
                "--size",
                "320x180",
                "--preset",
                "potato",
                "--scene",
                "stress",
            ],
            Some("loop { }\n"),
            &[0],
        ),
    ]
}

fn early_closes() -> Vec<Spec> {
    vec![
        spec(
            "camera_close_early",
            "genos-camera",
            18780,
            &[
                "--proof",
                "--frames",
                "3",
                "--size",
                "320x180",
                "--lifecycle-close-early",
            ],
            None,
            &[0],
        ),
        spec(
            "stress_close_early",
            "genos-stress",
            18781,
            &[
                "--proof",
                "--frames",
                "3",
                "--size",
                "320x180",
                "--preset",
                "potato",
                "--scale",
                "small",
                "--no-panel",
                "--lifecycle-close-early",
            ],
            None,
            &[0],
        ),
        spec(
            "lighting_close_early",
            "genos-lighting",
            18782,
            &[
                "--headless",
                "--frames",
                "3",
                "--size",
                "320x180",
                "--preset",
                "potato",
                "--scene",
                "stress",
                "--lifecycle-close-early",
            ],
            None,
            &[0],
        ),
    ]
}

fn plants() -> Vec<Spec> {
    vec![
        spec(
            "camera_plant",
            "genos-camera",
            18774,
            &["--proof", "--frames", "3", "--size", "320x180"],
            None,
            &[0],
        ),
        spec(
            "stress_plant",
            "genos-stress",
            18775,
            &[
                "--proof",
                "--frames",
                "3",
                "--size",
                "320x180",
                "--preset",
                "potato",
                "--scale",
                "small",
                "--no-panel",
            ],
            None,
            &[0],
        ),
        spec(
            "lighting_plant",
            "genos-lighting",
            18776,
            &[
                "--headless",
                "--frames",
                "3",
                "--size",
                "320x180",
                "--preset",
                "potato",
                "--scene",
                "stress",
            ],
            None,
            &[0],
        ),
    ]
}

fn spec(
    label: &'static str,
    name: &'static str,
    port: u16,
    args: &'static [&'static str],
    script: Option<&'static str>,
    codes: &'static [i32],
) -> Spec {
    Spec {
        label,
        name,
        port,
        args,
        script,
        codes,
    }
}

fn gap_of(run: &Finished) -> Option<f64> {
    parse_quit_gap(&run.stdout).or_else(|| parse_quit_gap(&run.stderr))
}

fn total_of(run: &Finished) -> Option<f64> {
    parse_quit_total(&run.stdout).or_else(|| parse_quit_total(&run.stderr))
}

fn phases_of(run: &Finished) -> Vec<PhaseSample> {
    let mut phases = parse_quit_phases(&run.stdout);
    if phases.is_empty() {
        phases = parse_quit_phases(&run.stderr);
    }
    phases
}

fn must_gap(name: &str, run: &Finished) -> f64 {
    gap_of(run)
        .ok_or_else(|| {
            format!(
                "{name} printed no {QUIT_GAP_PREFIX}: stdout {} stderr {}",
                run.stdout, run.stderr
            )
        })
        .expect("quit gap line")
}

fn must_total(name: &str, run: &Finished) -> f64 {
    total_of(run)
        .ok_or_else(|| {
            format!(
                "{name} printed no {QUIT_TOTAL_PREFIX}: stdout {} stderr {}",
                run.stdout, run.stderr
            )
        })
        .expect("quit total line")
}

fn phase_ms(phases: &[PhaseSample], name: &str) -> f64 {
    phases
        .iter()
        .find(|phase| phase.name == name)
        .map(|phase| phase.ms)
        .unwrap_or(0.0)
}

fn print_phases(label: &str, gap: f64, total: f64, phases: &[PhaseSample]) {
    println!("MEASURE {label}_gap_ms={gap:.3} total_ms={total:.3}");
    for phase in phases {
        println!("MEASURE {label}_phase_{}={:.3}", phase.name, phase.ms);
    }
}

fn assert_clean(label: &str, gap: f64, total: f64, phases: &[PhaseSample], run: &Finished) {
    assert!(
        gap_within_budget(gap),
        "{label} quit gap {gap:.1} ms is outside the 100 ms ping budget"
    );
    assert!(
        phases_within_budget(phases, total),
        "{label} phase or total is outside the 100 ms budget, total {total:.1} phases {phases:?}"
    );
    for name in FULL_PHASES {
        assert!(
            phases.iter().any(|phase| phase.name == *name),
            "{label} missing phase {name}; phases {phases:?}\nstdout {}\nstderr {}",
            run.stdout,
            run.stderr
        );
    }
    assert!(
        !run.stdout.contains("lifecycle_drain_forced")
            && !run.stderr.contains("lifecycle_drain_forced"),
        "{label} forced the fence drain\nstdout {}\nstderr {}",
        run.stdout,
        run.stderr
    );
}

fn launch(seat: &Seat, spec: &Spec, block_ms: Option<&str>) -> Result<Finished, String> {
    let bin = binary(spec.name)?;
    let work = std::env::temp_dir().join(format!(
        "genos-quit-{}-{}-{}",
        spec.label,
        std::process::id(),
        block_ms.unwrap_or("clean")
    ));
    std::fs::create_dir_all(&work).map_err(|err| err.to_string())?;
    let mut command = Command::new(&bin);
    command
        .args(spec.args)
        .arg("--out")
        .arg(&work)
        .current_dir(&work)
        .env("XDG_RUNTIME_DIR", &seat.runtime)
        .env("WAYLAND_DISPLAY", &seat.socket)
        .env("VK_ICD_FILENAMES", ICD)
        .env("VK_DRIVER_FILES", ICD)
        .env("GENOS_MCP_PORT", spec.port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if spec.name != "genos-camera" {
        command.arg("--display").arg(&seat.socket);
    }
    if let Some(ms) = block_ms {
        command.arg("--lifecycle-block-ms").arg(ms);
    }
    if let Some(body) = spec.script {
        let path = work.join("loop.rhai");
        std::fs::write(&path, body).map_err(|err| err.to_string())?;
        command.arg("--script").arg(&path);
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("spawn {} ({}): {err}", spec.label, bin.display()))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_thread = std::thread::spawn(move || read_pipe(stdout));
    let stderr_thread = std::thread::spawn(move || read_pipe(stderr));
    if !wait_child(&mut child, LIMIT) {
        let _ = child.kill();
        let _ = child.wait();
        let stdout = join_text(stdout_thread);
        let stderr = join_text(stderr_thread);
        return Err(format!(
            "{} timed out after {}s\nstdout {stdout}\nstderr {stderr}",
            spec.label,
            LIMIT.as_secs()
        ));
    }
    let status = child.wait().map_err(|err| err.to_string())?;
    Ok(Finished {
        code: status.code(),
        stdout: join_text(stdout_thread),
        stderr: join_text(stderr_thread),
    })
}

fn binary(name: &str) -> Result<PathBuf, String> {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let target = option_env!("CARGO_TARGET_DIR").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"),
        PathBuf::from,
    );
    let path = target.join(profile).join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "missing {}. Build the workspace binaries before this test.",
            path.display()
        ))
    }
}

fn read_pipe(pipe: Option<impl Read>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut text = String::new();
    let _ = pipe.read_to_string(&mut text);
    text
}

fn join_text(handle: std::thread::JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}

fn wait_child(child: &mut Child, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    let pair = (Mutex::new(()), Condvar::new());
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(_) => return false,
        }
        let guard = pair
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _ = pair
            .1
            .wait_timeout(guard, remaining.min(Duration::from_millis(50)));
    }
    false
}

impl Seat {
    fn open(socket: &str) -> Result<Self, String> {
        let runtime =
            std::env::temp_dir().join(format!("genos-quit-runtime-{}", std::process::id()));
        std::fs::create_dir_all(&runtime).map_err(|err| err.to_string())?;
        if let Err(err) = std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))
        {
            let _ = std::fs::remove_dir_all(&runtime);
            return Err(err.to_string());
        }
        let log = runtime.join("weston.log");
        let mut child = match Command::new("weston")
            .args([
                "--backend=headless",
                "--renderer=pixman",
                "--no-config",
                "--socket",
                socket,
                "--width=640",
                "--height=480",
                "--log",
            ])
            .arg(&log)
            .env("XDG_RUNTIME_DIR", &runtime)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let _ = std::fs::remove_dir_all(&runtime);
                return Err(format!("weston: {err}"));
            }
        };
        let socket_path = runtime.join(socket);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !socket_path.exists() {
            let exited = child.try_wait().ok().flatten().is_some();
            if Instant::now() >= deadline || exited {
                return Err(reap_weston(&mut child, &runtime, &log, socket));
            }
            pause(Duration::from_millis(20));
        }
        Ok(Self {
            runtime,
            socket: socket.to_string(),
            child,
        })
    }
}

fn reap_weston(child: &mut Child, runtime: &Path, log: &Path, socket: &str) -> String {
    let _ = child.kill();
    let _ = child.wait();
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let _ = std::fs::remove_dir_all(runtime);
    format!("weston did not open {socket}: {text}")
}

impl Drop for Seat {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}

fn pause(duration: Duration) {
    let deadline = Instant::now() + duration;
    let pair = (Mutex::new(()), Condvar::new());
    while Instant::now() < deadline {
        let guard = pair
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _ = pair.1.wait_timeout(guard, remaining);
    }
}
