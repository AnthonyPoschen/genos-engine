//! Live frames for one check.
//!
//! The path trace is the reference. These frames are the candidate. Owner
//! mode waits until the picture settles, or 120 frames, then keeps a short
//! tail. Lavapipe-small keeps a short sequence and does not wait. The box
//! step is warmed on the pose before the jump, then frames 1 through 8 are
//! the frames after it.

use std::path::Path;
use std::time::{Duration, Instant};

use genos_lighting_ref::Picture;
use genos_render::{Renderer, World};
use genos_scene::Camera;
use genos_window::Window;

use super::scenes::{self, Shot};

pub struct Job {
    pub scene: String,
    pub checkpoint: String,
    pub still: bool,
    pub event: Option<&'static str>,
}

pub struct LiveShot {
    pub scene: String,
    pub checkpoint: String,
    pub frames: Vec<Picture>,
    pub settled: Option<Picture>,
    pub note: String,
}

pub struct CaptureOutcome {
    pub shots: Vec<LiveShot>,
    pub skipped: Vec<(String, String, String)>,
}

pub struct CaptureOptions {
    pub width: u32,
    pub height: u32,
    pub full_effort: bool,
    pub lavapipe_small: bool,
    pub jobs: Vec<Job>,
}

const OWNER_SETTLE_CAP: u32 = 120;
const OWNER_STILL_TAIL: u32 = 4;
const OWNER_WARM_CAP: u32 = 30;
const EVENT_FRAMES: u32 = 8;
const LAVAPIPE_BUDGET: Duration = Duration::from_secs(180);

pub fn capture(opts: &CaptureOptions) -> Result<CaptureOutcome, String> {
    if opts.lavapipe_small && std::env::var_os("VK_ICD_FILENAMES").is_none() {
        let icd = Path::new("/usr/share/vulkan/icd.d/lvp_icd.json");
        if icd.exists() {
            // The loader reads this when the device opens. Capture has not
            // spawned another thread yet.
            unsafe { std::env::set_var("VK_ICD_FILENAMES", icd) }
        }
    }
    let mut window = Window::open_proof(opts.width, opts.height)?;
    window.set_title("Genos lighting reference check");
    let frame = window.pump();
    let mut renderer = Renderer::open(window.display, window.surface, frame.width, frame.height)?;
    if renderer.width() != opts.width || renderer.height() != opts.height {
        return Err(format!(
            "proof window is {}x{}, the reference is {}x{}",
            renderer.width(),
            renderer.height(),
            opts.width,
            opts.height
        ));
    }
    let started = Instant::now();
    let mut shots = Vec::new();
    let mut skipped = Vec::new();
    for (index, job) in opts.jobs.iter().enumerate() {
        if opts.lavapipe_small && index > 0 && started.elapsed() > LAVAPIPE_BUDGET {
            skipped.push((
                job.scene.clone(),
                job.checkpoint.clone(),
                "lavapipe-small time budget; this checkpoint was not captured".into(),
            ));
            continue;
        }
        let Some(shot) = scenes::shot(&job.scene, &job.checkpoint) else {
            skipped.push((
                job.scene.clone(),
                job.checkpoint.clone(),
                "no camera for this checkpoint".into(),
            ));
            continue;
        };
        match capture_one(&mut window, &mut renderer, opts, job, &shot) {
            Ok(live) => shots.push(live),
            Err(err) => skipped.push((job.scene.clone(), job.checkpoint.clone(), err)),
        }
    }
    Ok(CaptureOutcome { shots, skipped })
}

fn capture_one(
    window: &mut Window,
    renderer: &mut Renderer,
    opts: &CaptureOptions,
    job: &Job,
    shot: &Shot,
) -> Result<LiveShot, String> {
    if job.event == Some("box-step") {
        if let Some(before) = scenes::shot("stress-moving", "box-step-before") {
            let (scene, camera) = scenes::scene_and_camera(&before, opts.full_effort);
            let world = World::from_scene(scene);
            warm(window, renderer, &world, &camera, opts.lavapipe_small)?;
        }
    }
    let (scene, camera) = scenes::scene_and_camera(shot, opts.full_effort);
    let world = World::from_scene(scene);
    let (frames, settled, note) = if job.event.is_some() {
        let frames = read_frames(window, renderer, &world, &camera, EVENT_FRAMES)?;
        if opts.lavapipe_small {
            (
                frames,
                None,
                "short sequence after the box step; lavapipe-small does not wait to settle".into(),
            )
        } else {
            let settled = settle_one(window, renderer, &world, &camera, OWNER_SETTLE_CAP)?;
            let note = if settled.is_some() {
                "frames 1, 2, 4, and 8 after the box step; the candidate is the settled picture"
                    .to_string()
            } else {
                format!(
                    "frames 1, 2, 4, and 8 after the box step; the picture had not settled after {OWNER_SETTLE_CAP} frames"
                )
            };
            (frames, settled, note)
        }
    } else if job.still && !opts.lavapipe_small {
        let settled_ok = draw_until_settled(window, renderer, &world, &camera, OWNER_SETTLE_CAP)?;
        let frames = read_frames(window, renderer, &world, &camera, OWNER_STILL_TAIL)?;
        let note = if settled_ok {
            "picture settled, then a short still tail".into()
        } else {
            format!("picture had not settled after {OWNER_SETTLE_CAP} frames; the tail is what was captured")
        };
        (frames, None, note)
    } else if job.still {
        let frames = read_frames(window, renderer, &world, &camera, 2)?;
        (
            frames,
            None,
            "two frames; lavapipe-small does not wait to settle".into(),
        )
    } else {
        let n = if opts.lavapipe_small { 1 } else { 2 };
        let frames = read_frames(window, renderer, &world, &camera, n)?;
        (frames, None, "one moment of the moving pose".into())
    };
    Ok(LiveShot {
        scene: job.scene.clone(),
        checkpoint: job.checkpoint.clone(),
        frames,
        settled,
        note,
    })
}

fn warm(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    lavapipe_small: bool,
) -> Result<(), String> {
    if lavapipe_small {
        let _ = draw(window, renderer, world, camera, false)?;
        return Ok(());
    }
    let _ = draw_until_settled(window, renderer, world, camera, OWNER_WARM_CAP)?;
    Ok(())
}

fn draw_until_settled(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    cap: u32,
) -> Result<bool, String> {
    for _ in 0..cap {
        let _ = draw(window, renderer, world, camera, false)?;
        if renderer.picture_settled() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn settle_one(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    cap: u32,
) -> Result<Option<Picture>, String> {
    for _ in 0..cap {
        if renderer.picture_settled() {
            return draw(window, renderer, world, camera, true);
        }
        let _ = draw(window, renderer, world, camera, false)?;
    }
    Ok(None)
}

fn read_frames(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    count: u32,
) -> Result<Vec<Picture>, String> {
    let mut frames = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let picture = draw(window, renderer, world, camera, true)?
            .ok_or_else(|| "readback returned no picture".to_string())?;
        frames.push(picture);
    }
    Ok(frames)
}

fn draw(
    window: &mut Window,
    renderer: &mut Renderer,
    world: &World,
    camera: &Camera,
    readback: bool,
) -> Result<Option<Picture>, String> {
    let started = Instant::now();
    let _ = window.pump();
    let pixels = renderer.draw(world, camera, readback)?;
    let picture = match pixels {
        Some(pixels) => Some(Picture::from_bgra(
            renderer.width(),
            renderer.height(),
            &pixels,
        )?),
        None => None,
    };
    eprintln!(
        "lighting-ref: frame {:.2} s{}",
        started.elapsed().as_secs_f64(),
        if readback { "" } else { " (no readback)" }
    );
    Ok(picture)
}
